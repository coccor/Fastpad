//! The activity bar: the sidebar's narrow painted strip of view buttons at the main window's left
//! edge, with Settings at the bottom. The active view has a 2 px accent bar and full-strength
//! color, and the others are muted. The strip above the first button is caption, so the bar
//! answers `HTTRANSPARENT` there.

use super::side_panel::{self, draw_text, paint_buffered, point_of, with_bar_state};
use crate::config::SidebarView;
use crate::window::panel::{fill, scale};
use crate::window::sidebar_accessibility::{self, AccessibleItem, AccessibleSource, button_item};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, InvalidateRect, ScreenToClient,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DI_NORMAL, DefWindowProcW, DrawIconEx, GetClientRect, GetParent, HTTRANSPARENT, OBJID_CLIENT,
    WM_CAPTURECHANGED, WM_ERASEBKGND, WM_GETOBJECT, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_SETFOCUS,
};

/// Segoe MDL2 Assets glyphs: Library, Search, FavoriteStar, Setting.
const GLYPH_NOTEBOOK: &str = "\u{E8F1}";
const GLYPH_SEARCH: &str = "\u{E721}";
const GLYPH_FAVORITES: &str = "\u{E734}";
const GLYPH_SETTINGS: &str = "\u{E713}";
const ACCENT_WIDTH_96: i32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActivityButton {
    Notebook,
    Search,
    Favorites,
    Settings,
}

impl ActivityButton {
    pub(crate) const ALL: [Self; 4] = [
        Self::Notebook,
        Self::Search,
        Self::Favorites,
        Self::Settings,
    ];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The view this button shows; Settings shows none.
    pub(crate) const fn view(self) -> Option<SidebarView> {
        match self {
            Self::Notebook => Some(SidebarView::Notebook),
            Self::Search => Some(SidebarView::Search),
            Self::Favorites => Some(SidebarView::Favorites),
            Self::Settings => None,
        }
    }

    /// The tooltip and accessible name (the Notebook tooltip adds the notebook's name).
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Notebook => "Notebook",
            Self::Search => "Search",
            Self::Favorites => "Favorites",
            Self::Settings => "Settings",
        }
    }

    const fn glyph(self) -> &'static str {
        match self {
            Self::Notebook => GLYPH_NOTEBOOK,
            Self::Search => GLYPH_SEARCH,
            Self::Favorites => GLYPH_FAVORITES,
            Self::Settings => GLYPH_SETTINGS,
        }
    }
}

/// Pointer state: the hovered button, the one a press went down on, and whether leave tracking
/// is armed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BarState {
    pub(crate) hover: Option<ActivityButton>,
    pub(crate) pressed: Option<ActivityButton>,
    pub(crate) tracking: bool,
}

/// Button rectangles in the bar's `client` coordinates at `dpi`, indexed by
/// `ActivityButton::index`. The view buttons are bar-wide squares stacked down from the bottom of
/// the title strip (`titlebar::strip_height`), whose share of the bar is caption. Settings sits at
/// the bottom and never covers them.
pub(crate) fn button_rects(client: RECT, dpi: u32) -> [RECT; 4] {
    let size = (client.right - client.left).max(0);
    let top = client.top + crate::window::titlebar::strip_height(dpi);
    let square = |top: i32| RECT {
        left: client.left,
        top,
        right: client.left + size,
        bottom: top + size,
    };
    let settings_top = (client.bottom - size).max(top + 3 * size);
    [
        square(top),
        square(top + size),
        square(top + 2 * size),
        square(settings_top),
    ]
}

/// The app logo's rect in the bar's `client` coordinates at `dpi`: a `scale(20, dpi)` px square,
/// centred in the top square (`x` 0 to the bar's width, `y` 0 to `titlebar::strip_height`), above
/// `button_rects(..)[0].top`. Clamped to fit when the bar is narrower than the logo.
pub(crate) fn logo_rect(client: RECT, dpi: u32) -> RECT {
    let width = (client.right - client.left).max(0);
    let height = crate::window::titlebar::strip_height(dpi).max(0);
    let size = scale(20, dpi).min(width).min(height).max(0);
    let left = client.left + (width - size) / 2;
    let top = client.top + (height - size) / 2;
    RECT {
        left,
        top,
        right: left + size,
        bottom: top + size,
    }
}

pub(crate) fn button_at(rects: &[RECT; 4], x: i32, y: i32) -> Option<ActivityButton> {
    ActivityButton::ALL.into_iter().find(|button| {
        let rect = rects[button.index()];
        x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
    })
}

/// The Notebook button's tooltip and accessible name: "Notebook: <name>" while a notebook is
/// open, `ActivityButton::Notebook.label()` otherwise.
pub(crate) fn notebook_label(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("Notebook: {name}"),
        None => ActivityButton::Notebook.label().to_owned(),
    }
}

fn rects_for(bar: HWND) -> [RECT; 4] {
    let mut client = RECT::default();
    unsafe { GetClientRect(bar, &mut client) };
    button_rects(client, unsafe { GetDpiForWindow(bar) }.max(96))
}

pub(crate) fn register_class() -> crate::Result<&'static [u16]> {
    static CLASS: std::sync::OnceLock<Option<Vec<u16>>> = std::sync::OnceLock::new();
    side_panel::register_child_class(&CLASS, "FastPadActivityBar", 0, Some(bar_proc))
}

/// The Settings button's click handler: the palette listing only the settings commands.
fn open_settings(main: HWND) {
    super::main_window::open_settings_palette(main);
}

/// A click on `button`: an inactive view opens, the active one closes the panel, and Settings
/// runs `open_settings`.
pub(crate) fn activate(main: HWND, button: ActivityButton) {
    match button.view() {
        Some(view) if side_panel::current_view(main) == view => {
            side_panel::show_view(main, SidebarView::Hidden, false)
        }
        // Clicking Search puts the keyboard in its search box (spec §8).
        Some(view) => side_panel::show_view(main, view, view == SidebarView::Search),
        None => open_settings(main),
    }
}

unsafe extern "system" fn bar_proc(
    bar: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = unsafe { GetParent(bar) };
    match message {
        WM_PAINT => {
            paint(main, bar);
            0
        }
        WM_ERASEBKGND => 1,
        WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => unsafe {
            sidebar_accessibility::object_result(bar, &BAR_ACCESSIBLE, wparam)
        },
        sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACCESSIBLE => unsafe {
            sidebar_accessibility::answer(bar, lparam)
        },
        sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION => {
            sidebar_accessibility::run_action(bar, &BAR_ACCESSIBLE, wparam, lparam);
            0
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            focus_changed(bar, message == WM_SETFOCUS);
            0
        }
        WM_KEYDOWN if key_down(bar, wparam as u16) => 0,
        // Above the first button the main window's caption hit test applies.
        WM_NCHITTEST => {
            let (x, y) = point_of(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(bar, &mut point) };
            if point.y < super::main_window::title_layout(main).height {
                HTTRANSPARENT as LRESULT
            } else {
                unsafe { DefWindowProcW(bar, message, wparam, lparam) }
            }
        }
        WM_MOUSEMOVE => {
            side_panel::bar_pointer_moved(main, bar, message, wparam, lparam);
            let (x, y) = point_of(lparam);
            hover(main, bar, button_at(&rects_for(bar), x, y));
            0
        }
        WM_MOUSELEAVE => {
            with_bar_state(main, |state| state.tracking = false);
            hover(main, bar, None);
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            let target = button_at(&rects_for(bar), x, y);
            with_bar_state(main, |state| state.pressed = target);
            if target.is_some() {
                unsafe { SetCapture(bar) };
            }
            unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = point_of(lparam);
            let target = button_at(&rects_for(bar), x, y);
            let pressed = with_bar_state(main, |state| state.pressed.take()).flatten();
            if pressed.is_some() {
                unsafe { ReleaseCapture() };
            }
            unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
            if let Some(button) = pressed
                && target == Some(button)
            {
                activate(main, button);
            }
            0
        }
        WM_CAPTURECHANGED => {
            with_bar_state(main, |state| state.pressed = None);
            unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
            0
        }
        _ => unsafe { DefWindowProcW(bar, message, wparam, lparam) },
    }
}

fn hover(main: HWND, bar: HWND, target: Option<ActivityButton>) {
    let (changed, track) = with_bar_state(main, |state| {
        let changed = state.hover != target;
        state.hover = target;
        let track = target.is_some() && !state.tracking;
        state.tracking |= track;
        (changed, track)
    })
    .unwrap_or((false, false));
    if track {
        let mut event = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: bar,
            dwHoverTime: 0,
        };
        unsafe { TrackMouseEvent(&mut event) };
    }
    if changed {
        unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
    }
}

fn paint(main: HWND, bar: HWND) {
    let palette = super::main_window::current_palette(main);
    let dpi = unsafe { GetDpiForWindow(bar) }.max(96);
    let glyph = super::main_window::ui_fonts(main).bar_glyph;
    // `None` until the deferred chrome step loads it (or momentarily during a DPI change); the
    // corner then paints as it does today, empty.
    let logo = super::main_window::logo_icon(main, dpi);
    let state = with_bar_state(main, |state| *state).unwrap_or_default();
    let view = side_panel::current_view(main);
    let accent = scale(ACCENT_WIDTH_96, dpi);
    paint_buffered(bar, |dc, client| unsafe {
        fill(dc, client, palette.strip_background);
        if let Some(icon) = logo {
            let rect = logo_rect(client, dpi);
            let size = rect.right - rect.left;
            if size > 0 {
                DrawIconEx(
                    dc,
                    rect.left,
                    rect.top,
                    icon,
                    size,
                    size,
                    0,
                    std::ptr::null_mut(),
                    DI_NORMAL,
                );
            }
        }
        let rects = button_rects(client, dpi);
        for button in ActivityButton::ALL {
            let rect = rects[button.index()];
            let active = button.view() == Some(view);
            let hovered = state.hover == Some(button);
            if hovered {
                let background = if state.pressed == Some(button) {
                    palette.pressed_background
                } else {
                    palette.hover_background
                };
                fill(dc, rect, background);
            }
            if active {
                fill(
                    dc,
                    RECT {
                        right: rect.left + accent,
                        ..rect
                    },
                    palette.editor_foreground,
                );
            }
            let color = if hovered {
                palette.hover_foreground
            } else if active {
                palette.editor_foreground
            } else {
                palette.muted_foreground
            };
            draw_text(
                dc,
                button.glyph(),
                rect,
                glyph,
                color,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        paint_keyboard_focus(bar, dc);
    });
}

/// The four buttons as MSAA children, in `button_rects` order.
pub(crate) fn bar_items(
    rects: [RECT; 4],
    view: SidebarView,
    notebook: Option<&str>,
    focused: Option<usize>,
) -> Vec<AccessibleItem> {
    ActivityButton::ALL
        .into_iter()
        .map(|button| {
            let name = if button == ActivityButton::Notebook {
                notebook_label(notebook)
            } else {
                button.label().to_owned()
            };
            button_item(
                &name,
                button.view() == Some(view),
                focused == Some(button.index()),
                rects[button.index()],
            )
        })
        .collect()
}

fn bar_geometry(bar: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(bar, &mut client);
    }
    (
        client,
        unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(bar) }.max(96),
    )
}

pub(crate) fn accessible_items(bar: HWND) -> Vec<AccessibleItem> {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    let (client, dpi) = bar_geometry(bar);
    let notebook =
        super::library_host::folder(main).map(|folder| super::library_host::notebook_name(&folder));
    let focused = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == bar;
    bar_items(
        button_rects(client, dpi),
        super::side_panel::current_view(main),
        notebook.as_deref(),
        focused.then(|| super::side_panel::bar_focus(main)),
    )
}

fn bar_container(_: HWND) -> (String, u32) {
    (
        "Activity bar".to_owned(),
        windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_TOOLBAR,
    )
}

fn bar_count(_: HWND) -> usize {
    4
}

fn bar_item(bar: HWND, index: usize) -> Option<AccessibleItem> {
    accessible_items(bar).into_iter().nth(index)
}

fn bar_hit(bar: HWND, point: POINT) -> Option<usize> {
    let (client, dpi) = bar_geometry(bar);
    button_rects(client, dpi).iter().position(|rect| {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    })
}

fn bar_current(bar: HWND) -> Option<usize> {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    Some(super::side_panel::bar_focus(main))
}

fn bar_select(bar: HWND, index: usize) {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    super::side_panel::set_bar_focus(main, index);
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(bar, std::ptr::null(), 0);
    }
}

/// Presses button `index` exactly as a click does.
fn bar_activate(bar: HWND, index: usize) {
    let (client, dpi) = bar_geometry(bar);
    if let Some(rect) = button_rects(client, dpi).get(index) {
        sidebar_accessibility::click_item(bar, *rect);
    }
}

pub(crate) static BAR_ACCESSIBLE: AccessibleSource = AccessibleSource {
    container: bar_container,
    count: bar_count,
    item: bar_item,
    hit: bar_hit,
    current: bar_current,
    select: bar_select,
    activate: bar_activate,
    // Four fixed buttons: the index is the identity, and the order never changes.
    identity: |_, _| None,
    generation: |_| 0,
};

/// `WM_SETFOCUS` and `WM_KILLFOCUS`. Gaining the focus starts on the active view's button.
pub(crate) fn focus_changed(bar: HWND, gained: bool) {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    if gained {
        let index =
            super::side_panel::view_button(super::side_panel::current_view(main)).unwrap_or(0);
        super::side_panel::set_bar_focus(main, index);
        sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_FOCUS,
            bar,
            Some(index),
        );
    }
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(bar, std::ptr::null(), 0);
    }
}

/// `WM_KEYDOWN` on the bar: Up and Down move, Home and End jump, Enter and Space press.
pub(crate) fn key_down(bar: HWND, key: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_DOWN, VK_END, VK_HOME, VK_RETURN, VK_SPACE, VK_UP,
    };
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    let focus = super::side_panel::bar_focus(main);
    let next = match key {
        VK_UP => focus.saturating_sub(1),
        VK_DOWN => (focus + 1).min(3),
        VK_HOME => 0,
        VK_END => 3,
        VK_RETURN | VK_SPACE => {
            bar_activate(bar, focus);
            return true;
        }
        _ => return false,
    };
    if next != focus {
        bar_select(bar, next);
        sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_FOCUS,
            bar,
            Some(next),
        );
    }
    true
}

/// Draws the keyboard focus rectangle. Call it last in the bar's `WM_PAINT`, before `EndPaint`.
pub(crate) fn paint_keyboard_focus(bar: HWND, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
    if unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } != bar {
        return;
    }
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    let (client, dpi) = bar_geometry(bar);
    let rect = crate::window::panel::inset(
        button_rects(client, dpi)[super::side_panel::bar_focus(main)],
        crate::window::panel::scale(3, dpi),
    );
    unsafe {
        windows_sys::Win32::Graphics::Gdi::DrawFocusRect(hdc, &rect);
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityButton, button_at, button_rects, logo_rect, notebook_label, scale};
    use crate::config::SidebarView;
    use windows_sys::Win32::Foundation::RECT;

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    fn bar(width: i32, height: i32) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        }
    }

    #[test]
    fn buttons_stack_below_the_caption_strip_with_settings_at_the_bottom() {
        // Break caught: a first button inside the caption strip (it could not be clicked, the
        // strip drags the window), or Settings drawn over Favorites in a short window.
        // At 96 DPI the title strip is 40 px tall.
        let rects = button_rects(bar(44, 700), 96);
        assert_eq!(edges(rects[0]), (0, 40, 44, 84));
        assert_eq!(edges(rects[1]), (0, 84, 44, 128));
        assert_eq!(edges(rects[2]), (0, 128, 44, 172));
        assert_eq!(edges(rects[3]), (0, 656, 44, 700));
        let short = button_rects(bar(44, 150), 96);
        assert_eq!(short[3].top, short[2].bottom);
        assert_eq!(button_at(&rects, 10, 50), Some(ActivityButton::Notebook));
        assert_eq!(button_at(&rects, 10, 130), Some(ActivityButton::Favorites));
        assert_eq!(button_at(&rects, 10, 690), Some(ActivityButton::Settings));
        assert_eq!(button_at(&rects, 10, 20), None);
        assert_eq!(button_at(&rects, 10, 300), None);
    }

    #[test]
    fn the_logo_rect_is_a_centred_scale_20_square_above_the_first_button() {
        // Break caught: a logo drawn off-centre, at the wrong size, or spilling into the button
        // row it sits above.
        for dpi in [96, 144, 192] {
            let client = bar(200, 700);
            let rect = logo_rect(client, dpi);
            let size = scale(20, dpi);
            assert_eq!(rect.right - rect.left, size, "dpi {dpi}: wrong width");
            assert_eq!(rect.bottom - rect.top, size, "dpi {dpi}: wrong height");
            let width = client.right - client.left;
            let height = crate::window::titlebar::strip_height(dpi);
            assert_eq!(rect.left, (width - size) / 2, "dpi {dpi}: not centred (x)");
            assert_eq!(rect.top, (height - size) / 2, "dpi {dpi}: not centred (y)");
            assert!(
                rect.bottom <= button_rects(client, dpi)[0].top,
                "dpi {dpi}: the logo overlaps the first button"
            );
        }
    }

    #[test]
    fn the_logo_rect_is_clamped_to_a_bar_narrower_than_the_logo() {
        // Break caught: an unclamped rect that reaches past the bar's edges when the bar is
        // narrower than the logo would need.
        let dpi = 96;
        let size = scale(20, dpi);
        let narrow = bar(size - 6, 700);
        let rect = logo_rect(narrow, dpi);
        assert_eq!(rect.left, 0);
        assert_eq!(rect.right, size - 6);
        assert_eq!(rect.right - rect.left, size - 6);
    }

    #[test]
    fn each_view_button_names_its_view_and_settings_names_none() {
        assert_eq!(ActivityButton::Notebook.view(), Some(SidebarView::Notebook));
        assert_eq!(ActivityButton::Search.view(), Some(SidebarView::Search));
        assert_eq!(
            ActivityButton::Favorites.view(),
            Some(SidebarView::Favorites)
        );
        assert_eq!(ActivityButton::Settings.view(), None);
        for (index, button) in ActivityButton::ALL.into_iter().enumerate() {
            assert_eq!(button.index(), index);
        }
    }

    #[test]
    fn the_notebook_label_names_the_open_notebook() {
        // Break caught: a Notebook tooltip that never says which notebook is open, or one that
        // reads "Notebook: " with nothing after it while none is.
        assert_eq!(notebook_label(Some("Work")), "Notebook: Work");
        assert_eq!(notebook_label(None), "Notebook");
    }
}
