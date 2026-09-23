//! The Search view (note-search spec §4): a search box over the text of the open notebook's
//! notes, a summary line, the matching notes with their folders and the first match, and a
//! status line. `text_search_host` runs the search; this view shows its batches. Enter or a click
//! opens a result following the preview-tab rules (sidebar spec §6.4). The box's placeholder is
//! painted the way the find bar paints its placeholder. FastPad has no ComCtl32 v6 manifest, so
//! `EM_SETCUEBANNER` would show nothing.

use crate::config::SidebarView;
use crate::library::model::same_path;
use crate::library::text_search::{self, Progress, RunEnd, TextHit, hit_cmp};
use crate::platform::{last_error, wide_null};
use crate::search::{MatchOptions, SearchOption, escape};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::notebook_view::LOAD_FAILED;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, inset, scale, text_height};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{self, ViewPaint, draw_text, point_of};
use crate::window::text_search_host::{self, SearchBatch};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, EndPaint, HBRUSH, HDC, InvalidateRect, PAINTSTRUCT, SetBkColor,
    SetTextColor,
};
use windows_sys::Win32::UI::Controls::{
    EM_GETMARGINS, EM_REPLACESEL, EM_SETSEL, EM_UNDO, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_RETURN, VK_UP,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetParent, GetWindowTextLengthW, GetWindowTextW,
    MoveWindow, SW_HIDE, SW_SHOWNA, SendMessageW, SetWindowTextW, ShowWindow, WM_CAPTURECHANGED,
    WM_CHAR, WM_CLEAR, WM_CUT, WM_GETFONT, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PASTE, WM_SETFONT,
    WM_SETTEXT, WM_UNDO, WS_CHILD,
};

pub(crate) const NO_NOTEBOOK: &str = "Open a notebook to search it.";
pub(crate) const NO_MATCH: &str = "No notes match.";
pub(crate) const LOADING: &str = "Loading\u{2026}";
pub(crate) const TOO_SHORT: &str = "Type at least 2 characters.";

const HEADER_AT_96_DPI: i32 = 38;
const ROW_AT_96_DPI: i32 = 26;
const PADDING_AT_96_DPI: i32 = 12;
const FIELD_MARGIN_AT_96_DPI: i32 = 8;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const DOCUMENT_GLYPH: &str = "\u{E8A5}";
const SEARCH_HOOK_ID: usize = 0x4650_5356;

/// The box's placeholder: "Search <notebook>", with the notebook's display name.
pub(crate) fn placeholder(notebook: Option<&Path>) -> String {
    notebook
        .map(|notebook| format!("Search {}", library_host::notebook_name(notebook)))
        .unwrap_or_else(|| "Search".to_owned())
}

/// The search's progress, as the summary and status lines read it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchState {
    /// Nothing typed, or only white space.
    #[default]
    Idle,
    /// One character: text search starts at two.
    TooShort,
    Running(Progress),
    Done {
        progress: Progress,
        capped: bool,
    },
    /// The pattern can't run. The previous results stay.
    PatternError(String),
}

/// The line shown instead of the results, if any. `failed` is a notebook whose load failed.
pub(crate) fn notice_text(notebook_open: bool, loaded: bool, failed: bool) -> Option<&'static str> {
    if !notebook_open {
        Some(NO_NOTEBOOK)
    } else if failed {
        Some(LOAD_FAILED)
    } else if !loaded {
        Some(LOADING)
    } else {
        None
    }
}

/// The summary line under the box and whether it is an error: "N notes" while results arrive
/// and when the search is done, "No notes match." for a finished search without one.
pub(crate) fn summary_text(state: &SearchState, results: usize) -> Option<(String, bool)> {
    match state {
        SearchState::Idle => None,
        SearchState::TooShort => Some((TOO_SHORT.to_owned(), false)),
        SearchState::PatternError(message) => Some((message.clone(), true)),
        SearchState::Running(_) => (results > 0).then(|| (note_count(results, false), false)),
        SearchState::Done { capped, .. } => {
            let text = if results == 0 {
                NO_MATCH.to_owned()
            } else {
                note_count(results, *capped)
            };
            Some((text, false))
        }
    }
}

/// The status line at the bottom: progress while the search runs, what it skipped once done.
/// `None` hides it.
pub(crate) fn status_text(state: &SearchState) -> Option<String> {
    match state {
        SearchState::Running(progress) => Some(format!(
            "Searching\u{2026} {} of {}",
            thousands(progress.visited),
            thousands(progress.total)
        )),
        SearchState::Done { progress, .. } => match progress.skipped_total() {
            0 => None,
            1 => Some("1 note wasn't searched".to_owned()),
            skipped => Some(format!("{} notes weren't searched", thousands(skipped))),
        },
        _ => None,
    }
}

fn note_count(count: usize, capped: bool) -> String {
    if capped {
        format!("{}+ notes", thousands(text_search::RESULT_CAP))
    } else if count == 1 {
        "1 note".to_owned()
    } else {
        format!("{} notes", thousands(count))
    }
}

/// `value` with a comma between each group of three digits.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
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
    /// The options the results are for: the view's options when the last search began. The find
    /// bar is seeded from `query` and these (Task 7).
    pub(crate) run_options: MatchOptions,
    /// Sorted by `hit_cmp`, at most `text_search::RESULT_CAP`.
    pub(crate) results: Vec<TextHit>,
    /// Match case, whole word and regex, for the session.
    pub(crate) options: MatchOptions,
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
            run_options: MatchOptions::default(),
            results: Vec::new(),
            options: MatchOptions::default(),
            search: SearchState::Idle,
            replace_on_batch: false,
            restore: None,
            selected_by_batch: None,
            list: RowListState::new(scale(ROW_AT_96_DPI, dpi)),
            placeholder: placeholder(None),
            thumb_grab: None,
            order: 0,
        }
    }

    /// The painted search field, border included.
    pub(crate) fn field_rect(client: RECT, dpi: u32) -> RECT {
        let margin = scale(FIELD_MARGIN_AT_96_DPI, dpi);
        let height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
        let top = client.top + (scale(HEADER_AT_96_DPI, dpi) - height) / 2;
        RECT {
            left: client.left + margin,
            top,
            right: (client.right - margin).max(client.left + margin),
            bottom: top + height,
        }
    }

    /// Where the results are: everything under the header.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        RECT {
            top: (client.top + scale(HEADER_AT_96_DPI, dpi)).min(client.bottom),
            ..client
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
        summary_text(&self.search, self.results.len())
    }

    pub(crate) fn status_line(&self) -> Option<String> {
        status_text(&self.search)
    }

    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        unsafe {
            fill(paint.hdc, client, paint.background);
            if self.notebook.is_some() {
                let field = Self::field_rect(client, dpi);
                fill(paint.hdc, field, palette.selection_background);
                fill(paint.hdc, inset(field, 1), palette.editor_background);
            }
            // One line in place of the list: the notice, or the summary while there is no row.
            // Task 5 gives the summary and the status line places of their own.
            let line_text = self.notice().map(str::to_owned).or_else(|| {
                self.results
                    .is_empty()
                    .then(|| self.summary().map(|(text, _)| text))
                    .flatten()
            });
            if let Some(text) = line_text {
                let area = self.list_area(client, dpi);
                let status_line = RECT {
                    left: client.left + pad,
                    top: area.top,
                    right: client.right - pad,
                    bottom: area.top + scale(ROW_AT_96_DPI, dpi),
                };
                draw_text(
                    paint.hdc,
                    &text,
                    status_line,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
                return;
            }
        }
        let area = self.list_area(client, dpi);
        row_list::paint(
            paint.hdc,
            area,
            &self.list,
            &palette,
            paint.focused,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }

    /// One result: the file icon, the name, and its folder in dim text.
    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let Some(result) = self.results.get(index) else {
            return;
        };
        let dpi = paint.dpi;
        let palette = paint.palette;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let glyph = RECT {
            left: rect.left + pad,
            right: rect.left + pad + scale(GLYPH_AT_96_DPI, dpi),
            ..rect
        };
        let foreground = row_foreground(look, &palette);
        // Over the focused selection, dim text takes the selection's text color to stay legible.
        let muted = if look.selected && look.focused {
            foreground
        } else {
            palette.muted_foreground
        };
        let text = RECT {
            left: glyph.right + scale(GAP_AT_96_DPI, dpi),
            right: rect.right - pad,
            ..rect
        };
        unsafe {
            draw_text(
                hdc,
                DOCUMENT_GLYPH,
                glyph,
                paint.fonts.glyph,
                muted,
                line | DT_CENTER,
            );
            let width = draw_text(
                hdc,
                &result.name,
                text,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if !result.folder.is_empty() {
                draw_text(
                    hdc,
                    &result.folder,
                    RECT {
                        left: text.left + width + scale(GAP_AT_96_DPI, dpi),
                        ..text
                    },
                    paint.fonts.text,
                    muted,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
        }
    }

    fn row_under(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        self.list
            .row_at(point.y - area.top)
            .filter(|&index| index < self.results.len())
    }

    /// Where on the scroll thumb a press at `point` landed, if it landed on it.
    fn thumb_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<i32> {
        let area = self.list_area(client, dpi);
        self.list.thumb_hit(
            point.x - area.left,
            point.y - area.top,
            area.right - area.left,
            height(area),
        )
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

/// `EN_CHANGE` from the box. A query that can run restarts the debounce and nothing else. One
/// that can't (empty, all white space or one character) cancels the search and clears the list.
pub(crate) fn query_changed(hwnd: HWND) {
    let Some(edit) = with_view(hwnd, |view| view.edit).flatten() else {
        return;
    };
    let query = window_text(edit);
    if text_search_host::searchable(&query) {
        text_search_host::schedule(hwnd);
        return;
    }
    text_search_host::cancel(hwnd);
    let state = if query.trim().is_empty() {
        SearchState::Idle
    } else {
        SearchState::TooShort
    };
    let Some(panel) = with_view(hwnd, |view| {
        view.clear_results();
        view.query = query;
        view.search = state;
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
}

/// Part of `side_panel::refresh`. A new notebook cancels the search and clears the query and the
/// results. The same notebook runs the query again if its notes changed, or, while the view is
/// hidden, checks once it shows again.
pub(crate) fn library_changed(hwnd: HWND) {
    let notebook = library_host::folder(hwnd);
    let loaded = library_host::with_state(hwnd, |_| ()).is_some();
    let failed = library_host::load_failed(hwnd);
    let Some((edit, changed)) = with_view(hwnd, |view| {
        let changed = match (&view.notebook, &notebook) {
            (Some(old), Some(new)) => !same_path(old, new),
            (None, None) => false,
            _ => true,
        };
        view.loaded = loaded;
        view.failed = failed;
        if changed {
            view.notebook = notebook.clone();
            view.placeholder = placeholder(notebook.as_deref());
            view.clear_results();
            view.query.clear();
            view.search = SearchState::Idle;
            view.stale = false;
        }
        (view.edit, changed)
    }) else {
        return;
    };
    if changed {
        text_search_host::forget(hwnd);
        // Clearing the box sends EN_CHANGE, which leaves the view idle.
        if let Some(edit) = edit {
            let empty = wide_null("");
            unsafe {
                SetWindowTextW(edit, empty.as_ptr());
                InvalidateRect(edit, std::ptr::null(), 1);
            }
        }
        layout(hwnd);
    } else if side_panel::current_view(hwnd) == SidebarView::Search {
        text_search_host::library_changed(hwnd);
    } else {
        with_view(hwnd, |view| view.stale = true);
    }
}

/// The box's text and the options, or `None` before the box exists.
pub(crate) fn current_query(hwnd: HWND) -> Option<(String, MatchOptions)> {
    let (edit, options) = with_view(hwnd, |view| Some((view.edit?, view.options))).flatten()?;
    // Read with nothing of the App borrowed: WM_GETTEXT goes through the box's subclass.
    Some((window_text(edit), options))
}

/// The query and options the shown results ran with, or `None` without a view.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Task 7 seeds the find bar from it")
)]
pub(crate) fn run_query(hwnd: HWND) -> Option<(String, MatchOptions)> {
    with_view(hwnd, |view| (view.query.clone(), view.run_options))
}

/// `text_search_host::run_now` started a search for `query` over `total` notes.
pub(crate) fn begin_search(hwnd: HWND, query: &str, total: usize) {
    if let Some(panel) = with_view(hwnd, |view| {
        view.begin(query, total);
        view.panel
    }) {
        invalidate(panel);
    }
}

/// A batch of the current search (`text_search_host::batch_arrived`). The panel repaints only if
/// something it shows changed.
pub(crate) fn apply_batch(hwnd: HWND, batch: SearchBatch) {
    let Some(panel) = with_view(hwnd, |view| view.panel) else {
        return;
    };
    let (client, dpi) = geometry(panel);
    let changed = with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        view.apply(batch, height(area))
    })
    .unwrap_or(false);
    if changed {
        invalidate(panel);
    }
}

/// Shows a pattern's error in place of the summary, keeping the results, or clears it.
pub(crate) fn set_pattern_error(hwnd: HWND, error: Option<String>) {
    let Some(panel) = with_view(hwnd, |view| {
        match error {
            Some(message) => view.search = SearchState::PatternError(message),
            None if matches!(view.search, SearchState::PatternError(_)) => {
                view.search = SearchState::Idle;
            }
            None => {}
        }
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only `show_with_query` calls it, until Ctrl+Shift+F does (Task 6)"
    )
)]
pub(crate) fn options(hwnd: HWND) -> MatchOptions {
    with_view(hwnd, |view| view.options).unwrap_or_default()
}

/// Flips `option` and runs the query again at once.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the toggle buttons and Alt keys call it (Task 5)")
)]
pub(crate) fn toggle_option(hwnd: HWND, option: SearchOption) {
    let Some(panel) = with_view(hwnd, |view| {
        view.options = view.options.toggled(option);
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
    text_search_host::run_now(hwnd);
}

/// Ctrl+Shift+F with a one-line selection: `text` replaces the box's text (escaped first when
/// regex is on), all of it selected, and the search runs at once. The caller shows the view.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Ctrl+Shift+F calls it (Task 6)")
)]
pub(crate) fn show_with_query(hwnd: HWND, text: &str) {
    let text = if options(hwnd).regex {
        escape(text)
    } else {
        text.to_owned()
    };
    let Some(edit) = ensure_edit(hwnd) else {
        return;
    };
    let wide = wide_null(&text);
    // Setting the text sends EN_CHANGE, which starts the debounce; `run_now` replaces it.
    unsafe {
        SetWindowTextW(edit, wide.as_ptr());
        SendMessageW(edit, EM_SETSEL, 0, -1);
    }
    text_search_host::run_now(hwnd);
}

/// The listed notes' paths, relative to the notebook, in list order.
pub(crate) fn result_paths(hwnd: HWND) -> Vec<PathBuf> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| result.path.clone())
            .collect()
    })
    .unwrap_or_default()
}

/// The search box, made now if the view has none yet. A failure is reported once.
fn ensure_edit(hwnd: HWND) -> Option<HWND> {
    let (panel, edit, failed) = with_view(hwnd, |view| (view.panel, view.edit, view.edit_failed))?;
    if edit.is_some() || failed {
        return edit;
    }
    // Made with nothing of the App borrowed: creating the Edit sends messages to the panel.
    match create_edit(panel) {
        Ok(edit) => {
            if with_view(hwnd, |view| view.edit = Some(edit)).is_none() {
                unsafe { DestroyWindow(edit) };
                return None;
            }
            Some(edit)
        }
        Err(error) => {
            with_view(hwnd, |view| view.edit_failed = true);
            super::main_window::push_notice(
                hwnd,
                format!("FastPad could not show the search box: {error}"),
            );
            None
        }
    }
}

/// A hidden search box inside `panel`.
fn create_edit(panel: HWND) -> crate::Result<HWND> {
    let edit = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
    if unsafe { SetWindowSubclass(edit, Some(search_edit_proc), SEARCH_HOOK_ID, 0) } == 0 {
        let error = last_error();
        unsafe {
            DestroyWindow(edit);
        }
        return Err(error);
    }
    Ok(edit)
}

/// Places the box in the header and shows it while the Search view shows a notebook, making it
/// the first time. Part of `side_panel::layout`, and run whenever the view or the notebook
/// changes.
pub(crate) fn layout(hwnd: HWND) {
    let Some(has_notebook) = with_view(hwnd, |view| view.notebook.is_some()) else {
        return;
    };
    let show = has_notebook && side_panel::current_view(hwnd) == SidebarView::Search;
    let edit = if show {
        ensure_edit(hwnd)
    } else {
        with_view(hwnd, |view| view.edit).flatten()
    };
    let Some(edit) = edit else {
        return;
    };
    let panel = unsafe { GetParent(edit) };
    let (client, dpi) = geometry(panel);
    let text_font = super::main_window::ui_fonts(hwnd).text;
    unsafe {
        if !text_font.is_null() {
            SendMessageW(edit, WM_SETFONT, text_font as WPARAM, 0);
        }
    }
    let field = SearchView::field_rect(client, dpi);
    let text = text_height(edit, text_font).clamp(1, (field.bottom - field.top - 2).max(1));
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    let top = field.top + (field.bottom - field.top - text) / 2;
    unsafe {
        MoveWindow(
            edit,
            field.left + inset_x,
            top,
            (field.right - field.left - 2 * inset_x).max(0),
            text,
            1,
        );
    }
    with_view(hwnd, |view| {
        view.list.row_height = scale(ROW_AT_96_DPI, dpi)
    });
    if show {
        unsafe {
            ShowWindow(edit, SW_SHOWNA);
        }
    } else {
        hide_box(edit);
    }
}

/// Hides the box. A hidden window keeps the keyboard focus, so a focused box hands it to the
/// panel (which `side_panel` moves on to the editor when the panel closes).
fn hide_box(edit: HWND) {
    unsafe {
        if GetFocus() == edit {
            SetFocus(GetParent(edit));
        }
        ShowWindow(edit, SW_HIDE);
    }
}

/// `side_panel::show_view` switched to Search. `focus` puts the caret in the box (Ctrl+K). A
/// library change while the view was hidden runs the query again now.
pub(crate) fn shown(hwnd: HWND, focus: bool) {
    layout(hwnd);
    if with_view(hwnd, |view| std::mem::take(&mut view.stale)).unwrap_or(false) {
        text_search_host::library_changed(hwnd);
    }
    let Some((panel, edit, has_notebook)) = with_view(hwnd, |view| {
        (view.panel, view.edit, view.notebook.is_some())
    }) else {
        return;
    };
    if !focus {
        return;
    }
    unsafe {
        match edit {
            Some(edit) if has_notebook => {
                SetFocus(edit);
                SendMessageW(edit, EM_SETSEL, 0, -1);
            }
            _ => {
                SetFocus(panel);
            }
        }
    }
}

/// `side_panel::show_view` switched away from Search. The query stays.
pub(crate) fn hidden(hwnd: HWND) {
    if let Some(edit) = with_view(hwnd, |view| view.edit).flatten() {
        hide_box(edit);
    }
}

/// The panel's `WM_PAINT` while the Search view shows.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    with_view(hwnd, |view| view.set_colors(paint.palette));
    if let Some(app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.search.paint(paint);
    }
}

/// `WM_CTLCOLOREDIT` for the box.
pub(crate) fn control_color(hwnd: HWND, dc: HDC) -> HBRUSH {
    with_view(hwnd, |view| {
        unsafe {
            SetTextColor(dc, view.colors.editor_foreground);
            SetBkColor(dc, view.colors.editor_background);
        }
        view.brush
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Opens the selected result, or the first one, as the preview tab or a normal tab.
pub(crate) fn open_selected(hwnd: HWND, mode: OpenMode, focus_editor: bool) {
    let Some(path) = with_view(hwnd, |view| {
        let index = view.list.selected.unwrap_or(0);
        view.results.get(index).map(|result| result.path.clone())
    })
    .flatten() else {
        return;
    };
    open_result(hwnd, &path, mode, focus_editor);
}

fn open_result(hwnd: HWND, relative: &Path, mode: OpenMode, focus_editor: bool) {
    let Some(folder) = library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    if let Err(error) = super::main_window::open_note(hwnd, &path, mode, focus_editor) {
        super::main_window::push_notice(
            hwnd,
            format!("FastPad could not open {}: {error}", path.display()),
        );
    }
}

/// Down from the box: the focus moves into the results.
fn enter_results(hwnd: HWND, panel: HWND) {
    let (client, dpi) = geometry(panel);
    let has_results = with_view(hwnd, |view| {
        if view.results.is_empty() {
            return false;
        }
        let area = view.list_area(client, dpi);
        let index = view.list.selected.unwrap_or(0);
        view.list.select(index, height(area));
        true
    })
    .unwrap_or(false);
    if has_results {
        unsafe {
            SetFocus(panel);
        }
        invalidate(panel);
    }
}

/// Whether panel point (`x`, `y`) is on the painted search field, which is client area, not
/// window caption. The field shows only while a notebook is open.
pub(crate) fn header_hit(hwnd: HWND, panel: HWND, x: i32, y: i32) -> bool {
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| view.notebook.is_some()).unwrap_or(false)
        && inside(SearchView::field_rect(client, dpi), POINT { x, y })
}

/// A press on the field's padding, outside the box itself, puts the caret in the box. Reports
/// whether the press was on the field.
fn field_pressed(hwnd: HWND, panel: HWND, at: POINT) -> bool {
    if !header_hit(hwnd, panel, at.x, at.y) {
        return false;
    }
    // Focused with nothing of the App borrowed: SetFocus sends focus messages.
    if let Some(edit) = with_view(hwnd, |view| view.edit).flatten() {
        unsafe {
            SetFocus(edit);
        }
    }
    true
}

/// Input for the Search view's result list. `None` leaves the message to the panel.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (client, dpi) = geometry(panel);
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let at = point(lparam);
            let changed = with_view(hwnd, |view| {
                if let Some(grab) = view.thumb_grab {
                    let area = view.list_area(client, dpi);
                    return view.list.drag_thumb(grab, at.y - area.top, height(area));
                }
                let hover = view.row_under(at, client, dpi);
                view.list.set_hover(hover)
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            if with_view(hwnd, |view| view.list.set_hover(None)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let at = point(lparam);
            if field_pressed(hwnd, panel, at) {
                return Some(0);
            }
            // A press on the scroll thumb drags it, as in the Notebook view.
            let grabbed = message == WM_LBUTTONDOWN
                && with_view(hwnd, |view| {
                    view.thumb_grab = view.thumb_at(at, client, dpi);
                    view.thumb_grab.is_some()
                })
                .unwrap_or(false);
            if grabbed {
                unsafe {
                    SetCapture(panel);
                }
                return Some(0);
            }
            let path = with_view(hwnd, |view| {
                let index = view.row_under(at, client, dpi)?;
                let area = view.list_area(client, dpi);
                view.list.select(index, height(area));
                Some(view.results[index].path.clone())
            })
            .flatten();
            invalidate(panel);
            if let Some(path) = path {
                let mode = if message == WM_LBUTTONDBLCLK {
                    OpenMode::Permanent
                } else {
                    OpenMode::Preview
                };
                // A mouse click moves the focus to the editor (spec §6.4).
                open_result(hwnd, &path, mode, true);
            }
            Some(0)
        }
        WM_LBUTTONUP => {
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| view.thumb_grab = None);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            let scrolled = with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.wheel(delta, lines, height(area))
            })
            .unwrap_or(false);
            if scrolled {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if key == VK_RETURN {
                if ctrl {
                    open_selected(hwnd, OpenMode::Permanent, true);
                } else {
                    // Enter keeps the focus in the list, so arrows and Enter browse (spec §6.4).
                    open_selected(hwnd, OpenMode::Preview, false);
                }
                return Some(0);
            }
            let at_top = with_view(hwnd, |view| {
                view.list.selected.is_none_or(|index| index == 0)
            })
            .unwrap_or(true);
            if key == VK_UP && at_top {
                if let Some(edit) = with_view(hwnd, |view| view.edit).flatten() {
                    unsafe {
                        SetFocus(edit);
                    }
                }
                return Some(0);
            }
            let movement = ListKey::from_virtual_key(u32::from(key))?;
            with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.move_selection(movement, height(area))
            });
            invalidate(panel);
            Some(0)
        }
        // Typing in the list goes on in the box.
        WM_CHAR if (wparam as u32) >= 0x20 && wparam as u32 != 0x7f => {
            let edit = with_view(hwnd, |view| view.edit).flatten()?;
            unsafe {
                SetFocus(edit);
                SendMessageW(edit, WM_CHAR, wparam, lparam);
            }
            Some(0)
        }
        _ => None,
    }
}

/// The box's placeholder, painted where typed text starts.
fn paint_placeholder(hwnd: HWND, edit: HWND) -> bool {
    let Some((text, colors)) = with_view(hwnd, |view| (view.placeholder.clone(), view.colors))
    else {
        return false;
    };
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(edit, &mut paint) };
    if dc.is_null() {
        return true;
    }
    unsafe {
        let mut client = RECT::default();
        GetClientRect(edit, &mut client);
        fill(dc, client, colors.editor_background);
        let font = SendMessageW(edit, WM_GETFONT, 0, 0);
        client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
        draw_text(
            dc,
            &text,
            client,
            font as _,
            colors.muted_foreground,
            DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        EndPaint(edit, &paint);
    }
    true
}

unsafe extern "system" fn search_edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // The last message: drop the hook and let the Edit finish; nothing else is looked up.
        unsafe {
            RemoveWindowSubclass(hwnd, Some(search_edit_proc), SEARCH_HOOK_ID);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let panel = unsafe { GetParent(hwnd) };
    let main = unsafe { GetParent(panel) };
    // A single-line Edit beeps at Enter and Escape characters; both are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b) {
        return 0;
    }
    if message == WM_PAINT
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
        && paint_placeholder(main, hwnd)
    {
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
        match wparam as u16 {
            VK_DOWN | VK_NEXT => {
                enter_results(main, panel);
                return 0;
            }
            VK_RETURN if ctrl => {
                open_selected(main, OpenMode::Permanent, true);
                return 0;
            }
            VK_RETURN => {
                open_selected(main, OpenMode::Preview, false);
                return 0;
            }
            VK_ESCAPE => {
                super::main_window::focus_content(main);
                return 0;
            }
            _ => {}
        }
    }
    // The Edit repaints only the text it changes; the placeholder must go, or come back, whole.
    let edits_text = matches!(
        message,
        WM_CHAR
            | WM_KEYDOWN
            | WM_PASTE
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_SETTEXT
            | EM_UNDO
            | EM_REPLACESEL
    );
    let was_empty = edits_text && unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if edits_text && was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    result
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

impl crate::window::sidebar_accessibility::AccessibleView for SearchView {
    /// One list item per result. The search box is a real `Edit` with its own MSAA object.
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        self.results.len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        let result = self.results.get(index)?;
        let (rect, visible) = crate::window::sidebar_accessibility::row_rect(
            self.list_area(client, dpi),
            &self.list,
            index,
        );
        let name = if result.folder.is_empty() {
            result.name.clone()
        } else {
            format!("{}, {}", result.name, result.folder)
        };
        Some(crate::window::sidebar_accessibility::list_item(
            &name,
            self.list.selected == Some(index),
            focused,
            rect,
            visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        self.row_under(point, client, dpi)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.list.selected
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if index < self.results.len() {
            let area = self.list_area(client, dpi);
            self.list.select(index, area.bottom - area.top);
        }
    }

    fn accessible_identity(&self, index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        self.results
            .get(index)
            .map(|result| crate::window::sidebar_accessibility::identity_of(&result.path))
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LOADING, NO_MATCH, NO_NOTEBOOK, SearchState, SearchView, TOO_SHORT, notice_text,
        placeholder, status_text, summary_text,
    };
    use crate::library::text_search::{Progress, RunEnd, TextHit};
    use crate::search::{MatchOptions, SearchOption, Snippet};
    use crate::window::notebook_view::LOAD_FAILED;
    use crate::window::text_search_host::SearchBatch;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_notice_explains_an_empty_list() {
        // Break caught: a blank Search view with no notebook open, or "No notes match." while the
        // notebook is still loading.
        assert_eq!(notice_text(false, false, false), Some(NO_NOTEBOOK));
        assert_eq!(notice_text(true, false, false), Some(LOADING));
        assert_eq!(notice_text(true, false, true), Some(LOAD_FAILED));
        assert_eq!(notice_text(true, true, false), None);
    }

    #[test]
    fn the_placeholder_names_the_open_notebook() {
        // Break caught: the box saying "Search" with no hint of which notebook it searches.
        assert_eq!(
            placeholder(Some(Path::new(r"C:\Users\me\Work"))),
            "Search Work"
        );
        assert_eq!(placeholder(None), "Search");
    }

    #[test]
    fn the_summary_counts_notes_and_says_when_nothing_matches() {
        // Break caught: "No notes match." before the search finished, a count without its
        // thousands separator, "1 notes", the cap shown as "500 notes", or a regex error shown as
        // ordinary text.
        let done = |capped| SearchState::Done {
            progress: Progress::default(),
            capped,
        };
        let running = SearchState::Running(Progress::default());
        let line = |text: &str, error| Some((text.to_owned(), error));
        assert_eq!(summary_text(&SearchState::Idle, 0), None);
        assert_eq!(
            summary_text(&SearchState::TooShort, 0),
            line(TOO_SHORT, false)
        );
        assert_eq!(summary_text(&running, 0), None, "nothing found yet");
        assert_eq!(summary_text(&running, 1), line("1 note", false));
        assert_eq!(summary_text(&done(false), 0), line(NO_MATCH, false));
        assert_eq!(
            summary_text(&done(false), 1_234),
            line("1,234 notes", false)
        );
        assert_eq!(summary_text(&done(true), 500), line("500+ notes", false));
        assert_eq!(
            summary_text(&SearchState::PatternError("Unclosed group".to_owned()), 3),
            line("Unclosed group", true)
        );
    }

    #[test]
    fn the_status_line_shows_progress_and_what_was_skipped() {
        // Break caught: no progress while a big notebook is searched, a status line left up after
        // a clean search, or a skipped count with the wrong grammar.
        let progress = Progress {
            visited: 4_120,
            total: 9_800,
            skipped: [0; 4],
        };
        assert_eq!(
            status_text(&SearchState::Running(progress)).as_deref(),
            Some("Searching\u{2026} 4,120 of 9,800")
        );
        let done = |skipped| SearchState::Done {
            progress: Progress {
                visited: 9,
                total: 9,
                skipped,
            },
            capped: false,
        };
        assert_eq!(
            status_text(&done([0; 4])),
            None,
            "hidden when nothing was skipped"
        );
        assert_eq!(
            status_text(&done([0, 1, 0, 0])).as_deref(),
            Some("1 note wasn't searched")
        );
        assert_eq!(
            status_text(&done([2, 0, 1, 1])).as_deref(),
            Some("4 notes weren't searched")
        );
        assert_eq!(status_text(&SearchState::Idle), None);
        assert_eq!(
            status_text(&SearchState::PatternError("x".to_owned())),
            None
        );
    }

    fn hit(name: &str, folder: &str) -> TextHit {
        let file = format!("{name}.md");
        let path = if folder.is_empty() {
            PathBuf::from(file)
        } else {
            Path::new(folder).join(file)
        };
        TextHit {
            path,
            name: name.to_owned(),
            folder: folder.to_owned(),
            snippet: Snippet {
                text: format!("{name} needle"),
                highlight: name.len() + 1..name.len() + 7,
            },
            stamp: None,
        }
    }

    fn batch(hits: Vec<TextHit>, visited: usize, end: Option<RunEnd>) -> SearchBatch {
        SearchBatch {
            generation: 1,
            hits,
            progress: Progress {
                visited,
                total: 10,
                skipped: [0; 4],
            },
            end,
        }
    }

    fn rows(view: &SearchView) -> Vec<(String, String)> {
        view.results
            .iter()
            .map(|result| (result.name.clone(), result.folder.clone()))
            .collect()
    }

    fn row(name: &str, folder: &str) -> (String, String) {
        (name.to_owned(), folder.to_owned())
    }

    const HEIGHT: i32 = 400;

    #[test]
    fn batches_are_inserted_in_order_and_keep_the_selection_by_path() {
        // Break caught: rows appended in arrival order, or a row arriving above the selection
        // moving it, so Enter opens a different note than the one highlighted.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        assert!(view.apply(batch(vec![hit("m", ""), hit("c", "")], 4, None), HEIGHT));
        assert_eq!(rows(&view), [row("c", ""), row("m", "")]);
        assert_eq!(view.list.selected, Some(0), "the first result is selected");
        view.list.select(1, HEIGHT);
        let more = vec![hit("a", ""), hit("b", "sub"), hit("b", "")];
        assert!(view.apply(batch(more, 8, None), HEIGHT));
        assert_eq!(
            rows(&view),
            [
                row("a", ""),
                row("b", ""),
                row("b", "sub"),
                row("c", ""),
                row("m", "")
            ],
            "natural name order, then the root before a folder"
        );
        assert_eq!(view.list.selected, Some(4), "still m");
        assert!(view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT));
        assert_eq!(
            view.search,
            SearchState::Done {
                progress: Progress {
                    visited: 10,
                    total: 10,
                    skipped: [0; 4]
                },
                capped: false
            }
        );
    }

    #[test]
    fn a_rerun_keeps_the_results_until_its_first_batch_and_a_new_query_starts_empty() {
        // Break caught: the list blanking on every library change, a new query showing the old
        // query's rows until its own arrive, or the selected note lost across either.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
        view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
        view.list.select(2, HEIGHT);

        view.begin("needle", 10);
        assert_eq!(rows(&view).len(), 3, "kept until the first batch");
        assert!(matches!(view.search, SearchState::Running(_)));
        view.apply(batch(vec![hit("b", ""), hit("c", "")], 5, None), HEIGHT);
        assert_eq!(
            rows(&view),
            [row("b", ""), row("c", "")],
            "the first batch replaces them"
        );
        assert_eq!(view.list.selected, Some(1), "c is still selected");

        view.begin("needles", 10);
        assert!(view.results.is_empty(), "a new query starts empty");
        assert_eq!(view.list.selected, None);
        view.apply(batch(vec![hit("a", ""), hit("c", "")], 10, None), HEIGHT);
        assert_eq!(
            view.list.selected,
            Some(1),
            "the remembered note is selected again when it arrives"
        );
    }

    #[test]
    fn a_reruns_empty_progress_batch_keeps_the_old_results() {
        // Break caught: the list blanking for a moment on every re-run, because the first
        // interval batch (progress only, no hits) replaced the rows before any hit arrived.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
        view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
        view.list.select(1, HEIGHT);
        view.begin("needle", 10);
        view.apply(batch(Vec::new(), 3, None), HEIGHT);
        assert_eq!(rows(&view).len(), 3, "an empty batch replaces nothing");
        assert_eq!(view.list.selected, Some(1));
        view.apply(batch(vec![hit("a", ""), hit("b", "")], 6, None), HEIGHT);
        assert_eq!(
            rows(&view),
            [row("a", ""), row("b", "")],
            "the first hit replaces them"
        );
        assert_eq!(view.list.selected, Some(1), "b is still remembered");

        view.begin("needle", 10);
        view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT);
        assert!(
            view.results.is_empty(),
            "a re-run that ends without hits empties the list"
        );
    }

    #[test]
    fn the_options_a_search_ran_with_are_kept_and_new_options_start_empty() {
        // Break caught: the find bar seeded (Task 7) with options toggled after the search ran,
        // or rows found without match case still listed while the match-case run starts.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        view.apply(
            batch(vec![hit("a", "")], 10, Some(RunEnd::Completed)),
            HEIGHT,
        );
        assert_eq!(view.run_options, MatchOptions::default());
        view.options = view.options.toggled(SearchOption::Case);
        assert!(!view.run_options.case, "not until a search runs with it");
        view.begin("needle", 10);
        assert!(view.run_options.case);
        assert!(view.results.is_empty(), "other options are a new search");
    }

    #[test]
    fn a_selection_the_user_moves_during_a_search_wins_over_the_remembered_one() {
        // Break caught: the remembered note arriving late and snatching the selection from the
        // row the user just moved to.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
        view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
        view.list.select(2, HEIGHT);
        view.begin("needles", 10);
        view.apply(batch(vec![hit("a", ""), hit("b", "")], 5, None), HEIGHT);
        assert_eq!(
            view.list.selected,
            Some(0),
            "c has not arrived: the first row"
        );
        view.list.select(1, HEIGHT);
        view.apply(batch(vec![hit("c", "")], 10, None), HEIGHT);
        assert_eq!(view.list.selected, Some(1), "b, which the user picked");
    }

    #[test]
    fn a_batch_that_changes_nothing_shown_asks_for_no_repaint() {
        // Break caught: an InvalidateRect for every empty batch of a long search.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        assert!(view.apply(batch(vec![hit("a", "")], 3, None), HEIGHT));
        assert!(!view.apply(batch(Vec::new(), 3, None), HEIGHT));
        assert!(
            view.apply(batch(Vec::new(), 4, None), HEIGHT),
            "the progress moved"
        );
    }
}
