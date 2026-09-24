//! The sidebar: the activity bar and one side panel, two painted child windows along the main
//! window's left edge, present only in notes mode. Everything else in the window is laid out to
//! their right (`left_edge`). The panel paints the current view. The views plug in through the
//! `PanelView` dispatch (`paint_view`, `view_mouse`, `view_key`, `header_is_caption`).
//!
//! The strip above the first activity button and the empty part of the panel header belong to the
//! window caption. Both windows answer `WM_NCHITTEST` there with `HTTRANSPARENT`, so the main
//! window's own hit test applies: dragging, top-edge resizing and double-click to maximize.

use super::activity_bar::{self, ActivityButton, BarState};
use super::main_window::{
    app_ptr, change_setting, current_palette, invalidate_title_strip, layout_editor_and_find_bar,
    push_notice, return_focus_to_editor, ui_fonts,
};
use super::tooltip::Tooltip;
use crate::config::SidebarView;
use crate::config::defaults::{DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH};
use crate::platform::wide_null;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, scale};
use crate::window::sidebar_accessibility::{
    self, AccessibleItem, AccessibleMark, AccessibleSource, AccessibleView,
};
use crate::window::titlebar::create_ui_font;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DRAW_TEXT_FORMAT, DT_CALCRECT,
    DeleteDC, DeleteObject, DrawTextW, EndPaint, FW_BOLD, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT,
    InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, DestroyWindow, EN_CHANGE, GWL_STYLE, GetClientRect, GetCursorPos,
    GetParent, GetWindowLongPtrW, HTTRANSPARENT, IDC_ARROW, IDC_SIZEWE, IsChild, LoadCursorW,
    OBJID_CLIENT, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetCursor,
    SetWindowPos, ShowWindow, WM_CAPTURECHANGED, WM_CHAR, WM_COMMAND, WM_CONTEXTMENU,
    WM_CTLCOLOREDIT, WM_ERASEBKGND, WM_GETOBJECT, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS, WM_SYSCHAR, WM_SYSKEYDOWN, WNDCLASSW,
    WNDPROC, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// Sizes at 96 DPI, scaled with `panel::scale`.
pub(crate) const ACTIVITY_WIDTH_96: i32 = 44;
pub(crate) const EDITOR_MIN_WIDTH_96: i32 = 320;
pub(crate) const HEADER_HEIGHT_96: i32 = 38;
/// The strip along the panel's right edge that resizes it.
pub(crate) const GRIP_WIDTH_96: i32 = 4;

/// The sidebar's fonts at one DPI. Painting copies them out; `Sidebar` owns and deletes them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UiFonts {
    /// Row and body text: Segoe UI, 12 px at 96 DPI.
    pub(crate) text: HFONT,
    /// The match in a Search result's snippet: Segoe UI bold, 12 px. Made on the first paint of a
    /// snippet (`Sidebar::text_bold`), not with the others, so it adds nothing before first paint.
    pub(crate) text_bold: HFONT,
    /// Header titles in small capitals: Segoe UI semibold, 11 px.
    pub(crate) bold: HFONT,
    /// Unsaved rows and notices inside the list: Segoe UI italic, 12 px.
    pub(crate) italic: HFONT,
    /// Row and header-button icons: Segoe MDL2 Assets, 12 px.
    pub(crate) glyph: HFONT,
    /// The activity bar's icons: Segoe MDL2 Assets, 16 px.
    pub(crate) bar_glyph: HFONT,
}

impl Default for UiFonts {
    fn default() -> Self {
        Self {
            text: std::ptr::null_mut(),
            text_bold: std::ptr::null_mut(),
            bold: std::ptr::null_mut(),
            italic: std::ptr::null_mut(),
            glyph: std::ptr::null_mut(),
            bar_glyph: std::ptr::null_mut(),
        }
    }
}

impl UiFonts {
    fn create(dpi: u32) -> Self {
        let normal = FW_NORMAL as i32;
        Self {
            text: create_ui_font(scale(12, dpi), "Segoe UI", normal, false),
            text_bold: std::ptr::null_mut(),
            bold: create_ui_font(scale(11, dpi), "Segoe UI", FW_SEMIBOLD as i32, false),
            italic: create_ui_font(scale(12, dpi), "Segoe UI", normal, true),
            glyph: create_ui_font(scale(12, dpi), "Segoe MDL2 Assets", normal, false),
            bar_glyph: create_ui_font(scale(16, dpi), "Segoe MDL2 Assets", normal, false),
        }
    }

    fn delete(self) {
        for font in [
            self.text,
            self.text_bold,
            self.bold,
            self.italic,
            self.glyph,
            self.bar_glyph,
        ] {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }
}

/// The sidebar's windows and state, owned by `App.sidebar`. Whether a view is open and the saved
/// width live in `Settings`. This holds what `fastpad.ini` doesn't.
#[derive(Debug)]
pub(crate) struct Sidebar {
    pub(crate) bar: HWND,
    pub(crate) panel: HWND,
    /// The activity bar's tooltip, made when the pointer first moves over the bar.
    pub(crate) tooltip: Option<Tooltip>,
    /// The tooltip could not be made; it is not tried again.
    tooltip_failed: bool,
    pub(crate) bar_state: BarState,
    /// The activity-bar button the keyboard is on (`bar_focus`).
    pub(crate) bar_focus: usize,
    /// The Notebook view's rows, selection and hover.
    pub(crate) notebook: crate::window::notebook_view::NotebookView,
    pub(crate) favorites: crate::window::favorites_view::FavoritesView,
    pub(crate) search: crate::window::search_view::SearchView,
    /// The view Ctrl+B reopens while the panel is closed.
    last_view: SidebarView,
    /// The live width (96-DPI pixels) while the panel edge is dragged. The setting changes once,
    /// when the drag ends.
    drag_width: Option<u16>,
    /// The fonts and the DPI they were made for (`main_window::ui_fonts`).
    fonts: Option<(u32, UiFonts)>,
}

impl Sidebar {
    /// The fonts for `dpi`, created on first use and again after a DPI change.
    pub(crate) fn fonts(&mut self, dpi: u32) -> UiFonts {
        if let Some((font_dpi, fonts)) = self.fonts
            && font_dpi == dpi
        {
            return fonts;
        }
        if let Some((_, old)) = self.fonts.take() {
            old.delete();
        }
        let fonts = UiFonts::create(dpi);
        self.fonts = Some((dpi, fonts));
        fonts
    }

    /// The bold text font for `dpi`, made the first time a Search snippet paints at that DPI.
    /// It lives with the other fonts and goes with them.
    pub(crate) fn text_bold(&mut self, dpi: u32) -> HFONT {
        let font = self.fonts(dpi).text_bold;
        if !font.is_null() {
            return font;
        }
        let font = create_ui_font(scale(12, dpi), "Segoe UI", FW_BOLD as i32, false);
        if let Some((_, fonts)) = self.fonts.as_mut() {
            fonts.text_bold = font;
        }
        font
    }
}

impl Drop for Sidebar {
    fn drop(&mut self) {
        if let Some((_, fonts)) = self.fonts.take() {
            fonts.delete();
        }
    }
}

/// The view the panel is showing. Each view gets its painting and input
/// through the `match`es in `paint_view`, `view_mouse`, `view_key` and `header_is_caption`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelView {
    Notebook,
    Search,
    Favorites,
}

impl PanelView {
    pub(crate) const fn of(view: SidebarView) -> Option<Self> {
        match view {
            SidebarView::Notebook => Some(Self::Notebook),
            SidebarView::Search => Some(Self::Search),
            SidebarView::Favorites => Some(Self::Favorites),
            SidebarView::Hidden => None,
        }
    }
}

/// What a view paints with: the panel's buffered DC and everything a paint needs, built once per
/// `WM_PAINT` by `view_paint`.
#[derive(Clone, Copy)]
pub(crate) struct ViewPaint {
    pub(crate) hdc: HDC,
    /// The panel's whole client rectangle. Each view lays out its header and body inside it.
    pub(crate) client: RECT,
    pub(crate) palette: Palette,
    /// The panel's fill, `Palette::panel_background`, already painted.
    pub(crate) background: u32,
    pub(crate) fonts: UiFonts,
    pub(crate) dpi: u32,
    /// The panel window itself (not a child control) has the keyboard focus.
    pub(crate) focused: bool,
}

/// The `ViewPaint` for `panel`'s `hdc` and `client` rectangle. Call it with nothing of the App
/// borrowed. The panel's paint and the views' paint tests use it.
pub(crate) fn view_paint(main: HWND, panel: HWND, hdc: HDC, client: RECT) -> ViewPaint {
    let palette = current_palette(main);
    ViewPaint {
        hdc,
        client,
        palette,
        background: palette.panel_background(),
        fonts: ui_fonts(main),
        dpi: unsafe { GetDpiForWindow(panel) }.max(96),
        focused: unsafe { GetFocus() } == panel,
    }
}

/// Draws `text` in `rect` with `font` and `color` over a transparent background, and returns the
/// width it took, at most `rect`'s. `flags` are `DrawTextW`'s. Empty text draws nothing.
pub(crate) unsafe fn draw_text(
    hdc: HDC,
    text: &str,
    rect: RECT,
    font: HFONT,
    color: u32,
    flags: DRAW_TEXT_FORMAT,
) -> i32 {
    // An empty buffer's pointer dangles, and DT_END_ELLIPSIS lets DrawTextW touch it.
    if text.is_empty() {
        return 0;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut target = rect;
    let mut measured = rect;
    unsafe {
        let previous = (!font.is_null()).then(|| SelectObject(hdc, font));
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, color);
        DrawTextW(
            hdc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut measured,
            flags | DT_CALCRECT,
        );
        DrawTextW(hdc, wide.as_ptr(), wide.len() as i32, &mut target, flags);
        if let Some(previous) = previous {
            SelectObject(hdc, previous);
        }
    }
    (measured.right - measured.left)
        .min(rect.right - rect.left)
        .max(0)
}

/// The activity bar's and the panel's widths in device pixels, for a `client_width` wide window.
/// The panel (saved at `width_96` 96-DPI pixels, clamped to its range) gives way first, so the
/// editor keeps its minimum. It never goes below 0.
pub(crate) fn sidebar_widths(
    client_width: i32,
    dpi: u32,
    view_open: bool,
    width_96: u16,
) -> (i32, i32) {
    let client_width = client_width.max(0);
    let activity = scale(ACTIVITY_WIDTH_96, dpi).min(client_width);
    if !view_open {
        return (activity, 0);
    }
    let wanted = scale(
        i32::from(width_96.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)),
        dpi,
    );
    let room = client_width - activity - scale(EDITOR_MIN_WIDTH_96, dpi);
    (activity, wanted.min(room).max(0))
}

/// The 96-DPI width a drag to `panel_px` device pixels asks for, inside the allowed range.
pub(crate) fn drag_width_96(panel_px: i32, dpi: u32) -> u16 {
    let dpi = i64::from(dpi.max(1));
    let unscaled = (i64::from(panel_px.max(0)) * 96 + dpi / 2) / dpi;
    unscaled.clamp(i64::from(MIN_SIDEBAR_WIDTH), i64::from(MAX_SIDEBAR_WIDTH)) as u16
}

fn with_sidebar<R>(hwnd: HWND, action: impl FnOnce(&mut Sidebar) -> R) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_mut() }.sidebar.as_mut().map(action)
}

pub(crate) fn with_bar_state<R>(hwnd: HWND, action: impl FnOnce(&mut BarState) -> R) -> Option<R> {
    with_sidebar(hwnd, |sidebar| action(&mut sidebar.bar_state))
}

/// The activity bar and panel windows, while the sidebar exists.
pub(crate) fn windows(hwnd: HWND) -> Option<(HWND, HWND)> {
    with_sidebar(hwnd, |sidebar| (sidebar.bar, sidebar.panel))
}

/// Whether a view is open and the width to lay it out at, or `None` without a sidebar.
fn open_state(hwnd: HWND) -> Option<(bool, u16)> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let sidebar = app.sidebar.as_ref()?;
    Some((
        app.settings.sidebar_view != SidebarView::Hidden,
        sidebar.drag_width.unwrap_or(app.settings.sidebar_width),
    ))
}

/// The view the panel shows, `Hidden` while it is closed or there is no sidebar.
pub(crate) fn current_view(hwnd: HWND) -> SidebarView {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() })
        .filter(|app| app.sidebar.is_some())
        .map_or(SidebarView::Hidden, |app| app.settings.sidebar_view)
}

/// Where the rest of the window starts: the activity bar plus the open panel, in device pixels.
/// 0 with notes mode off.
pub(crate) fn left_edge(hwnd: HWND) -> i32 {
    let Some((open, width)) = open_state(hwnd) else {
        return 0;
    };
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let (activity, panel) = sidebar_widths(client.right - client.left, dpi, open, width);
    activity + panel
}

/// The activity bar and the panel with their state. Only what the first frame paints is made
/// here: the tooltip waits for the pointer (`bar_pointer_moved`) and the search box for the
/// Search view (`search_view::layout`), so the sidebar adds little before first paint.
pub(crate) fn create(hwnd: HWND) -> crate::Result<Sidebar> {
    let bar = create_child(
        hwnd,
        activity_bar::register_class()?,
        WS_CHILD | WS_CLIPSIBLINGS,
    )?;
    let panel = register_panel_class()
        .and_then(|class| create_child(hwnd, class, WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN));
    let panel = match panel {
        Ok(panel) => panel,
        Err(error) => {
            unsafe { DestroyWindow(bar) };
            return Err(error);
        }
    };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    Ok(Sidebar {
        bar,
        panel,
        tooltip: None,
        tooltip_failed: false,
        bar_state: BarState::default(),
        bar_focus: 0,
        notebook: crate::window::notebook_view::NotebookView::new(panel),
        favorites: crate::window::favorites_view::FavoritesView::new(dpi),
        search: crate::window::search_view::SearchView::new(panel, dpi),
        last_view: SidebarView::Notebook,
        drag_width: None,
        fonts: None,
    })
}

/// Creates or destroys the sidebar to match notes mode, without laying the window out.
fn sync_presence(hwnd: HWND, enabled: bool) {
    let present =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.sidebar.is_some());
    if enabled && !present {
        match create(hwnd) {
            Ok(sidebar) => match unsafe { app_ptr(hwnd) } {
                Some(mut app) => unsafe { app.as_mut() }.sidebar = Some(sidebar),
                None => destroy_windows(&sidebar),
            },
            Err(error) => push_notice(hwnd, format!("FastPad could not show the sidebar: {error}")),
        }
    } else if !enabled && present {
        let sidebar =
            unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.sidebar.take());
        if let Some(sidebar) = sidebar {
            if focus_is_in(sidebar.panel) || focus_is_in(sidebar.bar) {
                return_focus_to_editor(hwnd);
            }
            destroy_windows(&sidebar);
        }
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let view = app.settings.sidebar_view;
        if let Some(sidebar) = app.sidebar.as_mut()
            && view != SidebarView::Hidden
        {
            sidebar.last_view = view;
        }
    }
}

/// Creates or destroys the sidebar to match notes mode, then lays the window out again. It also
/// runs once `fastpad.ini` has been applied, when that changed the notes mode or the sidebar.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    sync_presence(hwnd, enabled);
    layout_editor_and_find_bar(hwnd);
    invalidate_title_strip(hwnd);
}

/// The sidebar for the window's first frame, made from the settings `bootstrap::run` read. It is
/// not laid out here: the first `WM_SIZE`, when `bootstrap::run` shows the window, lays it out
/// with everything else, before anything paints.
pub(crate) fn create_for_first_frame(hwnd: HWND, enabled: bool) {
    sync_presence(hwnd, enabled);
}

/// Destroys the sidebar's windows. The tooltips are owned by the main window, not the bar or the
/// panel, so they are destroyed explicitly.
fn destroy_windows(sidebar: &Sidebar) {
    if let Some(tooltip) = sidebar.tooltip {
        tooltip.destroy();
    }
    sidebar.notebook.destroy_tooltip();
    sidebar.search.destroy_tooltip();
    unsafe {
        DestroyWindow(sidebar.panel);
        DestroyWindow(sidebar.bar);
    }
}

/// Places the activity bar and the panel for the main window's `client` rectangle.
pub(crate) fn layout(hwnd: HWND, client: RECT, dpi: u32) {
    let Some((open, width_96)) = open_state(hwnd) else {
        return;
    };
    let Some((bar, panel)) = windows(hwnd) else {
        return;
    };
    let height = (client.bottom - client.top).max(0);
    let (activity, panel_width) = sidebar_widths(client.right - client.left, dpi, open, width_96);
    let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW;
    unsafe {
        SetWindowPos(bar, std::ptr::null_mut(), 0, 0, activity, height, flags);
        InvalidateRect(bar, std::ptr::null(), 0);
    }
    if panel_width > 0 {
        unsafe {
            SetWindowPos(
                panel,
                std::ptr::null_mut(),
                activity,
                0,
                panel_width,
                height,
                flags,
            );
            InvalidateRect(panel, std::ptr::null(), 0);
        }
    } else {
        if focus_is_in(panel) {
            return_focus_to_editor(hwnd);
        }
        unsafe { ShowWindow(panel, SW_HIDE) };
    }
    update_tools(hwnd);
    crate::window::search_view::layout(hwnd);
}

/// `show_view` without its win events.
fn show_view_now(hwnd: HWND, view: SidebarView, focus: bool) {
    let Some(panel) = with_sidebar(hwnd, |sidebar| {
        if view != SidebarView::Hidden {
            sidebar.last_view = view;
        }
        sidebar.panel
    }) else {
        return;
    };
    if view == SidebarView::Hidden && focus_is_in(panel) {
        return_focus_to_editor(hwnd);
    }
    change_setting(hwnd, |settings| {
        (settings.sidebar_view != view).then(|| {
            settings.sidebar_view = view;
            ("sidebar_view", view.token().to_owned())
        })
    });
    // Library changes rebuild the rows whatever the view, so only what they are built from
    // besides the library (the tabs, the expansion) can have moved on while it was hidden.
    if view == SidebarView::Notebook && crate::window::notebook_view::stale(hwnd) {
        crate::window::notebook_view::rebuild(hwnd);
    }
    layout_editor_and_find_bar(hwnd);
    invalidate_title_strip(hwnd);
    if focus && view != SidebarView::Hidden && is_shown(panel) {
        unsafe { SetFocus(panel) };
    }
    if view == SidebarView::Search {
        crate::window::search_view::shown(hwnd, focus);
    } else {
        crate::window::search_view::hidden(hwnd);
    }
}

/// `toggle` without its win events.
fn toggle_now(hwnd: HWND) {
    let Some(last) = with_sidebar(hwnd, |sidebar| sidebar.last_view) else {
        return;
    };
    let next = if current_view(hwnd) == SidebarView::Hidden {
        last
    } else {
        SidebarView::Hidden
    };
    show_view_now(hwnd, next, false);
}

/// `refresh` without its win events.
fn refresh_now(hwnd: HWND) {
    let Some((bar, panel)) = windows(hwnd) else {
        return;
    };
    crate::window::notebook_view::rebuild(hwnd);
    crate::window::favorites_view::refresh(hwnd, panel);
    crate::window::search_view::library_changed(hwnd);
    update_tools(hwnd);
    unsafe {
        InvalidateRect(bar, std::ptr::null(), 0);
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

/// `active_tab_changed` without its win events.
fn active_tab_changed_now(hwnd: HWND) {
    let Some((_, panel)) = windows(hwnd) else {
        return;
    };
    crate::window::notebook_view::active_tab_changed(hwnd);
    unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
}

/// Shows `view`, or closes the panel for `Hidden`, and saves it as `sidebar_view`. `focus` moves
/// the keyboard focus into the panel, or into the search box for Search. Raises the win events
/// for the panel's current child and the activity bar's pressed buttons.
pub(crate) fn show_view(hwnd: HWND, view: SidebarView, focus: bool) {
    let before = current_view(hwnd);
    with_accessible_events(hwnd, || show_view_now(hwnd, view, focus));
    bar_views_changed(hwnd, before);
}

/// Ctrl+B: closes the panel, or reopens the last view.
pub(crate) fn toggle(hwnd: HWND) {
    let before = current_view(hwnd);
    with_accessible_events(hwnd, || toggle_now(hwnd));
    bar_views_changed(hwnd, before);
}

/// Tells screen readers which activity-bar buttons' pressed state changed.
fn bar_views_changed(hwnd: HWND, before: SidebarView) {
    let after = current_view(hwnd);
    let Some((bar, _)) = windows(hwnd) else {
        return;
    };
    if before == after {
        return;
    }
    for index in [view_button(before), view_button(after)]
        .into_iter()
        .flatten()
    {
        sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE,
            bar,
            Some(index),
        );
    }
}

/// The library changed: rebuilds the Notebook view's rows from `LibraryState.tree` (no disk),
/// re-reads the notebook name for the Notebook tooltip, and repaints.
pub(crate) fn refresh(hwnd: HWND) {
    with_accessible_events(hwnd, || refresh_now(hwnd));
}

/// The active tab changed (`main_window::refresh_tabs` calls it): the Notebook view selects the
/// active note's row and expands its folders.
pub(crate) fn active_tab_changed(hwnd: HWND) {
    with_accessible_events(hwnd, || active_tab_changed_now(hwnd));
}

/// The pointer moved over the activity bar (`message` is the bar's mouse message). The first
/// move makes the bar's tooltip, which nothing before it needs, and hands it that move.
pub(crate) fn bar_pointer_moved(
    hwnd: HWND,
    bar: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) {
    let wanted = with_sidebar(hwnd, |sidebar| {
        sidebar.tooltip.is_none() && !sidebar.tooltip_failed
    })
    .unwrap_or(false);
    if !wanted {
        return;
    }
    // Made with nothing of the App borrowed: creating the control sends messages.
    let tooltip = Tooltip::create(bar);
    let kept = with_sidebar(hwnd, |sidebar| {
        sidebar.tooltip = tooltip;
        sidebar.tooltip_failed = tooltip.is_none();
    });
    match (tooltip, kept) {
        (Some(tooltip), Some(())) => {
            update_tools(hwnd);
            tooltip.relay(message, wparam, lparam);
        }
        (Some(tooltip), None) => tooltip.destroy(),
        _ => {}
    }
}

fn update_tools(hwnd: HWND) {
    let Some((bar, _)) = windows(hwnd) else {
        return;
    };
    let Some(tooltip) = with_sidebar(hwnd, |sidebar| sidebar.tooltip).flatten() else {
        return;
    };
    let mut client = RECT::default();
    unsafe { GetClientRect(bar, &mut client) };
    let rects = activity_bar::button_rects(client, unsafe { GetDpiForWindow(bar) }.max(96));
    let name = crate::window::library_host::folder(hwnd)
        .map(|folder| crate::window::library_host::notebook_name(&folder));
    let notebook = activity_bar::notebook_label(name.as_deref());
    for button in ActivityButton::ALL {
        let text = if button == ActivityButton::Notebook {
            notebook.as_str()
        } else {
            button.label()
        };
        tooltip.set_tool(button.index(), rects[button.index()], text);
    }
}

/// Runs `f` on the view the panel shows, with its client rectangle, DPI and focus.
fn with_accessible_view<R>(
    panel: HWND,
    f: impl FnOnce(&mut dyn AccessibleView, RECT, u32, bool) -> R,
) -> Option<R> {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(panel) };
    let view = current_view(main);
    let mut client = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(panel, &mut client);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
    let focused = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == panel;
    let mut app = unsafe { super::main_window::app_ptr(main) }?;
    let sidebar = unsafe { app.as_mut() }.sidebar.as_mut()?;
    match view {
        SidebarView::Notebook => Some(f(&mut sidebar.notebook, client, dpi, focused)),
        SidebarView::Search => Some(f(&mut sidebar.search, client, dpi, focused)),
        SidebarView::Favorites => Some(f(&mut sidebar.favorites, client, dpi, focused)),
        SidebarView::Hidden => None,
    }
}

/// How many MSAA children the panel has: one per header button and visible (flattened) row.
pub(crate) fn accessible_item_count(panel: HWND) -> usize {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_count(client, dpi)
    })
    .unwrap_or(0)
}

/// The panel's MSAA child `index` (0-based), built on its own.
pub(crate) fn accessible_item(panel: HWND, index: usize) -> Option<AccessibleItem> {
    with_accessible_view(panel, |view, client, dpi, focused| {
        view.accessible_item(index, client, dpi, focused)
    })
    .flatten()
}

fn accessible_container(panel: HWND) -> (String, u32) {
    use windows_sys::Win32::UI::Accessibility::{
        ROLE_SYSTEM_LIST, ROLE_SYSTEM_OUTLINE, ROLE_SYSTEM_PANE,
    };
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(panel) };
    match current_view(main) {
        SidebarView::Notebook => {
            // The same "Notebook: <name>" the activity bar's Notebook button reads.
            let name = super::library_host::folder(main)
                .map(|folder| super::library_host::notebook_name(&folder));
            (
                activity_bar::notebook_label(name.as_deref()),
                ROLE_SYSTEM_OUTLINE,
            )
        }
        SidebarView::Search => ("Search results".to_owned(), ROLE_SYSTEM_LIST),
        SidebarView::Favorites => ("Favorite notebooks".to_owned(), ROLE_SYSTEM_LIST),
        SidebarView::Hidden => ("Side panel".to_owned(), ROLE_SYSTEM_PANE),
    }
}

fn accessible_hit(panel: HWND, point: POINT) -> Option<usize> {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_hit(point, client, dpi)
    })
    .flatten()
}

fn accessible_current(panel: HWND) -> Option<usize> {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_current(client, dpi)
    })
    .flatten()
}

fn accessible_select(panel: HWND, index: usize) {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_select(index, client, dpi)
    });
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(panel, std::ptr::null(), 0);
    }
}

fn accessible_identity(panel: HWND, index: usize) -> Option<u64> {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_identity(index, client, dpi)
    })
    .flatten()
}

/// The shown view's order generation, told apart per view: switching views reorders too.
fn accessible_generation(panel: HWND) -> u64 {
    let main = unsafe { GetParent(panel) };
    let view = current_view(main) as u64;
    let generation =
        with_accessible_view(panel, |view, _, _, _| view.accessible_generation()).unwrap_or(0);
    generation.wrapping_mul(4).wrapping_add(view)
}

/// A child's default action: a click on its center, after scrolling it into view.
fn accessible_activate(panel: HWND, index: usize) {
    let Some(mut item) = accessible_item(panel, index) else {
        return;
    };
    if item.state & sidebar_accessibility::STATE_OFFSCREEN != 0 {
        accessible_select(panel, index);
        let Some(shown) = accessible_item(panel, index) else {
            return;
        };
        item = shown;
    }
    sidebar_accessibility::click_item(panel, item.rect);
}

pub(crate) static PANEL_ACCESSIBLE: AccessibleSource = AccessibleSource {
    container: accessible_container,
    count: accessible_item_count,
    item: accessible_item,
    hit: accessible_hit,
    current: accessible_current,
    select: accessible_select,
    activate: accessible_activate,
    identity: accessible_identity,
    generation: accessible_generation,
};

thread_local! {
    static ANNOUNCING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clears `ANNOUNCING` however the change returns.
struct Announcing;

impl Drop for Announcing {
    fn drop(&mut self) {
        ANNOUNCING.set(false);
    }
}

/// Runs `change` and raises the win events for what it did to the panel's current child (spec
/// §10). Nested calls run `change` alone, so one input raises one set of events.
pub(crate) fn with_accessible_events<R>(hwnd: HWND, change: impl FnOnce() -> R) -> R {
    let Some((_, panel)) = windows(hwnd) else {
        return change();
    };
    if ANNOUNCING.get() {
        return change();
    }
    ANNOUNCING.set(true);
    let guard = Announcing;
    let before = AccessibleMark::read(panel, &PANEL_ACCESSIBLE);
    let result = change();
    let after = AccessibleMark::read(panel, &PANEL_ACCESSIBLE);
    drop(guard);
    let focused = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == panel;
    sidebar_accessibility::raise(
        panel,
        &sidebar_accessibility::events_between(&before, &after, focused),
    );
    result
}

pub(crate) fn announcing() -> bool {
    ANNOUNCING.get()
}

/// The activity-bar button the keyboard is on (0 Notebook, 1 Search, 2 Favorites, 3 Settings).
pub(crate) fn bar_focus(hwnd: HWND) -> usize {
    unsafe { super::main_window::app_ptr(hwnd) }
        .and_then(|app| {
            unsafe { app.as_ref() }
                .sidebar
                .as_ref()
                .map(|s| s.bar_focus)
        })
        .unwrap_or(0)
}

pub(crate) fn set_bar_focus(hwnd: HWND, index: usize) {
    if let Some(mut app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_mut() }.sidebar.as_mut()
    {
        sidebar.bar_focus = index.min(3);
    }
}

/// The activity-bar button of a view.
pub(crate) fn view_button(view: SidebarView) -> Option<usize> {
    match view {
        SidebarView::Notebook => Some(0),
        SidebarView::Search => Some(1),
        SidebarView::Favorites => Some(2),
        SidebarView::Hidden => None,
    }
}

fn focus_is_in(window: HWND) -> bool {
    let focus = unsafe { GetFocus() };
    !focus.is_null() && (focus == window || unsafe { IsChild(window, focus) } != 0)
}

/// The window's own visible bit; the main window may be hidden (tests) or minimized.
fn is_shown(window: HWND) -> bool {
    (unsafe { GetWindowLongPtrW(window, GWL_STYLE) }) as u32 & WS_VISIBLE != 0
}

/// The signed client or screen point in a mouse message's `lParam`.
pub(crate) fn point_of(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    )
}

/// Registers a painted child window class once per process and returns its name. `cell` keeps
/// the outcome, so a failed registration is reported every time without retrying.
pub(crate) fn register_child_class(
    cell: &'static std::sync::OnceLock<Option<Vec<u16>>>,
    name: &str,
    style: u32,
    proc: WNDPROC,
) -> crate::Result<&'static [u16]> {
    cell.get_or_init(|| {
        let wide = wide_null(name);
        let class = WNDCLASSW {
            style,
            lpfnWndProc: proc,
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: wide.as_ptr(),
            ..Default::default()
        };
        let registered =
            unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS };
        registered.then_some(wide)
    })
    .as_deref()
    .ok_or(crate::FastPadError::Invariant(
        "a sidebar window class could not be registered",
    ))
}

fn register_panel_class() -> crate::Result<&'static [u16]> {
    static CLASS: std::sync::OnceLock<Option<Vec<u16>>> = std::sync::OnceLock::new();
    // Double-clicks reset the width from the edge and, from Task 10, open rows permanently.
    register_child_class(&CLASS, "FastPadSidePanel", CS_DBLCLKS, Some(panel_proc))
}

/// Paints `window` through an off-screen bitmap of its client size, so a repaint never flickers.
pub(crate) fn paint_buffered(window: HWND, draw: impl FnOnce(HDC, RECT)) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if dc.is_null() {
        return;
    }
    let mut client = RECT::default();
    unsafe { GetClientRect(window, &mut client) };
    let (width, height) = (client.right, client.bottom);
    if width > 0 && height > 0 {
        let memory = unsafe { CreateCompatibleDC(dc) };
        let bitmap = if memory.is_null() {
            std::ptr::null_mut()
        } else {
            unsafe { CreateCompatibleBitmap(dc, width, height) }
        };
        if bitmap.is_null() {
            draw(dc, client);
        } else {
            unsafe {
                let previous = SelectObject(memory, bitmap);
                draw(memory, client);
                BitBlt(dc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
                SelectObject(memory, previous);
                DeleteObject(bitmap);
            }
        }
        if !memory.is_null() {
            unsafe { DeleteDC(memory) };
        }
    }
    unsafe { EndPaint(window, &paint) };
}

unsafe extern "system" fn panel_proc(
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = unsafe { GetParent(panel) };
    if matches!(
        message,
        WM_KEYDOWN
            | WM_CHAR
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_MOUSEWHEEL
            | WM_COMMAND
            | sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION
    ) && !announcing()
    {
        return with_accessible_events(main, || unsafe {
            panel_proc(panel, message, wparam, lparam)
        });
    }
    match message {
        WM_PAINT => {
            paint_panel(main, panel);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => panel_hit_test(main, panel, wparam, lparam),
        WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => unsafe {
            sidebar_accessibility::object_result(panel, &PANEL_ACCESSIBLE, wparam)
        },
        sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACCESSIBLE => unsafe {
            sidebar_accessibility::answer(panel, lparam)
        },
        sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION => {
            sidebar_accessibility::run_action(panel, &PANEL_ACCESSIBLE, wparam, lparam);
            0
        }
        // Esc anywhere in the panel returns to the editor (spec §10). The search box's own Esc
        // is handled in its subclass.
        WM_KEYDOWN if wparam as u16 == VK_ESCAPE => {
            return_focus_to_editor(main);
            0
        }
        WM_SETCURSOR if resizing(main) || pointer_over_grip(panel) => {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE)) };
            1
        }
        WM_LBUTTONDOWN if over_grip(panel, point_of(lparam).0) => {
            begin_resize(main, panel);
            0
        }
        WM_MOUSEMOVE if drag_resize(main, panel, point_of(lparam).0) => 0,
        WM_LBUTTONUP if finish_resize(main, true) => 0,
        WM_LBUTTONDBLCLK if over_grip(panel, point_of(lparam).0) => {
            save_width(main, DEFAULT_SIDEBAR_WIDTH);
            0
        }
        // Capture taken away mid-drag (a task switch, a dialog): keep and save what was reached.
        WM_CAPTURECHANGED if finish_resize(main, false) => 0,
        // The Search view's Alt+C, Alt+W and Alt+R, before the menu band sees the letter.
        WM_SYSKEYDOWN | WM_SYSCHAR if current_view(main) == SidebarView::Search => {
            route(main, panel, message, wparam, lparam)
        }
        // Everything else a view may want goes to it first. Wheel and context-menu positions are
        // screen coordinates; the view converts them.
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_CAPTURECHANGED
        | WM_MOUSELEAVE | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_CONTEXTMENU | WM_MOUSEWHEEL
        | WM_KEYDOWN | WM_CHAR => route(main, panel, message, wparam, lparam),
        WM_SETFOCUS | WM_KILLFOCUS => {
            unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
            0
        }
        // The search box's text changed: re-run the search.
        WM_COMMAND if lparam != 0 && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE => {
            crate::window::search_view::query_changed(main);
            0
        }
        WM_CTLCOLOREDIT => {
            crate::window::search_view::control_color(main, wparam as HDC) as LRESULT
        }
        _ => unsafe { DefWindowProcW(panel, message, wparam, lparam) },
    }
}

/// Hands `message` to the shown view: keys to `view_key`, the rest to `view_mouse`.
/// `DefWindowProcW` handles whatever the view leaves (`None`).
fn route(main: HWND, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let answer = PanelView::of(current_view(main)).and_then(|view| match message {
        WM_KEYDOWN | WM_CHAR | WM_SYSKEYDOWN | WM_SYSCHAR => {
            view_key(main, view, panel, message, wparam, lparam)
        }
        _ => view_mouse(main, view, panel, message, wparam, lparam),
    });
    answer.unwrap_or_else(|| unsafe { DefWindowProcW(panel, message, wparam, lparam) })
}

/// The empty part of the header is caption: the main window's hit test decides there.
fn panel_hit_test(main: HWND, panel: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let (x, y) = point_of(lparam);
    let mut point = POINT { x, y };
    unsafe { ScreenToClient(panel, &mut point) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let caption = point.y >= 0
        && point.y < scale(HEADER_HEIGHT_96, dpi)
        && !over_grip(panel, point.x)
        && PanelView::of(current_view(main))
            .is_some_and(|view| header_is_caption(main, view, panel, point.x, point.y));
    if caption {
        HTTRANSPARENT as LRESULT
    } else {
        unsafe { DefWindowProcW(panel, WM_NCHITTEST, wparam, lparam) }
    }
}

fn paint_panel(main: HWND, panel: HWND) {
    let view = PanelView::of(current_view(main));
    paint_buffered(panel, |dc, client| {
        let paint = view_paint(main, panel, dc, client);
        unsafe { fill(dc, client, paint.background) };
        if let Some(view) = view {
            paint_view(main, view, &paint);
        }
        // A hairline where the editor area begins, over whatever the view painted.
        unsafe {
            fill(
                dc,
                RECT {
                    left: (client.right - 1).max(client.left),
                    ..client
                },
                paint.palette.strip_background,
            );
        }
    });
}

/// Paints `view` over the panel's background.
fn paint_view(main: HWND, view: PanelView, paint: &ViewPaint) {
    match view {
        PanelView::Notebook => crate::window::notebook_view::paint(main, paint),
        PanelView::Search => crate::window::search_view::paint(main, paint),
        PanelView::Favorites => crate::window::favorites_view::paint(main, paint),
    }
}

/// Mouse input (and `WM_CONTEXTMENU`, `WM_MOUSELEAVE`, `WM_CAPTURECHANGED`) for `view`, with the
/// message's own `wparam` and `lparam`. `None` leaves it to `DefWindowProcW`.
fn view_mouse(
    main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook => crate::window::notebook_view::handle(main, message, wparam, lparam),
        PanelView::Search => {
            crate::window::search_view::handle(main, panel, message, wparam, lparam)
        }
        PanelView::Favorites => {
            crate::window::favorites_view::handle(main, panel, message, wparam, lparam)
        }
    }
}

/// `WM_KEYDOWN`, `WM_CHAR`, `WM_SYSKEYDOWN` and `WM_SYSCHAR` while the panel has the focus. `None`
/// leaves the key to `DefWindowProcW`. Each view's `handle` takes keys and mouse messages alike.
fn view_key(
    main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    view_mouse(main, view, panel, message, wparam, lparam)
}

/// Whether header point `x`, `y` (panel client coordinates) is empty, so the window drags from
/// it: not the Notebook header's title and buttons, the Search header's field (the search box
/// and the padding painted around it), nor the Favorites header's Open notebook… button.
fn header_is_caption(main: HWND, view: PanelView, panel: HWND, x: i32, y: i32) -> bool {
    match view {
        PanelView::Notebook => !crate::window::notebook_view::header_hit(main, x, y),
        PanelView::Search => !crate::window::search_view::header_hit(main, panel, x, y),
        PanelView::Favorites => {
            let mut client = RECT::default();
            unsafe { GetClientRect(panel, &mut client) };
            let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
            !crate::window::favorites_view::header_controls(client, dpi)
                .iter()
                .any(|rect| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
        }
    }
}

fn over_grip(panel: HWND, x: i32) -> bool {
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    x >= client.right - scale(GRIP_WIDTH_96, dpi) && x < client.right
}

fn pointer_over_grip(panel: HWND) -> bool {
    let mut point = POINT::default();
    (unsafe { GetCursorPos(&mut point) }) != 0
        && unsafe { ScreenToClient(panel, &mut point) } != 0
        && over_grip(panel, point.x)
}

fn resizing(main: HWND) -> bool {
    with_sidebar(main, |sidebar| sidebar.drag_width.is_some()).unwrap_or(false)
}

fn begin_resize(main: HWND, panel: HWND) {
    let Some((_, width)) = open_state(main) else {
        return;
    };
    with_sidebar(main, |sidebar| sidebar.drag_width = Some(width));
    unsafe { SetCapture(panel) };
}

/// While the edge is dragged: `x` (panel coordinates) is the new width, since the panel's left
/// edge stays put. Reports whether a drag is in progress.
fn drag_resize(main: HWND, panel: HWND, x: i32) -> bool {
    if !resizing(main) {
        return false;
    }
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let width = drag_width_96(x, dpi);
    let changed = with_sidebar(main, |sidebar| {
        sidebar.drag_width.replace(width) != Some(width)
    })
    .unwrap_or(false);
    if changed {
        layout_editor_and_find_bar(main);
        invalidate_title_strip(main);
    }
    true
}

/// Ends a drag and saves the width once. `release` is set when the button went up; otherwise
/// capture was taken away. Reports whether a drag was in progress.
fn finish_resize(main: HWND, release: bool) -> bool {
    let Some(width) = with_sidebar(main, |sidebar| sidebar.drag_width.take()).flatten() else {
        return false;
    };
    if release {
        unsafe { ReleaseCapture() };
    }
    save_width(main, width);
    true
}

fn save_width(main: HWND, width: u16) {
    change_setting(main, |settings| {
        (settings.sidebar_width != width).then(|| {
            settings.sidebar_width = width;
            ("sidebar_width", width.to_string())
        })
    });
    layout_editor_and_find_bar(main);
    invalidate_title_strip(main);
}

#[cfg(test)]
mod tests {
    use super::{PanelView, drag_width_96, sidebar_widths};
    use crate::config::SidebarView;
    use crate::window::palette::Palette;

    #[test]
    fn the_panel_gives_way_before_the_editor_minimum_and_scales_with_dpi() {
        // Break caught: an editor squeezed below 320 px by a wide panel, a negative panel width
        // on a tiny window, or sizes that ignore the monitor's DPI.
        assert_eq!(sidebar_widths(1280, 96, true, 260), (44, 260));
        assert_eq!(sidebar_widths(1280, 96, false, 260), (44, 0));
        assert_eq!(sidebar_widths(1920, 144, true, 260), (66, 390));
        assert_eq!(sidebar_widths(600, 96, true, 260), (44, 236));
        assert_eq!(sidebar_widths(300, 96, true, 260), (44, 0));
        assert_eq!(sidebar_widths(20, 96, true, 260), (20, 0));
        assert_eq!(sidebar_widths(-5, 96, true, 260), (0, 0));
        // A hand-edited width outside the range is clamped, never trusted.
        assert_eq!(sidebar_widths(1280, 96, true, 100), (44, 180));
        assert_eq!(sidebar_widths(1280, 96, true, 900), (44, 480));
    }

    #[test]
    fn a_dragged_width_is_stored_at_96_dpi_inside_the_range() {
        assert_eq!(drag_width_96(300, 96), 300);
        assert_eq!(drag_width_96(390, 144), 260);
        assert_eq!(drag_width_96(100, 96), 180);
        assert_eq!(drag_width_96(-40, 96), 180);
        assert_eq!(drag_width_96(2000, 96), 480);
    }

    #[test]
    fn the_panel_shade_sits_between_the_strip_and_the_editor() {
        let palette = Palette {
            strip_background: 0x0020_4060,
            editor_background: 0x00a0_c0e0,
            ..Palette::neutral()
        };
        assert_eq!(palette.panel_background(), 0x0060_80a0);
    }

    #[test]
    fn every_open_view_has_a_panel_view_and_a_header_title() {
        assert_eq!(
            PanelView::of(SidebarView::Notebook),
            Some(PanelView::Notebook)
        );
        assert_eq!(PanelView::of(SidebarView::Search), Some(PanelView::Search));
        assert_eq!(
            PanelView::of(SidebarView::Favorites),
            Some(PanelView::Favorites)
        );
        assert_eq!(PanelView::of(SidebarView::Hidden), None);
    }
}
