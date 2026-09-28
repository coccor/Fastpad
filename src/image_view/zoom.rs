//! Zoom and pan arithmetic for the image view (image preview spec §6.3). Scales are image pixels
//! to device pixels: 1.0 shows one image pixel per screen pixel. Offsets are the image's top-left
//! corner in the view's client pixels.

pub const STEPS: [f32; 11] = [0.10, 0.25, 0.50, 0.67, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// Fit the view, never above 100%.
    Fit,
    Scale(f32),
}

pub fn fit_scale(image: (f32, f32), view: (f32, f32)) -> f32 {
    if image.0 <= 0.0 || image.1 <= 0.0 {
        return 1.0;
    }
    (view.0 / image.0)
        .min(view.1 / image.1)
        .clamp(STEPS[0] / 100.0, 1.0)
}

pub fn step_in(current: f32) -> f32 {
    STEPS
        .iter()
        .copied()
        .find(|step| *step > current * 1.001)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

pub fn step_out(current: f32) -> f32 {
    STEPS
        .iter()
        .rev()
        .copied()
        .find(|step| *step < current * 0.999)
        .unwrap_or(STEPS[0])
}

/// Centres an image smaller than the view; otherwise keeps it covering the view.
pub fn clamp_axis(offset: f32, image_len: f32, view_len: f32) -> f32 {
    if image_len <= view_len {
        ((view_len - image_len) / 2.0).round()
    } else {
        offset.clamp(view_len - image_len, 0.0)
    }
}

/// The offset that keeps the image point under `anchor` fixed while the scale goes old → new.
pub fn zoom_about(offset: (f32, f32), old: f32, new: f32, anchor: (f32, f32)) -> (f32, f32) {
    let ratio = new / old;
    (
        anchor.0 - (anchor.0 - offset.0) * ratio,
        anchor.1 - (anchor.1 - offset.1) * ratio,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shrinks_large_images_and_never_enlarges_small_ones() {
        // Break caught: a 100×50 icon blown up to fill the window, or a photo opening at 100%.
        assert_eq!(fit_scale((1600.0, 1200.0), (800.0, 900.0)), 0.5);
        assert_eq!(fit_scale((100.0, 50.0), (800.0, 600.0)), 1.0);
    }

    #[test]
    fn steps_move_to_the_next_listed_scale_from_any_scale() {
        // Break caught: zooming in from a fit scale of 0.37 jumping to 1.0 or staying put.
        assert_eq!(step_in(0.37), 0.50);
        assert_eq!(step_in(1.0), 1.5);
        assert_eq!(step_in(16.0), 16.0);
        assert_eq!(step_out(0.37), 0.25);
        assert_eq!(step_out(0.10), 0.10);
    }

    #[test]
    fn panning_is_clamped_and_small_images_stay_centred() {
        // Break caught: dragging a zoomed image off screen, or a small image stuck top-left.
        assert_eq!(clamp_axis(-5000.0, 2000.0, 800.0), -1200.0);
        assert_eq!(clamp_axis(300.0, 2000.0, 800.0), 0.0);
        assert_eq!(clamp_axis(-40.0, 200.0, 800.0), 300.0);
    }

    #[test]
    fn zooming_about_a_point_keeps_that_point_still() {
        // Break caught: Ctrl+wheel zoom drifting away from the pointer.
        let (x, _) = zoom_about((0.0, 0.0), 1.0, 2.0, (100.0, 100.0));
        assert_eq!(x, -100.0);
    }
}
