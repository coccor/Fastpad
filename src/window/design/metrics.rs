//! Sizes shared by more than one surface, in pixels at 96 DPI, and the one `scale` that turns
//! them into device pixels.

/// `value` (pixels at 96 DPI) scaled to `dpi`, rounded half up. A DPI of 0, which Windows returns
/// for a bad handle, counts as 96.
pub(crate) const fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

#[cfg(test)]
mod tests {
    use super::scale;

    #[test]
    fn scale_is_the_identity_at_96_dpi() {
        assert_eq!(scale(26, 96), 26);
        assert_eq!(scale(0, 96), 0);
    }

    #[test]
    fn scale_rounds_half_up_at_common_dpis() {
        // Break caught: a layout that drifts by a pixel per row at 125 % and 150 %.
        assert_eq!(scale(12, 120), 15); // 14.5 + 0.5 = 15
        assert_eq!(scale(26, 144), 39); // 39.0
        assert_eq!(scale(10, 192), 20);
        assert_eq!(scale(1, 144), 2); // 1.5 rounds up
    }

    #[test]
    fn a_dpi_of_zero_falls_back_to_96_instead_of_collapsing_the_layout() {
        // Break caught: `GetDpiForWindow` returns 0 for a bad handle; the old titlebar copy treated
        // that as a DPI of 1 and scaled every size to about zero.
        assert_eq!(scale(26, 0), 26);
        assert_eq!(scale(44, 0), 44);
    }
}
