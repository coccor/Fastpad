//! The diagram toolbar: a strip of buttons in the top-right corner of a FastDraw diagram's view
//! (Fit, Fill, zoom out, the zoom percent, zoom in, Open in FastDraw), as the VS Code preview has.
//! Layout and hit-testing are pure; the view paints it and routes the mouse to it.

use crate::preview::render::RectF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Fit,
    Fill,
    ZoomOut,
    Percent,
    ZoomIn,
    OpenInFastDraw,
}

/// The buttons, left to right.
pub const BUTTONS: [Button; 6] = [
    Button::Fit,
    Button::Fill,
    Button::ZoomOut,
    Button::Percent,
    Button::ZoomIn,
    Button::OpenInFastDraw,
];

/// Sizes in device-independent pixels.
const MARGIN: f32 = 8.0;
const PADDING: f32 = 4.0;
const GAP: f32 = 2.0;
const BUTTON: f32 = 28.0;
const PERCENT: f32 = 52.0;

/// Each button's rectangle in client pixels, left to right, in a view `client_width` wide.
pub fn layout(client_width: f32, dpi_scale: f32) -> [(Button, RectF); 6] {
    let width = |button: Button| {
        if button == Button::Percent {
            PERCENT
        } else {
            BUTTON
        }
    };
    let total: f32 =
        BUTTONS.iter().map(|button| width(*button)).sum::<f32>() + GAP * (BUTTONS.len() - 1) as f32;
    let top = (MARGIN + PADDING) * dpi_scale;
    let mut left = client_width - (MARGIN + PADDING + total) * dpi_scale;
    BUTTONS.map(|button| {
        let right = left + width(button) * dpi_scale;
        let rect = RectF::new(left, top, right, top + BUTTON * dpi_scale);
        left = right + GAP * dpi_scale;
        (button, rect)
    })
}

/// The strip behind the buttons.
pub fn strip(client_width: f32, dpi_scale: f32) -> RectF {
    let rects = layout(client_width, dpi_scale);
    let pad = PADDING * dpi_scale;
    RectF::new(
        rects[0].1.left - pad,
        rects[0].1.top - pad,
        rects[5].1.right + pad,
        rects[5].1.bottom + pad,
    )
}

/// The button under client point `point`.
pub fn button_at(client_width: f32, dpi_scale: f32, point: (f32, f32)) -> Option<Button> {
    layout(client_width, dpi_scale)
        .into_iter()
        .find(|(_, rect)| rect.contains(point.0, point.1))
        .map(|(button, _)| button)
}

/// Whether `point` is anywhere on the strip, buttons or the padding between them.
pub fn on_strip(client_width: f32, dpi_scale: f32, point: (f32, f32)) -> bool {
    strip(client_width, dpi_scale).contains(point.0, point.1)
}

/// The pointer's state over the strip, and the strip's tooltip control.
#[derive(Debug, Default)]
pub(crate) struct Pointer {
    pub(crate) hovered: Option<Button>,
    /// The button a press went down on, until the release: it acts only if released on it.
    pub(crate) pressed: Option<Button>,
    /// `TrackMouseEvent` was asked for the leave.
    pub(crate) tracking: bool,
    pub(crate) tooltip: Option<crate::window::tooltip::Tooltip>,
    pub(crate) tooltip_failed: bool,
    /// The client width the tooltip's rectangles were laid out for.
    pub(crate) tooltip_width: f32,
}

/// The icon glyph of `button` in the UI's icon font (Segoe MDL2 Assets); the percent is text.
pub fn glyph(button: Button) -> &'static str {
    match button {
        Button::Fit => "\u{E9A6}",
        Button::Fill => "\u{E740}",
        Button::ZoomOut => "\u{E71F}",
        Button::Percent => "",
        Button::ZoomIn => "\u{E8A3}",
        Button::OpenInFastDraw => "\u{E8A7}",
    }
}

pub fn tooltip(button: Button) -> &'static str {
    match button {
        Button::Fit => "Fit the whole diagram (F)",
        Button::Fill => "Fill the width (W)",
        Button::ZoomOut => "Zoom out (-)",
        Button::Percent => "Actual size (0)",
        Button::ZoomIn => "Zoom in (+)",
        Button::OpenInFastDraw => "Open in FastDraw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_sits_in_the_top_right_corner() {
        // Break caught: the strip drawn off the view's edge or over the band's left corner.
        let rects = layout(800.0, 1.0);
        assert_eq!(rects[5].1.right, 800.0 - MARGIN - PADDING);
        assert_eq!(rects[0].1.top, MARGIN + PADDING);
        assert_eq!(strip(800.0, 1.0).right, 800.0 - MARGIN);
        assert_eq!(strip(800.0, 1.0).top, MARGIN);
        for pair in rects.windows(2) {
            assert_eq!(pair[1].1.left, pair[0].1.right + GAP, "{pair:?}");
        }
        let order: Vec<Button> = rects.iter().map(|(button, _)| *button).collect();
        assert_eq!(order, BUTTONS);
        assert_eq!(rects[3].1.width(), PERCENT);
        assert_eq!(rects[0].1.height(), BUTTON);
    }

    #[test]
    fn the_strip_scales_with_dpi() {
        // Break caught: 28-pixel buttons on a 200% screen, too small to hit.
        let small = layout(800.0, 1.0);
        let large = layout(800.0, 2.0);
        for (a, b) in small.iter().zip(&large) {
            assert_eq!(b.1.width(), a.1.width() * 2.0);
            assert_eq!(b.1.height(), a.1.height() * 2.0);
        }
        assert_eq!(large[5].1.right, 800.0 - (MARGIN + PADDING) * 2.0);
    }

    #[test]
    fn button_at_finds_each_button_and_misses_gaps() {
        // Break caught: a press on one button acting as its neighbour, or the gap acting at all.
        let rects = layout(640.0, 1.5);
        for (button, rect) in rects {
            let centre = (
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0,
            );
            assert_eq!(button_at(640.0, 1.5, centre), Some(button));
        }
        let gap = (rects[0].1.right + GAP * 1.5 / 2.0, rects[0].1.top + 4.0);
        assert_eq!(button_at(640.0, 1.5, gap), None);
        assert!(on_strip(640.0, 1.5, gap));
        let left_of = (strip(640.0, 1.5).left - 1.0, rects[0].1.top + 4.0);
        assert_eq!(button_at(640.0, 1.5, left_of), None);
        assert!(!on_strip(640.0, 1.5, left_of));
    }
}
