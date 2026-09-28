use crate::window::palette::Palette;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_USE_IMMERSIVE_DARK_MODE, DwmDefWindowProc, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, DC_BRUSH, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CALCRECT,
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, DeleteDC, DeleteObject, DrawTextW, EndPaint, ExcludeClipRect, FW_NORMAL,
    FillRect, GetMonitorInfoW, GetStockObject, HDC, HFONT, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, MonitorFromWindow, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, ScreenToClient, SelectObject, SetBkMode, SetDCBrushColor, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::SetWindowTheme;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DestroyIcon, GetClientRect, HICON, HTCAPTION, HTCLIENT, HTCLOSE, HTMAXBUTTON,
    HTMINBUTTON, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, MINMAXINFO, NCCALCSIZE_PARAMS,
    SM_CXPADDEDBORDER, SM_CYFRAME, SM_CYSIZE, WM_NCCALCSIZE, WM_NCHITTEST,
};

const GLYPH_MINIMIZE: &str = "\u{E921}";
const GLYPH_MAXIMIZE: &str = "\u{E922}";
const GLYPH_RESTORE: &str = "\u{E923}";
pub(crate) const GLYPH_CLOSE: &str = "\u{E8BB}";
pub(crate) const GLYPH_MORE: &str = "\u{E712}";
pub(crate) const GLYPH_PREVIEW_SIDE: &str = "\u{E90D}";
pub(crate) const GLYPH_PREVIEW_FULL: &str = "\u{E8FF}";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

impl Size {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn left(self) -> i32 {
        self.left
    }

    pub const fn right(self) -> i32 {
        self.right
    }

    pub const fn center(self) -> Point {
        Point::new((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }

    pub const fn contains(self, point: Point) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    pub(crate) fn centered_square(self, size: i32) -> Self {
        let center = self.center();
        let half = size.min(self.right - self.left).min(self.bottom - self.top) / 2;
        Self::new(
            center.x - half,
            center.y - half,
            center.x + half,
            center.y + half,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitTarget {
    Client,
    Caption,
    Minimize,
    Maximize,
    Close,
    /// The application menu ("…").
    Overflow,
    ResizeTop,
    ResizeTopLeft,
    ResizeTopRight,
}

impl HitTarget {
    /// Caption buttons report non-client hit codes, so their pointer messages are non-client.
    pub const fn is_caption_button(self) -> bool {
        matches!(self, Self::Minimize | Self::Maximize | Self::Close)
    }

    pub const fn is_interactive(self) -> bool {
        matches!(
            self,
            Self::Minimize | Self::Maximize | Self::Close | Self::Overflow
        )
    }

    pub fn from_nonclient_code(code: usize) -> Option<Self> {
        match u32::try_from(code).ok()? {
            HTMINBUTTON => Some(Self::Minimize),
            HTMAXBUTTON => Some(Self::Maximize),
            HTCLOSE => Some(Self::Close),
            _ => None,
        }
    }
}

/// Which title-strip target the pointer is over and which one a primary button went down on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PointerState {
    pub hovered: Option<HitTarget>,
    pub pressed: Option<HitTarget>,
}

impl PointerState {
    #[must_use]
    pub fn hover(self, target: Option<HitTarget>) -> Self {
        Self {
            hovered: target.filter(|target| target.is_interactive()),
            ..self
        }
    }

    #[must_use]
    pub fn press(self, target: Option<HitTarget>) -> Self {
        Self {
            pressed: target.filter(|target| target.is_interactive()),
            ..self
        }
    }

    pub fn is_pressed(self, target: HitTarget) -> bool {
        self.pressed == Some(target)
    }

    /// Clears the press and returns the target to activate when released over the pressed target.
    #[must_use]
    pub fn release(self, target: Option<HitTarget>) -> (Self, Option<HitTarget>) {
        let activated = self.pressed.filter(|pressed| target == Some(*pressed));
        (
            Self {
                pressed: None,
                ..self
            },
            activated,
        )
    }

    /// Client and non-client leave notifications arrive independently; each clears only its own
    /// area's hover so an out-of-order leave cannot drop the other area's highlight.
    #[must_use]
    pub fn leave(self, nonclient: bool) -> Self {
        if self
            .hovered
            .is_some_and(|target| target.is_caption_button() == nonclient)
        {
            Self::default()
        } else {
            self
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TitleBarLayout {
    /// The empty strip between the sidebar and the app menu: drags the window, maximizes it on
    /// a double-click, and shows the window title. The tabs are in the editor group now.
    pub drag_region: Rect,
    /// The sidebar's share of the strip. It is caption (dragging, top-edge resizing,
    /// double-click to maximize), and empty without a sidebar.
    pub sidebar: Rect,
    pub minimize: Rect,
    pub maximize: Rect,
    pub close: Rect,
    /// The application menu ("…").
    pub overflow: Rect,
    pub height: i32,
    /// Height of the top band that resizes a restored window.
    pub resize_border: i32,
}

pub(crate) const fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 1 } else { dpi };
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

impl TitleBarLayout {
    pub fn calculate(client: Size, dpi: u32) -> Self {
        Self::calculate_with_offset(client, dpi, 0)
    }

    /// The strip with the caption starting at `left`, right of the sidebar. The caption buttons
    /// keep the right edge, and everything left of `left` is caption too.
    pub fn calculate_with_offset(client: Size, dpi: u32, left: i32) -> Self {
        let width = client.width.max(0);
        let dpi = dpi.max(1);
        let height = strip_height(dpi);
        let resize_border = unsafe {
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
        .clamp(1, (height / 2).max(1));
        let caption_width = scale(46, dpi).max(1).min(width / 3);
        let close = Rect::new(width - caption_width, 0, width, height);
        let maximize = Rect::new(close.left - caption_width, 0, close.left, height);
        let minimize = Rect::new(maximize.left - caption_width, 0, maximize.left, height);

        let actions_right = minimize.left.max(0);
        let overflow_left = (actions_right - scale(40, dpi)).max(0);
        let overflow = Rect::new(overflow_left, 0, actions_right, height);
        let left = left.clamp(0, overflow_left);

        Self {
            drag_region: Rect::new(left, 0, overflow_left, height),
            sidebar: Rect::new(0, 0, left, height),
            minimize,
            maximize,
            close,
            overflow,
            height,
            resize_border,
        }
    }

    pub fn hit_test(&self, point: Point) -> HitTarget {
        if self.close.contains(point) {
            return HitTarget::Close;
        }
        if self.maximize.contains(point) {
            return HitTarget::Maximize;
        }
        if self.minimize.contains(point) {
            return HitTarget::Minimize;
        }
        if self.overflow.contains(point) {
            return HitTarget::Overflow;
        }
        if self.sidebar.contains(point) || self.drag_region.contains(point) {
            return HitTarget::Caption;
        }
        HitTarget::Client
    }

    /// Like `hit_test`, but a restored window's top band resizes (the frame no longer reserves a
    /// native top border). Caption buttons always keep their targets; the right frame border outside
    /// the client area still reports the top-right corner through DefWindowProc.
    pub fn frame_hit_test(&self, point: Point, maximized: bool) -> HitTarget {
        let target = self.hit_test(point);
        if maximized || point.y < 0 || point.y >= self.resize_border {
            return target;
        }
        if target.is_caption_button() {
            target
        } else if point.x < self.resize_border {
            HitTarget::ResizeTopLeft
        } else if point.x >= self.close.right - self.resize_border {
            HitTarget::ResizeTopRight
        } else {
            HitTarget::ResizeTop
        }
    }

    fn strip(&self) -> Rect {
        Rect::new(0, 0, self.close.right, self.height)
    }
}

/// `work_area` is `Some` only for a maximized window: its client fills the monitor work area so no
/// frame or caption button lands off-screen. A restored window keeps the proposed top edge and the
/// default left/right/bottom resize borders.
pub fn frame_client_rect(proposed: Rect, default_client: Rect, work_area: Option<Rect>) -> Rect {
    match work_area {
        Some(work) => Rect::new(
            proposed.left.max(work.left),
            proposed.top.max(work.top),
            proposed.right.min(work.right),
            proposed.bottom.min(work.bottom),
        ),
        None => Rect {
            top: proposed.top,
            ..default_client
        },
    }
}

pub(crate) fn layout_for_window(hwnd: HWND) -> TitleBarLayout {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    TitleBarLayout::calculate_with_offset(
        Size::new(client.right - client.left, client.bottom - client.top),
        unsafe { GetDpiForWindow(hwnd) }.max(96),
        crate::window::side_panel::left_edge(hwnd),
    )
}

pub(crate) fn invalidate_strip(hwnd: HWND) {
    let strip = native_rect(layout_for_window(hwnd).strip());
    unsafe {
        InvalidateRect(hwnd, &strip, 0);
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TitleFontHandles {
    text: HFONT,
    italic: HFONT,
    glyph: HFONT,
}

impl Default for TitleFontHandles {
    fn default() -> Self {
        Self {
            text: std::ptr::null_mut(),
            italic: std::ptr::null_mut(),
            glyph: std::ptr::null_mut(),
        }
    }
}

/// Title-strip fonts for one DPI, deleted on drop (with the App at `WM_NCDESTROY`).
#[derive(Debug)]
pub(crate) struct TitleFonts {
    dpi: u32,
    handles: TitleFontHandles,
}

impl TitleFonts {
    pub(crate) fn create(dpi: u32) -> Self {
        Self {
            dpi,
            handles: TitleFontHandles {
                text: create_font(scale(12, dpi), "Segoe UI"),
                italic: create_ui_font(scale(12, dpi), "Segoe UI", FW_NORMAL as i32, true),
                glyph: create_font(scale(10, dpi), "Segoe MDL2 Assets"),
            },
        }
    }

    pub(crate) fn dpi(&self) -> u32 {
        self.dpi
    }

    pub(crate) fn handles(&self) -> TitleFontHandles {
        self.handles
    }
}

impl TitleFontHandles {
    /// The UI text font, null before chrome fonts exist.
    pub(crate) fn text(&self) -> HFONT {
        self.text
    }

    /// The Segoe MDL2 Assets glyph font, null before chrome fonts exist.
    pub(crate) fn glyph(&self) -> HFONT {
        self.glyph
    }

    /// The preview tab's label font; the plain text font until it exists.
    pub(crate) fn italic(&self) -> HFONT {
        if self.italic.is_null() {
            self.text
        } else {
            self.italic
        }
    }
}

impl Drop for TitleFonts {
    fn drop(&mut self) {
        for font in [self.handles.text, self.handles.italic, self.handles.glyph] {
            if !font.is_null() {
                unsafe {
                    DeleteObject(font);
                }
            }
        }
    }
}

/// The activity bar's logo icon, loaded (`main_window::load_logo_icon`) for one DPI and destroyed
/// on drop (with the App at `WM_NCDESTROY`), the same lifetime `TitleFonts` has.
#[derive(Debug)]
pub(crate) struct LogoIcon {
    dpi: u32,
    icon: HICON,
}

impl LogoIcon {
    /// Takes ownership of `icon`, already loaded for `dpi`.
    pub(crate) fn new(dpi: u32, icon: HICON) -> Self {
        Self { dpi, icon }
    }

    pub(crate) fn dpi(&self) -> u32 {
        self.dpi
    }

    pub(crate) fn icon(&self) -> HICON {
        self.icon
    }
}

impl Drop for LogoIcon {
    fn drop(&mut self) {
        if !self.icon.is_null() {
            unsafe {
                DestroyIcon(self.icon);
            }
        }
    }
}

/// A GDI font `pixel_height` device pixels tall. The title strip and the sidebar share it.
pub(crate) fn create_ui_font(pixel_height: i32, face: &str, weight: i32, italic: bool) -> HFONT {
    let face = crate::platform::wide_null(face);
    unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            weight,
            u32::from(italic),
            0,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            u32::from(DEFAULT_PITCH),
            face.as_ptr(),
        )
    }
}

fn create_font(pixel_height: i32, face: &str) -> HFONT {
    create_ui_font(pixel_height, face, FW_NORMAL as i32, false)
}

/// The title strip's height at `dpi`: 40 px at 96 DPI, never less than a caption button.
pub(crate) fn strip_height(dpi: u32) -> i32 {
    let dpi = dpi.max(1);
    scale(40, dpi).max(unsafe { GetSystemMetricsForDpi(SM_CYSIZE, dpi) })
}

pub(crate) struct TitlePaint<'a> {
    /// The window title, shown in the empty caption: the active tab's title and the app name.
    pub title: &'a str,
    /// Present once deferred chrome is built; the bar is painted along the bottom edge.
    pub status: Option<&'a crate::window::status::StatusBarText>,
    pub palette: Palette,
    pub fonts: TitleFontHandles,
    pub pointer: PointerState,
    /// The Alt/F10 menu band's state and heading rectangles while menu mode is active.
    pub menu: Option<(crate::window::menu_band::MenuMode, &'a [RECT])>,
}

pub(crate) unsafe fn paint(hwnd: HWND, input: &TitlePaint<'_>) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if dc.is_null() {
        return;
    }

    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let layout = layout_for_window(hwnd);
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    // The activity bar and the panel paint themselves; the frame never paints under them.
    let left = layout.sidebar.right;
    if left > 0 {
        unsafe {
            ExcludeClipRect(dc, 0, 0, left, client.bottom);
        }
    }
    let maximized = unsafe { IsZoomed(hwnd) } != 0;
    if paint.rcPaint.top < layout.height {
        unsafe { paint_strip_buffered(dc, &layout, dpi, maximized, input) };
    }

    let status_height = if input.status.is_some() {
        crate::window::status::status_height(dpi)
    } else {
        0
    };
    if let Some(status) = input.status {
        let bar = Rect::new(
            left,
            client.bottom - status_height,
            client.right,
            client.bottom,
        );
        let margin = scale(10, dpi);
        let gap = scale(24, dpi);
        let text = Rect::new(
            bar.left + margin,
            bar.top,
            (bar.right - margin).max(bar.left + margin),
            bar.bottom,
        );
        let format = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        unsafe {
            fill(dc, bar, input.palette.strip_background);
            SetBkMode(dc, TRANSPARENT as i32);
            let previous = select_font(dc, input.fonts.text);
            // The document details keep their full width; a long notice is what gets ellipsized.
            let right_width = if status.right.is_empty() {
                0
            } else {
                SetTextColor(dc, input.palette.muted_foreground);
                draw_text(dc, &status.right, text, format | DT_RIGHT);
                measure_text(dc, &status.right, format) + gap
            };
            SetTextColor(dc, input.palette.strip_foreground);
            draw_text(
                dc,
                &status.left,
                Rect::new(
                    text.left,
                    text.top,
                    (text.right - right_width).max(text.left),
                    text.bottom,
                ),
                format | DT_END_ELLIPSIS,
            );
            restore_font(dc, previous);
        }
    }

    if let Some((mode, headings)) = input.menu {
        unsafe {
            crate::window::menu_band::paint(
                dc,
                left,
                client.right,
                headings,
                mode,
                input.palette,
                input.fonts.text,
            );
        }
    }

    unsafe {
        EndPaint(hwnd, &paint);
    }
}

/// Fills an editor group's `content` and centers `hint` in it, shown in place of the hidden editor
/// while no tab is open.
pub(crate) unsafe fn paint_empty_hint(
    dc: HDC,
    content: RECT,
    hint: &str,
    palette: Palette,
    font: HFONT,
    dpi: u32,
) {
    let content = from_native(content);
    let margin = scale(24, dpi);
    let middle = (content.top + content.bottom) / 2;
    unsafe {
        fill(dc, content, palette.editor_background);
        SetBkMode(dc, TRANSPARENT as i32);
        let previous = select_font(dc, font);
        SetTextColor(dc, palette.muted_foreground);
        draw_text(
            dc,
            hint,
            Rect::new(
                content.left + margin,
                middle - scale(20, dpi),
                (content.right - margin).max(content.left + margin),
                middle + scale(20, dpi),
            ),
            DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
        );
        restore_font(dc, previous);
    }
}

/// Fills a Markdown preview divider.
pub(crate) unsafe fn paint_divider(dc: HDC, divider: RECT, palette: Palette) {
    unsafe { fill(dc, from_native(divider), palette.hover_background) };
}

unsafe fn paint_strip_buffered(
    dc: HDC,
    layout: &TitleBarLayout,
    dpi: u32,
    maximized: bool,
    input: &TitlePaint<'_>,
) {
    let strip = layout.strip();
    let (width, height) = (strip.right, strip.bottom);
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
        unsafe { draw_strip(dc, layout, dpi, maximized, input) };
    } else {
        unsafe {
            let previous = SelectObject(memory, bitmap);
            draw_strip(memory, layout, dpi, maximized, input);
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

unsafe fn draw_strip(
    dc: HDC,
    layout: &TitleBarLayout,
    dpi: u32,
    maximized: bool,
    input: &TitlePaint<'_>,
) {
    let palette = input.palette;
    let pointer = input.pointer;
    let centered = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
    unsafe {
        fill(dc, layout.strip(), palette.strip_background);
        SetBkMode(dc, TRANSPARENT as i32);
    }
    let previous_font = unsafe { select_font(dc, input.fonts.text) };

    let margin = scale(12, dpi);
    let caption = layout.drag_region;
    unsafe {
        SetTextColor(dc, palette.muted_foreground);
        draw_text(
            dc,
            input.title,
            Rect::new(
                caption.left + margin,
                caption.top,
                (caption.right - margin).max(caption.left + margin),
                caption.bottom,
            ),
            DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }

    let overflow_hovered = pointer.hovered == Some(HitTarget::Overflow);
    unsafe {
        select_font(dc, input.fonts.glyph);
        if overflow_hovered {
            fill(
                dc,
                layout.overflow.centered_square(scale(32, dpi)),
                if pointer.is_pressed(HitTarget::Overflow) {
                    palette.pressed_background
                } else {
                    palette.hover_background
                },
            );
        }
        SetTextColor(
            dc,
            if overflow_hovered {
                palette.hover_foreground
            } else {
                palette.muted_foreground
            },
        );
        draw_text(dc, GLYPH_MORE, layout.overflow, centered);
    }

    let maximize_glyph = if maximized {
        GLYPH_RESTORE
    } else {
        GLYPH_MAXIMIZE
    };
    for (target, rect, glyph) in [
        (HitTarget::Minimize, layout.minimize, GLYPH_MINIMIZE),
        (HitTarget::Maximize, layout.maximize, maximize_glyph),
        (HitTarget::Close, layout.close, GLYPH_CLOSE),
    ] {
        let hovered = pointer.hovered == Some(target);
        let pressed = hovered && pointer.is_pressed(target);
        let (background, foreground) = match (target, hovered, pressed) {
            (HitTarget::Close, true, true) => (
                Some(palette.close_pressed_background),
                palette.close_hover_foreground,
            ),
            (HitTarget::Close, true, false) => (
                Some(palette.close_hover_background),
                palette.close_hover_foreground,
            ),
            (_, true, true) => (Some(palette.pressed_background), palette.hover_foreground),
            (_, true, false) => (Some(palette.hover_background), palette.hover_foreground),
            _ => (None, palette.strip_foreground),
        };
        unsafe {
            if let Some(background) = background {
                fill(dc, rect, background);
            }
            SetTextColor(dc, foreground);
            draw_text(dc, glyph, rect, centered);
        }
    }

    unsafe { restore_font(dc, previous_font) };
}

pub(crate) unsafe fn nonclient_hit_test(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let mut dwm_result = 0;
    if unsafe { DwmDefWindowProc(hwnd, WM_NCHITTEST, wparam, lparam, &mut dwm_result) } != 0 {
        return dwm_result;
    }

    let mut point = POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        ScreenToClient(hwnd, &mut point);
    }
    let layout = layout_for_window(hwnd);
    if point.x < 0 || point.x >= layout.close.right || point.y < 0 || point.y >= layout.height {
        return unsafe { DefWindowProcW(hwnd, WM_NCHITTEST, wparam, lparam) };
    }
    let maximized = unsafe { IsZoomed(hwnd) } != 0;
    (match layout.frame_hit_test(Point::new(point.x, point.y), maximized) {
        HitTarget::Caption => HTCAPTION,
        HitTarget::Minimize => HTMINBUTTON,
        HitTarget::Maximize => HTMAXBUTTON,
        HitTarget::Close => HTCLOSE,
        HitTarget::ResizeTop => HTTOP,
        HitTarget::ResizeTopLeft => HTTOPLEFT,
        HitTarget::ResizeTopRight => HTTOPRIGHT,
        HitTarget::Client | HitTarget::Overflow => HTCLIENT,
    }) as LRESULT
}

pub(crate) unsafe fn reclaim_caption(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if lparam == 0 {
        return unsafe { DefWindowProcW(hwnd, WM_NCCALCSIZE, wparam, lparam) };
    }
    let client = if wparam == 0 {
        unsafe { &mut *(lparam as *mut RECT) }
    } else {
        unsafe { &mut (*(lparam as *mut NCCALCSIZE_PARAMS)).rgrc[0] }
    };
    let proposed = from_native(*client);
    let work_area = if unsafe { IsZoomed(hwnd) } != 0 {
        work_area_for(proposed)
    } else {
        None
    };
    let default_client = if work_area.is_some() {
        proposed
    } else {
        unsafe {
            DefWindowProcW(hwnd, WM_NCCALCSIZE, wparam, lparam);
        }
        from_native(*client)
    };
    *client = native_rect(frame_client_rect(proposed, default_client, work_area));
    0
}

fn work_area_for(rect: Rect) -> Option<Rect> {
    let monitor = unsafe { MonitorFromRect(&native_rect(rect), MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then(|| from_native(info.rcWork))
}

pub(crate) unsafe fn constrain_maximized_window(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return 0;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return 0;
    }
    let minmax = unsafe { &mut *(lparam as *mut MINMAXINFO) };
    minmax.ptMaxPosition.x = info.rcWork.left - info.rcMonitor.left;
    minmax.ptMaxPosition.y = info.rcWork.top - info.rcMonitor.top;
    minmax.ptMaxSize.x = info.rcWork.right - info.rcWork.left;
    minmax.ptMaxSize.y = info.rcWork.bottom - info.rcWork.top;
    0
}

/// Requests leave notification for the client area or, for caption buttons, the non-client area.
pub(crate) fn track_pointer_leave(hwnd: HWND, nonclient: bool) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | if nonclient { TME_NONCLIENT } else { 0 },
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        TrackMouseEvent(&mut event);
    }
}

/// Best-effort dark frame and editor scrollbars; failures silently keep the light appearance.
pub(crate) fn apply_frame_theme(hwnd: HWND, editor: HWND, dark: bool) {
    let enabled = windows_sys::core::BOOL::from(dark);
    let theme = crate::platform::wide_null("DarkMode_Explorer");
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&raw const enabled).cast(),
            std::mem::size_of::<windows_sys::core::BOOL>() as u32,
        );
        let _ = SetWindowTheme(
            editor,
            if dark {
                theme.as_ptr()
            } else {
                std::ptr::null()
            },
            std::ptr::null(),
        );
    }
}

pub(crate) unsafe fn fill(dc: HDC, rect: Rect, color: u32) {
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, &native_rect(rect), GetStockObject(DC_BRUSH));
    }
}

pub(crate) unsafe fn select_font(dc: HDC, font: HFONT) -> HFONT {
    if font.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { SelectObject(dc, font) }
}

pub(crate) unsafe fn restore_font(dc: HDC, previous: HFONT) {
    if !previous.is_null() {
        unsafe {
            SelectObject(dc, previous);
        }
    }
}

fn native_rect(rect: Rect) -> RECT {
    RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn from_native(rect: RECT) -> Rect {
    Rect::new(rect.left, rect.top, rect.right, rect.bottom)
}

unsafe fn measure_text(dc: HDC, text: &str, format: u32) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let mut rect = RECT::default();
    unsafe {
        DrawTextW(
            dc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut rect,
            format | DT_CALCRECT,
        );
    }
    rect.right - rect.left
}

pub(crate) unsafe fn draw_text(dc: HDC, text: &str, rect: Rect, format: u32) {
    // An empty Vec's pointer is dangling, and DT_END_ELLIPSIS makes DrawTextW read through it; the
    // bottom bar's left side is empty whenever no tab is open and no notice is pending.
    if text.is_empty() {
        return;
    }
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let mut rect = native_rect(rect);
    unsafe {
        DrawTextW(dc, wide.as_ptr(), wide.len() as i32, &mut rect, format);
    }
}

#[cfg(test)]
mod tests {
    use super::{HitTarget, LogoIcon, PointerState, Rect, Size, TitleBarLayout, frame_client_rect};
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTTOP};

    #[test]
    fn a_logo_icon_destroys_its_handle_on_drop() {
        // Break caught: a DPI change (which replaces `App.logo_icon`) leaking one GDI icon handle
        // every time, because `Drop` never calls `DestroyIcon`.
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIcon, GetIconInfo, ICONINFO};
        let and_mask = [0xffu8];
        let xor_mask = [0x00u8];
        let icon = unsafe {
            CreateIcon(
                std::ptr::null_mut(),
                1,
                1,
                1,
                1,
                and_mask.as_ptr(),
                xor_mask.as_ptr(),
            )
        };
        assert!(
            !icon.is_null(),
            "test setup: could not create a throwaway icon"
        );
        {
            let logo = LogoIcon::new(96, icon);
            assert_eq!(logo.dpi(), 96);
            let mut info = ICONINFO::default();
            assert_ne!(
                unsafe { GetIconInfo(logo.icon(), &mut info) },
                0,
                "the icon is alive before drop"
            );
            unsafe {
                windows_sys::Win32::Graphics::Gdi::DeleteObject(info.hbmMask);
                if !info.hbmColor.is_null() {
                    windows_sys::Win32::Graphics::Gdi::DeleteObject(info.hbmColor);
                }
            }
        }
        let mut info = ICONINFO::default();
        assert_eq!(
            unsafe { GetIconInfo(icon, &mut info) },
            0,
            "drop destroyed the icon"
        );
    }

    #[test]
    fn a_sidebar_offset_keeps_its_strip_as_caption() {
        // Break caught: a sidebar top strip that no longer drags the window or resizes it from
        // the top edge.
        let client = Size::new(1200, 800);
        let plain = TitleBarLayout::calculate(client, 96);
        assert_eq!(TitleBarLayout::calculate_with_offset(client, 96, 0), plain);
        let layout = TitleBarLayout::calculate_with_offset(client, 96, 304);
        assert_eq!(layout.sidebar, Rect::new(0, 0, 304, layout.height));
        assert_eq!(layout.drag_region.left, 304);
        assert_eq!(layout.drag_region.right, layout.overflow.left);
        assert_eq!(layout.close, plain.close);
        assert_eq!(
            layout.hit_test(super::Point::new(20, layout.height / 2)),
            HitTarget::Caption
        );
        assert_eq!(
            layout.frame_hit_test(super::Point::new(20, 0), false),
            HitTarget::ResizeTop
        );
        assert_eq!(
            layout.frame_hit_test(super::Point::new(0, 0), false),
            HitTarget::ResizeTopLeft
        );
        // An offset wider than the strip leaves an empty caption, never a negative one.
        let squeezed = TitleBarLayout::calculate_with_offset(Size::new(300, 800), 96, 5000);
        assert!(squeezed.drag_region.left <= squeezed.drag_region.right);
        assert_eq!(squeezed.close.right, 300);
    }

    #[test]
    fn caption_buttons_sit_flush_with_the_top_right_edge() {
        // Break caught: a layout that starts below y=0 or leaves room right of Close recreates the
        // native caption band above/beside FastPad's own buttons.
        let layout = TitleBarLayout::calculate(Size::new(1200, 800), 96);
        for rect in [layout.minimize, layout.maximize, layout.close] {
            assert_eq!(rect.top, 0);
            assert_eq!(rect.bottom, layout.height);
        }
        assert_eq!(layout.close.right, 1200);
        assert_eq!(layout.maximize.right, layout.close.left);
        assert_eq!(layout.minimize.right, layout.maximize.left);
        assert_eq!(layout.close.right - layout.close.left, 46);
        assert_eq!(
            TitleBarLayout::calculate(Size::new(1200, 800), 192)
                .close
                .left,
            1200 - 92
        );
    }

    #[test]
    fn restored_frame_keeps_the_proposed_top_edge_and_other_borders() {
        let proposed = Rect::new(100, 50, 1380, 770);
        let default_client = Rect::new(108, 81, 1372, 762);
        assert_eq!(
            frame_client_rect(proposed, default_client, None),
            Rect::new(108, 50, 1372, 762)
        );
    }

    #[test]
    fn maximized_frame_is_clamped_to_the_work_area() {
        // Break caught: a maximized window whose frame extends past the monitor would push the
        // caption buttons and the editor's edges off-screen.
        let work = Rect::new(0, 0, 1920, 1040);
        assert_eq!(
            frame_client_rect(
                Rect::new(-8, -8, 1928, 1048),
                Rect::new(0, 23, 1920, 1040),
                Some(work)
            ),
            work
        );
        assert_eq!(
            frame_client_rect(work, Rect::new(8, 31, 1912, 1032), Some(work)),
            work
        );
    }

    #[test]
    fn top_band_of_a_restored_window_resizes_except_over_caption_buttons() {
        let layout = TitleBarLayout::calculate(Size::new(1200, 800), 96);
        let band = layout.resize_border;
        assert!(band > 0 && band < layout.height);
        let caption = layout.drag_region.center();
        let top = super::Point::new(caption.x, 0);
        assert_eq!(layout.frame_hit_test(top, false), HitTarget::ResizeTop);
        assert_eq!(
            layout.frame_hit_test(super::Point::new(0, 0), false),
            HitTarget::ResizeTopLeft
        );
        assert_eq!(
            layout.frame_hit_test(super::Point::new(layout.close.right - 1, 0), false),
            HitTarget::Close,
            "the top-right corner of Close must close, not resize"
        );
        let maximize_top = super::Point::new(layout.maximize.center().x, 0);
        assert_eq!(
            layout.frame_hit_test(maximize_top, false),
            HitTarget::Maximize
        );
        assert_eq!(layout.frame_hit_test(top, true), HitTarget::Caption);
        assert_eq!(
            layout.frame_hit_test(super::Point::new(caption.x, band), false),
            HitTarget::Caption
        );
        assert_eq!(
            layout.frame_hit_test(layout.drag_region.center(), false),
            HitTarget::Caption
        );
    }

    #[test]
    fn nonclient_codes_map_only_to_caption_buttons() {
        assert_eq!(
            HitTarget::from_nonclient_code(HTMINBUTTON as usize),
            Some(HitTarget::Minimize)
        );
        assert_eq!(
            HitTarget::from_nonclient_code(HTMAXBUTTON as usize),
            Some(HitTarget::Maximize)
        );
        assert_eq!(
            HitTarget::from_nonclient_code(HTCLOSE as usize),
            Some(HitTarget::Close)
        );
        assert_eq!(HitTarget::from_nonclient_code(HTTOP as usize), None);
    }

    #[test]
    fn pointer_state_tracks_hover_press_and_release_over_the_same_target() {
        let idle = PointerState::default();
        let hovered = idle.hover(Some(HitTarget::Close));
        assert_eq!(hovered.hovered, Some(HitTarget::Close));
        assert_eq!(
            idle.hover(Some(HitTarget::Caption)),
            idle,
            "drag region is not a hover target"
        );

        let pressed = hovered.press(Some(HitTarget::Close));
        assert!(pressed.is_pressed(HitTarget::Close));
        let (released, activated) = pressed.release(Some(HitTarget::Close));
        assert_eq!(activated, Some(HitTarget::Close));
        assert_eq!(released.pressed, None);

        let (released, activated) = pressed
            .hover(Some(HitTarget::Maximize))
            .release(Some(HitTarget::Maximize));
        assert_eq!(
            activated, None,
            "release over a different button must not act"
        );
        assert_eq!(released.pressed, None);
    }

    #[test]
    fn leaving_clears_only_the_matching_area_hover() {
        // Break caught: a client-area leave that arrives after the pointer moved onto a caption
        // button (non-client) would otherwise drop the caption button's hover highlight.
        let caption = PointerState::default()
            .hover(Some(HitTarget::Minimize))
            .press(Some(HitTarget::Minimize));
        assert_eq!(caption.leave(false), caption);
        assert_eq!(caption.leave(true), PointerState::default());

        let menu = PointerState::default().hover(Some(HitTarget::Overflow));
        assert_eq!(menu.leave(true), menu);
        assert_eq!(menu.leave(false), PointerState::default());
    }

    #[test]
    fn hit_test_preserves_drag_and_maximize_regions() {
        let layout = TitleBarLayout::calculate(Size::new(1200, 800), 144);
        assert_eq!(
            layout.hit_test(layout.maximize.center()),
            HitTarget::Maximize
        );
        assert_eq!(
            layout.hit_test(layout.drag_region.center()),
            HitTarget::Caption
        );
    }

    #[test]
    fn the_whole_strip_left_of_the_app_menu_is_caption() {
        // Break caught: part of the old tab area left as client after the tabs moved into the
        // editor group, so the window can't be dragged or double-click-maximized there.
        let layout = TitleBarLayout::calculate(Size::new(1200, 800), 96);
        assert_eq!(layout.drag_region.left, 0);
        assert_eq!(layout.drag_region.right, layout.overflow.left);
        for x in [1, 300, layout.overflow.left - 1] {
            assert_eq!(
                layout.hit_test(super::Point::new(x, layout.height / 2)),
                HitTarget::Caption,
                "x = {x}"
            );
        }
    }

    #[test]
    fn narrow_window_never_overlaps_caption_buttons() {
        let layout = TitleBarLayout::calculate(Size::new(320, 600), 96);
        assert!(layout.drag_region.right() <= layout.minimize.left());
    }

    #[test]
    fn interactive_title_targets_are_disjoint() {
        let layout = TitleBarLayout::calculate(Size::new(1200, 800), 192);
        assert_eq!(
            layout.hit_test(layout.overflow.center()),
            HitTarget::Overflow
        );
        assert_eq!(
            layout.hit_test(layout.minimize.center()),
            HitTarget::Minimize
        );
        assert_eq!(layout.hit_test(layout.close.center()), HitTarget::Close);
    }
}
