//! Painting: background, checkerboard under the image's rectangle, the bitmap (linear below
//! 100%, nearest-neighbor above), a focus ring, and the loading, failed and SVG-error text.

use super::{DEFAULT_MAX_SIDE, ViewState, client_size, dpi_scale, focused, marked_shapes};
use crate::preview::fastdraw_hits::HitShape;
use crate::preview::render::{RectF, color_f, create_hwnd_target};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_BRUSH_PROPERTIES,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_BITMAP_PROPERTIES, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_EXTEND_MODE_WRAP, D2D1_ROUNDED_RECT,
    ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_CENTER, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::Error;
use windows_numerics::{Matrix3x2, Vector2};

const CHECKER_CELL: f32 = 8.0;
const TEXT_SIZE: f32 = 15.0;
const LINE_HEIGHT: f32 = 24.0;
const SVG_BAR_HEIGHT: f32 = 28.0;
pub(super) const SVG_BAND_TEXT: &str = "Can't render this SVG";

const BITMAP_PROPERTIES: D2D1_BITMAP_PROPERTIES = D2D1_BITMAP_PROPERTIES {
    pixelFormat: D2D1_PIXEL_FORMAT {
        format: DXGI_FORMAT_B8G8R8A8_UNORM,
        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
    },
    dpiX: 96.0,
    dpiY: 96.0,
};

/// A `FastPadError` from `create_hwnd_target` or `text_format`, as the COM error `WM_PAINT` expects.
fn com_error(error: crate::FastPadError) -> Error {
    match error {
        crate::FastPadError::Win32(code) => {
            Error::from_hresult(windows::core::HRESULT(code as i32))
        }
        _ => Error::from_hresult(windows::Win32::Foundation::E_FAIL),
    }
}

fn text_format(
    state: &ViewState,
    weight: DWRITE_FONT_WEIGHT,
) -> windows::core::Result<IDWriteTextFormat> {
    let format = state
        .graphics
        .text_format(
            crate::window::design::faces::current().text,
            TEXT_SIZE * dpi_scale(state.hwnd),
            weight,
            DWRITE_FONT_STYLE_NORMAL,
        )
        .map_err(com_error)?;
    unsafe {
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    }
    Ok(format)
}

fn draw_text(
    target: &ID2D1RenderTarget,
    text: &str,
    format: &IDWriteTextFormat,
    rect: RectF,
    color: u32,
) -> windows::core::Result<()> {
    let wide = text.encode_utf16().collect::<Vec<_>>();
    unsafe {
        let brush = target.CreateSolidColorBrush(&color_f(color), None)?;
        target.DrawText(
            &wide,
            format,
            &rect.to_d2d(),
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
    Ok(())
}

fn fill(target: &ID2D1RenderTarget, rect: RectF, color: u32) -> windows::core::Result<()> {
    unsafe {
        let brush = target.CreateSolidColorBrush(&color_f(color), None)?;
        target.FillRectangle(&rect.to_d2d(), &brush);
    }
    Ok(())
}

/// A 2×2 checker tile of the background and border colours, repeated every `cell` pixels.
fn ensure_checker(state: &mut ViewState, target: &ID2D1RenderTarget) -> windows::core::Result<()> {
    if state.checker.is_some() {
        return Ok(());
    }
    let pixel = |color: u32| {
        [
            ((color >> 16) & 0xFF) as u8,
            ((color >> 8) & 0xFF) as u8,
            (color & 0xFF) as u8,
            0xFF,
        ]
    };
    let (a, b) = (pixel(state.colors.background), pixel(state.colors.border));
    let tile = [a, b, b, a].concat();
    unsafe {
        let bitmap = target.CreateBitmap(
            D2D_SIZE_U {
                width: 2,
                height: 2,
            },
            Some(tile.as_ptr().cast()),
            8,
            &BITMAP_PROPERTIES,
        )?;
        let brush = target.CreateBitmapBrush(
            &bitmap,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_WRAP,
                extendModeY: D2D1_EXTEND_MODE_WRAP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            }),
            None,
        )?;
        let cell = CHECKER_CELL * dpi_scale(state.hwnd);
        brush.SetTransform(&Matrix3x2::scale(cell, cell));
        state.checker = Some(brush);
    }
    Ok(())
}

/// Draws the sharp tile of a deeply zoomed vector picture over its enlarged whole picture, placed
/// by the zoom it was rendered at (a tile from the previous zoom stands in until the next lands).
fn draw_tile(
    state: &mut ViewState,
    target: &ID2D1RenderTarget,
    dest: RectF,
) -> windows::core::Result<()> {
    let Some(tile) = &state.tile else {
        return Ok(());
    };
    let ratio = state.scale / tile.scale;
    let area = tile.area;
    let tile_dest = RectF::new(
        dest.left + area.x as f32 * ratio,
        dest.top + area.y as f32 * ratio,
        dest.left + (area.x + area.width) as f32 * ratio,
        dest.top + (area.y + area.height) as f32 * ratio,
    );
    if state.tile_bitmap.is_none() {
        state.tile_bitmap = Some(unsafe {
            target.CreateBitmap(
                D2D_SIZE_U {
                    width: area.width,
                    height: area.height,
                },
                Some(tile.pixels.as_ptr().cast()),
                area.width * 4,
                &BITMAP_PROPERTIES,
            )?
        });
    }
    // At its own zoom the tile is drawn pixel for pixel; a fractional offset must not blur it.
    let mode = if tile_dest.width() > area.width as f32 - 0.5 {
        D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
    } else {
        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
    };
    if let Some(bitmap) = &state.tile_bitmap {
        unsafe { target.DrawBitmap(bitmap, Some(&tile_dest.to_d2d()), 1.0, mode, None) };
    }
    Ok(())
}

/// Draws the picture's Direct2D scene over `dest`, clipped to the view, making the scene at the
/// first paint. A scene that cannot be made hands the picture back to resvg.
fn draw_vector(
    state: &mut ViewState,
    target: &ID2D1RenderTarget,
    dest: RectF,
    client: RectF,
) -> windows::core::Result<()> {
    if state.scene.is_none() {
        let tree = state.image.as_ref().and_then(|image| image.tree.clone());
        match tree.map(|tree| super::vector::Scene::build(&state.graphics.d2d, &tree)) {
            Some(Ok(scene)) => state.scene = Some(scene),
            _ => {
                state.gpu = false;
                super::start_svg_decode(state);
                return Ok(());
            }
        }
    }
    let Some(visible) = dest.intersect(&client) else {
        return Ok(());
    };
    let Some(scene) = &mut state.scene else {
        return Ok(());
    };
    unsafe { target.PushAxisAlignedClip(&visible.to_d2d(), D2D1_ANTIALIAS_MODE_ALIASED) };
    let drawn = scene.draw(target, (dest.left, dest.top), (dest.width(), dest.height()));
    unsafe { target.PopAxisAlignedClip() };
    if drawn.is_err() {
        state.scene = None;
        state.gpu = false;
        super::start_svg_decode(state);
    }
    Ok(())
}

/// How far an outline stands off its element, and how thick it is, in device-independent pixels.
const MARK_GAP: f32 = 2.0;
const MARK_WIDTH: f32 = 2.0;

/// Outlines the diagram elements on the marked line in the accent colour, over the picture.
fn draw_marks(
    state: &ViewState,
    target: &ID2D1RenderTarget,
    dpi: f32,
) -> windows::core::Result<()> {
    let Some(origin) = super::diagram_hits(state).map(|hits| hits.origin) else {
        return Ok(());
    };
    let mut shapes = marked_shapes(state).peekable();
    if shapes.peek().is_none() {
        return Ok(());
    }
    let brush = unsafe { target.CreateSolidColorBrush(&color_f(state.colors.find_match), None)? };
    let at = |point: (f64, f64)| {
        let (x, y) = super::zoom::from_scene(point, state.offset, state.scale, origin);
        Vector2 { X: x, Y: y }
    };
    let (gap, width) = (MARK_GAP * dpi, MARK_WIDTH * dpi);
    for shape in shapes {
        match shape {
            HitShape::Box { x, y, w, h } => {
                let (a, b) = (at((*x, *y)), at((x + w, y + h)));
                let rect = RectF::new(a.X - gap, a.Y - gap, b.X + gap, b.Y + gap);
                unsafe { target.DrawRectangle(&rect.to_d2d(), &brush, width, None) };
            }
            HitShape::Line(points) => {
                for pair in points.windows(2) {
                    unsafe { target.DrawLine(at(pair[0]), at(pair[1]), &brush, width, None) };
                }
            }
        }
    }
    Ok(())
}

/// The toolbar's icon size in device-independent pixels.
const ICON_SIZE: f32 = 14.0;

/// The diagram toolbar in the view's top-right corner: a rounded strip, the zoom mode in use shown
/// pressed, the hovered and pressed buttons tinted, and the percent as text.
fn draw_toolbar(
    state: &mut ViewState,
    target: &ID2D1RenderTarget,
    width: f32,
    dpi: f32,
) -> windows::core::Result<()> {
    use super::toolbar::{Button, glyph, layout, strip};
    if state.icon_format.is_none() {
        let format = state
            .graphics
            .text_format(
                crate::window::design::faces::current().icons,
                ICON_SIZE * dpi,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
            )
            .map_err(com_error)?;
        unsafe {
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        state.icon_format = Some(format);
    }
    if state.body_format.is_none() {
        state.body_format = Some(text_format(state, DWRITE_FONT_WEIGHT_NORMAL)?);
    }
    let colors = state.colors;
    let radius = 4.0 * dpi;
    let rounded = |rect: RectF| D2D1_ROUNDED_RECT {
        rect: rect.to_d2d(),
        radiusX: radius,
        radiusY: radius,
    };
    let back = strip(width, dpi);
    unsafe {
        let fill_brush = target.CreateSolidColorBrush(&color_f(colors.background), None)?;
        let border = target.CreateSolidColorBrush(&color_f(colors.border), None)?;
        target.FillRoundedRectangle(&rounded(back), &fill_brush);
        target.DrawRoundedRectangle(&rounded(back), &border, dpi, None);
    }
    for (button, rect) in layout(width, dpi) {
        let on = matches!(
            (button, state.zoom),
            (Button::Fit, super::Zoom::Fit) | (Button::Fill, super::Zoom::FillWidth)
        );
        let tint = if state.bar.pressed == Some(button) || on {
            Some(colors.selection)
        } else if state.bar.hovered == Some(button) {
            Some(colors.code_background)
        } else {
            None
        };
        if let Some(tint) = tint {
            unsafe {
                let brush = target.CreateSolidColorBrush(&color_f(tint), None)?;
                target.FillRoundedRectangle(&rounded(rect), &brush);
            }
        }
        let ink = if on {
            colors.selection_text
        } else {
            colors.text
        };
        let (text, format) = if button == Button::Percent {
            (
                format!("{}%", (state.scale * 100.0).round() as u32),
                state.body_format.as_ref(),
            )
        } else {
            (glyph(button).to_owned(), state.icon_format.as_ref())
        };
        if let Some(format) = format {
            draw_text(target, &text, format, rect, ink)?;
        }
    }
    Ok(())
}

pub(super) fn paint(state: &mut ViewState) -> windows::core::Result<()> {
    let hwnd = state.hwnd;
    let (width, height) = client_size(hwnd);
    if state.target.is_none() {
        let target = create_hwnd_target(
            &state.graphics,
            hwnd,
            width.max(1.0) as u32,
            height.max(1.0) as u32,
            96,
        )
        .map_err(com_error)?;
        state.max_side = unsafe { target.GetMaximumBitmapSize() }.clamp(1, DEFAULT_MAX_SIDE);
        state.bitmap = None;
        state.tile_bitmap = None;
        state.checker = None;
        state.target = Some(target);
    }
    let target: ID2D1RenderTarget = {
        use windows::core::Interface;
        state.target.as_ref().expect("created above").cast()?
    };
    let scale = dpi_scale(hwnd);
    let client = RectF::new(0.0, 0.0, width, height);
    unsafe {
        target.BeginDraw();
        target.SetTransform(&Matrix3x2::identity());
        target.Clear(Some(&color_f(state.colors.background)));
    }
    if let Some(image) = &state.image {
        let dest = RectF::new(
            state.offset.0,
            state.offset.1,
            state.offset.0 + image.natural_width as f32 * state.scale,
            state.offset.1 + image.natural_height as f32 * state.scale,
        );
        let (bitmap_width, bitmap_height, pixels_ptr) =
            (image.width, image.height, image.pixels.as_ptr());
        let vector = image.vector;
        let paper = image.diagram.as_ref().map(|facts| facts.paper);
        if let Some(paper) = paper
            && let Some(visible) = dest.intersect(&client)
        {
            // A diagram sits on its look's paper, which shows where the drawing is transparent.
            fill(&target, visible, paper)?;
        } else if !state.high_contrast
            && let Some(visible) = dest.intersect(&client)
        {
            ensure_checker(state, &target)?;
            if let Some(checker) = &state.checker {
                unsafe { target.FillRectangle(&visible.to_d2d(), checker) };
            }
        }
        if vector {
            draw_vector(state, &target, dest, client)?;
        } else if state.bitmap.is_none() {
            state.bitmap = Some(unsafe {
                target.CreateBitmap(
                    D2D_SIZE_U {
                        width: bitmap_width,
                        height: bitmap_height,
                    },
                    Some(pixels_ptr.cast()),
                    bitmap_width * 4,
                    &BITMAP_PROPERTIES,
                )?
            });
        }
        // A photo enlarged shows its pixels; a vector picture waiting for a sharper render is
        // better smooth than blocky.
        let mode = if dest.width() > bitmap_width as f32 + 0.5 && !super::shows_vector(state) {
            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR
        } else {
            D2D1_BITMAP_INTERPOLATION_MODE_LINEAR
        };
        if let Some(bitmap) = &state.bitmap {
            unsafe { target.DrawBitmap(bitmap, Some(&dest.to_d2d()), 1.0, mode, None) };
        }
        draw_tile(state, &target, dest)?;
        draw_marks(state, &target, scale)?;
    }
    let line = LINE_HEIGHT * scale;
    let middle = height / 2.0;
    if let Some(error) = &state.error {
        if state.title_format.is_none() {
            state.title_format = Some(text_format(state, DWRITE_FONT_WEIGHT_SEMI_BOLD)?);
        }
        if state.body_format.is_none() {
            state.body_format = Some(text_format(state, DWRITE_FONT_WEIGHT_NORMAL)?);
        }
        let (Some(title), Some(body)) = (&state.title_format, &state.body_format) else {
            unreachable!("created above");
        };
        draw_text(
            &target,
            error.title(),
            title,
            RectF::new(0.0, middle - line, width, middle),
            state.colors.text,
        )?;
        draw_text(
            &target,
            &error.message(),
            body,
            RectF::new(0.0, middle, width, middle + line),
            state.colors.muted,
        )?;
    } else if state.loading_visible && state.image.is_none() {
        if state.body_format.is_none() {
            state.body_format = Some(text_format(state, DWRITE_FONT_WEIGHT_NORMAL)?);
        }
        if let Some(body) = &state.body_format {
            draw_text(
                &target,
                "Loading\u{2026}",
                body,
                RectF::new(0.0, middle - line / 2.0, width, middle + line / 2.0),
                state.colors.muted,
            )?;
        }
    }
    if state.svg_error {
        if state.body_format.is_none() {
            state.body_format = Some(text_format(state, DWRITE_FONT_WEIGHT_NORMAL)?);
        }
        let band = RectF::new(0.0, 0.0, width, SVG_BAR_HEIGHT * scale);
        fill(&target, band, state.colors.code_background)?;
        if let Some(body) = &state.body_format {
            draw_text(
                &target,
                state.band.as_deref().unwrap_or(SVG_BAND_TEXT),
                body,
                band,
                state.colors.text,
            )?;
        }
    }
    if super::shows_toolbar(state) {
        draw_toolbar(state, &target, width, scale)?;
    }
    if focused(hwnd) {
        let ring = RectF::new(1.5, 1.5, width - 1.5, height - 1.5);
        unsafe {
            let brush = target.CreateSolidColorBrush(&color_f(state.colors.focus), None)?;
            target.DrawRectangle(&ring.to_d2d(), &brush, 1.0, None);
        }
    }
    unsafe { target.EndDraw(None, None) }
}
