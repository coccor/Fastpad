//! What the MSAA provider reads of the Search view: its children (the box, toggles, replace
//! controls, lines and results) and the name and state announcements.

use super::*;
use crate::config::SidebarView;
use crate::search::SearchOption;
use crate::window::option_toggles;
use crate::window::side_panel;
use crate::window::sidebar_accessibility::{self, AccessibleItem};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_STATECHANGE, GWL_STYLE, GetWindowLongPtrW, WS_VISIBLE,
};

impl SearchView {
    /// Whether the box shows. Read from its style: `IsWindowVisible` would also ask its
    /// ancestors, and a hidden test window hides everything.
    pub(super) fn box_shown(&self) -> bool {
        self.edit.is_some_and(|edit| {
            (unsafe { GetWindowLongPtrW(edit, GWL_STYLE) }) as u32 & WS_VISIBLE != 0
        })
    }

    /// The summary line's text (or the notice painted in its place) and the status line's, as
    /// `paint` draws them: while a notice shows, there is no status line.
    pub(super) fn shown_lines(&self) -> (Option<String>, Option<String>) {
        match self.notice() {
            Some(notice) => (Some(notice.to_owned()), None),
            None => (self.summary().map(|(text, _)| text), self.status_line()),
        }
    }

    /// The MSAA children before the results. The chevron comes after the toggles, so the box and
    /// the toggles keep the child IDs they had before replace existed.
    pub(super) fn head_children(&self) -> Vec<SearchChild> {
        let mut head = Vec::with_capacity(10);
        if self.box_shown() {
            head.push(SearchChild::Box);
            head.extend(SearchOption::ALL.map(SearchChild::Toggle));
            head.push(SearchChild::Chevron);
            if self.replace_field_shown() {
                head.push(SearchChild::ReplaceField);
                head.push(SearchChild::ReplaceAll);
            }
        }
        let (summary, status) = self.shown_lines();
        if summary.is_some() {
            head.push(SearchChild::Summary);
        }
        if status.is_some() {
            head.push(SearchChild::Status);
        }
        if let Some(row) = self.row_replace_child() {
            head.push(SearchChild::RowReplace(row));
        }
        head
    }

    /// Whether the replace field shows. Read from its style, as `box_shown` is.
    pub(super) fn replace_field_shown(&self) -> bool {
        self.replace_open
            && self.replace_edit.is_some_and(|edit| {
                (unsafe { GetWindowLongPtrW(edit, GWL_STYLE) }) as u32 & WS_VISIBLE != 0
            })
    }

    /// The selected result whose replace button is a child. There is one such child, the
    /// selected row's, so a screen reader user reaches it from the row they are on and the
    /// results keep their IDs as the selection moves.
    pub(super) fn row_replace_child(&self) -> Option<usize> {
        let row = self.list.selected?;
        (self.replace_field_shown() && self.notice().is_none() && row < self.results.len())
            .then_some(row)
    }

    pub(super) fn child_at(&self, index: usize) -> Option<SearchChild> {
        let head = self.head_children();
        match head.get(index) {
            Some(child) => Some(*child),
            None => {
                let result = index - head.len();
                (result < self.results.len()).then_some(SearchChild::Result(result))
            }
        }
    }

    pub(super) fn child_index(&self, child: SearchChild) -> Option<usize> {
        let head = self.head_children();
        match child {
            SearchChild::Result(index) => {
                (index < self.results.len()).then_some(head.len() + index)
            }
            _ => head.iter().position(|shown| *shown == child),
        }
    }
}

/// Raises `EVENT_OBJECT_NAMECHANGE` for the summary and status lines whose text changed, at
/// most once a second while a search runs (spec §10). `settled` (the search finished, failed or
/// can't run) always speaks, so the limit never swallows the final count. A line that went away
/// has no child left to name; the panel's reorder event covers it. Raised with nothing of the
/// App borrowed: an in-context hook may call back into the panel's accessible object.
pub(crate) fn announce_lines(hwnd: HWND, settled: bool) {
    if side_panel::current_view(hwnd) != SidebarView::Search {
        return;
    }
    let Some((panel, changed)) = with_view(hwnd, |view| {
        let (summary, status) = view.shown_lines();
        let (summary, status) = (summary.unwrap_or_default(), status.unwrap_or_default());
        let summary_changed = summary != view.spoken.summary;
        let status_changed = status != view.spoken.status;
        if !summary_changed && !status_changed {
            return None;
        }
        let running = matches!(view.search, SearchState::Running(_));
        let recent = view
            .spoken
            .at
            .is_some_and(|at| at.elapsed() < ANNOUNCE_INTERVAL);
        if running && !settled && recent {
            return None;
        }
        let mut changed = Vec::with_capacity(2);
        if summary_changed && let Some(index) = view.child_index(SearchChild::Summary) {
            changed.push(index);
        }
        if status_changed && let Some(index) = view.child_index(SearchChild::Status) {
            changed.push(index);
        }
        view.spoken = Spoken {
            summary,
            status,
            at: Some(std::time::Instant::now()),
        };
        Some((view.panel, changed))
    })
    .flatten() else {
        return;
    };
    for index in changed {
        sidebar_accessibility::notify(EVENT_OBJECT_NAMECHANGE, panel, Some(index));
    }
}

/// Tells screen readers a toggle's checked state changed.
pub(super) fn announce_toggle(hwnd: HWND, option: SearchOption) {
    announce_state(hwnd, SearchChild::Toggle(option));
}

/// Raises `EVENT_OBJECT_STATECHANGE` for `child`, if it is one of the view's children now: a
/// toggle, the chevron or Replace all. Raised with nothing of the App borrowed.
pub(super) fn announce_state(hwnd: HWND, child: SearchChild) {
    if side_panel::current_view(hwnd) != SidebarView::Search {
        return;
    }
    if let Some((panel, index)) =
        with_view(hwnd, |view| Some((view.panel, view.child_index(child)?))).flatten()
    {
        sidebar_accessibility::notify(EVENT_OBJECT_STATECHANGE, panel, Some(index));
    }
}

impl sidebar_accessibility::AccessibleView for SearchView {
    /// The box, the three toggles, the replace chevron (then, while the replace field is open,
    /// the field and Replace all), the summary and status lines while they show, the selected
    /// row's replace button while the field is open, then the results. The status line and the
    /// row button come before the results so the results' IDs stay put while results stream in
    /// and the selection moves.
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        self.head_children().len() + self.results.len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<AccessibleItem> {
        let field = SearchView::field_rect(client, dpi);
        Some(match self.child_at(index)? {
            SearchChild::Box => {
                let edit = self.edit?;
                // The kept text: a WM_GETTEXT here would run under the App borrow.
                sidebar_accessibility::field_item(
                    &self.placeholder,
                    self.box_text.clone(),
                    unsafe { GetFocus() } == edit,
                    field,
                    edit,
                )
            }
            SearchChild::Toggle(option) => {
                let position = SearchOption::ALL.iter().position(|o| *o == option)?;
                sidebar_accessibility::check_item(
                    option_toggles::label(option),
                    self.options.get(option),
                    option_toggles::toggle_rects(field, dpi)[position],
                )
            }
            SearchChild::Chevron => sidebar_accessibility::expander_item(
                "Toggle replace",
                self.replace_open,
                SearchView::chevron_rect(client, dpi),
            ),
            SearchChild::ReplaceField => {
                let replace = self.replace_edit?;
                // The kept text: a WM_GETTEXT here would run under the App borrow.
                sidebar_accessibility::field_item(
                    REPLACE_PLACEHOLDER,
                    self.replace_text.clone(),
                    unsafe { GetFocus() } == replace,
                    SearchView::replace_field_rect(client, dpi),
                    replace,
                )
            }
            SearchChild::ReplaceAll => sidebar_accessibility::action_item(
                "Replace all",
                self.replace_all_enabled(),
                SearchView::replace_all_rect(client, dpi),
            ),
            SearchChild::RowReplace(row) => {
                let hit = self.results.get(row)?;
                let (rect, visible) =
                    sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, row);
                let mut item = sidebar_accessibility::action_item(
                    &format!("Replace in {}", hit.name),
                    self.replace_all_enabled(),
                    SearchView::row_replace_rect(rect, dpi),
                );
                if !visible {
                    item.state |= sidebar_accessibility::STATE_OFFSCREEN;
                }
                item
            }
            SearchChild::Summary => sidebar_accessibility::text_item(
                &self.shown_lines().0.unwrap_or_default(),
                self.summary_rect(client, dpi),
            ),
            SearchChild::Status => sidebar_accessibility::text_item(
                &self.shown_lines().1.unwrap_or_default(),
                SearchView::status_rect(client, dpi),
            ),
            SearchChild::Result(row) => {
                let hit = self.results.get(row)?;
                let (rect, visible) =
                    sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, row);
                sidebar_accessibility::list_item(
                    &result_name(hit),
                    self.list.selected == Some(row),
                    focused,
                    rect,
                    visible,
                )
            }
        })
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let field = SearchView::field_rect(client, dpi);
        let (summary, status) = self.shown_lines();
        let row_button = self.row_replace_child().filter(|&row| {
            let (rect, _) =
                sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, row);
            inside(SearchView::row_replace_rect(rect, dpi), point)
        });
        let replace = self.replace_field_shown();
        let child = if self.box_shown() && inside(field, point) {
            option_toggles::hit(&option_toggles::toggle_rects(field, dpi), point)
                .map_or(SearchChild::Box, SearchChild::Toggle)
        } else if self.box_shown() && inside(SearchView::chevron_rect(client, dpi), point) {
            SearchChild::Chevron
        } else if replace && inside(SearchView::replace_all_rect(client, dpi), point) {
            SearchChild::ReplaceAll
        } else if replace && inside(SearchView::replace_field_rect(client, dpi), point) {
            SearchChild::ReplaceField
        } else if summary.is_some() && inside(self.summary_rect(client, dpi), point) {
            SearchChild::Summary
        } else if status.is_some() && inside(SearchView::status_rect(client, dpi), point) {
            SearchChild::Status
        } else if let Some(row) = row_button {
            SearchChild::RowReplace(row)
        } else {
            SearchChild::Result(self.row_under(point, client, dpi)?)
        };
        self.child_index(child)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.child_index(SearchChild::Result(self.list.selected?))
    }

    /// Selects a result. The row button's child scrolls its row into view, so its default action
    /// (a click on its center) lands on it.
    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if let Some(SearchChild::Result(row) | SearchChild::RowReplace(row)) = self.child_at(index)
        {
            let area = self.list_area(client, dpi);
            self.list.select(row, area.bottom - area.top);
        }
    }

    fn accessible_identity(&self, index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        Some(match self.child_at(index)? {
            SearchChild::Box => sidebar_accessibility::identity_of(&"search box"),
            SearchChild::Toggle(option) => {
                sidebar_accessibility::identity_of(&("toggle", option_toggles::label(option)))
            }
            SearchChild::Chevron => sidebar_accessibility::identity_of(&"replace toggle"),
            SearchChild::ReplaceField => sidebar_accessibility::identity_of(&"replace field"),
            SearchChild::ReplaceAll => sidebar_accessibility::identity_of(&"replace all"),
            SearchChild::RowReplace(row) => {
                sidebar_accessibility::identity_of(&("replace in", &self.results.get(row)?.path))
            }
            SearchChild::Summary => sidebar_accessibility::identity_of(&"summary"),
            SearchChild::Status => sidebar_accessibility::identity_of(&"status"),
            SearchChild::Result(row) => {
                sidebar_accessibility::identity_of(&self.results.get(row)?.path)
            }
        })
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}
