//! A virtual row list shared by the sidebar's views: selection, hover, scrolling, keyboard
//! paging, hit-testing and a thin scroll thumb over any number of fixed-height rows. The state is
//! plain data. `paint` draws only the rows in view, into the caller's double-buffered DC.
//!
//! Every `height` is the list area's height in pixels, and every `y` is relative to its top.

use crate::window::palette::Palette;
use crate::window::panel::fill;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{HDC, IntersectClipRect, RestoreDC, SaveDC};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DOWN, VK_END, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETWHEELSCROLLLINES, SystemParametersInfoW, WHEEL_DELTA,
};

/// `SPI_GETWHEELSCROLLLINES` reports this for "one screen at a time".
const WHEEL_PAGESCROLL: u32 = u32::MAX;

/// The user's wheel setting (`SPI_GETWHEELSCROLLLINES`), 3 lines if it can't be read. The
/// sidebar's lists pass it to `RowListState::wheel`.
pub(crate) fn wheel_lines() -> u32 {
    let mut lines = 3u32;
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            (&mut lines as *mut u32).cast(),
            0,
        )
    };
    if read == 0 { 3 } else { lines }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl ListKey {
    /// The list key a `WM_KEYDOWN` virtual key means, if any.
    pub(crate) fn from_virtual_key(key: u32) -> Option<Self> {
        match u16::try_from(key).ok()? {
            VK_UP => Some(Self::Up),
            VK_DOWN => Some(Self::Down),
            VK_HOME => Some(Self::Home),
            VK_END => Some(Self::End),
            VK_PRIOR => Some(Self::PageUp),
            VK_NEXT => Some(Self::PageDown),
            _ => None,
        }
    }
}

/// How `paint` shows one row; `draw_row` gets it to pick text colors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowLook {
    pub(crate) selected: bool,
    pub(crate) hover: bool,
    pub(crate) focused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowListState {
    pub(crate) count: usize,
    pub(crate) selected: Option<usize>,
    pub(crate) hover: Option<usize>,
    /// The first row in view.
    pub(crate) top: usize,
    pub(crate) row_height: i32,
    /// Wheel movement short of one line, kept for the next wheel message.
    wheel_remainder: i32,
}

/// The scroll thumb's width for rows `row_height` tall: 6 px at the 26 px, 96-DPI row, so it
/// scales with DPI through the row height.
pub(crate) const fn thumb_width(row_height: i32) -> i32 {
    let width = row_height * 6 / 26;
    if width < 2 { 2 } else { width }
}

impl RowListState {
    pub(crate) fn new(row_height: i32) -> Self {
        Self {
            count: 0,
            selected: None,
            hover: None,
            top: 0,
            row_height: row_height.max(1),
            wheel_remainder: 0,
        }
    }

    /// A new row count. A selection past the end moves to the last row, a hover past it clears,
    /// and `top` stays on a row. The next `ensure_visible` or scroll fills the view again.
    pub(crate) fn set_count(&mut self, count: usize) {
        self.count = count;
        self.selected = match (self.selected, count.checked_sub(1)) {
            (Some(selected), Some(last)) => Some(selected.min(last)),
            _ => None,
        };
        if self.hover.is_some_and(|hover| hover >= count) {
            self.hover = None;
        }
        self.top = self.top.min(count.saturating_sub(1));
    }

    /// Rows wholly in view, at least one: what a page is measured in.
    fn full_rows(&self, height: i32) -> usize {
        (height.max(0) / self.row_height).max(1) as usize
    }

    /// The last `top` that still fills the view.
    fn max_top(&self, height: i32) -> usize {
        self.count.saturating_sub(self.full_rows(height))
    }

    /// Rows at least partly in view from `top`: the rows `paint` draws.
    pub(crate) fn visible_rows(&self, height: i32) -> usize {
        let fits = (height.max(0) as usize).div_ceil(self.row_height as usize);
        fits.min(self.count.saturating_sub(self.top))
    }

    pub(crate) fn row_at(&self, y: i32) -> Option<usize> {
        if y < 0 {
            return None;
        }
        let index = self.top.checked_add((y / self.row_height) as usize)?;
        (index < self.count).then_some(index)
    }

    /// `index`'s top edge relative to the list's top, or `None` above the view or past the end.
    /// A row below the view has a top beyond `height`.
    pub(crate) fn row_top(&self, index: usize) -> Option<i32> {
        if index < self.top || index >= self.count {
            return None;
        }
        i32::try_from(index - self.top)
            .ok()?
            .checked_mul(self.row_height)
    }

    /// Sets the hovered row; reports whether it changed.
    pub(crate) fn set_hover(&mut self, hover: Option<usize>) -> bool {
        let hover = hover.filter(|&index| index < self.count);
        std::mem::replace(&mut self.hover, hover) != hover
    }

    /// Moves the selection for `key`, keeping it in view. With nothing selected, a move selects
    /// the first row in view. Reports whether the selection or the scroll changed.
    pub(crate) fn move_selection(&mut self, key: ListKey, height: i32) -> bool {
        let Some(last) = self.count.checked_sub(1) else {
            return false;
        };
        let page = self.full_rows(height).saturating_sub(1).max(1);
        let target = match (self.selected, key) {
            (_, ListKey::Home) => 0,
            (_, ListKey::End) => last,
            (None, _) => self.top.min(last),
            (Some(current), ListKey::Up) => current.saturating_sub(1),
            (Some(current), ListKey::Down) => current.saturating_add(1).min(last),
            (Some(current), ListKey::PageUp) => current.saturating_sub(page),
            (Some(current), ListKey::PageDown) => current.saturating_add(page).min(last),
        };
        let before = (self.selected, self.top);
        self.select(target, height);
        (self.selected, self.top) != before
    }

    /// Selects `index` (clamped to the last row) and scrolls it into view.
    pub(crate) fn select(&mut self, index: usize, height: i32) {
        let Some(last) = self.count.checked_sub(1) else {
            self.selected = None;
            return;
        };
        let index = index.min(last);
        self.selected = Some(index);
        self.ensure_visible(index, height);
    }

    /// Scrolls the least needed to show `index` wholly, and keeps the view filled.
    pub(crate) fn ensure_visible(&mut self, index: usize, height: i32) {
        let Some(last) = self.count.checked_sub(1) else {
            self.top = 0;
            return;
        };
        let index = index.min(last);
        let full = self.full_rows(height);
        if index < self.top {
            self.top = index;
        } else if index >= self.top + full {
            self.top = index + 1 - full;
        }
        self.top = self.top.min(self.max_top(height));
    }

    /// Scrolls by `lines` rows (negative is up), within the list. Reports whether it moved.
    pub(crate) fn scroll_lines(&mut self, lines: i32, height: i32) -> bool {
        let before = self.top;
        let top = if lines < 0 {
            self.top.saturating_sub(lines.unsigned_abs() as usize)
        } else {
            self.top.saturating_add(lines as usize)
        };
        self.top = top.min(self.max_top(height));
        self.top != before
    }

    /// A mouse wheel turn of `delta` (`WHEEL_DELTA` per notch, positive away from the user) at
    /// `lines_per_notch` rows per notch (`SPI_GETWHEELSCROLLLINES`; `u32::MAX` pages). Fractions
    /// of a line from precision touchpads carry over to the next turn.
    pub(crate) fn wheel(&mut self, delta: i32, lines_per_notch: u32, height: i32) -> bool {
        let per_notch = if lines_per_notch == WHEEL_PAGESCROLL {
            i32::try_from(self.full_rows(height)).unwrap_or(i32::MAX)
        } else {
            i32::try_from(lines_per_notch).unwrap_or(i32::MAX)
        }
        .clamp(1, 1_000);
        let total = self
            .wheel_remainder
            .saturating_add(delta.saturating_mul(per_notch));
        let notch = WHEEL_DELTA as i32;
        self.wheel_remainder = total % notch;
        self.scroll_lines(-(total / notch), height)
    }

    /// The scroll thumb as (top, length) in a track `height` tall, or `None` when every row
    /// fits. The thumb is at least one row tall, so it can always be grabbed.
    pub(crate) fn thumb(&self, height: i32) -> Option<(i32, i32)> {
        let full = self.full_rows(height);
        if height <= 0 || self.count <= full {
            return None;
        }
        let track = i64::from(height);
        let length = (track * full as i64 / self.count as i64)
            .max(i64::from(self.row_height).min(track))
            .min(track);
        let max_top = self.max_top(height);
        let top = (track - length) * self.top.min(max_top) as i64 / (max_top as i64).max(1);
        Some((top as i32, length as i32))
    }

    /// Where on the thumb a press at `x`, `y` landed (the grab offset for `drag_thumb`), for a
    /// list `width` wide, or `None` off the thumb.
    pub(crate) fn thumb_hit(&self, x: i32, y: i32, width: i32, height: i32) -> Option<i32> {
        let (top, length) = self.thumb(height)?;
        (x >= width - thumb_width(self.row_height) && x < width && y >= top && y < top + length)
            .then_some(y - top)
    }

    /// Drags the thumb so the point grabbed `grab_offset` into it sits at `y`. Reports whether
    /// the list scrolled.
    pub(crate) fn drag_thumb(&mut self, grab_offset: i32, y: i32, height: i32) -> bool {
        let Some((_, length)) = self.thumb(height) else {
            return false;
        };
        let travel = i64::from(height - length).max(1);
        let thumb_top = i64::from(y.saturating_sub(grab_offset)).clamp(0, travel);
        let max_top = self.max_top(height);
        let top = ((thumb_top * max_top as i64 + travel / 2) / travel) as usize;
        let before = self.top;
        self.top = top.min(max_top);
        self.top != before
    }
}

/// A row's text color: the selection's own color only over the focused selection, whose
/// background `paint` fills; an unfocused selection sits on the inactive-selection background and
/// keeps the editor's text color. Every sidebar list colors its rows with it.
pub(crate) fn row_foreground(look: RowLook, palette: &Palette) -> u32 {
    if look.selected && look.focused {
        palette
            .selection_foreground
            .unwrap_or(palette.editor_foreground)
    } else {
        palette.editor_foreground
    }
}

/// Paints the rows in view into `area` of `hdc`. For each row it paints the selection (the
/// unfocused color when `focused` is false) or the hover background, then calls `draw_row` with
/// the row's index, rectangle and look. Last it paints the thin scroll thumb at the right edge.
/// Rows out of view are never touched, and nothing is drawn outside `area`.
pub(crate) fn paint(
    hdc: HDC,
    area: RECT,
    state: &RowListState,
    palette: &Palette,
    focused: bool,
    draw_row: &mut dyn FnMut(HDC, usize, RECT, RowLook),
) {
    let height = area.bottom - area.top;
    if height <= 0 || area.right <= area.left {
        return;
    }
    unsafe {
        let saved = SaveDC(hdc);
        IntersectClipRect(hdc, area.left, area.top, area.right, area.bottom);
        for offset in 0..state.visible_rows(height) {
            let index = state.top + offset;
            let top = area.top + offset as i32 * state.row_height;
            let rect = RECT {
                left: area.left,
                top,
                right: area.right,
                bottom: top + state.row_height,
            };
            let look = RowLook {
                selected: state.selected == Some(index),
                hover: state.hover == Some(index),
                focused,
            };
            if look.selected {
                let background = if focused {
                    palette.selection_background
                } else {
                    palette.inactive_selection_background
                };
                fill(hdc, rect, background);
            } else if look.hover {
                fill(hdc, rect, palette.hover_background);
            }
            draw_row(hdc, index, rect, look);
        }
        if let Some((thumb_top, length)) = state.thumb(height) {
            let thumb = RECT {
                left: area.right - thumb_width(state.row_height),
                top: area.top + thumb_top,
                right: area.right,
                bottom: area.top + thumb_top + length,
            };
            fill(hdc, thumb, palette.line_number_foreground);
        }
        RestoreDC(hdc, saved);
    }
}

#[cfg(test)]
mod tests {
    use super::{ListKey, RowListState, RowLook, paint, thumb_width};
    use crate::window::palette::Palette;

    fn list(count: usize) -> RowListState {
        let mut list = RowListState::new(26);
        list.set_count(count);
        list
    }

    #[test]
    fn a_new_list_is_empty_and_ignores_keys() {
        let mut list = RowListState::new(0);
        assert_eq!(list.row_height, 1, "a zero row height would divide by zero");
        assert_eq!(list.selected, None);
        assert!(!list.move_selection(ListKey::Down, 100));
        list.select(3, 100);
        assert_eq!(list.selected, None);
        assert_eq!(list.visible_rows(100), 0);
        assert_eq!(list.thumb(100), None);
    }

    #[test]
    fn selection_and_hover_are_clamped_when_the_count_shrinks() {
        // Break caught: a rescan that removes rows under the selection leaving an index past the
        // end, which the next paint or Enter would read out of bounds.
        let mut list = list(10);
        list.select(9, 26 * 3);
        list.hover = Some(8);
        list.set_count(5);
        assert_eq!(list.selected, Some(4));
        assert_eq!(list.hover, None);
        assert!(list.top <= 4);
        list.set_count(0);
        assert_eq!(list.selected, None);
        assert_eq!(list.top, 0);
    }

    #[test]
    fn keyboard_moves_the_selection_and_keeps_it_in_view() {
        let height = 26 * 3;
        let mut list = list(5);
        assert!(list.move_selection(ListKey::Down, height));
        assert_eq!(
            list.selected,
            Some(0),
            "the first key selects the first row in view"
        );
        assert!(list.move_selection(ListKey::Down, height));
        assert_eq!(list.selected, Some(1));
        assert!(list.move_selection(ListKey::Up, height));
        assert!(!list.move_selection(ListKey::Up, height));
        assert!(list.move_selection(ListKey::End, height));
        assert_eq!((list.selected, list.top), (Some(4), 2));
        assert!(list.move_selection(ListKey::Home, height));
        assert_eq!((list.selected, list.top), (Some(0), 0));
        for _ in 0..3 {
            list.move_selection(ListKey::Down, height);
        }
        assert_eq!((list.selected, list.top), (Some(3), 1));
    }

    #[test]
    fn paging_through_ten_thousand_rows_never_leaves_the_list() {
        // Break caught: PageDown past the end (a selection or top beyond the rows, a panic on the
        // next paint), a page that skips rows without showing them, or a selection scrolled out
        // of view.
        let height = 26 * 20 + 13; // 20 whole rows and part of one more
        let mut list = list(10_000);
        let in_view = |list: &RowListState| {
            let selected = list.selected.unwrap();
            assert!(selected < 10_000);
            assert!(
                list.top <= selected && selected < list.top + 20,
                "selection out of view"
            );
            assert!(list.top <= 10_000 - 20, "top past the last full page");
        };
        assert!(list.move_selection(ListKey::PageDown, height));
        assert_eq!(list.selected, Some(0));
        let mut steps = 0;
        while list.move_selection(ListKey::PageDown, height) {
            steps += 1;
            in_view(&list);
            assert!(steps <= 10_000);
        }
        assert_eq!(list.selected, Some(9_999));
        assert_eq!(list.top, 10_000 - 20);
        assert_eq!(
            steps,
            9_999usize.div_ceil(19),
            "a page moves by the rows in view less one"
        );
        while list.move_selection(ListKey::PageUp, height) {
            in_view(&list);
        }
        assert_eq!((list.selected, list.top), (Some(0), 0));
        assert!(list.move_selection(ListKey::End, height));
        assert_eq!(list.selected, Some(9_999));
        assert!(!list.move_selection(ListKey::Down, height));
        list.set_count(3);
        list.ensure_visible(2, height);
        assert_eq!((list.selected, list.top), (Some(2), 0));
    }

    #[test]
    fn ensure_visible_scrolls_the_least_needed() {
        let height = 26 * 4;
        let mut list = list(100);
        list.ensure_visible(10, height);
        assert_eq!(list.top, 7, "the row lands on the last full line");
        list.ensure_visible(8, height);
        assert_eq!(list.top, 7, "a row already in view scrolls nothing");
        list.ensure_visible(2, height);
        assert_eq!(list.top, 2);
        list.ensure_visible(1_000, height);
        assert_eq!(list.top, 96);
    }

    #[test]
    fn hit_testing_maps_y_to_rows_and_back() {
        let mut list = list(10);
        assert_eq!(list.visible_rows(100), 4);
        assert_eq!(list.row_at(0), Some(0));
        assert_eq!(list.row_at(25), Some(0));
        assert_eq!(list.row_at(26), Some(1));
        assert_eq!(list.row_at(-1), None);
        assert_eq!(list.row_at(26 * 10), None);
        assert_eq!(list.row_top(1), Some(26));
        list.top = 2;
        assert_eq!(list.row_at(0), Some(2));
        assert_eq!(list.row_top(1), None);
        assert_eq!(list.row_top(2), Some(0));
        assert_eq!(list.row_top(10), None);
        assert_eq!(list.visible_rows(26 * 20), 8, "never more rows than remain");
        assert!(list.set_hover(Some(3)));
        assert!(!list.set_hover(Some(3)));
    }

    #[test]
    fn wheel_scrolls_whole_lines_and_keeps_touchpad_fractions() {
        // Break caught: a precision touchpad's small wheel deltas rounding to nothing, so the
        // list never scrolls, or a wheel that scrolls past the end.
        let height = 26 * 10;
        let mut list = list(100);
        assert!(list.wheel(-120, 3, height));
        assert_eq!(list.top, 3);
        assert!(list.wheel(120, 3, height));
        assert_eq!(list.top, 0);
        assert!(!list.wheel(120, 3, height), "already at the top");
        assert!(list.wheel(-40, 3, height));
        assert_eq!(list.top, 1);
        assert!(!list.wheel(-20, 3, height));
        assert!(list.wheel(-20, 3, height));
        assert_eq!(list.top, 2);
        assert!(list.wheel(-120, u32::MAX, height), "page scrolling");
        assert_eq!(list.top, 12);
        assert!(list.scroll_lines(1_000, height));
        assert_eq!(list.top, 90);
        assert!(list.scroll_lines(-1_000, height));
        assert_eq!(list.top, 0);
    }

    #[test]
    fn the_thumb_is_sized_by_the_visible_share_and_tracks_the_scroll() {
        let height = 26 * 10;
        let mut list = list(100);
        assert_eq!(list.thumb(height), Some((0, 26)));
        list.top = 45;
        assert_eq!(list.thumb(height), Some((117, 26)));
        list.top = 90;
        assert_eq!(list.thumb(height), Some((234, 26)));
        // Ten thousand rows keep a thumb at least one row tall, so it can be grabbed.
        assert_eq!(
            super::RowListState {
                top: 0,
                ..self::list(10_000)
            }
            .thumb(height),
            Some((0, 26))
        );
        assert_eq!(
            self::list(10).thumb(height),
            None,
            "no thumb when every row fits"
        );
        let width = 200;
        let bar = width - thumb_width(26);
        assert_eq!(list.thumb_hit(bar, 240, width, height), Some(6));
        assert_eq!(list.thumb_hit(bar - 1, 240, width, height), None);
        assert_eq!(list.thumb_hit(bar, 10, width, height), None);
    }

    #[test]
    fn dragging_the_thumb_scrolls_from_the_first_to_the_last_row() {
        let height = 26 * 10;
        let mut list = list(10_000);
        assert!(list.drag_thumb(10, 10 + 234, height));
        assert_eq!(list.top, 10_000 - 10);
        assert!(list.drag_thumb(0, 117, height));
        assert_eq!(list.top, 4_995);
        assert_eq!(list.thumb(height).unwrap().0, 117);
        assert!(!list.drag_thumb(0, 117, height));
        assert!(list.drag_thumb(0, -500, height));
        assert_eq!(list.top, 0);
        assert!(list.drag_thumb(0, 10_000, height));
        assert_eq!(list.top, 10_000 - 10);
        assert!(
            !self::list(3).drag_thumb(0, 50, height),
            "no thumb, no drag"
        );
    }

    #[test]
    fn virtual_keys_map_to_list_keys() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_UP,
        };
        let key = |vk: u16| ListKey::from_virtual_key(u32::from(vk));
        assert_eq!(key(VK_UP), Some(ListKey::Up));
        assert_eq!(key(VK_DOWN), Some(ListKey::Down));
        assert_eq!(key(VK_HOME), Some(ListKey::Home));
        assert_eq!(key(VK_END), Some(ListKey::End));
        assert_eq!(key(VK_PRIOR), Some(ListKey::PageUp));
        assert_eq!(key(VK_NEXT), Some(ListKey::PageDown));
        assert_eq!(key(VK_LEFT), None);
    }

    #[test]
    fn painting_draws_only_the_rows_in_view_with_the_selection_and_hover_backgrounds() {
        // Break caught: a paint that walks every one of ten thousand rows (a stall per frame),
        // draws past the list's bottom, or shows a focused selection in the unfocused color.
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
            ReleaseDC, SelectObject,
        };
        let palette = Palette {
            selection_background: 0x0000_00ff,
            inactive_selection_background: 0x0000_ff00,
            hover_background: 0x00ff_0000,
            line_number_foreground: 0x0000_ffff,
            editor_background: 0x0080_8080,
            ..Palette::neutral()
        };
        let mut state = RowListState::new(20);
        state.set_count(10_000);
        state.top = 5_000;
        state.selected = Some(5_001);
        state.hover = Some(5_002);
        let (width, height) = (100, 205);
        let area = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let previous = SelectObject(dc, bitmap);
            for focused in [true, false] {
                crate::window::panel::fill(dc, area, palette.editor_background);
                let mut drawn: Vec<(usize, i32, RowLook)> = Vec::new();
                paint(
                    dc,
                    area,
                    &state,
                    &palette,
                    focused,
                    &mut |_, index, rect, look| {
                        drawn.push((index, rect.top, look));
                    },
                );
                assert_eq!(
                    drawn.iter().map(|(index, ..)| *index).collect::<Vec<_>>(),
                    (5_000..5_011).collect::<Vec<_>>(),
                    "only the 11 rows at least partly in view"
                );
                assert_eq!(drawn[3].1, 60);
                assert!(drawn[1].2.selected && drawn[1].2.focused == focused);
                assert!(drawn[2].2.hover && !drawn[2].2.selected);
                let selection = if focused {
                    palette.selection_background
                } else {
                    palette.inactive_selection_background
                };
                assert_eq!(GetPixel(dc, 10, 20 + 5), selection);
                assert_eq!(GetPixel(dc, 10, 40 + 5), palette.hover_background);
                assert_eq!(GetPixel(dc, 10, 5), palette.editor_background);
                let (thumb_top, _) = state.thumb(height).unwrap();
                assert_eq!(
                    GetPixel(dc, width - 1, thumb_top + 1),
                    palette.line_number_foreground
                );
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }
}
