//! The match case, whole word and regular expression toggles (note-search spec §4 and §8): three
//! square buttons painted inside the right end of a search field. The Search view and the find
//! bar share their geometry, painting, hit-testing, Alt keys and wording.

use crate::search::{MatchOptions, SearchOption};
use crate::window::palette::Palette;
use crate::window::panel::{fill, scale};
use crate::window::side_panel::draw_text;
use windows_sys::Win32::Foundation::{LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, GetTextMetricsW, HDC, HFONT, SelectObject,
    TEXTMETRICW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_SYSCHAR, WM_SYSKEYDOWN};

const SIZE_AT_96_DPI: i32 = 22;
const GAP_AT_96_DPI: i32 = 2;
const RIGHT_PADDING_AT_96_DPI: i32 = 3;
/// Bit 29 of a key message's `lParam`: Alt is down.
const ALT_DOWN: LPARAM = 1 << 29;

/// The three toggles inside `field`'s right end, in `SearchOption::ALL` order: 22 px squares with
/// 2 px gaps and 3 px of padding on the right at 96 DPI, centered vertically.
pub(crate) fn toggle_rects(field: RECT, dpi: u32) -> [RECT; 3] {
    let size = scale(SIZE_AT_96_DPI, dpi);
    let gap = scale(GAP_AT_96_DPI, dpi);
    let top = field.top + (field.bottom - field.top - size) / 2;
    let mut right = field.right - scale(RIGHT_PADDING_AT_96_DPI, dpi);
    let mut rects = [RECT::default(); 3];
    for rect in rects.iter_mut().rev() {
        *rect = RECT {
            left: right - size,
            top,
            right,
            bottom: top + size,
        };
        right -= size + gap;
    }
    rects
}

/// How much narrower than the field the `Edit` inside it must be: the toggles, their padding, and
/// one more gap between the text and the first toggle.
pub(crate) fn reserved_width(dpi: u32) -> i32 {
    3 * scale(SIZE_AT_96_DPI, dpi)
        + 3 * scale(GAP_AT_96_DPI, dpi)
        + scale(RIGHT_PADDING_AT_96_DPI, dpi)
}

fn glyph(option: SearchOption) -> &'static str {
    match option {
        SearchOption::Case => "Aa",
        SearchOption::WholeWord => "ab",
        SearchOption::Regex => ".*",
    }
}

/// Paints the toggles into `rects` (`toggle_rects`) in `font`. An option that is on is filled
/// with the color FastPad outlines focused fields with (`selection_background`). The hovered one
/// that is off is shaded. "ab" is underlined, as in VS Code.
pub(crate) fn paint(
    hdc: HDC,
    rects: &[RECT; 3],
    options: MatchOptions,
    hover: Option<SearchOption>,
    palette: &Palette,
    font: HFONT,
) {
    for (option, rect) in SearchOption::ALL.into_iter().zip(rects) {
        let color = if options.get(option) {
            unsafe { fill(hdc, *rect, palette.selection_background) };
            palette
                .selection_foreground
                .unwrap_or(palette.editor_foreground)
        } else if hover == Some(option) {
            unsafe { fill(hdc, *rect, palette.hover_background) };
            palette.hover_foreground
        } else {
            palette.muted_foreground
        };
        let width = unsafe {
            draw_text(
                hdc,
                glyph(option),
                *rect,
                font,
                color,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
            )
        };
        if option == SearchOption::WholeWord {
            unsafe { underline(hdc, *rect, width, font, color) };
        }
    }
}

/// A 1 px line under text `width` wide, centered in `rect` as `DT_CENTER | DT_VCENTER` drew it.
unsafe fn underline(hdc: HDC, rect: RECT, width: i32, font: HFONT, color: u32) {
    let mut metrics = TEXTMETRICW::default();
    let measured = unsafe {
        let previous = (!font.is_null()).then(|| SelectObject(hdc, font));
        let measured = GetTextMetricsW(hdc, &mut metrics) != 0;
        if let Some(previous) = previous {
            SelectObject(hdc, previous);
        }
        measured
    };
    if !measured || width <= 0 {
        return;
    }
    let top = rect.top + (rect.bottom - rect.top - metrics.tmHeight) / 2;
    let y = top + metrics.tmAscent + 1;
    let left = rect.left + (rect.right - rect.left - width) / 2;
    unsafe {
        fill(
            hdc,
            RECT {
                left,
                top: y,
                right: left + width,
                bottom: y + 1,
            },
            color,
        );
    }
}

/// The toggle under `point`, if any.
pub(crate) fn hit(rects: &[RECT; 3], point: POINT) -> Option<SearchOption> {
    SearchOption::ALL
        .into_iter()
        .zip(rects)
        .find(|(_, rect)| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        })
        .map(|(option, _)| option)
}

/// The toggle an Alt+`vk` flips: Alt+C, Alt+W and Alt+R.
pub(crate) fn alt_key(vk: u32) -> Option<SearchOption> {
    match char::from_u32(vk)? {
        'C' => Some(SearchOption::Case),
        'W' => Some(SearchOption::WholeWord),
        'R' => Some(SearchOption::Regex),
        _ => None,
    }
}

/// The toggle a key message flips: only a `WM_SYSKEYDOWN` for C, W or R with Alt held (bit 29
/// of `lparam`). F10 also arrives as `WM_SYSKEYDOWN`, without that bit.
pub(crate) fn alt_option(message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<SearchOption> {
    if message != WM_SYSKEYDOWN || lparam & ALT_DOWN == 0 {
        return None;
    }
    alt_key(u32::try_from(wparam).ok()?)
}

/// Whether a message is the `WM_SYSCHAR` that follows a toggle's Alt letter, which must be
/// swallowed: the menu band would beep, or open a menu with that mnemonic.
pub(crate) fn is_toggle_char(message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    message == WM_SYSCHAR
        && lparam & ALT_DOWN != 0
        && u32::try_from(wparam)
            .ok()
            .and_then(char::from_u32)
            .filter(char::is_ascii_alphabetic)
            .is_some_and(|letter| alt_key(u32::from(letter.to_ascii_uppercase())).is_some())
}

/// The toggle's name, as screen readers read it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "screen readers read it (Task 8)")
)]
pub(crate) fn label(option: SearchOption) -> &'static str {
    match option {
        SearchOption::Case => "Match case",
        SearchOption::WholeWord => "Match whole word",
        SearchOption::Regex => "Use regular expression",
    }
}

/// The toggle's tooltip: its name and its shortcut.
pub(crate) fn tooltip(option: SearchOption) -> &'static str {
    match option {
        SearchOption::Case => "Match case (Alt+C)",
        SearchOption::WholeWord => "Match whole word (Alt+W)",
        SearchOption::Regex => "Use regular expression (Alt+R)",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        alt_key, alt_option, hit, is_toggle_char, label, paint, reserved_width, toggle_rects,
        tooltip,
    };
    use crate::search::{MatchOptions, SearchOption};
    use crate::window::palette::Palette;
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_CHAR, WM_KEYDOWN, WM_SYSCHAR, WM_SYSKEYDOWN,
    };

    const FIELD: RECT = RECT {
        left: 8,
        top: 5,
        right: 252,
        bottom: 33,
    };

    const ALT_DOWN: isize = 1 << 29;

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    #[test]
    fn the_toggles_sit_inside_the_right_end_of_the_field_and_scale_with_dpi() {
        // Break caught: toggles drawn over the typed text or off the field's right edge, a box
        // that runs under them, or sizes that ignore a 150% monitor.
        let rects = toggle_rects(FIELD, 96);
        assert_eq!(edges(rects[0]), (179, 8, 201, 30));
        assert_eq!(edges(rects[1]), (203, 8, 225, 30));
        assert_eq!(edges(rects[2]), (227, 8, 249, 30));
        assert!(
            FIELD.right - reserved_width(96) < rects[0].left,
            "the box stops short"
        );
        let field = RECT {
            left: 12,
            top: 8,
            right: 378,
            bottom: 51,
        };
        let rects = toggle_rects(field, 144);
        assert_eq!(rects[2].right, 378 - 5);
        assert_eq!(rects[2].right - rects[2].left, 33);
        assert_eq!(rects[1].right, rects[2].left - 3);
        assert_eq!(
            rects[2].top - field.top,
            field.bottom - rects[2].bottom,
            "centered"
        );
        assert!(field.right - reserved_width(144) < rects[0].left);
    }

    #[test]
    fn a_point_hits_the_toggle_under_it_and_nothing_in_the_gaps() {
        let rects = toggle_rects(FIELD, 96);
        assert_eq!(
            hit(&rects, POINT { x: 179, y: 8 }),
            Some(SearchOption::Case)
        );
        assert_eq!(
            hit(&rects, POINT { x: 214, y: 19 }),
            Some(SearchOption::WholeWord)
        );
        assert_eq!(
            hit(&rects, POINT { x: 248, y: 29 }),
            Some(SearchOption::Regex)
        );
        assert_eq!(hit(&rects, POINT { x: 202, y: 19 }), None, "the gap");
        assert_eq!(
            hit(&rects, POINT { x: 249, y: 19 }),
            None,
            "the right padding"
        );
        assert_eq!(hit(&rects, POINT { x: 214, y: 30 }), None, "below");
        assert_eq!(hit(&rects, POINT { x: 100, y: 19 }), None, "over the text");
    }

    #[test]
    fn alt_c_w_and_r_are_the_three_options_and_nothing_else() {
        // Break caught: Alt+F or Alt+E (the menu band's File and Edit) taken by the Search box,
        // or a lowercase character code read as a key.
        assert_eq!(alt_key(u32::from(b'C')), Some(SearchOption::Case));
        assert_eq!(alt_key(u32::from(b'W')), Some(SearchOption::WholeWord));
        assert_eq!(alt_key(u32::from(b'R')), Some(SearchOption::Regex));
        for other in *b"FESVZcw" {
            assert_eq!(alt_key(u32::from(other)), None, "{}", char::from(other));
        }
    }

    #[test]
    fn only_an_alt_letter_key_down_flips_an_option_and_only_its_character_is_swallowed() {
        // Break caught: F10 (a WM_SYSKEYDOWN without Alt) or a plain C key flipping match case,
        // or the WM_SYSCHAR after Alt+C reaching the menu band (a beep or a menu opening).
        let c = usize::from(b'C');
        assert_eq!(
            alt_option(WM_SYSKEYDOWN, c, ALT_DOWN),
            Some(SearchOption::Case)
        );
        assert_eq!(
            alt_option(WM_SYSKEYDOWN, usize::from(b'R'), ALT_DOWN),
            Some(SearchOption::Regex)
        );
        assert_eq!(alt_option(WM_SYSKEYDOWN, c, 0), None, "no Alt: F10");
        assert_eq!(alt_option(WM_KEYDOWN, c, ALT_DOWN), None);
        assert_eq!(alt_option(WM_SYSKEYDOWN, usize::from(b'F'), ALT_DOWN), None);
        assert!(is_toggle_char(WM_SYSCHAR, usize::from(b'c'), ALT_DOWN));
        assert!(is_toggle_char(WM_SYSCHAR, usize::from(b'W'), ALT_DOWN));
        assert!(is_toggle_char(WM_SYSCHAR, usize::from(b'r'), ALT_DOWN));
        assert!(!is_toggle_char(WM_SYSCHAR, usize::from(b'f'), ALT_DOWN));
        assert!(!is_toggle_char(WM_SYSCHAR, usize::from(b'c'), 0));
        assert!(!is_toggle_char(WM_CHAR, usize::from(b'c'), ALT_DOWN));
    }

    #[test]
    fn labels_and_tooltips_name_each_option_and_its_shortcut() {
        assert_eq!(label(SearchOption::Case), "Match case");
        assert_eq!(label(SearchOption::WholeWord), "Match whole word");
        assert_eq!(label(SearchOption::Regex), "Use regular expression");
        assert_eq!(tooltip(SearchOption::Case), "Match case (Alt+C)");
        assert_eq!(tooltip(SearchOption::WholeWord), "Match whole word (Alt+W)");
        assert_eq!(
            tooltip(SearchOption::Regex),
            "Use regular expression (Alt+R)"
        );
    }

    #[test]
    fn an_on_toggle_is_filled_a_hovered_one_shaded_and_an_off_one_left_clear() {
        // Break caught: an option that is on looking the same as one that is off.
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
            ReleaseDC, SelectObject,
        };
        let palette = Palette {
            selection_background: 0x0000_00ff,
            hover_background: 0x0000_ff00,
            ..Palette::neutral()
        };
        let background = 0x0080_8080;
        let rects = toggle_rects(FIELD, 96);
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 260, 40);
            let previous = SelectObject(dc, bitmap);
            let all = RECT {
                left: 0,
                top: 0,
                right: 260,
                bottom: 40,
            };
            crate::window::panel::fill(dc, all, background);
            let options = MatchOptions {
                case: true,
                ..MatchOptions::default()
            };
            paint(
                dc,
                &rects,
                options,
                Some(SearchOption::WholeWord),
                &palette,
                std::ptr::null_mut(),
            );
            let corner = |rect: RECT| GetPixel(dc, rect.left + 1, rect.top + 1);
            assert_eq!(corner(rects[0]), palette.selection_background, "on");
            assert_eq!(corner(rects[1]), palette.hover_background, "hovered");
            assert_eq!(corner(rects[2]), background, "off");
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }
}
