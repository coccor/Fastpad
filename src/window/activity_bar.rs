//! The activity bar: the sidebar's narrow painted strip of view buttons at the main window's left
//! edge, with Settings at the bottom. The active view has a 2 px accent bar and full-strength
//! color, and the others are muted. The strip above the first button is caption, so the bar
//! answers `HTTRANSPARENT` there.

use super::side_panel::{self, draw_text, paint_buffered, point_of, with_bar_state};
use crate::config::SidebarView;
use crate::window::panel::{fill, scale};
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
    DefWindowProcW, GetClientRect, GetParent, HTTRANSPARENT, WM_CAPTURECHANGED, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT,
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
    let state = with_bar_state(main, |state| *state).unwrap_or_default();
    let view = side_panel::current_view(main);
    let accent = scale(ACCENT_WIDTH_96, dpi);
    paint_buffered(bar, |dc, client| unsafe {
        fill(dc, client, palette.strip_background);
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
    });
}

#[cfg(test)]
mod tests {
    use super::{ActivityButton, button_at, button_rects, notebook_label};
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
