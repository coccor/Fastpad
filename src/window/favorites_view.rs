//! The Favorites view (sidebar spec §7). It lists favorite notebooks by name, highlights the open
//! one, shows a filled star that removes a favorite, and ends with an "Open notebook…" footer
//! row. Clicking a notebook opens it and shows the Notebook view.

use crate::library::model::same_path;
use crate::library::tree::natural_cmp;
use crate::library::{local, normalize_folder};
use crate::window::commands::CommandId;
use crate::window::library_host;
use crate::window::menus::{self, MenuEntry};
use crate::window::panel::{fill, scale};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{ViewPaint, draw_text, point_of};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    HDC, InvalidateRect, ScreenToClient,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_DELETE, VK_RETURN, VK_SPACE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, WM_CAPTURECHANGED, WM_CONTEXTMENU, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
};

const HEADER_AT_96_DPI: i32 = 38;
const ROW_AT_96_DPI: i32 = 26;
const PADDING_AT_96_DPI: i32 = 12;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const HEADER_BUTTON_AT_96_DPI: i32 = 28;

const FOLDER_GLYPH: &str = "\u{E8B7}";
const FILLED_STAR_GLYPH: &str = "\u{E735}";
const OPEN_GLYPH: &str = "\u{E838}";

pub(crate) const HEADER_TEXT: &str = "FAVORITES";
pub(crate) const EMPTY_TEXT: &str = "Star a notebook to keep it here.";
pub(crate) const OPEN_NOTEBOOK: &str = "Open notebook\u{2026}";

/// One favorite notebook as the view lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FavoriteRow {
    pub folder: PathBuf,
    pub name: String,
    /// The dim parent-folder hint, shown only when two favorites share a name.
    pub hint: Option<String>,
    /// This is the open notebook.
    pub open: bool,
}

/// The favorites sorted by name, then by hint, with the open notebook marked.
pub(crate) fn favorite_rows(favorites: &[PathBuf], open: Option<&Path>) -> Vec<FavoriteRow> {
    let open = open.map(normalize_folder);
    let mut rows = favorites
        .iter()
        .zip(local::display_names(favorites))
        .map(|(folder, (name, hint))| FavoriteRow {
            open: open
                .as_ref()
                .is_some_and(|open| same_path(open, &normalize_folder(folder))),
            folder: folder.clone(),
            name,
            hint,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        natural_cmp(&left.name, &right.name).then_with(|| {
            natural_cmp(
                left.hint.as_deref().unwrap_or(""),
                right.hint.as_deref().unwrap_or(""),
            )
        })
    });
    rows
}

/// What a click, key or menu choice asks for. The caller runs it with no view borrowed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FavoriteAction {
    Open(PathBuf),
    Remove(PathBuf),
    Reveal(PathBuf),
    /// The header button and the footer row: the folder dialog.
    Browse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum KeyResult {
    /// Not a key this view handles; the panel's default handling runs.
    Unhandled,
    /// Handled here (the selection moved); the caller repaints.
    Handled,
    Run(FavoriteAction),
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

const fn height(rect: RECT) -> i32 {
    let height = rect.bottom - rect.top;
    if height > 0 { height } else { 0 }
}

#[derive(Debug)]
pub(crate) struct FavoritesView {
    pub(crate) rows: Vec<FavoriteRow>,
    /// One row per favorite, then the "Open notebook…" footer.
    pub(crate) list: RowListState,
    header_hover: bool,
    /// While the scroll thumb is dragged: how far below its top it was grabbed.
    thumb_grab: Option<i32>,
    /// Bumped whenever the rows change order (`AccessibleView::accessible_generation`).
    order: u64,
}

impl FavoritesView {
    pub(crate) fn new(dpi: u32) -> Self {
        let mut list = RowListState::new(scale(ROW_AT_96_DPI, dpi));
        list.set_count(1);
        Self {
            rows: Vec::new(),
            list,
            header_hover: false,
            thumb_grab: None,
            order: 0,
        }
    }

    /// Replaces the rows. The selection stays on the same notebook, or moves to the row that took
    /// the place of a removed one. `height` is the list area's height.
    pub(crate) fn set_rows(&mut self, rows: Vec<FavoriteRow>, dpi: u32, height: i32) {
        let previous = self.list.selected;
        let kept = previous
            .and_then(|index| self.rows.get(index))
            .map(|row| row.folder.clone());
        let reordered = rows.len() != self.rows.len()
            || rows
                .iter()
                .zip(&self.rows)
                .any(|(new, old)| !same_path(&new.folder, &old.folder));
        if reordered {
            self.order = self.order.wrapping_add(1);
        }
        self.rows = rows;
        self.list.row_height = scale(ROW_AT_96_DPI, dpi);
        self.list.set_count(self.rows.len() + 1);
        let index = kept
            .and_then(|folder| {
                self.rows
                    .iter()
                    .position(|row| same_path(&row.folder, &folder))
            })
            .or_else(|| previous.map(|index| index.min(self.rows.len())));
        if let Some(index) = index {
            self.list.select(index, height);
        }
        // A shorter list may leave the view scrolled past its end.
        self.list.scroll_lines(0, height);
    }

    /// The header's "Open notebook…" button.
    pub(crate) fn header_button(client: RECT, dpi: u32) -> RECT {
        let size = scale(HEADER_BUTTON_AT_96_DPI, dpi);
        let header = scale(HEADER_AT_96_DPI, dpi);
        let right = client.right - scale(GAP_AT_96_DPI, dpi);
        let top = client.top + (header - size) / 2;
        RECT {
            left: right - size,
            top,
            right,
            bottom: top + size,
        }
    }

    /// Where the rows are. With no favorites, the empty-state line sits above the footer row.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let mut top = client.top + scale(HEADER_AT_96_DPI, dpi);
        if self.rows.is_empty() {
            top += scale(ROW_AT_96_DPI, dpi);
        }
        RECT {
            top: top.min(client.bottom),
            ..client
        }
    }

    fn star_left(area: RECT, dpi: u32) -> i32 {
        area.right - scale(ROW_AT_96_DPI, dpi)
    }

    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let header = RECT {
            bottom: client.top + scale(HEADER_AT_96_DPI, dpi),
            ..client
        };
        unsafe {
            fill(paint.hdc, client, paint.background);
            draw_text(
                paint.hdc,
                HEADER_TEXT,
                RECT {
                    left: header.left + pad,
                    ..header
                },
                paint.fonts.bold,
                palette.muted_foreground,
                line | DT_LEFT,
            );
            let button = Self::header_button(client, dpi);
            if self.header_hover {
                fill(paint.hdc, button, palette.hover_background);
            }
            draw_text(
                paint.hdc,
                OPEN_GLYPH,
                button,
                paint.fonts.glyph,
                palette.editor_foreground,
                line | DT_CENTER,
            );
            if self.rows.is_empty() {
                let empty = RECT {
                    left: client.left + pad,
                    top: header.bottom,
                    right: client.right - pad,
                    bottom: header.bottom + scale(ROW_AT_96_DPI, dpi),
                };
                draw_text(
                    paint.hdc,
                    EMPTY_TEXT,
                    empty,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
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

    /// One row's glyph and text. `row_list::paint` has already filled its selection or hover
    /// background.
    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let glyph = RECT {
            left: rect.left + pad,
            right: rect.left + pad + scale(GLYPH_AT_96_DPI, dpi),
            ..rect
        };
        let text_left = glyph.right + scale(GAP_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let foreground = row_foreground(look, &palette);
        // Over the focused selection, dim text takes the selection's text color to stay legible.
        let muted = if look.selected && look.focused {
            foreground
        } else {
            palette.muted_foreground
        };
        unsafe {
            let Some(row) = self.rows.get(index) else {
                draw_text(
                    hdc,
                    OPEN_GLYPH,
                    glyph,
                    paint.fonts.glyph,
                    muted,
                    line | DT_CENTER,
                );
                draw_text(
                    hdc,
                    OPEN_NOTEBOOK,
                    RECT {
                        left: text_left,
                        right: rect.right - pad,
                        ..rect
                    },
                    paint.fonts.text,
                    foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
                return;
            };
            // The open notebook gets an accent bar, so it isn't marked by color alone.
            if row.open {
                fill(
                    hdc,
                    RECT {
                        right: rect.left + scale(3, dpi),
                        ..rect
                    },
                    palette.selection_background,
                );
            }
            draw_text(
                hdc,
                FOLDER_GLYPH,
                glyph,
                paint.fonts.glyph,
                muted,
                line | DT_CENTER,
            );
            let show_star = look.hover || look.selected;
            let star = RECT {
                left: Self::star_left(rect, dpi),
                ..rect
            };
            if show_star {
                draw_text(
                    hdc,
                    FILLED_STAR_GLYPH,
                    star,
                    paint.fonts.glyph,
                    foreground,
                    line | DT_CENTER,
                );
            }
            let text_right = if show_star {
                star.left
            } else {
                rect.right - pad
            };
            let name = RECT {
                left: text_left,
                right: text_right,
                ..rect
            };
            let width = draw_text(
                hdc,
                &row.name,
                name,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if let Some(hint) = &row.hint {
                draw_text(
                    hdc,
                    hint,
                    RECT {
                        left: text_left + width + scale(GAP_AT_96_DPI, dpi),
                        ..name
                    },
                    paint.fonts.text,
                    muted,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
        }
    }

    /// `WM_MOUSEMOVE`. Returns whether anything needs repainting: the hover changed or, while the
    /// scroll thumb is dragged, the list scrolled.
    pub(crate) fn hover(&mut self, point: POINT, client: RECT, dpi: u32) -> bool {
        let area = self.list_area(client, dpi);
        if let Some(grab) = self.thumb_grab {
            return self.list.drag_thumb(grab, point.y - area.top, height(area));
        }
        let row = if inside(area, point) {
            self.list.row_at(point.y - area.top)
        } else {
            None
        };
        let header = inside(Self::header_button(client, dpi), point);
        let row_changed = self.list.set_hover(row);
        let header_changed = std::mem::replace(&mut self.header_hover, header) != header;
        row_changed || header_changed
    }

    /// `WM_MOUSELEAVE`. Returns whether anything was hovered.
    pub(crate) fn leave(&mut self) -> bool {
        let row_changed = self.list.set_hover(None);
        std::mem::take(&mut self.header_hover) || row_changed
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

    /// `WM_LBUTTONDOWN`. Selects the row under `point` and says what the click does.
    pub(crate) fn click(&mut self, point: POINT, client: RECT, dpi: u32) -> Option<FavoriteAction> {
        if inside(Self::header_button(client, dpi), point) {
            return Some(FavoriteAction::Browse);
        }
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        let index = self.list.row_at(point.y - area.top)?;
        self.list.select(index, height(area));
        let Some(row) = self.rows.get(index) else {
            return Some(FavoriteAction::Browse);
        };
        Some(if point.x >= Self::star_left(area, dpi) {
            FavoriteAction::Remove(row.folder.clone())
        } else {
            FavoriteAction::Open(row.folder.clone())
        })
    }

    /// `WM_KEYDOWN` while the panel has the focus.
    pub(crate) fn key(&mut self, key: u16, client: RECT, dpi: u32) -> KeyResult {
        if let Some(movement) = ListKey::from_virtual_key(u32::from(key)) {
            let area = self.list_area(client, dpi);
            self.list.move_selection(movement, height(area));
            return KeyResult::Handled;
        }
        let selected = self.list.selected.map(|index| self.rows.get(index));
        match (key, selected) {
            (VK_RETURN | VK_SPACE, Some(Some(row))) => {
                KeyResult::Run(FavoriteAction::Open(row.folder.clone()))
            }
            (VK_RETURN | VK_SPACE, Some(None)) => KeyResult::Run(FavoriteAction::Browse),
            (VK_DELETE, Some(Some(row))) => {
                KeyResult::Run(FavoriteAction::Remove(row.folder.clone()))
            }
            (VK_RETURN | VK_SPACE | VK_DELETE, _) => KeyResult::Handled,
            _ => KeyResult::Unhandled,
        }
    }

    /// The favorite a context menu acts on, and where the menu opens in client coordinates. A
    /// right-click selects the row under `point`. The keyboard (`None`) uses the selected row.
    pub(crate) fn menu_target(
        &mut self,
        point: Option<POINT>,
        client: RECT,
        dpi: u32,
    ) -> Option<(PathBuf, POINT)> {
        let area = self.list_area(client, dpi);
        let (index, at) = match point {
            Some(point) => {
                if !inside(area, point) {
                    return None;
                }
                let index = self.list.row_at(point.y - area.top)?;
                self.list.select(index, height(area));
                (index, point)
            }
            None => {
                let index = self.list.selected?;
                let top = self.list.row_top(index)?;
                let at = POINT {
                    x: area.left + scale(PADDING_AT_96_DPI, dpi),
                    y: area.top + top + self.list.row_height,
                };
                (index, at)
            }
        };
        Some((self.rows.get(index)?.folder.clone(), at))
    }

    /// `WM_MOUSEWHEEL`'s delta at `lines` rows per notch (`row_list::wheel_lines`). Returns
    /// whether the list scrolled.
    pub(crate) fn wheel(&mut self, delta: i32, lines: u32, client: RECT, dpi: u32) -> bool {
        let area = self.list_area(client, dpi);
        self.list.wheel(delta, lines, height(area))
    }
}

/// Runs `f` on the Favorites view with nothing else of the App borrowed.
fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut FavoritesView) -> R) -> Option<R> {
    let mut app = unsafe { super::main_window::app_ptr(hwnd) }?;
    unsafe { app.as_mut() }
        .sidebar
        .as_mut()
        .map(|sidebar| f(&mut sidebar.favorites))
}

fn geometry(panel: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(panel, &mut client);
    }
    (client, unsafe { GetDpiForWindow(panel) }.max(96))
}

fn invalidate(panel: HWND) {
    unsafe {
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

fn point(lparam: LPARAM) -> POINT {
    let (x, y) = point_of(lparam);
    POINT { x, y }
}

/// Re-reads the favorites and the open notebook. Part of `side_panel::refresh`.
pub(crate) fn refresh(hwnd: HWND, panel: HWND) {
    let favorites = library_host::favorites(hwnd);
    let open = library_host::folder(hwnd);
    let rows = favorite_rows(&favorites, open.as_deref());
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        view.set_rows(rows, dpi, height(area));
    });
    invalidate(panel);
}

/// The panel's `WM_PAINT` while the Favorites view shows.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    if let Some(app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.favorites.paint(paint);
    }
}

/// Header rectangles that are controls, not a window drag area.
pub(crate) fn header_controls(client: RECT, dpi: u32) -> Vec<RECT> {
    vec![FavoritesView::header_button(client, dpi)]
}

/// Input for the Favorites view. `None` leaves the message to the panel's default handling.
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
            if with_view(hwnd, |view| view.hover(at, client, dpi)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            if with_view(hwnd, FavoritesView::leave).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN => {
            if unsafe { GetFocus() } != panel {
                unsafe {
                    SetFocus(panel);
                }
            }
            let at = point(lparam);
            // A press on the scroll thumb drags it, as in the Notebook view.
            let grabbed = with_view(hwnd, |view| {
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
            let action = with_view(hwnd, |view| view.click(at, client, dpi)).flatten();
            invalidate(panel);
            if let Some(action) = action {
                run(hwnd, action, false);
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
        WM_RBUTTONDOWN => {
            // The button-up becomes WM_CONTEXTMENU, which selects the row and opens its menu.
            unsafe {
                SetFocus(panel);
            }
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            if with_view(hwnd, |view| view.wheel(delta, lines, client, dpi)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let result = with_view(hwnd, |view| view.key(wparam as u16, client, dpi))
                .unwrap_or(KeyResult::Unhandled);
            match result {
                KeyResult::Unhandled => None,
                KeyResult::Handled => {
                    invalidate(panel);
                    Some(0)
                }
                KeyResult::Run(action) => {
                    invalidate(panel);
                    run(hwnd, action, true);
                    Some(0)
                }
            }
        }
        WM_CONTEXTMENU => {
            // Shift+F10 and the context-menu key send (-1, -1) instead of a screen point.
            let keyboard = lparam as u32 == u32::MAX;
            let at = (!keyboard).then(|| {
                let mut at = point(lparam);
                unsafe {
                    ScreenToClient(panel, &mut at);
                }
                at
            });
            let target = with_view(hwnd, |view| view.menu_target(at, client, dpi)).flatten();
            invalidate(panel);
            if let Some((folder, at)) = target
                && let Some(action) = show_menu(hwnd, panel, at, folder)
            {
                run(hwnd, action, keyboard);
            }
            Some(0)
        }
        _ => None,
    }
}

/// Runs a Favorites action. `keyboard` keeps the focus in the panel when the Notebook view
/// replaces this one.
pub(crate) fn run(hwnd: HWND, action: FavoriteAction, keyboard: bool) {
    match action {
        // Checked on a worker: a missing folder shows a notice and changes nothing (spec §4.3).
        // Once the notebook is open, the Notebook view shows it.
        FavoriteAction::Open(folder) => {
            library_host::open_listed_notebook_in_view(hwnd, &folder, keyboard);
        }
        FavoriteAction::Remove(folder) => library_host::remove_favorite(hwnd, &folder),
        FavoriteAction::Reveal(folder) => library_host::reveal(hwnd, &folder),
        FavoriteAction::Browse => library_host::choose_and_open_folder(hwnd),
    }
}

/// A row's context menu (spec §7) at `point`, in panel client coordinates, for `folder`. The
/// entries are commands because `menus::track_popup` returns one, and the view reads them itself,
/// as the Notebook view's menus do: `OpenFolder` opens this notebook, `ToggleNotebookFavorite`
/// removes it and `NoteRevealInExplorer` reveals it. No command numbers are spent on menu-only
/// entries. Tests answer it with `menus::answer_next_popup_menu`.
fn show_menu(hwnd: HWND, panel: HWND, point: POINT, folder: PathBuf) -> Option<FavoriteAction> {
    // `track_popup` takes main-window client coordinates.
    let mut at = point;
    unsafe {
        ClientToScreen(panel, &mut at);
        ScreenToClient(hwnd, &mut at);
    }
    let entries = [
        MenuEntry::Command("&Open", CommandId::OpenFolder),
        MenuEntry::Command("&Remove from favorites", CommandId::ToggleNotebookFavorite),
        MenuEntry::Command("Reveal in &Explorer", CommandId::NoteRevealInExplorer),
    ];
    match menus::track_popup(hwnd, &entries, at)? {
        CommandId::OpenFolder => Some(FavoriteAction::Open(folder)),
        CommandId::ToggleNotebookFavorite => Some(FavoriteAction::Remove(folder)),
        CommandId::NoteRevealInExplorer => Some(FavoriteAction::Reveal(folder)),
        _ => None,
    }
}

/// The rows the view lists, for in-process tests.
#[cfg(test)]
pub(crate) fn shown_rows(hwnd: HWND) -> Vec<FavoriteRow> {
    with_view(hwnd, |view| view.rows.clone()).unwrap_or_default()
}

impl crate::window::sidebar_accessibility::AccessibleView for FavoritesView {
    /// The header button, one item per favorite, then the footer row.
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        1 + self.rows.len() + 1
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{button_item, list_item, row_rect};
        if index == 0 {
            return Some(button_item(
                OPEN_NOTEBOOK,
                false,
                false,
                Self::header_button(client, dpi),
            ));
        }
        let row = index - 1;
        if row > self.rows.len() {
            return None;
        }
        let (rect, visible) = row_rect(self.list_area(client, dpi), &self.list, row);
        let selected = self.list.selected == Some(row);
        let name = match self.rows.get(row) {
            Some(favorite) => {
                let mut name = favorite.name.clone();
                if let Some(hint) = &favorite.hint {
                    name.push_str(", ");
                    name.push_str(hint);
                }
                if favorite.open {
                    name.push_str(", open");
                }
                name
            }
            None => OPEN_NOTEBOOK.to_owned(),
        };
        Some(list_item(&name, selected, focused, rect, visible))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        if inside(Self::header_button(client, dpi), point) {
            return Some(0);
        }
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        self.list.row_at(point.y - area.top).map(|row| row + 1)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.list.selected.map(|row| row + 1)
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if let Some(row) = index.checked_sub(1).filter(|&row| row <= self.rows.len()) {
            let area = self.list_area(client, dpi);
            self.list.select(row, area.bottom - area.top);
        }
    }

    /// Favorites by folder. The header button and the footer have none: they never move.
    fn accessible_identity(&self, index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        let favorite = self.rows.get(index.checked_sub(1)?)?;
        Some(crate::window::sidebar_accessibility::identity_of(
            &favorite.folder,
        ))
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}

#[cfg(test)]
mod tests {
    use super::{FavoriteAction, FavoritesView, KeyResult, favorite_rows};
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_DOWN, VK_RETURN};

    const CLIENT: RECT = RECT {
        left: 0,
        top: 0,
        right: 260,
        bottom: 400,
    };

    fn view(favorites: &[&str]) -> FavoritesView {
        let favorites = favorites.iter().map(PathBuf::from).collect::<Vec<_>>();
        let mut view = FavoritesView::new(96);
        view.set_rows(favorite_rows(&favorites, None), 96, 400);
        view
    }

    #[test]
    fn favorites_are_sorted_by_name_with_the_open_one_marked_and_clashes_hinted() {
        // Break caught: favorites listed in folders.ini order, the open notebook not marked
        // (paths differ only in case), or two "Notes" folders shown identically.
        let favorites =
            [r"C:\b\Notes", r"C:\Work 10", r"C:\a\Notes", r"C:\Work 2"].map(PathBuf::from);
        let rows = favorite_rows(&favorites, Some(Path::new(r"c:\WORK 2")));
        let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Notes", "Notes", "Work 2", "Work 10"]);
        assert_eq!(rows[0].folder, PathBuf::from(r"C:\a\Notes"));
        assert!(rows[0].hint.is_some() && rows[1].hint.is_some());
        assert_ne!(rows[0].hint, rows[1].hint);
        assert_eq!(rows[2].hint, None);
        assert!(rows[2].open);
        assert!(!rows[3].open && !rows[0].open);
    }

    #[test]
    fn a_click_on_the_star_removes_elsewhere_opens_and_the_footer_browses() {
        // Break caught: the star opening the notebook instead of removing it, or the footer row
        // and header button doing nothing.
        let mut view = view(&[r"C:\a", r"C:\b"]);
        let first = PathBuf::from(r"C:\a");
        // The header is 38 px and each row 26 px at 96 DPI.
        assert_eq!(
            view.click(POINT { x: 100, y: 50 }, CLIENT, 96),
            Some(FavoriteAction::Open(first.clone()))
        );
        assert_eq!(
            view.click(POINT { x: 250, y: 50 }, CLIENT, 96),
            Some(FavoriteAction::Remove(first))
        );
        assert_eq!(
            view.click(
                POINT {
                    x: 100,
                    y: 38 + 2 * 26 + 5
                },
                CLIENT,
                96
            ),
            Some(FavoriteAction::Browse)
        );
        assert_eq!(
            view.click(POINT { x: 240, y: 19 }, CLIENT, 96),
            Some(FavoriteAction::Browse)
        );
        assert_eq!(view.click(POINT { x: 100, y: 390 }, CLIENT, 96), None);
    }

    #[test]
    fn keys_move_the_selection_and_enter_opens_the_selected_row_or_the_footer() {
        // Break caught: Enter doing nothing in the Favorites view, Delete removing an unselected
        // favorite, or the footer row being unreachable from the keyboard.
        let mut view = view(&[r"C:\a", r"C:\b"]);
        view.list.select(0, 400);
        assert_eq!(view.key(VK_DOWN, CLIENT, 96), KeyResult::Handled);
        assert_eq!(
            view.key(VK_RETURN, CLIENT, 96),
            KeyResult::Run(FavoriteAction::Open(PathBuf::from(r"C:\b")))
        );
        assert_eq!(
            view.key(VK_DELETE, CLIENT, 96),
            KeyResult::Run(FavoriteAction::Remove(PathBuf::from(r"C:\b")))
        );
        assert_eq!(view.key(VK_DOWN, CLIENT, 96), KeyResult::Handled);
        assert_eq!(
            view.key(VK_RETURN, CLIENT, 96),
            KeyResult::Run(FavoriteAction::Browse)
        );
        assert_eq!(view.key(VK_DELETE, CLIENT, 96), KeyResult::Handled);
    }

    #[test]
    fn a_removed_favorite_moves_the_selection_to_its_neighbor() {
        // Break caught: a stale selection index past the end after the last favorite was removed.
        let mut view = view(&[r"C:\a", r"C:\b"]);
        view.list.select(1, 400);
        view.set_rows(favorite_rows(&[PathBuf::from(r"C:\a")], None), 96, 400);
        assert_eq!(
            view.list.selected,
            Some(1),
            "the footer row takes its place"
        );
        view.set_rows(Vec::new(), 96, 400);
        assert_eq!(view.list.count, 1);
        assert_eq!(view.list.selected, Some(0));
    }
}
