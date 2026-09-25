//! Where the Notebook view's parts go (open editors spec §3.1): the title band in the window's
//! title strip, the Open Editors header and its rows, the notebook's root row with its buttons,
//! and the body below, which holds the tree or the view's state. Pure: rectangles only.

use crate::window::notebook_view::HeaderButton;
use crate::window::panel::scale;
use crate::window::side_panel::HEADER_HEIGHT_96;
use windows_sys::Win32::Foundation::RECT;

/// Every row's height at 96 DPI: section headers, Open Editors rows and tree rows alike.
pub(crate) const ROW_HEIGHT: i32 = 26;
/// Open Editors rows visible before the section scrolls on its own (spec §3.1).
pub(crate) const MAX_EDITOR_ROWS: usize = 9;
const ROOT_BUTTON: i32 = 22;
const CHEVRON_LEFT: i32 = 4;
const CHEVRON: i32 = 16;
const RIGHT_PAD: i32 = 6;

#[derive(Clone, Copy)]
pub(crate) struct PanelLayout {
    /// The view's name, in the window's title strip: all caption.
    pub title: RECT,
    pub editors_header: RECT,
    /// The Open Editors rows; empty when collapsed or with no tabs.
    pub editors_list: RECT,
    /// The notebook's root row.
    pub root: RECT,
    /// Everything under the root row: the tree, or the view's state.
    pub body: RECT,
}

/// The panel's bands for `client` at `dpi`, with `editors` tabs, stacked from the top and cut at
/// the panel's bottom edge, so a short panel never turns a band inside out.
pub(crate) fn panel_layout(
    client: RECT,
    dpi: u32,
    editors: usize,
    editors_expanded: bool,
) -> PanelLayout {
    let row = scale(ROW_HEIGHT, dpi);
    let cut = |y: i32| y.min(client.bottom);
    let title_bottom = cut(client.top + scale(HEADER_HEIGHT_96, dpi));
    let header_bottom = cut(title_bottom + row);
    let shown = if editors_expanded {
        editors.min(MAX_EDITOR_ROWS)
    } else {
        0
    };
    let list_bottom = cut(header_bottom + row * shown as i32);
    let root_bottom = cut(list_bottom + row);
    let band = |top: i32, bottom: i32| RECT {
        left: client.left,
        top,
        right: client.right,
        bottom,
    };
    PanelLayout {
        title: band(client.top, title_bottom),
        editors_header: band(title_bottom, header_bottom),
        editors_list: band(header_bottom, list_bottom),
        root: band(list_bottom, root_bottom),
        body: band(root_bottom, client.bottom),
    }
}

/// A section header row's chevron.
pub(crate) fn section_chevron(row: RECT, dpi: u32) -> RECT {
    let left = (row.left + scale(CHEVRON_LEFT, dpi)).min(row.right);
    RECT {
        left,
        top: row.top,
        right: (left + scale(CHEVRON, dpi)).min(row.right),
        bottom: row.bottom,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RootParts {
    pub chevron: RECT,
    pub name: RECT,
    /// Left to right: star, New note, New folder, "…".
    pub buttons: [(HeaderButton, RECT); 4],
}

/// The root row's chevron, the notebook's name after it, and its four buttons at the right.
pub(crate) fn root_parts(row: RECT, dpi: u32) -> RootParts {
    let chevron = section_chevron(row, dpi);
    let size = scale(ROOT_BUTTON, dpi);
    let top = row.top + (row.bottom - row.top - size) / 2;
    let right = row.right - scale(RIGHT_PAD, dpi);
    let slot = |from_right: i32| RECT {
        left: (right - (from_right + 1) * size).max(chevron.right),
        top,
        right: (right - from_right * size).max(chevron.right),
        bottom: top + size,
    };
    let buttons = [
        (HeaderButton::Favorite, slot(3)),
        (HeaderButton::NewNote, slot(2)),
        (HeaderButton::NewFolder, slot(1)),
        (HeaderButton::More, slot(0)),
    ];
    let name = RECT {
        left: chevron.right,
        top: row.top,
        right: buttons[0].1.left.max(chevron.right),
        bottom: row.bottom,
    };
    RootParts {
        chevron,
        name,
        buttons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    const CLIENT: RECT = RECT {
        left: 0,
        top: 0,
        right: 260,
        bottom: 600,
    };

    #[test]
    fn the_bands_stack_title_editors_root_then_body() {
        // Break caught: the Open Editors rows painted over the root row, the title band not the
        // strip's 38 px (so the window can't be dragged from it), or a body that starts above
        // the root row.
        let layout = panel_layout(CLIENT, 96, 3, true);
        assert_eq!(edges(layout.title), (0, 0, 260, 38));
        assert_eq!(edges(layout.editors_header), (0, 38, 260, 64));
        assert_eq!(edges(layout.editors_list), (0, 64, 260, 64 + 3 * 26));
        assert_eq!(edges(layout.root), (0, 142, 260, 168));
        assert_eq!(edges(layout.body), (0, 168, 260, 600));
    }

    #[test]
    fn a_collapsed_or_empty_section_has_no_rows_and_twenty_tabs_show_nine() {
        // Break caught: a collapsed section still taking room, or a long tab list pushing the
        // notebook off the panel (spec §3.1).
        let collapsed = panel_layout(CLIENT, 96, 5, false);
        assert_eq!(collapsed.editors_list.top, collapsed.editors_list.bottom);
        assert_eq!(collapsed.root.top, 64);
        let empty = panel_layout(CLIENT, 96, 0, true);
        assert_eq!(empty.root.top, 64);
        let many = panel_layout(CLIENT, 96, 20, true);
        assert_eq!(many.editors_list.bottom - many.editors_list.top, 9 * 26);
        assert_eq!(panel_layout(CLIENT, 192, 1, true).title.bottom, 76);
    }

    #[test]
    fn a_short_panel_cuts_every_band_at_its_bottom() {
        let short = RECT {
            bottom: 80,
            ..CLIENT
        };
        let layout = panel_layout(short, 96, 9, true);
        for band in [
            layout.title,
            layout.editors_header,
            layout.editors_list,
            layout.root,
            layout.body,
        ] {
            assert!(band.top <= band.bottom && band.bottom <= 80);
        }
    }

    #[test]
    fn the_root_buttons_sit_right_to_left_and_the_name_stops_before_them() {
        // Break caught: the notebook name drawn under the star, or buttons that don't follow the
        // panel's right edge.
        let row = RECT {
            left: 0,
            top: 142,
            right: 260,
            bottom: 168,
        };
        let parts = root_parts(row, 96);
        let [(a, star), (b, new), (c, folder), (d, more)] = parts.buttons;
        assert_eq!(
            (a, b, c, d),
            (
                HeaderButton::Favorite,
                HeaderButton::NewNote,
                HeaderButton::NewFolder,
                HeaderButton::More
            )
        );
        assert_eq!(edges(more), (232, 144, 254, 166));
        assert_eq!(
            (star.right, new.right, folder.right),
            (new.left, folder.left, more.left)
        );
        assert_eq!(parts.name.right, star.left);
        assert_eq!(parts.name.left, parts.chevron.right);
        assert_eq!(edges(parts.chevron), (4, 142, 20, 168));
        let narrow = root_parts(RECT { right: 40, ..row }, 96);
        assert!(narrow.name.left <= narrow.name.right);
    }
}
