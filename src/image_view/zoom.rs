//! Zoom and pan arithmetic for the image view (image preview spec §6.3). Scales are image pixels
//! to device pixels: 1.0 shows one image pixel per screen pixel. Offsets are the image's top-left
//! corner in the view's client pixels.

pub const STEPS: [f32; 11] = [0.10, 0.25, 0.50, 0.67, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// Fit the view, never above 100%.
    Fit,
    /// Fill the view's width, scrolling down a tall picture; small pictures are enlarged.
    FillWidth,
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

/// The scale that makes `image` exactly as wide as `view`, from the smallest fit scale up to the
/// largest step.
pub fn fill_width_scale(image: (f32, f32), view: (f32, f32)) -> f32 {
    if image.0 <= 0.0 || image.1 <= 0.0 {
        return 1.0;
    }
    (view.0 / image.0).clamp(STEPS[0] / 100.0, STEPS[STEPS.len() - 1])
}

/// The device pixels per picture unit that `zoom` asks for. `unit` is the device pixels one
/// unit takes at 100%: 1 for a photo, whose 100% is its own pixels, and the display scale for a
/// drawing (SVG, diagram), whose units are points as a browser draws them. Fit's 100% cap and the
/// zoom steps are in those 100%s.
pub fn device_scale(zoom: Zoom, natural: (f32, f32), view: (f32, f32), unit: f32) -> f32 {
    let shown = (natural.0 * unit, natural.1 * unit);
    let level = match zoom {
        Zoom::Fit => fit_scale(shown, view),
        Zoom::FillWidth => fill_width_scale(shown, view),
        Zoom::Scale(scale) => scale,
    };
    level * unit
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

/// The point of a drawn SVG under client point `client`, in the SVG's own units. `scale` is
/// device pixels per SVG unit (the SVG's natural size is its viewBox) and `origin` is the viewBox's
/// top-left corner.
pub fn to_scene(
    client: (f32, f32),
    offset: (f32, f32),
    scale: f32,
    origin: (f64, f64),
) -> (f64, f64) {
    let scale = f64::from(scale);
    (
        f64::from(client.0 - offset.0) / scale + origin.0,
        f64::from(client.1 - offset.1) / scale + origin.1,
    )
}

/// `to_scene`'s inverse: where SVG point `point` is drawn in client pixels.
pub fn from_scene(
    point: (f64, f64),
    offset: (f32, f32),
    scale: f32,
    origin: (f64, f64),
) -> (f32, f32) {
    let scale = f64::from(scale);
    (
        ((point.0 - origin.0) * scale) as f32 + offset.0,
        ((point.1 - origin.1) * scale) as f32 + offset.1,
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
    fn a_drawing_at_100_percent_takes_one_point_per_unit_on_a_scaled_display() {
        // Break caught: diagrams and SVGs drawn at 80% of their size on a 125% display, because
        // 100% meant one unit per physical pixel as it does for photos.
        let view = (2000.0, 1500.0);
        assert_eq!(
            device_scale(Zoom::Scale(1.0), (200.0, 100.0), view, 1.25),
            1.25
        );
        assert_eq!(
            device_scale(Zoom::Scale(2.0), (200.0, 100.0), view, 1.25),
            2.5
        );
        // Fit stops at 100%, which is now a point per unit.
        assert_eq!(device_scale(Zoom::Fit, (200.0, 100.0), view, 1.25), 1.25);
        // A large drawing still fits the view exactly.
        assert_eq!(device_scale(Zoom::Fit, (4000.0, 1000.0), view, 1.25), 0.5);
        // Fill makes the drawing exactly as wide as the view.
        assert_eq!(
            device_scale(Zoom::FillWidth, (500.0, 100.0), view, 1.25),
            4.0
        );
    }

    #[test]
    fn a_photo_keeps_one_pixel_per_screen_pixel() {
        // Break caught: a photo at 100% blurred up to the display scale.
        let view = (2000.0, 1500.0);
        assert_eq!(
            device_scale(Zoom::Scale(1.0), (200.0, 100.0), view, 1.0),
            1.0
        );
        assert_eq!(device_scale(Zoom::Fit, (200.0, 100.0), view, 1.0), 1.0);
    }

    #[test]
    fn fill_width_enlarges_a_small_picture_past_100_percent() {
        // Break caught: Fill capped at 100% like Fit, so the two buttons do the same.
        assert_eq!(fill_width_scale((100.0, 50.0), (400.0, 300.0)), 4.0);
    }

    #[test]
    fn fill_width_shrinks_a_wide_picture_to_the_width() {
        // Break caught: Fill fitting the height too, leaving a wide diagram narrower than the pane.
        assert_eq!(fill_width_scale((2000.0, 100.0), (500.0, 300.0)), 0.25);
        assert_eq!(fill_width_scale((500.0, 4000.0), (500.0, 300.0)), 1.0);
    }

    #[test]
    fn fill_width_is_capped_at_the_largest_step() {
        // Break caught: a tiny diagram filled to a scale the zoom steps cannot come back from.
        assert_eq!(fill_width_scale((10.0, 10.0), (1000.0, 10.0)), 16.0);
        assert_eq!(fill_width_scale((0.0, 10.0), (1000.0, 10.0)), 1.0);
    }

    #[test]
    fn scene_point_round_trips_at_any_zoom_and_offset() {
        // Break caught: a click on a zoomed or panned diagram finding the element beside it.
        let origin = (-12.5, 8.0);
        for scale in [0.1_f32, 1.0, 2.0, 16.0] {
            for offset in [(0.0, 0.0), (-300.0, 40.0)] {
                let client = from_scene((40.0, 70.0), offset, scale, origin);
                let back = to_scene(client, offset, scale, origin);
                let tolerance = 1.0 / f64::from(scale);
                assert!(
                    (back.0 - 40.0).abs() <= tolerance,
                    "{scale} {offset:?} {back:?}"
                );
                assert!(
                    (back.1 - 70.0).abs() <= tolerance,
                    "{scale} {offset:?} {back:?}"
                );
            }
        }
        // The scene's corner sits at the picture's offset.
        assert_eq!(
            to_scene((-300.0, 40.0), (-300.0, 40.0), 2.0, origin),
            origin
        );
        assert_eq!(
            to_scene((-298.0, 40.0), (-300.0, 40.0), 2.0, origin),
            (-11.5, 8.0)
        );
    }

    #[test]
    fn zooming_about_a_point_keeps_that_point_still() {
        // Break caught: Ctrl+wheel zoom drifting away from the pointer.
        let (x, _) = zoom_about((0.0, 0.0), 1.0, 2.0, (100.0, 100.0));
        assert_eq!(x, -100.0);
    }
}
