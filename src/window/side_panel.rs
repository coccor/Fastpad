//! The sidebar: the activity bar and one side panel, two painted child windows along the main
//! window's left edge, present only in notes mode. Everything else in the window is laid out to
//! their right (`left_edge`). The panel paints the current view. The views plug in through the
//! `PanelView` dispatch (`paint_view`, `view_mouse`, `view_key`, `header_is_caption`), which
//! Tasks 10 and 12 extend.
//!
//! The strip above the first activity button and the empty part of the panel header belong to the
//! window caption. Both windows answer `WM_NCHITTEST` there with `HTTRANSPARENT`, so the main
//! window's own hit test applies: dragging, top-edge resizing and double-click to maximize.

use super::activity_bar::{self, ActivityButton, BarState};
use super::main_window::{
    app_ptr, change_setting, current_palette, focus_content, invalidate_title_strip,
    layout_editor_and_find_bar, push_notice, tab_count, ui_fonts,
};
use super::tooltip::Tooltip;
use crate::config::SidebarView;
use crate::config::defaults::{DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH};
use crate::platform::wide_null;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, scale};
use crate::window::titlebar::create_ui_font;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DRAW_TEXT_FORMAT, DT_CALCRECT,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT, InvalidateRect, PAINTSTRUCT, SRCCOPY,
    ScreenToClient, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, DestroyWindow, GWL_STYLE, GetClientRect, GetCursorPos, GetParent,
    GetWindowLongPtrW, HTTRANSPARENT, IDC_ARROW, IDC_SIZEWE, IsChild, LoadCursorW, RegisterClassW,
    SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetCursor, SetWindowPos, ShowWindow,
    WM_CAPTURECHANGED, WM_CHAR, WM_CONTEXTMENU, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST,
    WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS, WNDCLASSW, WNDPROC,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// Sizes at 96 DPI, scaled with `panel::scale`.
pub(crate) const ACTIVITY_WIDTH_96: i32 = 44;
pub(crate) const EDITOR_MIN_WIDTH_96: i32 = 320;
pub(crate) const HEADER_HEIGHT_96: i32 = 38;
/// The strip along the panel's right edge that resizes it.
pub(crate) const GRIP_WIDTH_96: i32 = 4;
const HEADER_INSET_96: i32 = 16;

/// The sidebar's fonts at one DPI. Painting copies them out; `Sidebar` owns and deletes them.
#[derive(Clone, Copy, Debug)]
#[allow(
    dead_code,
    reason = "the text, italic and glyph fonts are read from Task 10's Notebook view on"
)]
pub(crate) struct UiFonts {
    /// Row and body text: Segoe UI, 12 px at 96 DPI.
    pub(crate) text: HFONT,
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
            bold: create_ui_font(scale(11, dpi), "Segoe UI", FW_SEMIBOLD as i32, false),
            italic: create_ui_font(scale(12, dpi), "Segoe UI", normal, true),
            glyph: create_ui_font(scale(12, dpi), "Segoe MDL2 Assets", normal, false),
            bar_glyph: create_ui_font(scale(16, dpi), "Segoe MDL2 Assets", normal, false),
        }
    }

    fn delete(self) {
        for font in [
            self.text,
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
    pub(crate) tooltip: Option<Tooltip>,
    pub(crate) bar_state: BarState,
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
}

impl Drop for Sidebar {
    fn drop(&mut self) {
        if let Some((_, fonts)) = self.fonts.take() {
            fonts.delete();
        }
    }
}

/// The view the panel is showing. Tasks 10 and 12 give each view its painting and input
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

    /// The header's title, in small capitals.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Notebook => "NOTEBOOK",
            Self::Search => "SEARCH",
            Self::Favorites => "FAVORITES",
        }
    }
}

/// What a view paints with: the panel's buffered DC and everything a paint needs, built once per
/// `WM_PAINT` by `view_paint`.
#[derive(Clone, Copy)]
#[allow(
    dead_code,
    reason = "`focused` is read from Task 10's Notebook view on"
)]
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
    Ok(Sidebar {
        bar,
        panel,
        tooltip: Tooltip::create(bar),
        bar_state: BarState::default(),
        last_view: SidebarView::Notebook,
        drag_width: None,
        fonts: None,
    })
}

/// Creates or destroys the sidebar to match notes mode, then lays the window out again. It also
/// runs once `fastpad.ini` has been applied, to pick up the saved view.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    let present =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.sidebar.is_some());
    if enabled && !present {
        match create(hwnd) {
            Ok(sidebar) => match unsafe { app_ptr(hwnd) } {
                Some(mut app) => unsafe { app.as_mut() }.sidebar = Some(sidebar),
                None => unsafe {
                    DestroyWindow(sidebar.panel);
                    DestroyWindow(sidebar.bar);
                },
            },
            Err(error) => push_notice(hwnd, format!("FastPad could not show the sidebar: {error}")),
        }
    } else if !enabled && present {
        let sidebar =
            unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.sidebar.take());
        if let Some(sidebar) = sidebar {
            if focus_is_in(sidebar.panel) || focus_is_in(sidebar.bar) {
                return_focus(hwnd);
            }
            // The bar owns the tooltip, which goes with it.
            unsafe {
                DestroyWindow(sidebar.panel);
                DestroyWindow(sidebar.bar);
            }
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
    layout_editor_and_find_bar(hwnd);
    invalidate_title_strip(hwnd);
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
            return_focus(hwnd);
        }
        unsafe { ShowWindow(panel, SW_HIDE) };
    }
    update_tools(hwnd);
}

/// Shows `view`, or closes the panel for `Hidden`, and saves it as `sidebar_view`. `focus` moves
/// the keyboard focus into the panel. Task 12 moves it into the search box for Search.
pub(crate) fn show_view(hwnd: HWND, view: SidebarView, focus: bool) {
    let Some(panel) = with_sidebar(hwnd, |sidebar| {
        if view != SidebarView::Hidden {
            sidebar.last_view = view;
        }
        sidebar.panel
    }) else {
        return;
    };
    if view == SidebarView::Hidden && focus_is_in(panel) {
        return_focus(hwnd);
    }
    change_setting(hwnd, |settings| {
        (settings.sidebar_view != view).then(|| {
            settings.sidebar_view = view;
            ("sidebar_view", view.token().to_owned())
        })
    });
    layout_editor_and_find_bar(hwnd);
    invalidate_title_strip(hwnd);
    if focus && view != SidebarView::Hidden && is_shown(panel) {
        unsafe { SetFocus(panel) };
    }
}

/// Ctrl+B: closes the panel, or reopens the last view.
pub(crate) fn toggle(hwnd: HWND) {
    let Some(last) = with_sidebar(hwnd, |sidebar| sidebar.last_view) else {
        return;
    };
    let next = if current_view(hwnd) == SidebarView::Hidden {
        last
    } else {
        SidebarView::Hidden
    };
    show_view(hwnd, next, false);
}

/// The library changed: re-reads what the sidebar shows of it (the notebook name in the
/// Notebook tooltip) and repaints.
pub(crate) fn refresh(hwnd: HWND) {
    let Some((bar, panel)) = windows(hwnd) else {
        return;
    };
    update_tools(hwnd);
    unsafe {
        InvalidateRect(bar, std::ptr::null(), 0);
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

/// The active tab changed (`main_window::refresh_tabs` calls it). Task 10 selects the active
/// note's row here.
pub(crate) fn active_tab_changed(hwnd: HWND) {
    if let Some((_, panel)) = windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
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
    let name = crate::window::library_host::folder(hwnd).and_then(|folder| {
        folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });
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

fn focus_is_in(window: HWND) -> bool {
    let focus = unsafe { GetFocus() };
    !focus.is_null() && (focus == window || unsafe { IsChild(window, focus) } != 0)
}

/// Focus leaving the sidebar goes to the content, or to the frame while no tab is open.
fn return_focus(hwnd: HWND) {
    if tab_count(hwnd) > 0 {
        focus_content(hwnd);
    } else {
        unsafe { SetFocus(hwnd) };
    }
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
    match message {
        WM_PAINT => {
            paint_panel(main, panel);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => panel_hit_test(main, panel, wparam, lparam),
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
        // Everything else a view may want goes to it first. Wheel and context-menu positions are
        // screen coordinates; the view converts them.
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_CAPTURECHANGED
        | WM_MOUSELEAVE | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_CONTEXTMENU | WM_MOUSEWHEEL
        | WM_KEYDOWN | WM_CHAR => route(main, panel, message, wparam, lparam),
        WM_SETFOCUS | WM_KILLFOCUS => {
            unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
            0
        }
        _ => unsafe { DefWindowProcW(panel, message, wparam, lparam) },
    }
}

/// Hands `message` to the shown view: keys to `view_key`, the rest to `view_mouse`.
/// `DefWindowProcW` handles whatever the view leaves (`None`).
fn route(main: HWND, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let answer = PanelView::of(current_view(main)).and_then(|view| match message {
        WM_KEYDOWN | WM_CHAR => view_key(main, view, panel, message, wparam, lparam),
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

/// Paints `view` over the panel's background. Task 10 gives the Notebook view its own paint,
/// and Task 12 the other two.
fn paint_view(_main: HWND, view: PanelView, paint: &ViewPaint) {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => {
            paint_header_title(view, paint);
        }
    }
}

/// A view's header title alone, until the view paints itself.
fn paint_header_title(view: PanelView, paint: &ViewPaint) {
    let inset = scale(HEADER_INSET_96, paint.dpi);
    let title = RECT {
        left: paint.client.left + inset,
        right: (paint.client.right - inset).max(paint.client.left + inset),
        bottom: (paint.client.top + scale(HEADER_HEIGHT_96, paint.dpi)).min(paint.client.bottom),
        ..paint.client
    };
    unsafe {
        draw_text(
            paint.hdc,
            view.title(),
            title,
            paint.fonts.bold,
            paint.palette.muted_foreground,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

/// Mouse input (and `WM_CONTEXTMENU`, `WM_MOUSELEAVE`, `WM_CAPTURECHANGED`) for `view`, with the
/// message's own `wparam` and `lparam`. `None` leaves it to `DefWindowProcW`. Tasks 10 and 12
/// replace these arms with their views' handlers.
fn view_mouse(
    _main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => {
            (message == WM_LBUTTONDOWN).then(|| {
                unsafe { SetFocus(panel) };
                0
            })
        }
    }
}

/// `WM_KEYDOWN` and `WM_CHAR` while the panel has the focus. `None` leaves the key to
/// `DefWindowProcW`. Task 10 handles the tree's keys and Task 12 the lists' keys.
fn view_key(
    _main: HWND,
    view: PanelView,
    _panel: HWND,
    _message: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => None,
    }
}

/// Whether header point `x`, `y` (panel client coordinates) is empty, so the window drags from
/// it. Task 10 excludes the Notebook header's buttons and title, and Task 12 the Favorites
/// header's Open notebook… button.
fn header_is_caption(_main: HWND, view: PanelView, _panel: HWND, _x: i32, _y: i32) -> bool {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => true,
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
        assert_eq!(PanelView::Favorites.title(), "FAVORITES");
    }
}
