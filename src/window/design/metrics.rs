//! Sizes shared by more than one surface, in pixels at 96 DPI, and the one `scale` that turns
//! them into device pixels.

/// `value` (pixels at 96 DPI) scaled to `dpi`, rounded half up. A DPI of 0, which Windows returns
/// for a bad handle, counts as 96.
pub(crate) const fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

/// The corner radius of cards, controls and buttons.
pub(crate) const CONTROL_RADIUS: i32 = 4;
/// The corner radius of the rounded top of a tab.
#[allow(
    dead_code,
    reason = "adopted by the tab strip and the sidebar in the next tasks"
)]
pub(crate) const TAB_RADIUS: i32 = 8;
/// How far a sidebar row's hover and selection fill is inset from the panel's left and right edges.
#[allow(
    dead_code,
    reason = "adopted by the tab strip and the sidebar in the next tasks"
)]
pub(crate) const ROW_INSET_X: i32 = 4;
/// How far that fill is inset from the row's top and bottom, which leaves a gap between rows.
#[allow(
    dead_code,
    reason = "adopted by the tab strip and the sidebar in the next tasks"
)]
pub(crate) const ROW_INSET_Y: i32 = 1;
/// The keyboard-focus ring's stroke, and its gap outside the control it rings.
pub(crate) const FOCUS_RING: i32 = 2;
pub(crate) const FOCUS_GAP: i32 = 1;
/// The height of a side panel's title row, shared by every sidebar view.
pub(crate) const PANEL_HEADER: i32 = 38;
/// The height of a row in the sidebar's lists (notebook tree, open editors, favorites).
pub(crate) const SIDEBAR_ROW: i32 = 26;
/// The size of the activity bar's icon glyphs.
pub(crate) const ICON: i32 = 16;
/// The size of the sidebar's icon glyphs: tree rows, carets, and the header buttons.
pub(crate) const SIDEBAR_ICON: i32 = 12;
/// The layout grid. Only tests use it, to track the sizes not yet on it.
#[cfg(test)]
pub(crate) const GRID: i32 = 4;

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

    use super::{
        CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING, GRID, ICON, PANEL_HEADER, ROW_INSET_X, ROW_INSET_Y,
        SIDEBAR_ICON, SIDEBAR_ROW, TAB_RADIUS,
    };

    #[test]
    fn shared_metrics_keep_the_values_the_old_constants_had() {
        // Break caught: a "no visible change" refactor that shifts a control by a pixel.
        assert_eq!(CONTROL_RADIUS, 4); // was soft_paint::RADIUS_AT_96_DPI
        assert_eq!(TAB_RADIUS, 8);
        assert_eq!(ROW_INSET_X, 4);
        assert_eq!(ROW_INSET_Y, 1);
        assert_eq!(FOCUS_RING, 2); // was soft_paint::FOCUS_WIDTH_AT_96_DPI
        assert_eq!(FOCUS_GAP, 1); // was soft_paint::FOCUS_GAP_AT_96_DPI
        assert_eq!(PANEL_HEADER, 38); // was side_panel::HEADER_HEIGHT_96
        assert_eq!(SIDEBAR_ROW, 26); // was notebook_layout::ROW_HEIGHT
        assert_eq!(ICON, 16); // the activity bar icon size
        assert_eq!(SIDEBAR_ICON, 12); // the sidebar icon size
    }

    #[test]
    fn the_layout_sizes_still_off_the_4px_grid_are_the_known_ones() {
        // Later steps move these onto the grid; when one moves, this list shrinks. Hairline strokes
        // (the focus ring and gap) are exempt from the grid.
        let sizes = [
            ("CONTROL_RADIUS", CONTROL_RADIUS),
            ("ICON", ICON),
            ("SIDEBAR_ICON", SIDEBAR_ICON),
            ("PANEL_HEADER", PANEL_HEADER),
            ("SIDEBAR_ROW", SIDEBAR_ROW),
        ];
        let off_grid: Vec<&str> = sizes
            .iter()
            .filter(|(_, value)| value % GRID != 0)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(off_grid, ["PANEL_HEADER", "SIDEBAR_ROW"]);
    }
}
