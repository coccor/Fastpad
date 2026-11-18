//! The magnifying-glass pointers over a picture: a click zooms in (a plus in the lens), Ctrl+click
//! zooms out (a minus). Drawn in code at the cursor size for the view's DPI as black strokes with
//! a white outline and a white lens, so they read on any picture. Windows has no stock magnifier.

use std::cell::RefCell;
use std::collections::HashMap;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateCursor, HCURSOR, SM_CXCURSOR};

/// What one cursor pixel shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pixel {
    Clear,
    Black,
    White,
}

/// The drawing is laid out on a 32-pixel grid and scaled to the cursor size.
const GRID: f32 = 32.0;
const LENS: (f32, f32) = (12.0, 12.0);
const LENS_INNER: f32 = 7.0;
const LENS_OUTER: f32 = 9.5;
const OUTLINE: f32 = 1.25;
const HANDLE_FROM: (f32, f32) = (18.5, 18.5);
const HANDLE_TO: (f32, f32) = (27.5, 27.5);
const HANDLE_HALF_WIDTH: f32 = 1.9;
const SIGN_HALF_LENGTH: f32 = 4.0;
const SIGN_HALF_WIDTH: f32 = 1.0;

/// The magnifier's pixels at `size` × `size`, row by row: a plus in the lens when `plus`, else a
/// minus.
pub(crate) fn magnifier_pixels(size: u32, plus: bool) -> Vec<Pixel> {
    let scale = size as f32 / GRID;
    let mut pixels = Vec::with_capacity((size * size) as usize);
    for row in 0..size {
        for column in 0..size {
            let x = (column as f32 + 0.5) / scale;
            let y = (row as f32 + 0.5) / scale;
            pixels.push(pixel_at(x, y, plus));
        }
    }
    pixels
}

fn pixel_at(x: f32, y: f32, plus: bool) -> Pixel {
    let lens = (x - LENS.0).hypot(y - LENS.1);
    let handle = distance_to_segment((x, y), HANDLE_FROM, HANDLE_TO);
    let (dx, dy) = ((x - LENS.0).abs(), (y - LENS.1).abs());
    let across = dx <= SIGN_HALF_LENGTH && dy <= SIGN_HALF_WIDTH;
    let down = plus && dy <= SIGN_HALF_LENGTH && dx <= SIGN_HALF_WIDTH;
    let ring = (LENS_INNER..=LENS_OUTER).contains(&lens);
    if ring || handle <= HANDLE_HALF_WIDTH || across || down {
        Pixel::Black
    } else if lens <= LENS_OUTER + OUTLINE || handle <= HANDLE_HALF_WIDTH + OUTLINE {
        Pixel::White
    } else {
        Pixel::Clear
    }
}

fn distance_to_segment(point: (f32, f32), from: (f32, f32), to: (f32, f32)) -> f32 {
    let (vx, vy) = (to.0 - from.0, to.1 - from.1);
    let along =
        (((point.0 - from.0) * vx + (point.1 - from.1) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    (point.0 - (from.0 + vx * along)).hypot(point.1 - (from.1 + vy * along))
}

/// The AND and XOR planes `CreateCursor` takes: rows of bits, most significant first, each row
/// padded to a 16-bit boundary. AND 1 keeps the screen; XOR 1 over AND 0 is white.
pub(crate) fn cursor_planes(pixels: &[Pixel], size: u32) -> (Vec<u8>, Vec<u8>) {
    let stride = size.div_ceil(16) as usize * 2;
    let mut and = vec![0xFF_u8; stride * size as usize];
    let mut xor = vec![0_u8; stride * size as usize];
    for (index, pixel) in pixels.iter().enumerate() {
        let (row, column) = (index / size as usize, index % size as usize);
        let byte = row * stride + column / 8;
        let bit = 0x80 >> (column % 8);
        match pixel {
            Pixel::Clear => {}
            Pixel::Black => and[byte] &= !bit,
            Pixel::White => {
                and[byte] &= !bit;
                xor[byte] |= bit;
            }
        }
    }
    (and, xor)
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
    let (and, xor) = cursor_planes(&magnifier_pixels(size, plus), size);
    let hotspot = (LENS.0 * size as f32 / GRID).round() as i32;
    let cursor = unsafe {
        CreateCursor(
            GetModuleHandleW(std::ptr::null()),
            hotspot,
            hotspot,
            size as i32,
            size as i32,
            and.as_ptr().cast(),
            xor.as_ptr().cast(),
        )
    };
    if cursor.is_null() {
        return None;
    }
    CURSORS.with(|cursors| cursors.borrow_mut().insert((size, plus), cursor as usize));
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::{Pixel, cursor_planes, magnifier_pixels};

    fn at(pixels: &[Pixel], size: u32, x: u32, y: u32) -> Pixel {
        pixels[(y * size + x) as usize]
    }

    #[test]
    fn the_plus_has_a_vertical_bar_and_the_minus_does_not() {
        // Break caught: Ctrl showing the same pointer as a plain hover.
        let plus = magnifier_pixels(32, true);
        let minus = magnifier_pixels(32, false);
        assert_eq!(at(&plus, 32, 12, 9), Pixel::Black);
        assert_eq!(at(&minus, 32, 12, 9), Pixel::White);
        assert_eq!(at(&plus, 32, 9, 12), Pixel::Black);
        assert_eq!(at(&minus, 32, 9, 12), Pixel::Black);
    }

    #[test]
    fn the_corners_stay_clear_and_the_handle_reaches_the_bottom_right() {
        // Break caught: an opaque square cursor hiding the picture, or the handle missing.
        let pixels = magnifier_pixels(48, true);
        assert_eq!(at(&pixels, 48, 0, 47), Pixel::Clear);
        assert_eq!(at(&pixels, 48, 47, 0), Pixel::Clear);
        assert_eq!(at(&pixels, 48, 38, 38), Pixel::Black);
    }

    #[test]
    fn the_planes_pad_rows_and_mark_white_in_xor_only_over_a_cleared_and() {
        // Break caught: rows packed without padding (a sheared cursor) or white drawn inverted.
        let pixels = [Pixel::White, Pixel::Black, Pixel::Clear, Pixel::Clear]
            .into_iter()
            .cycle()
            .take(24 * 24)
            .collect::<Vec<_>>();
        let (and, xor) = cursor_planes(&pixels, 24);
        assert_eq!(and.len(), 4 * 24);
        assert_eq!(and[0], 0b0011_0011);
        assert_eq!(xor[0], 0b1000_1000);
        assert_eq!(and[3], 0xFF);
    }
}
