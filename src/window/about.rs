//! Help → About FastPad: the version, copyright, and links to the repository and the third-party
//! licenses. `MessageBoxW` would stay light-themed in a dark window, so this is an owned popup
//! painted in the theme's colors that runs its own modal loop, as `MessageBoxW` does, with the
//! main window disabled underneath.

use super::modal::ModalScope;
use super::palette::Palette;
use super::panel::{fill, inset, scale, text_height};
use super::titlebar::create_ui_font;
use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, EndPaint, FW_NORMAL, FW_SEMIBOLD, GetDC,
    HDC, HFONT, InvalidateRect, PAINTSTRUCT, ReleaseDC, ScreenToClient, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW,
    DrawIconEx, GW_OWNER, GWLP_USERDATA, GetClientRect, GetCursorPos, GetMessageW, GetWindow,
    GetWindowLongPtrW, GetWindowRect, HCURSOR, HICON, HTCAPTION, HTCLIENT, IDC_ARROW, IDC_HAND,
    IMAGE_ICON, IsWindow, LoadCursorW, LoadImageW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CLIPCHILDREN,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

/// The main window's icon resource (see `main_window::APP_ICON_RESOURCE_ID`).
const APP_ICON_RESOURCE_ID: usize = 1;

const TITLE: &str = "FastPad";
const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const COPYRIGHT: &str = "\u{a9} 2026 Cocioaba Cornel \u{b7} MIT License";
const OK_LABEL: &str = "OK";

/// One of the box's two links.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Link {
    Repository,
    Licenses,
}

impl Link {
    pub(crate) const ALL: [Self; 2] = [Self::Repository, Self::Licenses];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Repository => "Source code on GitHub",
            Self::Licenses => "Third-party licenses",
        }
    }

    pub(crate) const fn url(self) -> &'static str {
        match self {
            Self::Repository => env!("CARGO_PKG_REPOSITORY"),
            Self::Licenses => concat!(env!("CARGO_PKG_REPOSITORY"), "/blob/main/LICENSES.md"),
        }
    }
}

/// What the pointer or the keyboard can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    Link(Link),
    Ok,
}

/// Tab order: the links top to bottom, then OK.
const TAB_ORDER: [Target; 3] = [
    Target::Link(Link::Repository),
    Target::Link(Link::Licenses),
    Target::Ok,
];

/// The target Tab (or Shift+Tab when `forward` is false) moves to from `current`, wrapping.
pub(crate) fn next_target(current: Target, forward: bool) -> Target {
    let count = TAB_ORDER.len();
    let index = TAB_ORDER
        .iter()
        .position(|target| *target == current)
        .unwrap_or(count - 1);
    let next = if forward {
        (index + 1) % count
    } else {
        (index + count - 1) % count
    };
    TAB_ORDER[next]
}

pub(crate) fn version_text() -> String {
    format!("Version {}", env!("CARGO_PKG_VERSION"))
}

const WIDTH_AT_96_DPI: i32 = 380;
const PADDING_AT_96_DPI: i32 = 20;
const ICON_AT_96_DPI: i32 = 48;
const ICON_GAP_AT_96_DPI: i32 = 16;
const SECTION_GAP_AT_96_DPI: i32 = 16;
const LINE_GAP_AT_96_DPI: i32 = 4;
const BUTTON_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;

/// Where everything sits in the box's client area.
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub icon: RECT,
    pub title: RECT,
    pub version: RECT,
    pub description: RECT,
    pub copyright: RECT,
    /// In `Link::ALL` order, each as wide as its label.
    pub links: [RECT; 2],
    pub ok: RECT,
}

impl Layout {
    /// The layout at `dpi` for lines `title_height` and `body_height` tall and links whose labels
    /// measure `link_widths`.
    pub(crate) fn calculate(
        dpi: u32,
        title_height: i32,
        body_height: i32,
        link_widths: [i32; 2],
    ) -> Self {
        let width = scale(WIDTH_AT_96_DPI, dpi);
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let icon_size = scale(ICON_AT_96_DPI, dpi);
        let line_gap = scale(LINE_GAP_AT_96_DPI, dpi);
        let section_gap = scale(SECTION_GAP_AT_96_DPI, dpi);
        let right = width - padding;
        let line = |left: i32, top: i32, height: i32| RECT {
            left,
            top,
            right,
            bottom: top + height,
        };

        let icon = RECT {
            left: padding,
            top: padding,
            right: padding + icon_size,
            bottom: padding + icon_size,
        };
        let text_left = icon.right + scale(ICON_GAP_AT_96_DPI, dpi);
        // The title and version sit as a block centered on the icon.
        let heading_height = title_height + line_gap + body_height;
        let title = line(
            text_left,
            padding + (icon_size - heading_height).max(0) / 2,
            title_height,
        );
        let version = line(text_left, title.bottom + line_gap, body_height);

        let description = line(
            padding,
            icon.bottom.max(version.bottom) + section_gap,
            body_height,
        );
        let copyright = line(padding, description.bottom + line_gap, body_height);

        let mut top = copyright.bottom + section_gap;
        let links = link_widths.map(|link_width| {
            let rect = RECT {
                left: padding,
                top,
                right: (padding + link_width).min(right),
                bottom: top + body_height,
            };
            top = rect.bottom + line_gap;
            rect
        });

        let button_top = links[1].bottom + section_gap;
        let ok = RECT {
            left: right - scale(BUTTON_WIDTH_AT_96_DPI, dpi),
            top: button_top,
            right,
            bottom: button_top + scale(BUTTON_HEIGHT_AT_96_DPI, dpi),
        };
        Self {
            width,
            height: ok.bottom + padding,
            icon,
            title,
            version,
            description,
            copyright,
            links,
            ok,
        }
    }

    /// The link or button under client point `x`, `y`.
    pub(crate) fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.ok) {
            return Some(Target::Ok);
        }
        Link::ALL
            .into_iter()
            .zip(&self.links)
            .find(|(_, rect)| inside(rect))
            .map(|(link, _)| Target::Link(link))
    }

    fn rect_of(&self, target: Target) -> RECT {
        match target {
            Target::Link(Link::Repository) => self.links[0],
            Target::Link(Link::Licenses) => self.links[1],
            Target::Ok => self.ok,
        }
    }
}

/// The open box's state, owned by its window through `GWLP_USERDATA`.
struct About {
    colors: Palette,
    link_color: u32,
    layout: Layout,
    title_font: HFONT,
    body_font: HFONT,
    link_font: HFONT,
    /// Null when the module has no icon resource, as in test binaries.
    icon: HICON,
    focus: Target,
    hot: Option<Target>,
    pressed: Option<Target>,
    tracking_leave: bool,
}

impl Drop for About {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.title_font as _);
            DeleteObject(self.body_font as _);
            DeleteObject(self.link_font as _);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}

/// Shows the About box over `owner` and returns once it is closed. `link_color` is the theme's
/// link color, as the Markdown preview draws links.
pub(crate) fn show(owner: HWND, colors: Palette, link_color: u32) {
    let _modal = ModalScope::enter(owner);
    let Some(dialog) = create(owner, colors, link_color) else {
        return;
    };
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(dialog, SW_SHOW);
        SetFocus(dialog);
    }
    #[cfg(test)]
    {
        let answer = ANSWERS
            .with(|answers| answers.borrow_mut().pop_front())
            .expect("an About box opened in a test without answer_next");
        answer(dialog);
    }
    let mut message = MSG::default();
    while unsafe { IsWindow(dialog) } != 0 {
        match unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } {
            0 => {
                // WM_QUIT belongs to the outer loop: put it back for that loop to see.
                unsafe { PostQuitMessage(message.wParam as i32) };
                break;
            }
            -1 => break,
            _ => unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
    close(dialog);
    unsafe { EnableWindow(owner, 1) };
}

fn create(owner: HWND, colors: Palette, link_color: u32) -> Option<HWND> {
    let class = register_class()?;
    let title = wide_null("About FastPad");
    let dialog = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            0,
            0,
            owner,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if dialog.is_null() {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    let title_font = create_ui_font(scale(20, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
    let body_font = create_ui_font(scale(13, dpi), "Segoe UI", FW_NORMAL as i32, false);
    let link_font = create_underlined_font(scale(13, dpi));
    let link_widths = Link::ALL.map(|link| measure(dialog, link_font, link.label()));
    let layout = Layout::calculate(
        dpi,
        text_height(dialog, title_font),
        text_height(dialog, body_font),
        link_widths,
    );
    let icon_size = layout.icon.right - layout.icon.left;
    let icon = unsafe {
        LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            APP_ICON_RESOURCE_ID as *const u16,
            IMAGE_ICON,
            icon_size,
            icon_size,
            0,
        )
    } as HICON;
    let state = Box::new(About {
        colors,
        link_color,
        layout,
        title_font,
        body_font,
        link_font,
        icon,
        focus: Target::Ok,
        hot: None,
        pressed: None,
        tracking_leave: false,
    });
    unsafe { SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize) };

    // Centered over the owner, with rounded corners where Windows 11 draws them.
    let mut frame = RECT::default();
    unsafe { GetWindowRect(owner, &mut frame) };
    let left = frame.left + (frame.right - frame.left - layout.width) / 2;
    let top = frame.top + (frame.bottom - frame.top - layout.height) / 2;
    unsafe {
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            left,
            top,
            layout.width,
            layout.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let corners = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            dialog,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const corners).cast(),
            std::mem::size_of_val(&corners) as u32,
        );
    }
    Some(dialog)
}

/// Re-enables the owner before the box goes, so Windows hands activation back to it rather than
/// to some other application.
fn close(dialog: HWND) {
    if unsafe { IsWindow(dialog) } == 0 {
        return;
    }
    unsafe {
        EnableWindow(owner(dialog), 1);
        DestroyWindow(dialog);
    }
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadAbout"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(about_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

fn create_underlined_font(pixel_height: i32) -> HFONT {
    use windows_sys::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        OUT_DEFAULT_PRECIS,
    };
    let face = wide_null("Segoe UI");
    unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            1,
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

/// The pixel width of `text` in `font`.
fn measure(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    unsafe {
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return 0;
        }
        let previous = SelectObject(dc, font as _);
        let mut text = wide_null(text);
        let mut rect = RECT::default();
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        ReleaseDC(hwnd, dc);
        rect.right - rect.left
    }
}

/// The main window; read from the window rather than `About`, which the caller may be borrowing.
fn owner(dialog: HWND) -> HWND {
    unsafe { GetWindow(dialog, GW_OWNER) }
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut About> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut About;
    // SAFETY: set once in `create` from `Box::into_raw` and cleared in WM_NCDESTROY; the window
    // procedure runs on this thread only and never holds two of these at once.
    unsafe { pointer.as_mut() }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

/// Follows a link or closes the box.
fn activate(hwnd: HWND, target: Target) {
    match target {
        Target::Ok => close(hwnd),
        Target::Link(link) => open_url(hwnd, link.url()),
    }
}

#[cfg(not(test))]
fn open_url(hwnd: HWND, url: &str) {
    if !super::preview_host::shell_open(hwnd, url) {
        super::main_window::push_notice(owner(hwnd), format!("FastPad could not open {url}."));
    }
}

#[cfg(test)]
fn open_url(_hwnd: HWND, url: &str) {
    OPENED.with(|opened| opened.borrow_mut().push(url.to_owned()));
}

fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam & 0xffff) as i16 as i32,
        ((lparam >> 16) & 0xffff) as i16 as i32,
    )
}

unsafe extern "system" fn about_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(about) = state(hwnd) else {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    match message {
        WM_PAINT => {
            paint(hwnd, about);
            0
        }
        WM_ERASEBKGND => 1,
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_KEYDOWN => {
            match wparam as u16 {
                VK_ESCAPE => close(hwnd),
                VK_RETURN | VK_SPACE => activate(hwnd, about.focus),
                VK_TAB => {
                    let back = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
                    about.focus = next_target(about.focus, !back);
                    invalidate(hwnd);
                }
                _ => {}
            }
            0
        }
        // Everything but the links and the button drags the box, like a caption.
        WM_NCHITTEST => {
            let mut point = POINT {
                x: lparam_point(lparam).0,
                y: lparam_point(lparam).1,
            };
            unsafe { ScreenToClient(hwnd, &mut point) };
            if about.layout.target_at(point.x, point.y).is_some() {
                HTCLIENT as LRESULT
            } else {
                HTCAPTION as LRESULT
            }
        }
        WM_SETCURSOR => {
            let mut point = POINT::default();
            unsafe {
                GetCursorPos(&mut point);
                ScreenToClient(hwnd, &mut point);
            }
            let cursor = match about.layout.target_at(point.x, point.y) {
                Some(Target::Link(_)) => IDC_HAND,
                _ => IDC_ARROW,
            };
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor) as HCURSOR) };
            1
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            let hot = about.layout.target_at(x, y);
            if !about.tracking_leave {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                about.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
            }
            if hot != about.hot {
                about.hot = hot;
                invalidate(hwnd);
            }
            0
        }
        windows_sys::Win32::UI::Controls::WM_MOUSELEAVE => {
            about.tracking_leave = false;
            if about.hot.take().is_some() {
                invalidate(hwnd);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            about.pressed = about.layout.target_at(x, y);
            if let Some(target) = about.pressed {
                about.focus = target;
                unsafe { SetCapture(hwnd) };
                invalidate(hwnd);
            }
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let pressed = about.pressed.take();
            unsafe { ReleaseCapture() };
            invalidate(hwnd);
            if let Some(target) = pressed
                && about.layout.target_at(x, y) == Some(target)
            {
                activate(hwnd, target);
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut About;
            // SAFETY: the pointer came from `Box::into_raw` in `create` and is released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, about: &About) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if dc.is_null() {
        return;
    }
    let colors = about.colors;
    let layout = &about.layout;
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
        fill(dc, client, colors.muted_foreground);
        fill(dc, inset(client, 1), colors.panel_background());
        SetBkMode(dc, TRANSPARENT as i32);
        if !about.icon.is_null() {
            DrawIconEx(
                dc,
                layout.icon.left,
                layout.icon.top,
                about.icon,
                layout.icon.right - layout.icon.left,
                layout.icon.bottom - layout.icon.top,
                0,
                std::ptr::null_mut(),
                windows_sys::Win32::UI::WindowsAndMessaging::DI_NORMAL,
            );
        }
        draw_text(
            dc,
            about.title_font,
            colors.editor_foreground,
            TITLE,
            layout.title,
            DT_LEFT,
        );
        draw_text(
            dc,
            about.body_font,
            colors.muted_foreground,
            &version_text(),
            layout.version,
            DT_LEFT,
        );
        for (text, rect) in [
            (DESCRIPTION, layout.description),
            (COPYRIGHT, layout.copyright),
        ] {
            draw_text(
                dc,
                about.body_font,
                colors.editor_foreground,
                text,
                rect,
                DT_LEFT,
            );
        }
        for (link, rect) in Link::ALL.into_iter().zip(layout.links) {
            draw_text(
                dc,
                about.link_font,
                about.link_color,
                link.label(),
                rect,
                DT_LEFT,
            );
        }

        let button = if about.pressed == Some(Target::Ok) {
            colors.pressed_background
        } else if about.hot == Some(Target::Ok) {
            colors.hover_background
        } else {
            colors.strip_background
        };
        fill(dc, layout.ok, colors.muted_foreground);
        fill(dc, inset(layout.ok, 1), button);
        draw_text(
            dc,
            about.body_font,
            colors.editor_foreground,
            OK_LABEL,
            layout.ok,
            DT_CENTER,
        );

        let focus = layout.rect_of(about.focus);
        let ring = match about.focus {
            Target::Ok => inset(focus, scale(3, GetDpiForWindow(hwnd).max(96))),
            Target::Link(_) => inset(focus, -2),
        };
        SetTextColor(dc, colors.editor_foreground);
        DrawFocusRect(dc, &ring);
        EndPaint(hwnd, &paint);
    }
}

unsafe fn draw_text(dc: HDC, font: HFONT, color: u32, text: &str, rect: RECT, align: u32) {
    let mut text = wide_null(text);
    let mut rect = rect;
    unsafe {
        let previous = SelectObject(dc, font as _);
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(dc, previous);
    }
}

#[cfg(test)]
type Answer = Box<dyn FnOnce(HWND)>;

#[cfg(test)]
thread_local! {
    static ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `answer` with the next About box's window once it is shown, before its modal loop starts;
/// the answer posts the input that closes it.
#[cfg(test)]
pub(crate) fn answer_next(answer: impl FnOnce(HWND) + 'static) {
    ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// The URLs the About box's links opened on this thread. Tests never start a browser.
#[cfg(test)]
pub(crate) fn take_opened_urls() -> Vec<String> {
    OPENED.with(|opened| std::mem::take(&mut *opened.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_and_links_come_from_the_package() {
        // Break caught: a hand-typed version or URL that drifts from Cargo.toml on the next bump.
        assert_eq!(
            version_text(),
            format!("Version {}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(Link::Repository.url(), "https://github.com/coccor/FastPad");
        assert_eq!(
            Link::Licenses.url(),
            "https://github.com/coccor/FastPad/blob/main/LICENSES.md"
        );
        assert!(!DESCRIPTION.is_empty());
    }

    #[test]
    fn tab_cycles_links_then_ok_and_shift_tab_goes_back() {
        let repository = Target::Link(Link::Repository);
        let licenses = Target::Link(Link::Licenses);
        assert_eq!(next_target(Target::Ok, true), repository);
        assert_eq!(next_target(repository, true), licenses);
        assert_eq!(next_target(licenses, true), Target::Ok);
        assert_eq!(next_target(repository, false), Target::Ok);
        assert_eq!(next_target(Target::Ok, false), licenses);
    }

    #[test]
    fn the_layout_stacks_its_lines_and_hit_tests_links_and_the_button() {
        // Break caught: overlapping lines, a link clickable across the whole row instead of on
        // its label, or a button outside the box.
        let layout = Layout::calculate(96, 27, 17, [120, 110]);
        let order = [
            layout.version,
            layout.description,
            layout.copyright,
            layout.links[0],
            layout.links[1],
            layout.ok,
        ];
        assert!(layout.title.bottom <= layout.version.top);
        for (index, pair) in order.windows(2).enumerate() {
            assert!(
                pair[0].bottom <= pair[1].top,
                "line {index} overlaps the next"
            );
        }
        assert!(layout.icon.bottom <= layout.description.top);
        assert_eq!(layout.links[0].right - layout.links[0].left, 120);
        assert!(layout.ok.right <= layout.width && layout.ok.bottom < layout.height);

        let middle = |rect: RECT| ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        let (x, y) = middle(layout.links[1]);
        assert_eq!(layout.target_at(x, y), Some(Target::Link(Link::Licenses)));
        let (x, y) = middle(layout.ok);
        assert_eq!(layout.target_at(x, y), Some(Target::Ok));
        assert_eq!(
            layout.target_at(layout.links[0].right + 5, layout.links[0].top),
            None
        );
        let (x, y) = middle(layout.title);
        assert_eq!(layout.target_at(x, y), None);
    }

    #[test]
    fn the_layout_scales_with_dpi() {
        let normal = Layout::calculate(96, 27, 17, [120, 110]);
        let double = Layout::calculate(192, 54, 34, [240, 220]);
        assert_eq!(double.width, normal.width * 2);
        assert_eq!(double.height, normal.height * 2);
    }
}
