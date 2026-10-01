//! Rounded fills drawn with GDI alone, so the tab strip and the sidebar need no Direct2D (which
//! the app loads only when a dialog opens). The body is plain `FillRect`; only the corner pixels
//! are computed, each one a blend of the shape color and the known color behind it.

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{HDC, SetPixelV};

use crate::catppuccin::blend;
use crate::window::design::metrics::scale;
use crate::window::palette::Palette;
use crate::window::panel::fill;

/// Which corners of a rect are rounded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Corners(u8);

impl Corners {
    pub(crate) const NONE: Corners = Corners(0);
    pub(crate) const TOP_LEFT: Corners = Corners(1);
    pub(crate) const TOP_RIGHT: Corners = Corners(2);
    pub(crate) const BOTTOM_LEFT: Corners = Corners(4);
    pub(crate) const BOTTOM_RIGHT: Corners = Corners(8);
    pub(crate) const TOP: Corners = Corners(3);
    pub(crate) const ALL: Corners = Corners(15);

    const fn has(self, corner: Corners) -> bool {
        self.0 & corner.0 != 0
    }
}

/// `at_96_dpi` scaled to `dpi`, or 0 in high contrast, which keeps square, unblended fills.
pub(crate) fn radius_for(palette: &Palette, at_96_dpi: i32, dpi: u32) -> i32 {
    if palette.high_contrast {
        0
    } else {
        scale(at_96_dpi, dpi)
    }
}

/// How much of the pixel (`x`, `y`) of a corner square lies inside the corner's arc, 0 to 255.
/// (0, 0) is the outermost pixel and the arc's center is `radius` pixels in from both edges. Each
/// pixel is sampled at 4x4 points. A radius of 0 has no corner, so it counts as fully inside.
pub(crate) fn coverage(x: i32, y: i32, radius: i32) -> u32 {
    if radius <= 0 {
        return 255;
    }
    let center = 8 * radius;
    let mut inside = 0;
    for j in 0..4 {
        for i in 0..4 {
            let dx = 8 * x + 2 * i + 1 - center;
            let dy = 8 * y + 2 * j + 1 - center;
            if dx * dx + dy * dy <= center * center {
                inside += 1;
            }
        }
    }
    (inside * 255 + 8) / 16
}

/// Fills `rect` with `color`, rounding the `corners` by `radius` pixels (clamped to half the
/// shorter side). `behind` is the flat color already under the shape, which the smoothed edge
/// pixels blend toward. An empty or inverted rect draws nothing.
pub(crate) unsafe fn fill_rounded(
    dc: HDC,
    rect: RECT,
    radius: i32,
    corners: Corners,
    color: u32,
    behind: u32,
) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }
    let radius = radius.min(width / 2).min(height / 2).max(0);
    if radius == 0 || corners == Corners::NONE {
        unsafe { fill(dc, rect, color) };
        return;
    }
    let inset = |corner: Corners| if corners.has(corner) { radius } else { 0 };
    unsafe {
        let band = |top, bottom, left_inset, right_inset| {
            fill(
                dc,
                RECT {
                    left: rect.left + left_inset,
                    top,
                    right: rect.right - right_inset,
                    bottom,
                },
                color,
            );
        };
        band(
            rect.top,
            rect.top + radius,
            inset(Corners::TOP_LEFT),
            inset(Corners::TOP_RIGHT),
        );
        band(rect.top + radius, rect.bottom - radius, 0, 0);
        band(
            rect.bottom - radius,
            rect.bottom,
            inset(Corners::BOTTOM_LEFT),
            inset(Corners::BOTTOM_RIGHT),
        );
        for (corner, flip_x, flip_y) in [
            (Corners::TOP_LEFT, false, false),
            (Corners::TOP_RIGHT, true, false),
            (Corners::BOTTOM_LEFT, false, true),
            (Corners::BOTTOM_RIGHT, true, true),
        ] {
            if !corners.has(corner) {
                continue;
            }
            for y in 0..radius {
                for x in 0..radius {
                    let covered = coverage(x, y, radius);
                    if covered == 0 {
                        continue;
                    }
                    let px = if flip_x {
                        rect.right - 1 - x
                    } else {
                        rect.left + x
                    };
                    let py = if flip_y {
                        rect.bottom - 1 - y
                    } else {
                        rect.top + y
                    };
                    SetPixelV(dc, px, py, blend(color, behind, covered));
                }
            }
        }
    }
}

/// Fills `rect` as a box with a one-pixel `border`: the outer rounded shape in the border color,
/// then the inner shape in `fill_color`, inset by one pixel with the radius reduced to match, so
/// the border keeps an even width around the corners. `behind` is the flat color under the box.
#[allow(
    dead_code,
    reason = "used by the palette and Search view in the next tasks"
)]
pub(crate) unsafe fn fill_bordered(
    dc: HDC,
    rect: RECT,
    radius: i32,
    fill_color: u32,
    border: u32,
    behind: u32,
) {
    let inner = RECT {
        left: rect.left + 1,
        top: rect.top + 1,
        right: rect.right - 1,
        bottom: rect.bottom - 1,
    };
    unsafe {
        fill_rounded(dc, rect, radius, Corners::ALL, border, behind);
        fill_rounded(
            dc,
            inner,
            (radius - 1).max(0),
            Corners::ALL,
            fill_color,
            border,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
        ReleaseDC, SelectObject,
    };

    #[test]
    fn the_outermost_corner_pixel_is_outside_and_the_inner_ones_are_inside() {
        // Break caught: an inverted arc that fills the corner and trims the middle.
        for radius in [3, 4, 8, 16] {
            assert_eq!(coverage(0, 0, radius), 0, "outer pixel at radius {radius}");
            assert_eq!(
                coverage(radius - 1, radius - 1, radius),
                255,
                "inner pixel at radius {radius}"
            );
        }
    }

    #[test]
    fn coverage_is_symmetric_across_the_diagonal_and_grows_toward_the_center() {
        // Break caught: a swapped x and y, or coverage falling off toward the middle.
        let radius = 8;
        for y in 0..radius {
            for x in 0..radius {
                assert_eq!(coverage(x, y, radius), coverage(y, x, radius), "({x}, {y})");
                if x + 1 < radius {
                    assert!(
                        coverage(x + 1, y, radius) >= coverage(x, y, radius),
                        "({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn a_zero_radius_has_full_coverage() {
        assert_eq!(coverage(0, 0, 0), 255);
    }

    #[test]
    fn a_tiny_radius_leaves_the_outer_pixel_partly_covered() {
        // At radius 2 the outermost pixel straddles the arc: 6 of its 16 samples are inside.
        assert_eq!(coverage(0, 0, 2), 96);
    }

    #[test]
    fn the_high_contrast_palette_gets_square_corners() {
        use crate::platform::theme::Theme;
        use crate::window::palette::Palette;
        let normal = Palette::for_theme(Theme::ALL[0], false);
        let high = Palette::for_theme(Theme::ALL[0], true);
        assert_eq!(radius_for(&normal, 8, 96), 8);
        assert_eq!(radius_for(&normal, 8, 192), 16);
        assert_eq!(radius_for(&high, 8, 96), 0);
    }

    const BEHIND: u32 = 0x0010_2030;
    const FILL: u32 = 0x00ee_ddcc;

    /// Paints into a 40x40 memory bitmap, then hands `read` a pixel reader.
    fn with_canvas(draw: impl FnOnce(HDC), read: impl FnOnce(&dyn Fn(i32, i32) -> u32)) {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 40, 40);
            let previous = SelectObject(dc, bitmap);
            crate::window::panel::fill(
                dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: 40,
                    bottom: 40,
                },
                BEHIND,
            );
            draw(dc);
            read(&|x, y| GetPixel(dc, x, y));
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }

    #[test]
    fn a_rounded_fill_covers_the_middle_and_leaves_the_far_corner_and_outside_alone() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
        };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 6, Corners::ALL, FILL, BEHIND) },
            |pixel| {
                assert_eq!(pixel(20, 20), FILL, "center");
                assert_eq!(pixel(10, 10), BEHIND, "outer corner");
                assert_eq!(pixel(29, 29), BEHIND, "opposite outer corner");
                assert_eq!(pixel(20, 10), FILL, "top edge, middle");
                assert_eq!(pixel(9, 20), BEHIND, "left of the rect");
                assert_eq!(pixel(30, 20), BEHIND, "right of the rect");
            },
        );
    }

    #[test]
    fn only_the_requested_corners_are_rounded() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
        };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 6, Corners::TOP, FILL, BEHIND) },
            |pixel| {
                assert_eq!(pixel(10, 10), BEHIND, "top-left is rounded");
                assert_eq!(pixel(29, 10), BEHIND, "top-right is rounded");
                assert_eq!(pixel(10, 29), FILL, "bottom-left stays square");
                assert_eq!(pixel(29, 29), FILL, "bottom-right stays square");
            },
        );
    }

    #[test]
    fn an_edge_pixel_is_a_blend_between_the_fill_and_the_background() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
        };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 8, Corners::ALL, FILL, BEHIND) },
            |pixel| {
                // (11, 12) lies on the arc of an 8px radius, so it is neither pure color.
                let edge = pixel(11, 12);
                assert_ne!(edge, FILL);
                assert_ne!(edge, BEHIND);
            },
        );
    }

    #[test]
    fn a_radius_larger_than_the_rect_clamps_and_stays_inside_it() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 16,
        };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 50, Corners::ALL, FILL, BEHIND) },
            |pixel| {
                assert_eq!(pixel(20, 12), FILL, "inside");
                assert_eq!(pixel(20, 9), BEHIND, "above");
                assert_eq!(pixel(20, 16), BEHIND, "below");
                assert_eq!(pixel(9, 12), BEHIND, "left");
                assert_eq!(pixel(30, 12), BEHIND, "right");
            },
        );
    }

    #[test]
    fn an_empty_or_inverted_rect_draws_nothing() {
        let inverted = RECT {
            left: 30,
            top: 30,
            right: 10,
            bottom: 10,
        };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, inverted, 4, Corners::ALL, FILL, BEHIND) },
            |pixel| assert_eq!(pixel(20, 20), BEHIND),
        );
    }

    #[test]
    fn a_zero_radius_is_a_plain_fill() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
        };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 0, Corners::ALL, FILL, BEHIND) },
            |pixel| assert_eq!(pixel(10, 10), FILL),
        );
    }

    const BORDER: u32 = 0x0000_8040;

    #[test]
    fn a_bordered_box_has_a_rounded_outside_a_border_and_a_fill() {
        // Break caught: a border that is not drawn, or one that stays square at the corner.
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
        };
        with_canvas(
            |dc| unsafe { fill_bordered(dc, rect, 6, FILL, BORDER, BEHIND) },
            |pixel| {
                assert_eq!(
                    pixel(10, 10),
                    BEHIND,
                    "outer corner stays the surface color"
                );
                assert_eq!(pixel(20, 10), BORDER, "top border");
                assert_eq!(pixel(10, 20), BORDER, "left border");
                assert_eq!(pixel(29, 20), BORDER, "right border");
                assert_eq!(pixel(20, 29), BORDER, "bottom border");
                assert_eq!(pixel(20, 11), FILL, "just inside the top border");
                assert_eq!(pixel(20, 20), FILL, "center");
                assert_eq!(pixel(9, 20), BEHIND, "outside the rect");
            },
        );
    }

    #[test]
    fn a_bordered_box_with_no_radius_is_two_square_fills() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 30,
            bottom: 30,
        };
        with_canvas(
            |dc| unsafe { fill_bordered(dc, rect, 0, FILL, BORDER, BEHIND) },
            |pixel| {
                assert_eq!(pixel(10, 10), BORDER, "corner is border");
                assert_eq!(pixel(11, 11), FILL, "just inside the corner");
            },
        );
    }

    #[test]
    fn a_bordered_box_too_small_for_a_fill_stays_inside_its_rect() {
        let rect = RECT {
            left: 10,
            top: 10,
            right: 12,
            bottom: 12,
        };
        with_canvas(
            |dc| unsafe { fill_bordered(dc, rect, 4, FILL, BORDER, BEHIND) },
            |pixel| {
                assert_eq!(pixel(9, 9), BEHIND);
                assert_eq!(pixel(12, 12), BEHIND);
            },
        );
    }
}
