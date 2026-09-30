//! The Search view (note-search spec §4): a search box over the text of the open notebook's
//! notes, a summary line, the matching notes with their folders and the first match, and a
//! status line. `text_search_host` runs the search; this view shows its batches. Enter or a click
//! opens a result following the preview-tab rules (sidebar spec §6.4). The box's placeholder is
//! painted the way the find bar paints its placeholder. FastPad has no ComCtl32 v6 manifest, so
//! `EM_SETCUEBANNER` would show nothing.

use crate::library::text_search::{Progress, RunEnd, TextHit, hit_cmp};
use crate::search::{MatchOptions, SearchOption};
use crate::window::design::metrics::scale;
use crate::window::icon_sets::images::IconImages;
use crate::window::palette::Palette;
use crate::window::row_list::RowListState;
use crate::window::side_panel::point_of;
use crate::window::text_search_host::SearchBatch;
use crate::window::tooltip::Tooltip;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, HBRUSH, InvalidateRect};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowTextLengthW, GetWindowTextW,
};

mod accessibility;
mod edits;
mod geometry;
mod input;
mod paint;
mod query;
mod text;

pub(crate) use accessibility::*;
pub(crate) use edits::*;
pub(crate) use input::*;
pub(crate) use paint::*;
pub(crate) use query::*;
pub(crate) use text::*;

pub(crate) const NO_NOTEBOOK: &str = "Open a notebook to search it.";
pub(crate) const NO_MATCH: &str = "No notes match.";
pub(crate) const LOADING: &str = "Loading\u{2026}";
pub(crate) const TOO_SHORT: &str = "Type at least 2 characters.";
/// The summary while a replace runs.
pub(crate) const REPLACING: &str = "Replacing\u{2026}";

const HEADER_AT_96_DPI: i32 = 38;
/// The summary line, the notice and the status line.
const LINE_AT_96_DPI: i32 = 22;
/// A result: two line slots with a little room above and below. The 12 px Segoe UI line is 16 px
/// tall at 96 DPI; the old one-line row was 26 px.
const ROW_AT_96_DPI: i32 = 42;
const ROW_LINE_AT_96_DPI: i32 = 18;
const ROW_INSET_AT_96_DPI: i32 = 3;
const PADDING_AT_96_DPI: i32 = 12;
const FIELD_MARGIN_AT_96_DPI: i32 = 8;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const SEARCH_HOOK_ID: usize = 0x4650_5356;
/// The status line's tooltip. The toggles are tools 0 to 2, in `SearchOption::ALL` order.
const STATUS_TOOL: usize = 3;
const TOOLTIP_WIDTH_AT_96_DPI: i32 = 300;
/// The chevron left of the search box that opens and closes the replace field (spec §11).
const CHEVRON_LEFT_AT_96_DPI: i32 = 4;
const CHEVRON_WIDTH_AT_96_DPI: i32 = 18;
const CHEVRON_GAP_AT_96_DPI: i32 = 2;
/// The replace field's row, under the header while the field is open.
const REPLACE_ROW_AT_96_DPI: i32 = 34;
/// Replace all, at the right of the replace field.
const REPLACE_ALL_WIDTH_AT_96_DPI: i32 = 26;
/// A result's replace button: a square at the row's right end.
const ROW_BUTTON_AT_96_DPI: i32 = 22;
/// Segoe MDL2 Assets: ChevronRight, ChevronDown, and Switch for both replace buttons.
const CHEVRON_CLOSED_GLYPH: &str = "\u{E76C}";
const CHEVRON_OPEN_GLYPH: &str = "\u{E70D}";
const REPLACE_GLYPH: &str = "\u{E8AB}";
const REPLACE_PLACEHOLDER: &str = "Replace";
const REPLACE_HOOK_ID: usize = 0x4650_5352;
/// The header buttons' and the row button's tooltips, after `STATUS_TOOL`.
const CHEVRON_TOOL: usize = 4;
const REPLACE_ALL_TOOL: usize = 5;
const ROW_REPLACE_TOOL: usize = 6;
/// The clear-search button's tooltip.
const CLEAR_TOOL: usize = 7;
/// The clear button's gap to the toggles, and Segoe MDL2 Assets' Cancel glyph.
const CLEAR_GAP_AT_96_DPI: i32 = 2;
const CLEAR_GLYPH: &str = "\u{E711}";
const TITLE: &str = "SEARCH";
/// The title's left inset, as the Notebook view's.
const TITLE_INSET_AT_96_DPI: i32 = 12;
/// A result's file icon, as the tree's.
const ICON_AT_96_DPI: i32 = 16;

/// A painted button in the Search view's header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderButton {
    Chevron,
    ReplaceAll,
    /// Clears the query and the results.
    Clear,
}

/// A rectangle as an array, so the tooltip tools can be compared (`RECT` has no `PartialEq`).
const fn edges(rect: RECT) -> [i32; 4] {
    [rect.left, rect.top, rect.right, rect.bottom]
}

const fn rect_of(edges: [i32; 4]) -> RECT {
    RECT {
        left: edges[0],
        top: edges[1],
        right: edges[2],
        bottom: edges[3],
    }
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

const fn height(rect: RECT) -> i32 {
    let height = rect.bottom - rect.top;
    if height > 0 { height } else { 0 }
}

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    buffer.truncate(copied.max(0) as usize);
    String::from_utf16_lossy(&buffer)
}

/// How often the summary and status lines may announce a change while a search runs (spec §10).
const ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// What screen readers last heard the summary and status lines say, and when.
#[derive(Debug, Default)]
pub(crate) struct Spoken {
    summary: String,
    status: String,
    at: Option<std::time::Instant>,
}

/// One of the Search view's MSAA children (see the view's `AccessibleView` impl).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SearchChild {
    Box,
    Toggle(SearchOption),
    Chevron,
    ReplaceField,
    ReplaceAll,
    Summary,
    Status,
    /// The replace button of the selected result `usize`.
    RowReplace(usize),
    Result(usize),
}

/// A result's accessible name (spec §10): "<name>, <folder>: <snippet>". A note at the
/// notebook's root has no folder, so it reads "<name>: <snippet>".
pub(crate) fn result_name(hit: &TextHit) -> String {
    if hit.folder.is_empty() {
        format!("{}: {}", hit.name, hit.snippet.text)
    } else {
        format!("{}, {}: {}", hit.name, hit.folder, hit.snippet.text)
    }
}

#[derive(Debug)]
pub(crate) struct SearchView {
    panel: HWND,
    /// The search box, made the first time the Search view shows a notebook (`layout`).
    edit: Option<HWND>,
    /// Making the box failed and was reported; it is not tried again.
    edit_failed: bool,
    brush: HBRUSH,
    colors: Palette,
    notebook: Option<PathBuf>,
    /// The library's state has loaded.
    loaded: bool,
    /// The notebook's load failed (`library_host::load_failed`).
    failed: bool,
    /// The library changed while the view was hidden: `shown` checks whether the query must run
    /// again.
    stale: bool,
    /// The query the results are for: the one the last search began with.
    pub(crate) query: String,
    /// The box's text, kept at each `EN_CHANGE` (`query_changed`), so screen readers read it
    /// without a `WM_GETTEXT` under the App borrow.
    box_text: String,
    /// What the summary and status lines last announced (`announce_lines`).
    spoken: Spoken,
    /// The options the results are for: the view's options when the last search began. The find
    /// bar is seeded from `query` and these (Task 7).
    pub(crate) run_options: MatchOptions,
    /// Sorted by `hit_cmp`, at most `text_search::RESULT_CAP`.
    pub(crate) results: Vec<TextHit>,
    /// Match case, whole word and regex, for the session.
    pub(crate) options: MatchOptions,
    /// The replace field is open (spec §11). Ctrl+Shift+H opens it, the chevron opens and closes
    /// it, and Ctrl+Shift+F leaves it as it is.
    replace_open: bool,
    /// The replace field, made the first time it opens.
    replace_edit: Option<HWND>,
    /// Making the replace field failed and was reported; it is not tried again.
    replace_edit_failed: bool,
    /// The replace field's text, kept at each `EN_CHANGE` (`replace_changed`), so the replace and
    /// screen readers read it without a `WM_GETTEXT` under the App borrow.
    replace_text: String,
    /// The result whose replace button is under the pointer.
    row_hover_button: Option<usize>,
    /// The header button under the pointer.
    header_hover: Option<HeaderButton>,
    /// A replace runs (`set_replacing`): the summary says so and no result opens.
    replacing: bool,
    /// The rows a row replace was asked for (`row_replace_requested`), for in-process tests.
    #[cfg(test)]
    row_replace_requests: Vec<usize>,
    /// How many times Replace all was asked for (`replace_all_requested`), for in-process tests.
    #[cfg(test)]
    replace_all_requests: usize,
    pub(crate) search: SearchState,
    /// The same query is running again: its first batch replaces the results.
    replace_on_batch: bool,
    /// The note selected when the running search began, selected again when it arrives.
    restore: Option<PathBuf>,
    /// The note the last batch left selected. A different selection at the next batch means the
    /// user moved it, and `restore` gives way.
    selected_by_batch: Option<PathBuf>,
    pub(crate) list: RowListState,
    placeholder: String,
    /// While the scroll thumb is dragged: how far below its top it was grabbed.
    thumb_grab: Option<i32>,
    /// Bumped whenever the results change (`AccessibleView::accessible_generation`).
    order: u64,
    /// The toggle under the pointer.
    toggle_hover: Option<SearchOption>,
    /// The toggles' and the status line's tooltip, made when the pointer first moves over the
    /// Search view.
    tooltip: Option<Tooltip>,
    /// The tooltip could not be made; it is not tried again.
    tooltip_failed: bool,
    /// The tools the tooltip has, so a pointer move changes them only when they differ.
    tools_shown: Vec<(usize, [i32; 4], String)>,
    /// The result rows' file-icon bitmaps, made on first paint (which only borrows the view).
    images: RefCell<IconImages>,
}

impl SearchView {
    /// The view for `panel`. Its search box waits until the view first shows a notebook.
    pub(crate) fn new(panel: HWND, dpi: u32) -> Self {
        let colors = Palette::neutral();
        Self {
            panel,
            edit: None,
            edit_failed: false,
            brush: unsafe { CreateSolidBrush(colors.editor_background) },
            colors,
            notebook: None,
            loaded: false,
            failed: false,
            stale: false,
            query: String::new(),
            box_text: String::new(),
            spoken: Spoken::default(),
            run_options: MatchOptions::default(),
            results: Vec::new(),
            options: MatchOptions::default(),
            replace_open: false,
            replace_edit: None,
            replace_edit_failed: false,
            replace_text: String::new(),
            row_hover_button: None,
            header_hover: None,
            replacing: false,
            #[cfg(test)]
            row_replace_requests: Vec::new(),
            #[cfg(test)]
            replace_all_requests: 0,
            search: SearchState::Idle,
            replace_on_batch: false,
            restore: None,
            selected_by_batch: None,
            list: RowListState::new(scale(ROW_AT_96_DPI, dpi)),
            placeholder: placeholder(None),
            thumb_grab: None,
            order: 0,
            toggle_hover: None,
            tooltip: None,
            tooltip_failed: false,
            tools_shown: Vec::new(),
            images: RefCell::new(IconImages::new()),
        }
    }

    fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        unsafe {
            DeleteObject(self.brush);
            self.brush = CreateSolidBrush(colors.editor_background);
        }
        self.colors = colors;
    }

    fn path_at(&self, index: usize) -> Option<PathBuf> {
        self.results.get(index).map(|result| result.path.clone())
    }

    fn position(&self, path: &Path) -> Option<usize> {
        self.results.iter().position(|result| result.path == path)
    }

    /// Empties the list and forgets its selection and scroll.
    fn clear_results(&mut self) {
        if !self.results.is_empty() {
            self.order = self.order.wrapping_add(1);
        }
        self.results.clear();
        self.list.set_count(0);
        self.list.top = 0;
        self.list.selected = None;
        self.replace_on_batch = false;
        self.restore = None;
        self.selected_by_batch = None;
    }

    /// A search for `query` over `total` notes began, with the view's options. The same query
    /// and options again keep the results until its first batch with a hit or its end; a new one
    /// clears them. Either way the selected note is remembered, to be selected again when it
    /// arrives.
    fn begin(&mut self, query: &str, total: usize) {
        let selected = self.list.selected.and_then(|index| self.path_at(index));
        if self.query == query && self.run_options == self.options && !self.results.is_empty() {
            self.replace_on_batch = true;
        } else {
            self.clear_results();
            self.query = query.to_owned();
        }
        self.run_options = self.options;
        self.restore = selected;
        self.selected_by_batch = None;
        self.search = SearchState::Running(Progress {
            total,
            ..Progress::default()
        });
    }

    /// Puts `batch`'s hits in sorted place, keeping the selection and the top row by path, for a
    /// list area `height` tall. Reports whether anything painted changed: a row in view, the
    /// selection, the scroll, the summary or the status line.
    fn apply(&mut self, batch: SearchBatch, height: i32) -> bool {
        let lines = (self.summary(), self.status_line());
        let before = (self.list.top, self.list.selected);
        // A re-run's rows stay until a batch brings a hit or the end: an interval batch with only
        // progress in it must not blank the list.
        let replacing = self.replace_on_batch && (!batch.hits.is_empty() || batch.end.is_some());
        if self.replace_on_batch && !replacing {
            // Only progress: the old rows and their selection stay as they are.
            self.search = SearchState::Running(batch.progress);
            return (self.summary(), self.status_line()) != lines;
        }
        let current = self.list.selected.and_then(|index| self.path_at(index));
        let (selected, top) = if replacing {
            self.replace_on_batch = false;
            // The note selected now, where the search began or where the user moved it since,
            // is selected again when it arrives.
            if current.is_some() {
                self.restore = current;
            }
            (None, None)
        } else {
            (current, self.path_at(self.list.top))
        };
        if !replacing && selected != self.selected_by_batch {
            // The user moved the selection since the last batch: it stays where they put it.
            self.restore = None;
        }
        let mut rows_changed = replacing;
        if replacing {
            self.results.clear();
            self.list.top = 0;
            self.list.selected = None;
        }
        let rows_in_view = (height.max(0) as usize).div_ceil(self.list.row_height.max(1) as usize);
        let in_view = self.list.top + rows_in_view;
        let arrived = !batch.hits.is_empty();
        for hit in batch.hits {
            match self.results.binary_search_by(|probe| hit_cmp(probe, &hit)) {
                Ok(index) => {
                    rows_changed |= index < in_view;
                    self.results[index] = hit;
                }
                Err(index) => {
                    rows_changed |= index < in_view;
                    self.results.insert(index, hit);
                }
            }
        }
        if arrived || replacing {
            self.order = self.order.wrapping_add(1);
        }
        self.list.set_count(self.results.len());
        if let Some(index) = top.and_then(|path| self.position(&path)) {
            self.list.top = index;
        }
        let restored = self.restore.as_deref().and_then(|path| self.position(path));
        if restored.is_some() {
            self.restore = None;
        }
        let index = restored
            .or_else(|| selected.and_then(|path| self.position(&path)))
            .or_else(|| (!self.results.is_empty()).then_some(0));
        match index {
            Some(index) => self.list.select(index, height),
            None => self.list.selected = None,
        }
        self.selected_by_batch = self.list.selected.and_then(|index| self.path_at(index));
        if batch.end.is_some() {
            self.restore = None;
        }
        self.search = match batch.end {
            None => SearchState::Running(batch.progress),
            Some(end) => SearchState::Done {
                progress: batch.progress,
                capped: end == RunEnd::Capped,
            },
        };
        rows_changed
            || (self.list.top, self.list.selected) != before
            || (self.summary(), self.status_line()) != lines
    }

    /// The line shown instead of the results.
    pub(crate) fn notice(&self) -> Option<&'static str> {
        notice_text(self.notebook.is_some(), self.loaded, self.failed)
    }

    pub(crate) fn summary(&self) -> Option<(String, bool)> {
        if self.replacing {
            return Some((REPLACING.to_owned(), false));
        }
        summary_text(&self.search, self.results.len())
    }

    pub(crate) fn status_line(&self) -> Option<String> {
        status_text(&self.search)
    }
}

impl Drop for SearchView {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.brush);
        }
    }
}

fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut SearchView) -> R) -> Option<R> {
    let mut app = unsafe { super::main_window::app_ptr(hwnd) }?;
    unsafe { app.as_mut() }
        .sidebar
        .as_mut()
        .map(|sidebar| f(&mut sidebar.search))
}

fn geometry(panel: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(panel, &mut client);
    }
    (client, unsafe { GetDpiForWindow(panel) }.max(96))
}

fn invalidate(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

fn point(lparam: LPARAM) -> POINT {
    let (x, y) = point_of(lparam);
    POINT { x, y }
}
/// The results listed, as (name, snippet text), for in-process tests.
#[cfg(test)]
pub(crate) fn shown_results(hwnd: HWND) -> Vec<(String, String)> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| (result.name.clone(), result.snippet.text.clone()))
            .collect()
    })
    .unwrap_or_default()
}

/// The notice shown instead of the results.
#[cfg(test)]
pub(crate) fn status(hwnd: HWND) -> Option<&'static str> {
    with_view(hwnd, |view| view.notice()).flatten()
}

#[cfg(test)]
pub(crate) fn summary(hwnd: HWND) -> Option<(String, bool)> {
    with_view(hwnd, |view| view.summary()).flatten()
}

#[cfg(test)]
pub(crate) fn search_state(hwnd: HWND) -> SearchState {
    with_view(hwnd, |view| view.search.clone()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn edit_hwnd(hwnd: HWND) -> Option<HWND> {
    with_view(hwnd, |view| view.edit).flatten()
}

/// Whether the replace field is open: Replace all and the rows' buttons need it.
pub(crate) fn replace_open(hwnd: HWND) -> bool {
    with_view(hwnd, |view| view.replace_open).unwrap_or(false)
}

/// The rows a row replace was asked for, and how many times Replace all was, since the view was
/// made.
#[cfg(test)]
pub(crate) fn replace_requests(hwnd: HWND) -> (Vec<usize>, usize) {
    with_view(hwnd, |view| {
        (view.row_replace_requests.clone(), view.replace_all_requests)
    })
    .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn replace_edit_hwnd(hwnd: HWND) -> Option<HWND> {
    with_view(hwnd, |view| view.replace_edit).flatten()
}

#[cfg(test)]
mod tests;
