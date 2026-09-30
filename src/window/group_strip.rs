//! An editor group's tab strip (split editors spec §4.2): its tabs, across its whole width. For a
//! group at the top of the editor area the strip is the title bar row (spec §4.1); its commands
//! are on right-click and in the menus. Pure geometry and hit-testing first, then painting; the group window routes the
//! pointer here.

use crate::config::FileIconSet;
use crate::window::file_icons::NoteKind;
use crate::window::icon_sets::TreeItem;
use crate::window::icon_sets::images::IconImages;
use crate::window::notebook_view::draw_item_icon;
use crate::window::palette::{FileIcons, Palette};
use crate::window::titlebar::{
    GLYPH_CLOSE, Point, Rect, TitleFontHandles, draw_text, fill, restore_font, scale, select_font,
};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, HDC, IntersectClipRect,
    RestoreDC, SRCCOPY, SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

/// A tab's file icon box, in 96 dpi pixels, and the gap after it.
const ICON_BOX: i32 = 16;
const ICON_GAP: i32 = 6;

/// The width a tab's icon and its gap take from the label at `dpi`.
fn icon_slot(dpi: u32) -> i32 {
    scale(ICON_BOX + ICON_GAP, dpi)
}

/// What a point in the strip is over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StripTarget {
    Tab(usize),
    CloseTab(usize),
    /// The band along the bottom of the tab viewport while the tabs overflow it.
    ScrollBar,
    /// Strip with nothing on it: double-click opens a tab, right-click the tab-strip menu.
    Empty,
}

impl StripTarget {
    /// Whether the target highlights under the pointer and acts on a press and release.
    pub const fn is_interactive(self) -> bool {
        !matches!(self, Self::Empty)
    }
}

/// Which strip target the pointer is over and which one a primary button went down on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StripPointer {
    pub(crate) hovered: Option<StripTarget>,
    pub(crate) pressed: Option<StripTarget>,
}

impl StripPointer {
    #[must_use]
    pub(crate) fn hover(self, target: Option<StripTarget>) -> Self {
        Self {
            hovered: target.filter(|target| target.is_interactive()),
            ..self
        }
    }

    #[must_use]
    pub(crate) fn press(self, target: Option<StripTarget>) -> Self {
        Self {
            pressed: target.filter(|target| target.is_interactive()),
            ..self
        }
    }

    pub(crate) fn is_pressed(self, target: StripTarget) -> bool {
        self.pressed == Some(target)
    }

    /// Clears the press and returns the target to activate when released over the pressed target.
    #[must_use]
    pub(crate) fn release(self, target: Option<StripTarget>) -> (Self, Option<StripTarget>) {
        let activated = self.pressed.filter(|pressed| target == Some(*pressed));
        (
            Self {
                pressed: None,
                ..self
            },
            activated,
        )
    }
}

/// The strip's height at `dpi`: as tall as the title strip, which it is for a top group.
pub fn strip_height(dpi: u32) -> i32 {
    crate::window::titlebar::strip_height(dpi)
}

/// How wide `group`'s strip is: the group's width, except that a group at the top of the window
/// stops its strip at the caption buttons, which stay the main window's.
/// Computed from the windows alone, so the accessibility provider can call it off the UI thread.
pub(crate) fn strip_width(group: windows_sys::Win32::Foundation::HWND) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    let main = crate::platform::win32::root_window(group);
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    let mut frame = windows_sys::Win32::Foundation::RECT::default();
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe {
        GetClientRect(group, &mut client);
        GetClientRect(main, &mut frame);
        windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, main, &mut origin, 1);
    }
    if origin.y > 0 {
        return client.right;
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(main) }.max(96);
    let caption = crate::window::titlebar::TitleBarLayout::calculate(
        crate::window::titlebar::Size::new(frame.right, frame.bottom),
        dpi,
    )
    .minimize
    .left;
    client.right.min(caption - origin.x).max(0)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StripLayout {
    pub height: i32,
    /// The visible tab viewport; tabs scrolled outside it are clipped and never hit.
    pub tabs: Rect,
    /// How far the tabs are scrolled left, clamped to `max_scroll`.
    pub scroll: i32,
    pub max_scroll: i32,
    /// The draggable scroll band, present only while the tabs overflow the viewport.
    pub scroll_bar: Option<Rect>,
    min_thumb: i32,
    tab_width: i32,
    tab_rects: Vec<Rect>,
    close_tab_rects: Vec<Rect>,
}

impl StripLayout {
    /// The strip of a group `width` pixels wide with `tab_count` tabs, scrolled by `scroll`.
    pub fn calculate(width: i32, dpi: u32, tab_count: usize, scroll: i32) -> Self {
        let width = width.max(0);
        let dpi = dpi.max(1);
        let height = strip_height(dpi);
        let tabs = Rect::new(0, 0, width, height);
        let viewport = tabs.right;
        let tab_width = if tab_count == 0 {
            0
        } else {
            (viewport / tab_count as i32).clamp(
                scale(120, dpi) + icon_slot(dpi),
                scale(200, dpi) + icon_slot(dpi),
            )
        };
        let content_width = tab_width.saturating_mul(tab_count as i32);
        let max_scroll = (content_width - viewport).max(0);
        let scroll = scroll.clamp(0, max_scroll);

        let close_size = scale(32, dpi).min(tab_width);
        let mut tab_rects = Vec::with_capacity(tab_count);
        let mut close_tab_rects = Vec::with_capacity(tab_count);
        for index in 0..tab_count {
            let left = index as i32 * tab_width - scroll;
            let right = left + tab_width;
            tab_rects.push(Rect::new(left, 0, right, height));
            close_tab_rects.push(Rect::new(right - close_size, 0, right, height));
        }
        let scroll_bar = (max_scroll > 0)
            .then(|| Rect::new(tabs.left, height - scale(8, dpi), tabs.right, height));

        Self {
            height,
            tabs,
            scroll,
            max_scroll,
            scroll_bar,
            min_thumb: scale(24, dpi),
            tab_width,
            tab_rects,
            close_tab_rects,
        }
    }

    pub fn hit_test(&self, point: Point) -> StripTarget {
        if self.scroll_bar.is_some_and(|bar| bar.contains(point)) {
            return StripTarget::ScrollBar;
        }
        if self.tabs.contains(point) {
            for (index, rect) in self.close_tab_rects.iter().enumerate() {
                if rect.contains(point) {
                    return StripTarget::CloseTab(index);
                }
            }
            for (index, rect) in self.tab_rects.iter().enumerate() {
                if rect.contains(point) {
                    return StripTarget::Tab(index);
                }
            }
        }
        StripTarget::Empty
    }

    pub fn tab(&self, index: usize) -> Option<Rect> {
        self.tab_rects.get(index).copied()
    }

    pub fn close_tab(&self, index: usize) -> Option<Rect> {
        self.close_tab_rects.get(index).copied()
    }

    /// The whole strip, from the group's left edge to its right.
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.tabs.right, self.height)
    }

    /// The insertion point (0 to the tab count) nearest strip `x`: the boundary between two tabs
    /// the point is closest to (split editors spec §6.1).
    pub fn insertion_index(&self, x: i32) -> usize {
        if self.tab_width == 0 {
            return 0;
        }
        let count = self.tab_rects.len() as i32;
        (x + self.scroll + self.tab_width / 2)
            .div_euclid(self.tab_width)
            .clamp(0, count) as usize
    }

    /// Where insertion point `index`'s bar is drawn, kept inside the tab viewport.
    pub fn insertion_x(&self, index: usize) -> i32 {
        (index as i32 * self.tab_width - self.scroll).clamp(self.tabs.left, self.tabs.right)
    }

    /// The scroll offset that brings the whole of tab `index` into the viewport.
    pub fn scroll_to_reveal(&self, index: usize) -> i32 {
        let left = index as i32 * self.tab_width;
        let right = left + self.tab_width;
        let viewport = self.tabs.right - self.tabs.left;
        let scroll = if left < self.scroll {
            left
        } else if right > self.scroll + viewport {
            right - viewport
        } else {
            self.scroll
        };
        scroll.clamp(0, self.max_scroll)
    }

    /// The scroll offset after a mouse wheel turn of `delta`; one notch moves half a tab.
    pub fn scroll_by_wheel(&self, delta: i32, wheel_delta: i32) -> i32 {
        let step = (self.tab_width / 2).max(1);
        let pixels = (i64::from(delta) * i64::from(step) / i64::from(wheel_delta.max(1))) as i32;
        self.scroll.saturating_add(pixels).clamp(0, self.max_scroll)
    }

    /// The thumb inside `scroll_bar`, sized by how much of the tab strip is visible.
    pub fn scroll_thumb(&self) -> Option<Rect> {
        let bar = self.scroll_bar?;
        let track = bar.right - bar.left;
        let thumb = self.thumb_width(track);
        let left = bar.left + self.scroll * (track - thumb) / self.max_scroll;
        Some(Rect::new(left, bar.top, left + thumb, bar.bottom))
    }

    fn thumb_width(&self, track: i32) -> i32 {
        let content = track + self.max_scroll;
        (track * track / content.max(1))
            .max(self.min_thumb)
            .min(track)
    }

    /// The scroll offset that puts the thumb's left edge at `thumb_left`.
    pub fn scroll_for_thumb(&self, thumb_left: i32) -> i32 {
        let Some(bar) = self.scroll_bar else {
            return 0;
        };
        let track = bar.right - bar.left;
        let travel = (track - self.thumb_width(track)).max(1);
        let offset = i64::from((thumb_left - bar.left).clamp(0, travel));
        (offset * i64::from(self.max_scroll) / i64::from(travel)) as i32
    }
}

pub(crate) struct StripPaint<'a> {
    pub(crate) titles: &'a [&'a str],
    /// Each tab's icon kind, parallel to `titles`.
    pub(crate) kinds: &'a [NoteKind],
    pub(crate) icons: FileIcons,
    pub(crate) icon_set: FileIconSet,
    pub(crate) light_theme: bool,
    pub(crate) active: usize,
    /// The preview tab's index; its label is drawn in italics.
    pub(crate) preview_tab: Option<usize>,
    pub(crate) palette: Palette,
    pub(crate) fonts: TitleFontHandles,
    pub(crate) pointer: StripPointer,
    /// The bar along the top of the active tab, when there are several groups.
    pub(crate) accent: Option<u32>,
}

/// The bar along the top of a group's active tab (split editors spec §4.2): only with several
/// groups, so one group looks as it always has.
pub(crate) fn tab_accent(active_group: bool, group_count: usize, palette: &Palette) -> Option<u32> {
    (group_count > 1).then_some(if active_group {
        palette.editor_foreground
    } else {
        palette.muted_foreground
    })
}

/// Paints the strip into `dc` at the group's origin, through an off-screen bitmap.
pub(crate) unsafe fn paint(dc: HDC, layout: &StripLayout, dpi: u32, input: &StripPaint<'_>) {
    let bounds = layout.bounds();
    let (width, height) = (bounds.right, bounds.bottom);
    if width <= 0 || height <= 0 {
        return;
    }
    let memory = unsafe { CreateCompatibleDC(dc) };
    let bitmap = if memory.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { CreateCompatibleBitmap(dc, width, height) }
    };
    if bitmap.is_null() {
        unsafe { draw(dc, layout, dpi, input) };
    } else {
        unsafe {
            let previous = SelectObject(memory, bitmap);
            draw(memory, layout, dpi, input);
            BitBlt(dc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
            SelectObject(memory, previous);
            DeleteObject(bitmap);
        }
    }
    if !memory.is_null() {
        unsafe {
            DeleteDC(memory);
        }
    }
}

unsafe fn draw(dc: HDC, layout: &StripLayout, dpi: u32, input: &StripPaint<'_>) {
    let palette = input.palette;
    let pointer = input.pointer;
    let centered = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
    unsafe {
        fill(dc, layout.bounds(), palette.strip_background);
        SetBkMode(dc, TRANSPARENT as i32);
    }
    let previous_font = unsafe { select_font(dc, input.fonts.text()) };

    let saved = unsafe { SaveDC(dc) };
    unsafe {
        IntersectClipRect(
            dc,
            layout.tabs.left,
            layout.tabs.top,
            layout.tabs.right,
            layout.tabs.bottom,
        );
    }
    let mut images = IconImages::new();
    for (index, title) in input.titles.iter().enumerate() {
        let (Some(tab), Some(close)) = (layout.tab(index), layout.close_tab(index)) else {
            continue;
        };
        if tab.right <= layout.tabs.left || tab.left >= layout.tabs.right {
            continue;
        }
        let selected = index == input.active;
        let tab_hovered = matches!(
            pointer.hovered,
            Some(StripTarget::Tab(hovered) | StripTarget::CloseTab(hovered)) if hovered == index
        );
        let (background, foreground) = if selected {
            (palette.active_tab_background(), palette.editor_foreground)
        } else if tab_hovered {
            (palette.hover_background, palette.hover_foreground)
        } else {
            (palette.strip_background, palette.muted_foreground)
        };
        let close_hovered = pointer.hovered == Some(StripTarget::CloseTab(index));
        unsafe {
            fill(dc, tab, background);
            if selected && let Some(accent) = input.accent {
                fill(
                    dc,
                    Rect::new(tab.left, tab.top, tab.right, tab.top + scale(2, dpi)),
                    accent,
                );
            }
            if let Some(&kind) = input.kinds.get(index) {
                let px = scale(ICON_BOX, dpi);
                let left = tab.left + scale(12, dpi);
                let top = tab.top + (tab.bottom - tab.top - px) / 2;
                draw_item_icon(
                    dc,
                    TreeItem::Note(kind),
                    RECT {
                        left,
                        top,
                        right: left + px,
                        bottom: top + px,
                    },
                    px,
                    palette.muted_foreground,
                    &palette,
                    &input.icons,
                    &mut images,
                    input.icon_set,
                    input.light_theme,
                );
            }
            select_font(
                dc,
                if input.preview_tab == Some(index) {
                    input.fonts.italic()
                } else {
                    input.fonts.text()
                },
            );
            SetTextColor(dc, foreground);
            draw_text(
                dc,
                title,
                Rect::new(
                    tab.left + scale(12, dpi) + icon_slot(dpi),
                    tab.top,
                    close.left.max(tab.left),
                    tab.bottom,
                ),
                DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if close_hovered {
                let pressed = pointer.is_pressed(StripTarget::CloseTab(index));
                fill(
                    dc,
                    close.centered_square(scale(24, dpi)),
                    if pressed {
                        palette.pressed_background
                    } else {
                        palette.hover_background
                    },
                );
            }
            select_font(dc, input.fonts.glyph());
            SetTextColor(
                dc,
                if close_hovered || (tab_hovered && !selected) {
                    palette.hover_foreground
                } else {
                    palette.muted_foreground
                },
            );
            draw_text(dc, GLYPH_CLOSE, close, centered);
        }
    }
    if saved != 0 {
        unsafe {
            RestoreDC(dc, saved);
        }
    }
    if let Some(thumb) = layout.scroll_thumb() {
        // A slim line at rest that thickens into the full grab band under the pointer.
        let active = pointer.hovered == Some(StripTarget::ScrollBar)
            || pointer.is_pressed(StripTarget::ScrollBar);
        let thumb = if active {
            thumb
        } else {
            Rect::new(
                thumb.left,
                thumb.bottom - scale(3, dpi),
                thumb.right,
                thumb.bottom,
            )
        };
        unsafe { fill(dc, thumb, palette.pressed_background) };
    }

    unsafe { restore_font(dc, previous_font) };
}

#[cfg(test)]
mod tests {
    use super::{StripLayout, StripPointer, StripTarget};
    use crate::window::titlebar::{Point, Rect};

    fn centre(rect: Rect) -> Point {
        rect.center()
    }

    #[test]
    fn every_painted_rect_hits_its_own_target() {
        // Break caught: paint and hit-test disagreeing, so a click lands on the neighbouring tab
        // or on a button that is not drawn there.
        for (width, count, scroll) in [(900, 3, 0), (900, 12, 250), (300, 2, 0)] {
            let layout = StripLayout::calculate(width, 144, count, scroll);
            for index in 0..count {
                let tab = layout.tab(index).unwrap();
                let close = layout.close_tab(index).unwrap();
                let label = Point::new((tab.left + close.left) / 2, (tab.top + tab.bottom) / 2);
                if label.x > layout.tabs.left && label.x < layout.tabs.right {
                    assert_eq!(layout.hit_test(label), StripTarget::Tab(index));
                }
                if close.left >= layout.tabs.left && close.right <= layout.tabs.right {
                    let middle = Point::new(centre(close).x, close.top + 2);
                    assert_eq!(layout.hit_test(middle), StripTarget::CloseTab(index));
                }
            }
            assert_eq!(layout.tabs.right, width);
        }
    }

    #[test]
    fn a_tab_keeps_room_for_its_icon_and_a_label() {
        // Break caught: the icon eating the label's width, so a crowded tab shows only an icon
        // and an ellipsis.
        for dpi in [96, 144, 192] {
            let layout = StripLayout::calculate(4000, dpi, 30, 0);
            let (tab, close) = (layout.tab(0).unwrap(), layout.close_tab(0).unwrap());
            let label_left = tab.left + super::scale(12, dpi) + super::icon_slot(dpi);
            assert!(close.left - label_left >= super::scale(120 - 12 - 32, dpi));
            assert_eq!(
                tab.right - tab.left,
                super::scale(120, dpi) + super::icon_slot(dpi)
            );
            // The icon and label area is the tab, not the close button.
            let label = Point::new(label_left + 2, tab.top + 2);
            assert_eq!(layout.hit_test(label), StripTarget::Tab(0));
        }
    }

    #[test]
    fn the_space_after_the_last_tab_is_empty_strip() {
        // Break caught: a double-click after the last tab not opening a new tab because it hit a
        // stale tab rectangle.
        let layout = StripLayout::calculate(1200, 96, 2, 0);
        let after = layout.tab(1).unwrap().right + 10;
        assert_eq!(layout.hit_test(Point::new(after, 10)), StripTarget::Empty);
        let empty = StripLayout::calculate(1200, 96, 0, 0);
        assert_eq!(empty.hit_test(Point::new(10, 10)), StripTarget::Empty);
        assert_eq!(empty.max_scroll, 0);
        assert_eq!(empty.scroll_bar, None);
    }

    #[test]
    fn the_tabs_span_the_whole_strip() {
        // Break caught: a strip that still reserves room for a "…" button, or is laid out against
        // the window instead of the group.
        let layout = StripLayout::calculate(700, 96, 3, 0);
        assert_eq!(layout.tabs.left, 0);
        assert_eq!(layout.tabs.right, 700);
        assert_eq!(layout.bounds().right, 700);
        assert_eq!(
            layout.hit_test(Point::new(695, layout.height / 2)),
            StripTarget::Empty
        );
        assert_eq!(layout.tab(0).unwrap().left, 0);
        assert_eq!(layout.tab(1).unwrap().left, layout.tab(0).unwrap().right);
        // A group narrower than its buttons leaves an empty viewport, never a negative one.
        let squeezed = StripLayout::calculate(30, 96, 2, 0);
        assert!(squeezed.tabs.left <= squeezed.tabs.right);
    }

    #[test]
    fn crowded_tabs_scroll_inside_the_viewport() {
        // Break caught: tabs shrinking to unreadable slivers or spilling over the buttons, or a
        // scroll offset the thumb and wheel can't reach.
        let layout = StripLayout::calculate(1200, 96, 30, 0);
        assert!(layout.max_scroll > 0);
        assert!(layout.tab(1).unwrap().left - layout.tab(0).unwrap().left >= 120);
        assert_eq!(layout.tabs.right, 1200);
        assert_eq!(
            layout.hit_test(layout.tab(29).unwrap().center()),
            StripTarget::Empty
        );

        let reveal = layout.scroll_to_reveal(29);
        assert_eq!(reveal, layout.max_scroll);
        let scrolled = StripLayout::calculate(1200, 96, 30, reveal);
        assert!(scrolled.tab(29).unwrap().right <= scrolled.tabs.right);
        let label = Point::new(scrolled.tab(29).unwrap().left + 10, scrolled.height / 3);
        assert_eq!(scrolled.hit_test(label), StripTarget::Tab(29));
        assert_eq!(scrolled.scroll_to_reveal(0), 0);

        let bar = layout
            .scroll_bar
            .expect("overflowing tabs get a scroll bar");
        let thumb = layout.scroll_thumb().unwrap();
        assert_eq!(layout.hit_test(thumb.center()), StripTarget::ScrollBar);
        assert_eq!(layout.scroll_for_thumb(bar.left - 50), 0);
        assert_eq!(layout.scroll_for_thumb(bar.right), layout.max_scroll);
        let dragged = StripLayout::calculate(1200, 96, 30, layout.scroll_for_thumb(bar.right));
        assert_eq!(dragged.scroll_thumb().unwrap().right, bar.right);
        assert_eq!(scrolled.scroll_by_wheel(-120_000, 120), 0);
        assert_eq!(
            StripLayout::calculate(1200, 96, 2, 500).scroll,
            0,
            "tabs that fit never stay scrolled"
        );
    }

    #[test]
    fn the_pointer_acts_only_on_release_over_the_pressed_target() {
        // Break caught: a press on one tab and a release on another closing or activating the
        // second, or empty strip highlighting under the pointer.
        let idle = StripPointer::default();
        assert_eq!(idle.hover(Some(StripTarget::Empty)), idle);
        let pressed = idle
            .hover(Some(StripTarget::CloseTab(1)))
            .press(Some(StripTarget::CloseTab(1)));
        assert_eq!(
            pressed.release(Some(StripTarget::CloseTab(1))).1,
            Some(StripTarget::CloseTab(1))
        );
        assert_eq!(pressed.release(Some(StripTarget::CloseTab(2))).1, None);
    }

    #[test]
    fn the_accent_marks_the_active_group_only_when_there_are_several() {
        // Break caught: one group gaining a bar 0.2.0 never had, or every group's active tab
        // looking equally active.
        let palette = crate::window::palette::Palette::neutral();
        assert_eq!(super::tab_accent(true, 1, &palette), None);
        assert_eq!(
            super::tab_accent(true, 2, &palette),
            Some(palette.editor_foreground)
        );
        assert_eq!(
            super::tab_accent(false, 2, &palette),
            Some(palette.muted_foreground)
        );
    }

    #[test]
    fn the_insertion_point_is_the_nearest_tab_boundary() {
        // Break caught: a drop on the right half of a tab landing before it.
        let layout = StripLayout::calculate(1200, 96, 3, 0);
        let width = layout.tab(0).unwrap().right;
        assert_eq!(layout.insertion_index(-5), 0);
        assert_eq!(layout.insertion_index(width / 2 - 1), 0);
        assert_eq!(layout.insertion_index(width / 2 + 1), 1);
        assert_eq!(layout.insertion_index(10_000), 3);
        assert_eq!(layout.insertion_x(0), 0);
        assert_eq!(layout.insertion_x(1), width);
        assert_eq!(
            StripLayout::calculate(1200, 96, 0, 0).insertion_index(300),
            0
        );
    }

    #[test]
    fn the_insertion_point_follows_the_scroll_and_its_bar_stays_in_view() {
        // Break caught: insertion points computed as if the strip were not scrolled.
        let layout = StripLayout::calculate(300, 96, 10, 100);
        let width = layout.tab(1).unwrap().right - layout.tab(1).unwrap().left;
        assert_eq!(
            layout.insertion_index(0),
            ((100 + width / 2) / width) as usize
        );
        assert_eq!(layout.insertion_x(0), 0, "clamped to the viewport");
        assert_eq!(layout.insertion_x(10), layout.tabs.right);
    }
}
