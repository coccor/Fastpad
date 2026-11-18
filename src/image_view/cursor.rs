//! The magnifying-glass pointers over a picture: a click zooms in (a plus in the lens), Ctrl+click
//! zooms out (a minus). Windows has no stock magnifier, so it is drawn here, anti-aliased with
//! tiny-skia, at the cursor size for the view's DPI and about the size of the arrow: black strokes
//! with a soft white halo and a white lens, so it reads on any picture.

use resvg::tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
use std::cell::RefCell;
use std::collections::HashMap;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject,
};
use windows_sys::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, HCURSOR, ICONINFO, SM_CXCURSOR,
};

/// The drawing is laid out on the 32-pixel cursor cell of 96 DPI and scaled to the cursor size.
const GRID: f32 = 32.0;
/// On a whole pixel, so the 2-pixel plus and minus bars land on pixel edges and stay crisp.
const LENS: (f32, f32) = (9.0, 9.0);
const LENS_RADIUS: f32 = 6.0;
const RING_WIDTH: f32 = 1.6;
const HANDLE_FROM: (f32, f32) = (13.4, 13.4);
const HANDLE_TO: (f32, f32) = (18.3, 18.3);
const HANDLE_WIDTH: f32 = 2.6;
const SIGN_HALF_LENGTH: f32 = 3.0;
const SIGN_WIDTH: f32 = 2.0;
/// How far the white halo reaches past every black stroke.
const HALO: f32 = 1.1;

/// The magnifier at `size` × `size` as straight-alpha BGRA rows, top row first: a plus in the lens
/// when `plus`, else a minus. `None` only if tiny-skia cannot make the canvas.
pub(crate) fn magnifier_bgra(size: u32, plus: bool) -> Option<Vec<u8>> {
    let mut pixmap = Pixmap::new(size, size)?;
    let scale = size as f32 / GRID;
    let transform = Transform::from_scale(scale, scale);
    let black = paint(Color::from_rgba8(0, 0, 0, 255));
    let halo = paint(Color::from_rgba8(255, 255, 255, 235));
    let lens = paint(Color::from_rgba8(255, 255, 255, 255));

    let circle = PathBuilder::from_circle(LENS.0, LENS.1, LENS_RADIUS)?;
    let handle = line(HANDLE_FROM, HANDLE_TO)?;
    let mut sign = PathBuilder::new();
    sign.move_to(LENS.0 - SIGN_HALF_LENGTH, LENS.1);
    sign.line_to(LENS.0 + SIGN_HALF_LENGTH, LENS.1);
    if plus {
        sign.move_to(LENS.0, LENS.1 - SIGN_HALF_LENGTH);
        sign.line_to(LENS.0, LENS.1 + SIGN_HALF_LENGTH);
    }
    let sign = sign.finish()?;

    // The halo under everything, then the pale lens, then the black strokes.
    let halo_width = |width: f32| stroke(width + 2.0 * HALO, LineCap::Round);
    pixmap.stroke_path(&circle, &halo, &halo_width(RING_WIDTH), transform, None);
    pixmap.stroke_path(&handle, &halo, &halo_width(HANDLE_WIDTH), transform, None);
    pixmap.fill_path(&circle, &lens, FillRule::Winding, transform, None);
    let ring = stroke(RING_WIDTH, LineCap::Round);
    pixmap.stroke_path(&circle, &black, &ring, transform, None);
    let grip = stroke(HANDLE_WIDTH, LineCap::Round);
    pixmap.stroke_path(&handle, &black, &grip, transform, None);
    let bars = stroke(SIGN_WIDTH, LineCap::Butt);
    pixmap.stroke_path(&sign, &black, &bars, transform, None);

    Some(
        pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.blue(), color.green(), color.red(), color.alpha()]
            })
            .collect(),
    )
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn stroke(width: f32, line_cap: LineCap) -> Stroke {
    Stroke {
        width,
        line_cap,
        ..Stroke::default()
    }
}

fn line(from: (f32, f32), to: (f32, f32)) -> Option<resvg::tiny_skia::Path> {
    let mut path = PathBuilder::new();
    path.move_to(from.0, from.1);
    path.line_to(to.0, to.1);
    path.finish()
}

thread_local! {
    /// Every magnifier made, by size and sign; they live as long as the process.
    static CURSORS: RefCell<HashMap<(u32, bool), usize>> = RefCell::new(HashMap::new());
}

/// The magnifier pointer for `dpi`: zoom in when `plus`, zoom out otherwise. `None` when Windows
/// could not make it, so the caller keeps the arrow.
pub(crate) fn magnifier(dpi: u32, plus: bool) -> Option<HCURSOR> {
    let size = (unsafe { GetSystemMetricsForDpi(SM_CXCURSOR, dpi) }.max(16)) as u32;
    if let Some(cursor) = CURSORS.with(|cursors| cursors.borrow().get(&(size, plus)).copied()) {
        return Some(cursor as HCURSOR);
    }
    let pixels = magnifier_bgra(size, plus)?;
    let hotspot = (LENS.0 * size as f32 / GRID).round() as u32;
    let cursor = create_cursor(&pixels, size, hotspot)?;
    CURSORS.with(|cursors| cursors.borrow_mut().insert((size, plus), cursor as usize));
    Some(cursor)
}

/// A 32-bit alpha cursor from straight-alpha BGRA rows.
fn create_cursor(pixels: &[u8], size: u32, hotspot: u32) -> Option<HCURSOR> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            // Negative: rows run top to bottom, as `pixels` does.
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let color = unsafe {
        CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        )
    };
    if color.is_null() {
        return None;
    }
    if bits.is_null() {
        unsafe { DeleteObject(color) };
        return None;
    }
    // SAFETY: the section holds `size * size` 32-bit pixels, exactly `pixels.len()` bytes.
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len()) };
    // The alpha channel decides what shows; the mask must still exist, all zero.
    let mask_bits = vec![0_u8; size.div_ceil(16) as usize * 2 * size as usize];
    let mask = unsafe { CreateBitmap(size as i32, size as i32, 1, 1, mask_bits.as_ptr().cast()) };
    let cursor = if mask.is_null() {
        std::ptr::null_mut()
    } else {
        let info = ICONINFO {
            fIcon: 0,
            xHotspot: hotspot,
            yHotspot: hotspot,
            hbmMask: mask,
            hbmColor: color,
        };
        unsafe { CreateIconIndirect(&info) }
    };
    // The cursor keeps its own copies of both bitmaps.
    unsafe {
        DeleteObject(color);
        if !mask.is_null() {
            DeleteObject(mask);
        }
    }
    (!cursor.is_null()).then_some(cursor)
}

#[cfg(test)]
mod tests {
    use super::magnifier_bgra;

    /// The (blue, green, red, alpha) of pixel (`x`, `y`).
    fn at(pixels: &[u8], size: u32, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let index = ((y * size + x) * 4) as usize;
        (
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        )
    }

    #[test]
    fn the_plus_has_a_vertical_bar_and_the_minus_does_not() {
        // Break caught: Ctrl showing the same pointer as a plain hover.
        let plus = magnifier_bgra(32, true).unwrap();
        let minus = magnifier_bgra(32, false).unwrap();
        // The bars are two whole pixels wide (8 and 9) and solid, never a grey smear.
        for x in [8, 9] {
            assert_eq!(at(&plus, 32, x, 6), (0, 0, 0, 255));
            assert_eq!(at(&minus, 32, x, 6), (255, 255, 255, 255));
        }
        for y in [8, 9] {
            assert_eq!(at(&minus, 32, 6, y), (0, 0, 0, 255));
        }
        assert_eq!(at(&minus, 32, 6, 7), (255, 255, 255, 255));
    }

    #[test]
    fn the_glass_stays_about_the_size_of_the_arrow() {
        // Break caught: a pointer filling the whole cursor cell, as the first version did.
        for size in [32, 48] {
            let pixels = magnifier_bgra(size, true).unwrap();
            let reach = (0..size)
                .flat_map(|y| (0..size).map(move |x| (x, y)))
                .filter(|&(x, y)| at(&pixels, size, x, y).3 > 0)
                .map(|(x, y)| x.max(y))
                .max()
                .unwrap();
            assert!(reach * 32 <= 21 * size, "reach {reach} at {size}");
        }
    }

    #[test]
    fn the_handle_is_black_and_edges_are_anti_aliased() {
        // Break caught: a missing handle, or hard two-tone edges with no partial alpha.
        let pixels = magnifier_bgra(32, true).unwrap();
        assert_eq!(at(&pixels, 32, 15, 15), (0, 0, 0, 255));
        assert!(pixels.chunks(4).any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    }
}
