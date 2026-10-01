//! The themed prompt that replaces `MessageBoxW` for the close and confirm questions: an owned
//! popup in the theme's colors with action-named buttons, running its own modal loop like
//! `about.rs`. This file's first half is pure layout and key logic.
#![allow(dead_code, reason = "used by modal.rs from Task 4")]

use super::design::metrics::scale;
use super::design::text_scale::scale_text;
use super::soft_paint::{TITLE_CLOSE_WIDTH_AT_96_DPI, TITLE_HEIGHT_AT_96_DPI};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE};

const WIDTH_AT_96_DPI: i32 = 400;
const PADDING_AT_96_DPI: i32 = 20;
const TITLE_PAD_AT_96_DPI: i32 = 10;
const MESSAGE_GAP_AT_96_DPI: i32 = 12;
const FOOTER_GAP_AT_96_DPI: i32 = 20;
const FOOTER_HEIGHT_AT_96_DPI: i32 = 56;
const BUTTON_MIN_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;
const BUTTON_PAD_AT_96_DPI: i32 = 16;
const BUTTON_GAP_AT_96_DPI: i32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    Button(usize),
    TitleClose,
}

fn button_widths(dpi: u32, label_widths: &[i32]) -> Vec<i32> {
    label_widths
        .iter()
        .map(|width| {
            scale(BUTTON_MIN_WIDTH_AT_96_DPI, dpi).max(width + 2 * scale(BUTTON_PAD_AT_96_DPI, dpi))
        })
        .collect()
}

fn buttons_total(dpi: u32, widths: &[i32]) -> i32 {
    widths.iter().sum::<i32>() + scale(BUTTON_GAP_AT_96_DPI, dpi) * (widths.len() as i32 - 1).max(0)
}

/// The dialog's width: 400px at 96 DPI, or wider when the buttons need more.
pub(crate) fn dialog_width(dpi: u32, label_widths: &[i32]) -> i32 {
    let needed =
        buttons_total(dpi, &button_widths(dpi, label_widths)) + 2 * scale(PADDING_AT_96_DPI, dpi);
    scale(WIDTH_AT_96_DPI, dpi).max(needed)
}

/// The width the message wraps to.
pub(crate) fn content_width(dpi: u32, label_widths: &[i32]) -> i32 {
    dialog_width(dpi, label_widths) - 2 * scale(PADDING_AT_96_DPI, dpi)
}

#[derive(Clone)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title_band: RECT,
    pub title: RECT,
    pub title_close: RECT,
    pub message: RECT,
    pub footer: RECT,
    pub buttons: Vec<RECT>,
    pub(crate) dpi: u32,
}

impl Layout {
    /// `width` is `dialog_width`; `title_height` and `message_height` are the measured text heights.
    pub(crate) fn calculate(
        dpi: u32,
        width: i32,
        title_height: i32,
        message_height: i32,
        label_widths: &[i32],
    ) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let band_height = scale_text(TITLE_HEIGHT_AT_96_DPI, dpi)
            .max(title_height + 2 * scale(TITLE_PAD_AT_96_DPI, dpi));
        let title_band = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: band_height,
        };
        let title_close = RECT {
            left: width - scale(TITLE_CLOSE_WIDTH_AT_96_DPI, dpi),
            top: 0,
            right: width,
            bottom: scale_text(TITLE_HEIGHT_AT_96_DPI, dpi).min(band_height),
        };
        let title_top = (band_height - title_height) / 2;
        let title = RECT {
            left: padding,
            top: title_top,
            right: title_close.left,
            bottom: title_top + title_height,
        };
        let message_top = band_height + scale(MESSAGE_GAP_AT_96_DPI, dpi);
        let message = RECT {
            left: padding,
            top: message_top,
            right: width - padding,
            bottom: message_top + message_height,
        };
        let footer_top = message.bottom + scale(FOOTER_GAP_AT_96_DPI, dpi);
        let footer = RECT {
            left: 0,
            top: footer_top,
            right: width,
            bottom: footer_top + scale_text(FOOTER_HEIGHT_AT_96_DPI, dpi),
        };
        let button_height = scale_text(BUTTON_HEIGHT_AT_96_DPI, dpi);
        let button_top = footer.top + (footer.bottom - footer.top - button_height) / 2;
        let gap = scale(BUTTON_GAP_AT_96_DPI, dpi);
        let mut right = width - padding;
        let mut buttons: Vec<RECT> = button_widths(dpi, label_widths)
            .into_iter()
            .rev()
            .map(|button_width| {
                let rect = RECT {
                    left: right - button_width,
                    top: button_top,
                    right,
                    bottom: button_top + button_height,
                };
                right = rect.left - gap;
                rect
            })
            .collect();
        buttons.reverse();
        Self {
            width,
            height: footer.bottom,
            title_band,
            title,
            title_close,
            message,
            footer,
            buttons,
            dpi,
        }
    }

    pub(crate) fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.title_close) {
            return Some(Target::TitleClose);
        }
        self.buttons.iter().position(inside).map(Target::Button)
    }
}

/// The focus after Tab (`forward`) or Shift+Tab, wrapping.
pub(crate) fn next_focus(current: usize, count: usize, forward: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
}

/// The button a key chooses, if any: Esc is the last (Cancel), Enter and Space the focused one,
/// and `quick` maps letters to buttons.
pub(crate) fn key_choice(
    key: u16,
    focus: usize,
    count: usize,
    quick: &[(u16, usize)],
) -> Option<usize> {
    match key {
        VK_ESCAPE => count.checked_sub(1),
        VK_RETURN | VK_SPACE => (count > 0).then(|| focus.min(count - 1)),
        _ => quick
            .iter()
            .find(|(letter, index)| *letter == key && *index < count)
            .map(|(_, index)| *index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE};

    fn inside(outer: RECT, inner: RECT) -> bool {
        inner.left >= outer.left
            && inner.top >= outer.top
            && inner.right <= outer.right
            && inner.bottom <= outer.bottom
    }

    #[test]
    fn everything_fits_at_every_dpi_and_text_size() {
        // Break caught: a long message or a long button label clipping or overlapping at 192 DPI
        // and 225 % text.
        let _factor = crate::window::design::text_scale::FactorGuard::new();
        for factor in [100, 225] {
            crate::window::design::text_scale::set_factor_for_test(factor);
            for dpi in [96, 120, 144, 192] {
                for labels in [vec![88, 60], vec![60, 120, 60], vec![420, 70]] {
                    let width = dialog_width(dpi, &labels);
                    let layout =
                        Layout::calculate(dpi, width, 24 * factor as i32 / 100, 300, &labels);
                    let client = RECT {
                        left: 0,
                        top: 0,
                        right: layout.width,
                        bottom: layout.height,
                    };
                    for rect in [
                        layout.title_band,
                        layout.title,
                        layout.message,
                        layout.footer,
                    ]
                    .into_iter()
                    .chain(layout.buttons.iter().copied())
                    {
                        assert!(inside(client, rect), "{factor} {dpi} {labels:?}");
                    }
                    assert!(layout.title_band.bottom <= layout.message.top);
                    assert!(layout.message.bottom <= layout.footer.top);
                    for button in &layout.buttons {
                        assert!(inside(layout.footer, *button));
                    }
                    for pair in layout.buttons.windows(2) {
                        assert!(pair[0].right < pair[1].left, "buttons never overlap");
                    }
                }
            }
        }
    }

    #[test]
    fn the_dialog_is_400px_and_grows_only_for_wide_buttons() {
        assert_eq!(dialog_width(96, &[60, 70]), 400);
        assert!(dialog_width(96, &[420, 70]) > 400);
        assert_eq!(dialog_width(192, &[60, 70]), 800);
    }

    #[test]
    fn the_message_height_grows_the_dialog() {
        let short = Layout::calculate(96, 400, 24, 20, &[60, 70]);
        let long = Layout::calculate(96, 400, 24, 120, &[60, 70]);
        assert_eq!(long.height - short.height, 100);
    }

    #[test]
    fn buttons_are_right_aligned_with_the_last_at_the_padding() {
        let layout = Layout::calculate(96, 400, 24, 40, &[60, 70, 60]);
        assert_eq!(layout.buttons.len(), 3);
        assert_eq!(layout.buttons[2].right, 400 - 20);
        assert!(layout.buttons[0].left < layout.buttons[1].left);
        assert_eq!(
            layout.target_at(layout.buttons[1].left + 1, layout.buttons[1].top + 1),
            Some(Target::Button(1))
        );
        assert_eq!(
            layout.target_at(layout.title_close.left + 1, layout.title_close.top + 1),
            Some(Target::TitleClose)
        );
        assert_eq!(layout.target_at(1, layout.message.top + 1), None);
    }

    #[test]
    fn keys_choose_cancel_on_escape_the_focus_on_enter_and_quick_letters() {
        // Break caught: Esc or a stray key choosing a destructive button; Enter ignoring the focus.
        let quick = [(u16::from(b'S'), 0), (u16::from(b'D'), 1)];
        assert_eq!(key_choice(VK_ESCAPE, 0, 3, &quick), Some(2));
        assert_eq!(key_choice(VK_RETURN, 0, 3, &quick), Some(0));
        assert_eq!(key_choice(VK_RETURN, 1, 3, &quick), Some(1));
        assert_eq!(key_choice(VK_SPACE, 2, 3, &quick), Some(2));
        assert_eq!(key_choice(u16::from(b'S'), 2, 3, &quick), Some(0));
        assert_eq!(key_choice(u16::from(b'D'), 0, 3, &quick), Some(1));
        assert_eq!(key_choice(u16::from(b'X'), 0, 3, &quick), None);
        assert_eq!(
            key_choice(u16::from(b'S'), 0, 2, &[]),
            None,
            "no quick keys, no choice"
        );
        assert_eq!(key_choice(VK_ESCAPE, 0, 2, &[]), Some(1));
    }

    #[test]
    fn tab_cycles_forward_and_back_and_wraps() {
        assert_eq!(next_focus(0, 3, true), 1);
        assert_eq!(next_focus(2, 3, true), 0);
        assert_eq!(next_focus(0, 3, false), 2);
        assert_eq!(next_focus(0, 1, true), 0);
    }
}
