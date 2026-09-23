//! The Search view (sidebar spec §8): a search box over the open notebook's note names, and the
//! matches with their folders. Enter or a click opens a match following the preview-tab rules
//! (§6.4). The box's placeholder is painted the way the find bar paints its placeholder. FastPad
//! has no ComCtl32 v6 manifest, so `EM_SETCUEBANNER` would show nothing.

use crate::config::SidebarView;
use crate::library::model::same_path;
use crate::library::name_search::{self, NameMatch};
use crate::platform::{last_error, wide_null};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, inset, scale, text_height};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{self, ViewPaint, draw_text, point_of};
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

pub(crate) const RESULT_LIMIT: usize = 500;
pub(crate) const NO_NOTEBOOK: &str = "Open a notebook to search it.";
pub(crate) const NO_MATCH: &str = "No notes match.";
pub(crate) const LOADING: &str = "Loading\u{2026}";

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

/// The line shown instead of results, if any.
pub(crate) fn status_text(
    notebook_open: bool,
    loaded: bool,
    query: &str,
    results: usize,
) -> Option<&'static str> {
    if !notebook_open {
        Some(NO_NOTEBOOK)
    } else if query.trim().is_empty() {
        None
    } else if !loaded {
        Some(LOADING)
    } else if results == 0 {
        Some(NO_MATCH)
    } else {
        None
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

#[derive(Debug)]
pub(crate) struct SearchView {
    edit: HWND,
    brush: HBRUSH,
    colors: Palette,
    notebook: Option<PathBuf>,
    /// The library's state has loaded, so an empty result list means no match.
    loaded: bool,
    /// The notebook's note paths, relative to it. They are copied from the library on the first
    /// search after a change and dropped when the library changes, so an idle Search view holds
    /// nothing.
    notes: Option<Vec<PathBuf>>,
    pub(crate) query: String,
    pub(crate) results: Vec<NameMatch>,
    pub(crate) list: RowListState,
    placeholder: String,
    /// While the scroll thumb is dragged: how far below its top it was grabbed.
    thumb_grab: Option<i32>,
    /// Bumped whenever the results change (`AccessibleView::accessible_generation`).
    order: u64,
}

impl SearchView {
    /// Creates the hidden search box inside `panel`.
    pub(crate) fn create(panel: HWND, dpi: u32) -> crate::Result<Self> {
        let edit = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
        if unsafe { SetWindowSubclass(edit, Some(search_edit_proc), SEARCH_HOOK_ID, 0) } == 0 {
            let error = last_error();
            unsafe {
                DestroyWindow(edit);
            }
            return Err(error);
        }
        let colors = Palette::neutral();
        Ok(Self {
            edit,
            brush: unsafe { CreateSolidBrush(colors.editor_background) },
            colors,
            notebook: None,
            loaded: false,
            notes: None,
            query: String::new(),
            results: Vec::new(),
            list: RowListState::new(scale(ROW_AT_96_DPI, dpi)),
            placeholder: placeholder(None),
            thumb_grab: None,
            order: 0,
        })
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

    /// Recomputes the results for `query`, with the first one selected.
    fn filter(&mut self, query: &str, client: RECT, dpi: u32) {
        self.query = query.to_owned();
        let results = match &self.notes {
            Some(notes) if !query.trim().is_empty() => {
                name_search::search(notes, query, RESULT_LIMIT)
            }
            _ => Vec::new(),
        };
        if results != self.results {
            self.order = self.order.wrapping_add(1);
        }
        self.results = results;
        let area = self.list_area(client, dpi);
        self.list.row_height = scale(ROW_AT_96_DPI, dpi);
        self.list.set_count(self.results.len());
        self.list.top = 0;
        self.list.selected = None;
        if !self.results.is_empty() {
            self.list.select(0, height(area));
        }
    }

    pub(crate) fn status(&self) -> Option<&'static str> {
        status_text(
            self.notebook.is_some(),
            self.loaded,
            &self.query,
            self.results.len(),
        )
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
            if let Some(status) = self.status() {
                let area = self.list_area(client, dpi);
                let status_line = RECT {
                    left: client.left + pad,
                    top: area.top,
                    right: client.right - pad,
                    bottom: area.top + scale(ROW_AT_96_DPI, dpi),
                };
                draw_text(
                    paint.hdc,
                    status,
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

/// `EN_CHANGE` from the box: re-runs the search.
pub(crate) fn query_changed(hwnd: HWND) {
    let Some(edit) = with_view(hwnd, |view| view.edit) else {
        return;
    };
    let query = window_text(edit);
    let needs_notes =
        !query.trim().is_empty() && with_view(hwnd, |view| view.notes.is_none()).unwrap_or(false);
    if needs_notes {
        let notes = library_host::with_state(hwnd, |state| {
            state
                .notes
                .iter()
                .map(|note| note.path.clone())
                .collect::<Vec<_>>()
        });
        with_view(hwnd, |view| {
            view.loaded = notes.is_some();
            if notes.is_some() {
                view.notes = notes;
            }
        });
    }
    let panel = unsafe { GetParent(edit) };
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| view.filter(&query, client, dpi));
    invalidate(panel);
}

/// Part of `side_panel::refresh`. A new notebook clears the query. The same notebook re-runs it
/// against the changed note list.
pub(crate) fn library_changed(hwnd: HWND) {
    let notebook = library_host::folder(hwnd);
    let loaded = library_host::with_state(hwnd, |_| ()).is_some();
    let Some((edit, changed)) = with_view(hwnd, |view| {
        let changed = match (&view.notebook, &notebook) {
            (Some(old), Some(new)) => !same_path(old, new),
            (None, None) => false,
            _ => true,
        };
        view.notes = None;
        view.loaded = loaded;
        if changed {
            view.notebook = notebook.clone();
            view.placeholder = placeholder(notebook.as_deref());
        }
        (view.edit, changed)
    }) else {
        return;
    };
    if changed {
        // Clearing the box sends EN_CHANGE, which empties the results.
        let empty = wide_null("");
        unsafe {
            SetWindowTextW(edit, empty.as_ptr());
            InvalidateRect(edit, std::ptr::null(), 1);
        }
        layout(hwnd);
    } else {
        query_changed(hwnd);
    }
}

/// Places the box in the header and shows it while the Search view shows a notebook. Part of
/// `side_panel::layout`, and run whenever the view or the notebook changes.
pub(crate) fn layout(hwnd: HWND) {
    let Some((edit, has_notebook)) = with_view(hwnd, |view| (view.edit, view.notebook.is_some()))
    else {
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
    let show = has_notebook && side_panel::current_view(hwnd) == SidebarView::Search;
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

/// `side_panel::show_view` switched to Search. `focus` puts the caret in the box (Ctrl+K).
pub(crate) fn shown(hwnd: HWND, focus: bool) {
    layout(hwnd);
    let Some((edit, has_notebook)) = with_view(hwnd, |view| (view.edit, view.notebook.is_some()))
    else {
        return;
    };
    if !focus {
        return;
    }
    unsafe {
        if has_notebook {
            SetFocus(edit);
            SendMessageW(edit, EM_SETSEL, 0, -1);
        } else {
            SetFocus(GetParent(edit));
        }
    }
}

/// `side_panel::show_view` switched away from Search. The query stays.
pub(crate) fn hidden(hwnd: HWND) {
    if let Some(edit) = with_view(hwnd, |view| view.edit) {
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
                if let Some(edit) = with_view(hwnd, |view| view.edit) {
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
            let edit = with_view(hwnd, |view| view.edit)?;
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

/// The results listed, as (name, folder), for in-process tests.
#[cfg(test)]
pub(crate) fn shown_results(hwnd: HWND) -> Vec<(String, String)> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| (result.name.clone(), result.folder.clone()))
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn status(hwnd: HWND) -> Option<&'static str> {
    with_view(hwnd, |view| view.status()).flatten()
}

#[cfg(test)]
pub(crate) fn edit_hwnd(hwnd: HWND) -> Option<HWND> {
    with_view(hwnd, |view| view.edit)
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
    use super::{LOADING, NO_MATCH, NO_NOTEBOOK, placeholder, status_text};
    use std::path::Path;

    #[test]
    fn the_status_line_explains_an_empty_list() {
        // Break caught: a blank Search view with no notebook open, "No notes match." shown
        // before anything is typed, or a match count of zero reported while still loading.
        assert_eq!(status_text(false, false, "x", 0), Some(NO_NOTEBOOK));
        assert_eq!(status_text(true, true, "", 0), None);
        assert_eq!(status_text(true, true, "   ", 0), None);
        assert_eq!(status_text(true, false, "x", 0), Some(LOADING));
        assert_eq!(status_text(true, true, "x", 0), Some(NO_MATCH));
        assert_eq!(status_text(true, true, "x", 3), None);
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
}
