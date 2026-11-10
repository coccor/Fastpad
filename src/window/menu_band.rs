//! The Alt/F10 menu: a painted column of File/Edit/Search/View/Help headings stacked in the
//! window's top-left corner, under the title strip. It replaces a native menu bar, which Windows
//! would draw unthemed over the reclaimed caption. The column exists only while menu mode is
//! active and floats over the window in its own popup (`main_window::menu_overlay`), so nothing
//! under it moves; each heading opens its native dropdown from `menus::MenuBar` to its right.

use crate::platform::wide_null;
use crate::window::design::metrics::scale;
use crate::window::design::text_scale::scale_text;
use crate::window::palette::Palette;
use crate::window::panel::fill;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DrawTextW, GetDC, HDC, HFONT, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

/// Headings in dropdown order; `&` marks each mnemonic, as in a native menu bar.
pub(crate) const MENU_TITLES: [&str; 5] = ["&File", "&Edit", "&Search", "&View", "&Help"];
/// The index of the View heading, whose dropdown holds the Markdown preview commands.
pub(crate) const FILE_MENU_INDEX: usize = 0;
pub(crate) const VIEW_MENU_INDEX: usize = 3;
/// The index of the Help heading, whose dropdown holds About.
pub(crate) const HELP_MENU_INDEX: usize = 4;

const ROW_HEIGHT_AT_96_DPI: i32 = 28;
const HEADING_PADDING_AT_96_DPI: i32 = 12;
/// The column's inset around its rows, inside the 1 px border.
const COLUMN_INSET_AT_96_DPI: i32 = 4;
const MIN_COLUMN_WIDTH_AT_96_DPI: i32 = 120;
/// The column's gap from the window's left edge.
pub(crate) const COLUMN_MARGIN_AT_96_DPI: i32 = 4;
const BORDER: i32 = 1;

/// Menu mode: which heading is highlighted, and whether its dropdown is open.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MenuMode {
    pub hot: usize,
    pub open: bool,
}

pub(crate) fn row_height(dpi: u32) -> i32 {
    scale_text(ROW_HEIGHT_AT_96_DPI, dpi)
}

fn inset(dpi: u32) -> i32 {
    BORDER + scale(COLUMN_INSET_AT_96_DPI, dpi)
}

/// Heading rectangles for label widths `widths`, stacked top to bottom in a column whose outer
/// top-left corner is (`left`, `top`). Every row is as wide as the widest label needs.
pub(crate) fn heading_rects(widths: &[i32], left: i32, top: i32, dpi: u32) -> Vec<RECT> {
    let padding = scale(HEADING_PADDING_AT_96_DPI, dpi);
    let inset = inset(dpi);
    let widest = widths.iter().copied().max().unwrap_or(0) + 2 * padding;
    let width = widest.max(scale(MIN_COLUMN_WIDTH_AT_96_DPI, dpi) - 2 * inset);
    let height = row_height(dpi);
    let left = left + inset;
    (0..widths.len() as i32)
        .map(|row| RECT {
            left,
            top: top + inset + row * height,
            right: left + width,
            bottom: top + inset + (row + 1) * height,
        })
        .collect()
}

/// The whole column around `headings`: its border and inset included.
pub(crate) fn column_rect(headings: &[RECT], dpi: u32) -> Option<RECT> {
    let (first, last) = (headings.first()?, headings.last()?);
    let inset = inset(dpi);
    Some(RECT {
        left: first.left - inset,
        top: first.top - inset,
        right: first.right + inset,
        bottom: last.bottom + inset,
    })
}

pub(crate) fn heading_at(headings: &[RECT], x: i32, y: i32) -> Option<usize> {
    headings
        .iter()
        .position(|rect| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
}

/// The heading whose `&` mnemonic is `key` (an uppercase virtual-key letter or a typed character).
pub(crate) fn mnemonic_heading(key: u32) -> Option<usize> {
    let key = char::from_u32(key)?.to_ascii_uppercase();
    MENU_TITLES.iter().position(|title| {
        title
            .split_once('&')
            .and_then(|(_, rest)| rest.chars().next())
            .is_some_and(|mnemonic| mnemonic.to_ascii_uppercase() == key)
    })
}

/// The heading beside `current`, wrapping around both ends.
pub(crate) const fn neighbor(current: usize, forward: bool) -> usize {
    let count = MENU_TITLES.len();
    if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
}

/// Measures each title in `font` for `heading_rects`.
pub(crate) fn measure_titles(hwnd: HWND, font: HFONT) -> Vec<i32> {
    unsafe {
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return vec![0; MENU_TITLES.len()];
        }
        let previous = (!font.is_null()).then(|| SelectObject(dc, font as _));
        let widths = MENU_TITLES
            .iter()
            .map(|title| {
                let mut text = wide_null(title);
                let mut rect = RECT::default();
                DrawTextW(
                    dc,
                    text.as_mut_ptr(),
                    -1,
                    &mut rect,
                    DT_CALCRECT | DT_SINGLELINE,
                );
                rect.right - rect.left
            })
            .collect();
        if let Some(previous) = previous {
            SelectObject(dc, previous);
        }
        ReleaseDC(hwnd, dc);
        widths
    }
}

/// Paints the column into `dc`, whose origin is the column's top-left corner, with `mode.hot`
/// highlighted (pressed while its dropdown is open). `headings` are in the coordinates
/// `column_rect` was computed in.
pub(crate) unsafe fn paint(
    dc: HDC,
    headings: &[RECT],
    mode: MenuMode,
    palette: Palette,
    font: HFONT,
    dpi: u32,
) {
    let Some(column) = column_rect(headings, dpi) else {
        return;
    };
    let local = |rect: RECT| RECT {
        left: rect.left - column.left,
        top: rect.top - column.top,
        right: rect.right - column.left,
        bottom: rect.bottom - column.top,
    };
    let outer = local(column);
    let border = if palette.high_contrast {
        palette.strip_foreground
    } else {
        palette.stroke
    };
    let padding = scale(HEADING_PADDING_AT_96_DPI, dpi);
    unsafe {
        fill(dc, outer, border);
        fill(
            dc,
            RECT {
                left: outer.left + BORDER,
                top: outer.top + BORDER,
                right: outer.right - BORDER,
                bottom: outer.bottom - BORDER,
            },
            palette.strip_background,
        );
        SetBkMode(dc, TRANSPARENT as i32);
        let previous = (!font.is_null()).then(|| SelectObject(dc, font as _));
        for (index, (title, rect)) in MENU_TITLES.iter().zip(headings).enumerate() {
            let rect = local(*rect);
            let foreground = if index == mode.hot {
                let background = if mode.open {
                    palette.pressed_background
                } else {
                    palette.hover_background
                };
                fill(dc, rect, background);
                palette.hover_foreground
            } else {
                palette.strip_foreground
            };
            SetTextColor(dc, foreground);
            let mut text = wide_null(title);
            let mut text_rect = RECT {
                left: rect.left + padding,
                right: rect.right - padding,
                ..rect
            };
            // No DT_NOPREFIX: the mnemonic letters are underlined, as keyboard menu mode shows them.
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                -1,
                &mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }
        if let Some(previous) = previous {
            SelectObject(dc, previous);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_follow_the_text_size() {
        let _factor = crate::window::design::text_scale::FactorGuard::new();
        // Break caught: menu headings clipped in a 28 px row at a larger Windows text size.
        use crate::window::design::text_scale::set_factor_for_test;
        set_factor_for_test(100);
        assert_eq!(row_height(96), 28);
        set_factor_for_test(225);
        let height = row_height(96);
        set_factor_for_test(100);
        assert_eq!(height, 63);
    }

    #[test]
    fn headings_stack_in_one_column_from_the_corner() {
        // Break caught: the headings laid out side by side in a band under the tabs again.
        let headings = heading_rects(&[20, 30, 40, 25], 4, 40, 96);
        assert_eq!(headings.len(), 4);
        assert_eq!(headings[0].left, 4 + 1 + 4);
        assert_eq!(headings[0].top, 40 + 1 + 4);
        for pair in headings.windows(2) {
            assert_eq!(pair[0].bottom, pair[1].top);
            assert_eq!(pair[0].left, pair[1].left);
            assert_eq!(pair[0].right, pair[1].right);
        }
        assert!(
            headings
                .iter()
                .all(|rect| rect.bottom - rect.top == row_height(96))
        );
        // Every row is as wide as the widest label with its padding, and never too narrow.
        assert_eq!(headings[0].right - headings[0].left, 120 - 2 * 5);
        let wide = heading_rects(&[200], 0, 0, 96);
        assert_eq!(wide[0].right - wide[0].left, 200 + 24);
        let column = column_rect(&headings, 96).unwrap();
        assert_eq!((column.left, column.top), (4, 40));
        assert_eq!(column.bottom, headings[3].bottom + 5);
        assert!(column_rect(&[], 96).is_none());
    }

    #[test]
    fn hit_testing_finds_the_heading_under_the_pointer() {
        let headings = heading_rects(&[20, 30, 40, 25], 0, 32, 96);
        assert_eq!(
            heading_at(&headings, headings[0].left, headings[0].top),
            Some(0)
        );
        assert_eq!(
            heading_at(&headings, headings[1].left + 3, headings[1].top),
            Some(1)
        );
        assert_eq!(
            heading_at(&headings, headings[0].left, headings[0].top - 1),
            None
        );
        assert_eq!(
            heading_at(&headings, headings[3].left, headings[3].bottom),
            None
        );
        assert_eq!(
            heading_at(&headings, headings[3].right, headings[3].top),
            None
        );
    }

    #[test]
    fn mnemonics_and_arrow_navigation_address_the_headings() {
        assert_eq!(mnemonic_heading(u32::from(b'F')), Some(0));
        assert_eq!(mnemonic_heading(u32::from(b'e')), Some(1));
        assert_eq!(mnemonic_heading(u32::from(b'S')), Some(2));
        assert_eq!(mnemonic_heading(u32::from(b'V')), Some(3));
        assert_eq!(mnemonic_heading(u32::from(b'h')), Some(4));
        assert_eq!(mnemonic_heading(u32::from(b'Q')), None);
        assert_eq!(neighbor(0, false), 4);
        assert_eq!(neighbor(4, true), 0);
        assert_eq!(neighbor(1, true), 2);
    }
}
