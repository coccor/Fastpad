//! The Search view's geometry: the header's field, chevron and button rectangles, the list
//! and line areas, what a point hits, and the tooltip tools that follow them.

use super::*;
use crate::search::SearchOption;
use crate::window::design::metrics::panel_header;
use crate::window::design::metrics::scale;
use crate::window::design::text_scale::scale_text;
use crate::window::option_toggles;
use crate::window::sidebar_accessibility;
use windows_sys::Win32::Foundation::{POINT, RECT};

impl SearchView {
    /// The title band along the top ("Search"): the window's title strip, as in the Notebook
    /// view, so all of it is caption.
    pub(crate) fn title_rect(client: RECT, dpi: u32) -> RECT {
        RECT {
            bottom: (client.top + panel_header(dpi)).min(client.bottom),
            ..client
        }
    }

    /// Where the header (the search field's row) begins: under the title band.
    fn header_top(client: RECT, dpi: u32) -> i32 {
        Self::title_rect(client, dpi).bottom
    }

    /// The painted search field, border included, right of the chevron.
    pub(crate) fn field_rect(client: RECT, dpi: u32) -> RECT {
        let margin = scale(FIELD_MARGIN_AT_96_DPI, dpi);
        let height = scale_text(FIELD_HEIGHT_AT_96_DPI, dpi);
        let top = Self::header_top(client, dpi) + (scale(HEADER_AT_96_DPI, dpi) - height) / 2;
        let left = client.left
            + scale(CHEVRON_LEFT_AT_96_DPI, dpi)
            + scale(CHEVRON_WIDTH_AT_96_DPI, dpi)
            + scale(CHEVRON_GAP_AT_96_DPI, dpi);
        RECT {
            left,
            top,
            right: (client.right - margin).max(left),
            bottom: top + height,
        }
    }

    /// The clear-search button: a square left of the toggles, at the field's right end.
    pub(crate) fn clear_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let toggles = option_toggles::toggle_rects(field, dpi);
        let size = scale(option_toggles::SIZE_AT_96_DPI, dpi);
        let right = (toggles[0].left - scale(CLEAR_GAP_AT_96_DPI, dpi)).max(field.left);
        RECT {
            left: (right - size).max(field.left),
            top: toggles[0].top,
            right,
            bottom: toggles[0].top + size,
        }
    }

    /// The chevron that opens and closes the replace field, left of the search field and as tall.
    pub(crate) fn chevron_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let left = client.left + scale(CHEVRON_LEFT_AT_96_DPI, dpi);
        RECT {
            left,
            top: field.top,
            right: left + scale(CHEVRON_WIDTH_AT_96_DPI, dpi),
            bottom: field.bottom,
        }
    }

    /// Replace all, in the replace row: as tall as the search field, ending at its right edge.
    pub(crate) fn replace_all_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let height = field.bottom - field.top;
        let top = Self::header_top(client, dpi)
            + scale(HEADER_AT_96_DPI, dpi)
            + (scale_text(REPLACE_ROW_AT_96_DPI, dpi) - height) / 2;
        RECT {
            left: (field.right - scale(REPLACE_ALL_WIDTH_AT_96_DPI, dpi)).max(field.left),
            top,
            right: field.right,
            bottom: top + height,
        }
    }

    /// The painted replace field, border included: under the search field, short of Replace all.
    pub(crate) fn replace_field_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let all = Self::replace_all_rect(client, dpi);
        RECT {
            left: field.left,
            top: all.top,
            right: (all.left - scale(GAP_AT_96_DPI, dpi)).max(field.left),
            bottom: all.bottom,
        }
    }

    /// A result's replace button in `row` (the whole row's rectangle): a square, vertically
    /// centered, in from the right edge by the rows' padding, clear of the scroll thumb.
    pub(crate) fn row_replace_rect(row: RECT, dpi: u32) -> RECT {
        let size = scale(ROW_BUTTON_AT_96_DPI, dpi);
        let right = (row.right - scale(PADDING_AT_96_DPI, dpi)).max(row.left);
        let top = row.top + (row.bottom - row.top - size) / 2;
        RECT {
            left: (right - size).max(row.left),
            top,
            right,
            bottom: top + size,
        }
    }

    /// Where the header ends, and the replace row under it while the replace field is open.
    pub(super) fn head_bottom(&self, client: RECT, dpi: u32) -> i32 {
        let replace = if self.replace_open {
            scale_text(REPLACE_ROW_AT_96_DPI, dpi)
        } else {
            0
        };
        (Self::header_top(client, dpi) + scale(HEADER_AT_96_DPI, dpi) + replace).min(client.bottom)
    }

    /// Where the results are: under the header, the replace row while it shows, and the summary
    /// line, above the status line when it shows.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let top = (self.head_bottom(client, dpi) + scale(LINE_AT_96_DPI, dpi)).min(client.bottom);
        let bottom = if self.status_line().is_some() {
            (client.bottom - scale(LINE_AT_96_DPI, dpi)).max(top)
        } else {
            client.bottom
        };
        RECT {
            top,
            bottom,
            ..client
        }
    }

    /// The summary line under the header (and the replace row), where the notice shows too.
    pub(crate) fn summary_rect(&self, client: RECT, dpi: u32) -> RECT {
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let top = self.head_bottom(client, dpi);
        RECT {
            left: client.left + pad,
            top,
            right: client.right - pad,
            bottom: (top + scale(LINE_AT_96_DPI, dpi)).min(client.bottom),
        }
    }

    /// Whether Replace all and the rows' replace buttons can run: a search finished with results
    /// (spec §11), no notice shows in their place, and no replace runs (a press would be ignored).
    pub(crate) fn replace_all_enabled(&self) -> bool {
        matches!(self.search, SearchState::Done { .. })
            && !self.results.is_empty()
            && self.notice().is_none()
            && !self.replacing
    }

    /// Whether row `index` shows its replace button: the hovered and the selected row, while the
    /// replace field is open.
    pub(super) fn row_button_shown(&self, index: usize) -> bool {
        self.replace_open && (self.list.hover == Some(index) || self.list.selected == Some(index))
    }

    /// The row whose replace button is under `point`, while that button shows.
    pub(super) fn row_button_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let index = self.row_under(point, client, dpi)?;
        if !self.row_button_shown(index) {
            return None;
        }
        let (row, _) =
            sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, index);
        inside(Self::row_replace_rect(row, dpi), point).then_some(index)
    }

    /// Whether the clear button shows: the box has text.
    pub(super) fn clear_shown(&self) -> bool {
        self.edit.is_some() && !self.box_text.is_empty()
    }

    /// The header button under `point`: the chevron while the box shows, the clear button while
    /// the box has text, Replace all while the replace field is open.
    pub(super) fn header_button_at(
        &self,
        point: POINT,
        client: RECT,
        dpi: u32,
    ) -> Option<HeaderButton> {
        self.edit?;
        if inside(Self::chevron_rect(client, dpi), point) {
            Some(HeaderButton::Chevron)
        } else if self.clear_shown() && inside(Self::clear_rect(client, dpi), point) {
            Some(HeaderButton::Clear)
        } else if self.replace_open && inside(Self::replace_all_rect(client, dpi), point) {
            Some(HeaderButton::ReplaceAll)
        } else {
            None
        }
    }

    /// The status line along the bottom.
    pub(crate) fn status_rect(client: RECT, dpi: u32) -> RECT {
        let pad = scale(PADDING_AT_96_DPI, dpi);
        RECT {
            left: client.left + pad,
            top: (client.bottom - scale(LINE_AT_96_DPI, dpi)).max(client.top),
            right: client.right - pad,
            bottom: client.bottom,
        }
    }

    /// The toggle under `point`, while the field shows.
    pub(super) fn toggle_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<SearchOption> {
        self.edit?;
        option_toggles::hit(
            &option_toggles::toggle_rects(Self::field_rect(client, dpi), dpi),
            point,
        )
    }

    /// The tooltip's tools: the toggles while the field shows, and the status line while it says
    /// notes were skipped. An empty text removes a tool.
    pub(super) fn tooltip_tools(&self, client: RECT, dpi: u32) -> Vec<(usize, [i32; 4], String)> {
        let rects = option_toggles::toggle_rects(Self::field_rect(client, dpi), dpi);
        let mut tools = SearchOption::ALL
            .into_iter()
            .zip(rects)
            .enumerate()
            .map(|(id, (option, rect))| {
                let text = if self.edit.is_some() {
                    option_toggles::tooltip(option).to_owned()
                } else {
                    String::new()
                };
                (id, edges(rect), text)
            })
            .collect::<Vec<_>>();
        let skipped = match &self.search {
            SearchState::Done { progress, .. } if self.notice().is_none() => {
                skipped_tooltip(progress)
            }
            _ => String::new(),
        };
        let chevron = if self.edit.is_some() {
            "Toggle replace"
        } else {
            ""
        };
        tools.push((
            CHEVRON_TOOL,
            edges(Self::chevron_rect(client, dpi)),
            chevron.to_owned(),
        ));
        let clear = if self.clear_shown() {
            "Clear search"
        } else {
            ""
        };
        tools.push((
            CLEAR_TOOL,
            edges(Self::clear_rect(client, dpi)),
            clear.to_owned(),
        ));
        let all = if self.replace_open {
            "Replace all (Ctrl+Alt+Enter)"
        } else {
            ""
        };
        tools.push((
            REPLACE_ALL_TOOL,
            edges(Self::replace_all_rect(client, dpi)),
            all.to_owned(),
        ));
        let row = self
            .row_hover_button
            .filter(|_| self.replace_open)
            .map(|index| {
                let (row, _) =
                    sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, index);
                Self::row_replace_rect(row, dpi)
            });
        tools.push((
            ROW_REPLACE_TOOL,
            edges(row.unwrap_or_default()),
            if row.is_some() { "Replace" } else { "" }.to_owned(),
        ));
        tools.push((STATUS_TOOL, edges(Self::status_rect(client, dpi)), skipped));
        tools
    }

    /// Destroys the view's tooltip, if it made one. The popup is owned by the main window, so
    /// destroying the panel does not take it along (`side_panel::destroy_windows` calls this).
    pub(crate) fn destroy_tooltip(&self) {
        if let Some(tooltip) = self.tooltip {
            tooltip.destroy();
        }
    }
}

impl SearchView {
    pub(super) fn row_under(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        self.list
            .row_at(point.y - area.top)
            .filter(|&index| index < self.results.len())
    }

    /// Where on the scroll thumb a press at `point` landed, if it landed on it.
    pub(super) fn thumb_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<i32> {
        let area = self.list_area(client, dpi);
        self.list.thumb_hit(
            point.x - area.left,
            point.y - area.top,
            area.right - area.left,
            height(area),
        )
    }
}
