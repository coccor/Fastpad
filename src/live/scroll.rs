//! Scroll maths for the Live view, in pixels of the layout's document coordinates. Keeping a line
//! steady across height changes uses `preview::heights::HeightIndex::{anchor, scroll_for_anchor}`.

/// The scroll offset that keeps `top..bottom` (the caret's row) visible in a viewport `height`
/// tall that is now at `scroll`, with `margin` pixels to spare where it fits. Scrolls only as far
/// as needed; a row taller than the viewport shows its top.
pub fn reveal(scroll: f32, height: f32, top: f32, bottom: f32, margin: f32) -> f32 {
    let margin = margin.min(((height - (bottom - top)) / 2.0).max(0.0));
    if top - margin < scroll || bottom - top > height {
        (top - margin).max(0.0)
    } else if bottom + margin > scroll + height {
        bottom + margin - height
    } else {
        scroll
    }
}

/// `scroll` limited so the viewport stays within a document `total` pixels tall: the last row can
/// reach the bottom of the viewport but not leave it.
pub fn clamp(scroll: f32, height: f32, total: f32) -> f32 {
    scroll.min(total - height).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::{clamp, reveal};

    #[test]
    fn reveal_leaves_a_visible_row_alone() {
        assert_eq!(reveal(100.0, 200.0, 150.0, 170.0, 10.0), 100.0);
    }

    #[test]
    fn reveal_scrolls_up_and_down_only_as_needed() {
        assert_eq!(reveal(100.0, 200.0, 80.0, 100.0, 10.0), 70.0);
        assert_eq!(reveal(100.0, 200.0, 290.0, 310.0, 10.0), 120.0);
        assert_eq!(reveal(100.0, 200.0, 0.0, 20.0, 10.0), 0.0);
    }

    #[test]
    fn reveal_shows_the_top_of_a_row_taller_than_the_viewport() {
        assert_eq!(reveal(0.0, 100.0, 300.0, 500.0, 10.0), 300.0);
    }

    #[test]
    fn clamp_keeps_the_last_row_at_the_bottom() {
        assert_eq!(clamp(900.0, 200.0, 1000.0), 800.0);
        assert_eq!(clamp(-5.0, 200.0, 1000.0), 0.0);
        assert_eq!(clamp(50.0, 200.0, 100.0), 0.0);
    }
}
