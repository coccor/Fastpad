//! Where everything in the Settings dialog sits: the `Layout` of title, nav, cards and
//! footer at a DPI and size, its hit testing and scrolling, and the toggle knob's rect.

use super::*;

/// Where everything sits. Headings and rows are in content coordinates, placed in the
/// scrolling `body` by `row_rect`/`heading_rect`. A row's rect is its card, without the gap
/// under it.
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title: RECT,
    pub title_close: RECT,
    pub body: RECT,
    pub nav: RECT,
    pub nav_items: [RECT; 2],
    pub headings: [RECT; 3],
    pub rows: [RECT; Row::ALL.len()],
    pub content_height: i32,
    pub edit_ini: RECT,
    pub close: RECT,
    pub(super) dpi: u32,
    pub(super) link_width: i32,
}

impl Layout {
    /// The layout at `dpi` at its natural size, at most `max_width` wide and `max_height` tall
    /// (the work area), with the Edit fastpad.ini link `link_width` wide.
    pub(crate) fn calculate(dpi: u32, max_width: i32, max_height: i32, link_width: i32) -> Self {
        let width = scale(WIDTH_AT_96_DPI, dpi);
        // Only the content's height is read off this one.
        let probe = Self::sized(dpi, width, 0, link_width);
        let chrome = probe.title.bottom + scale(FOOTER_HEIGHT_AT_96_DPI, dpi);
        let natural = chrome + probe.content_height;
        let smallest = chrome + probe.row_pitch() * MIN_VISIBLE_ROWS;
        Self::sized(
            dpi,
            width.min(max_width),
            natural.min(max_height.max(smallest)),
            link_width,
        )
    }

    /// The smallest size the user can drag the dialog to.
    pub(crate) fn min_size(dpi: u32) -> (i32, i32) {
        (
            scale(MIN_WIDTH_AT_96_DPI, dpi),
            scale(MIN_HEIGHT_AT_96_DPI, dpi),
        )
    }

    /// The layout `width` by `height`, as the dialog opens or as the user sized it.
    pub(crate) fn sized(dpi: u32, width: i32, height: i32, link_width: i32) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let title_height = scale(TITLE_HEIGHT_AT_96_DPI, dpi);
        let heading_height = scale(HEADING_HEIGHT_AT_96_DPI, dpi);
        let card_height = scale(CARD_HEIGHT_AT_96_DPI, dpi);
        let row_pitch = card_height + scale(CARD_GAP_AT_96_DPI, dpi);
        let footer_height = scale(FOOTER_HEIGHT_AT_96_DPI, dpi);

        // The × fills the title row's top-right corner, full height, like a caption button.
        let title_close_left = width - scale(TITLE_CLOSE_WIDTH_AT_96_DPI, dpi);
        let title = RECT {
            left: padding,
            top: 0,
            right: title_close_left,
            bottom: title_height,
        };
        let title_close = RECT {
            left: title_close_left,
            top: 0,
            right: width,
            bottom: title_height,
        };

        let nav_width = scale(NAV_WIDTH_AT_96_DPI, dpi).min(width / 3);
        let line = |top: i32, height: i32| RECT {
            left: nav_width + padding,
            top,
            right: width - padding,
            bottom: top + height,
        };
        let mut headings = [RECT::default(); 3];
        let mut rows = [RECT::default(); Row::ALL.len()];
        let mut top = 0;
        for section in Section::ALL {
            headings[section as usize] = line(top, heading_height);
            top += heading_height;
            for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
                rows[row as usize] = line(top, card_height);
                top += row_pitch;
            }
        }
        // The last card's gap separates it from the footer.
        let content_height = top;

        let body = RECT {
            left: nav_width,
            top: title_height,
            right: width,
            bottom: height - footer_height,
        };
        let nav = RECT {
            left: 0,
            top: title_height,
            right: nav_width,
            bottom: body.bottom,
        };
        let item_height = scale(NAV_ITEM_HEIGHT_AT_96_DPI, dpi);
        let nav_inset = scale(NAV_INSET_AT_96_DPI, dpi);
        let nav_items = std::array::from_fn(|index| {
            let top = nav.top + nav_inset + index as i32 * item_height;
            RECT {
                left: nav_inset,
                top,
                right: nav_width - nav_inset,
                bottom: top + item_height,
            }
        });
        let button_height = scale(BUTTON_HEIGHT_AT_96_DPI, dpi);
        let button_top = body.bottom + (footer_height - button_height) / 2;
        let close = RECT {
            left: width - padding - scale(BUTTON_WIDTH_AT_96_DPI, dpi),
            top: button_top,
            right: width - padding,
            bottom: button_top + button_height,
        };
        let edit_ini = RECT {
            left: padding,
            top: button_top,
            right: (padding + link_width).min(close.left),
            bottom: button_top + button_height,
        };
        Self {
            width,
            height,
            title,
            title_close,
            body,
            nav,
            nav_items,
            headings,
            rows,
            content_height,
            edit_ini,
            close,
            dpi,
            link_width,
        }
    }

    pub(crate) fn max_scroll(&self) -> i32 {
        (self.content_height - (self.body.bottom - self.body.top)).max(0)
    }

    pub(crate) fn list_row_height(&self) -> i32 {
        scale(CONTROL_HEIGHT_AT_96_DPI, self.dpi)
    }

    /// The corner radius of cards and controls.
    pub(crate) fn radius(&self) -> i32 {
        scale(RADIUS_AT_96_DPI, self.dpi)
    }

    /// A card and the gap under it, as `calculate` stacks them; a wheel notch scrolls three.
    pub(super) fn row_pitch(&self) -> i32 {
        scale(CARD_HEIGHT_AT_96_DPI, self.dpi) + scale(CARD_GAP_AT_96_DPI, self.dpi)
    }

    fn place(&self, rect: RECT, scroll: i32) -> RECT {
        let offset = self.body.top - scroll;
        RECT {
            top: rect.top + offset,
            bottom: rect.bottom + offset,
            ..rect
        }
    }

    /// Row `row`'s full rect in client coordinates at `scroll`.
    pub(crate) fn row_rect(&self, row: Row, scroll: i32) -> RECT {
        self.place(self.rows[row as usize], scroll)
    }

    pub(crate) fn heading_rect(&self, section: Section, scroll: i32) -> RECT {
        self.place(self.headings[section as usize], scroll)
    }

    /// The control of `row`, right-aligned inside the padding of its card `row_rect`.
    /// `segments` is how many a segmented row shows.
    pub(crate) fn control_rect(&self, row: Row, row_rect: RECT, segments: usize) -> RECT {
        let dpi = self.dpi;
        let (width, height) = match row.control() {
            Control::Dropdown => {
                let room = row_rect.right
                    - row_rect.left
                    - scale(CARD_PADDING_AT_96_DPI, dpi) * 2
                    - scale(LABEL_MIN_WIDTH_AT_96_DPI, dpi);
                (
                    scale(DROPDOWN_WIDTH_AT_96_DPI, dpi).min(room).max(0),
                    scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
                )
            }
            Control::Segmented => {
                let each = if row == Row::TabWidth {
                    TAB_SEGMENT_WIDTH_AT_96_DPI
                } else {
                    SEGMENT_WIDTH_AT_96_DPI
                };
                (
                    segments as i32 * scale(each, dpi),
                    scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
                )
            }
            Control::Stepper => (
                scale(STEP_BUTTON_AT_96_DPI, dpi) * 2 + scale(STEP_VALUE_AT_96_DPI, dpi),
                scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
            ),
            Control::Check => (
                scale(TOGGLE_WIDTH_AT_96_DPI, dpi),
                scale(TOGGLE_HEIGHT_AT_96_DPI, dpi),
            ),
        };
        let top = row_rect.top + (row_rect.bottom - row_rect.top - height) / 2;
        let right = row_rect.right - scale(CARD_PADDING_AT_96_DPI, dpi);
        RECT {
            left: right - width,
            top,
            right,
            bottom: top + height,
        }
    }

    /// A segmented control's segments, left to right, sharing its width.
    pub(crate) fn segment_rects(&self, control: RECT, segments: usize) -> Vec<RECT> {
        let count = segments.max(1) as i32;
        let each = (control.right - control.left) / count;
        (0..count)
            .map(|index| RECT {
                left: control.left + index * each,
                right: if index == count - 1 {
                    control.right
                } else {
                    control.left + (index + 1) * each
                },
                ..control
            })
            .collect()
    }

    /// The stepper's –, value and + parts.
    pub(crate) fn stepper_rects(&self, control: RECT) -> [RECT; 3] {
        let button = scale(STEP_BUTTON_AT_96_DPI, self.dpi);
        [
            RECT {
                right: control.left + button,
                ..control
            },
            RECT {
                left: control.left + button,
                right: control.right - button,
                ..control
            },
            RECT {
                left: control.right - button,
                ..control
            },
        ]
    }

    /// What client point `x`, `y` is on at `scroll`. A toggle row is hit anywhere on its card,
    /// label included. Other rows are hit only on their control.
    pub(crate) fn hit(
        &self,
        x: i32,
        y: i32,
        scroll: i32,
        view: &SettingsView,
        page: Page,
    ) -> Option<Hit> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.title_close) {
            return Some(Hit::TitleClose);
        }
        if inside(&self.close) {
            return Some(Hit::Close);
        }
        if inside(&self.edit_ini) {
            return Some(Hit::EditIni);
        }
        if let Some(index) = self.nav_items.iter().position(&inside) {
            return Some(Hit::Nav(Page::ALL[index]));
        }
        if page != Page::General || !inside(&self.body) {
            return None;
        }
        let row = Row::ALL
            .into_iter()
            .find(|row| inside(&self.row_rect(*row, scroll)))?;
        let segments = view.segments(row).len();
        let control = self.control_rect(row, self.row_rect(row, scroll), segments);
        let part = match row.control() {
            Control::Check => Some(Part::Whole),
            Control::Dropdown => inside(&control).then_some(Part::Whole),
            Control::Segmented => self
                .segment_rects(control, segments)
                .iter()
                .position(&inside)
                .map(Part::Segment),
            Control::Stepper => {
                let [minus, value, plus] = self.stepper_rects(control);
                [
                    (minus, Part::Minus),
                    (value, Part::Value),
                    (plus, Part::Plus),
                ]
                .into_iter()
                .find(|(rect, _)| inside(rect))
                .map(|(_, part)| part)
            }
        };
        part.map(|part| Hit::Row(row, part))
    }

    /// The scroll from `scroll` that shows `focus`'s row whole. A section's first row brings its
    /// heading into view too.
    pub(crate) fn scroll_to_show(&self, focus: Focus, scroll: i32) -> i32 {
        let Focus::Row(row) = focus else {
            return scroll;
        };
        let rect = self.rows[row as usize];
        let first_in_section = Row::ALL
            .into_iter()
            .find(|candidate| candidate.section() == row.section())
            == Some(row);
        let top = if first_in_section {
            self.headings[row.section() as usize].top
        } else {
            rect.top
        };
        let visible = self.body.bottom - self.body.top;
        let scroll = if top < scroll {
            top
        } else if rect.bottom > scroll + visible {
            rect.bottom - visible
        } else {
            scroll
        };
        scroll.clamp(0, self.max_scroll())
    }
}

/// The round knob of a toggle switch whose track is `track`: inset on every side, at the right
/// when `on`, at the left when off.
pub(crate) fn toggle_knob(track: RECT, on: bool, dpi: u32) -> RECT {
    let inset = scale(KNOB_INSET_AT_96_DPI, dpi);
    let size = (track.bottom - track.top - 2 * inset).max(0);
    let left = if on {
        track.right - inset - size
    } else {
        track.left + inset
    };
    RECT {
        left,
        top: track.top + inset,
        right: left + size,
        bottom: track.top + inset + size,
    }
}
