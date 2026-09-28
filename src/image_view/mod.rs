//! The image view (image preview spec §6): a lazily created Direct2D child window showing one
//! image with fit, zoom and pan. Image tabs (`window::image_host`) and the SVG preview
//! (`window::preview_host`) each own one.
//!
//! Calls that can send a message back to this window (`SetScrollInfo`, `SetFocus`,
//! `ReleaseCapture`, `NotifyWinEvent`) run only after the state borrow ends, in `finish`.

pub mod accessible;
pub mod decode;
mod paint;
pub mod zoom;

use crate::Result;
use crate::library::DiskStamp;
use crate::platform::{last_error, wide_null};
use crate::preview::colors::PreviewColors;
use crate::preview::dwrite::Graphics;
use crate::window::{WM_FASTPAD_IMAGE_DECODED, WM_FASTPAD_IMAGE_STATUS};
use accessible::AccessibleText;
use decode::{Decoded, FullImage, ImageError, Source};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1BitmapBrush, ID2D1HwndRenderTarget};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, ScreenToClient, ValidateRect};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::NotifyWinEvent;
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_NEXT,
    VK_PRIOR, VK_RIGHT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CS_DBLCLKS, CreateWindowExW, DLGC_WANTARROWS, DefWindowProcW, DestroyWindow,
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_VALUECHANGE, GWLP_USERDATA, GetClientRect, GetParent,
    GetScrollInfo, GetWindowLongPtrW, HTCLIENT, IDC_ARROW, IDC_SIZEALL, KillTimer, LoadCursorW,
    OBJID_CLIENT, PostMessageW, RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP,
    SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_PAGE,
    SIF_POS, SIF_RANGE, SIF_TRACKPOS, SetCursor, SetTimer, SetWindowLongPtrW, WM_CAPTURECHANGED,
    WM_DPICHANGED_AFTERPARENT, WM_ERASEBKGND, WM_GETDLGCODE, WM_GETOBJECT, WM_HSCROLL, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WM_TIMER,
    WM_VSCROLL, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS, WS_HSCROLL, WS_TABSTOP, WS_VSCROLL,
};
use zoom::Zoom;

const CLASS_NAME: &str = "FastPadImageView";
const LOADING_TIMER: usize = 1;
const SVG_TIMER: usize = 2;
const LOADING_DELAY_MS: u32 = 150;
const DEFAULT_MAX_SIDE: u32 = 16_384;
/// One arrow key, scroll-bar line or wheel notch pans this far, in pixels at 96 DPI.
const PAN_STEP: f32 = 48.0;
/// `MK_CONTROL` and `MK_SHIFT` from WinUser.h; their windows-sys home needs a feature FastPad does
/// not enable.
const MK_CONTROL: u32 = 0x0008;
const MK_SHIFT: u32 = 0x0004;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Phase {
    #[default]
    Empty,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImageStats {
    pub phase: Phase,
    pub scale: f32,
    pub offset: (f32, f32),
    pub decodes: u32,
    pub has_target: bool,
    pub svg_error: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageViewStatus {
    pub size: Option<(u32, u32)>,
    pub format: Option<&'static str>,
    pub zoom_percent: Option<u32>,
    pub failed: bool,
}

/// What the view was last asked to show, so repeating the request does not decode again.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    File {
        path: PathBuf,
        stamp: Option<DiskStamp>,
    },
    Svg(Arc<str>),
}

struct ViewState {
    hwnd: HWND,
    target: Option<ID2D1HwndRenderTarget>,
    bitmap: Option<ID2D1Bitmap>,
    checker: Option<ID2D1BitmapBrush>,
    title_format: Option<IDWriteTextFormat>,
    body_format: Option<IDWriteTextFormat>,
    colors: PreviewColors,
    high_contrast: bool,
    name: String,
    shown: Option<Shown>,
    generation: u64,
    image: Option<FullImage>,
    error: Option<ImageError>,
    /// The last SVG render failed while an earlier image is still shown.
    svg_error: bool,
    loading: bool,
    loading_visible: bool,
    zoom: Zoom,
    scale: f32,
    offset: (f32, f32),
    /// Pointer position and offset when a drag started.
    drag: Option<((i32, i32), (f32, f32))>,
    max_side: u32,
    decodes: u32,
    paint_retried: bool,
    /// What `finish` last published, so it only notifies about changes.
    last_status: ImageViewStatus,
    last_scroll: Option<(ScrollRange, ScrollRange)>,
    accessible: AccessibleText,
    graphics: Rc<Graphics>,
}

/// One scroll bar's range, page and position in pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ScrollRange {
    max: i32,
    page: u32,
    pos: i32,
}

/// A handle to an image view; copies are cheap and all refer to the same window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageView {
    hwnd: HWND,
}

impl ImageView {
    pub fn create(
        parent: HWND,
        graphics: Rc<Graphics>,
        colors: PreviewColors,
        high_contrast: bool,
    ) -> Result<Self> {
        let hwnd = create_window(parent)?;
        let state = Box::new(ViewState {
            hwnd,
            target: None,
            bitmap: None,
            checker: None,
            title_format: None,
            body_format: None,
            colors,
            high_contrast,
            name: String::new(),
            shown: None,
            generation: 0,
            image: None,
            error: None,
            svg_error: false,
            loading: false,
            loading_visible: false,
            zoom: Zoom::Fit,
            scale: 1.0,
            offset: (0.0, 0.0),
            drag: None,
            max_side: DEFAULT_MAX_SIDE,
            decodes: 0,
            paint_retried: false,
            last_status: ImageViewStatus::default(),
            last_scroll: None,
            accessible: Arc::new(RwLock::new((String::new(), String::new()))),
            graphics,
        });
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize) };
        Ok(Self { hwnd })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn destroy(self) {
        unsafe { DestroyWindow(self.hwnd) };
    }

    fn with<R>(&self, action: impl FnOnce(&mut ViewState) -> R) -> Option<R> {
        with_state(self.hwnd, action)
    }

    /// Shows the image file at `path`. Asking again for the same file and stamp keeps what is
    /// shown; a new stamp decodes again and keeps the zoom, a new file starts at fit.
    pub fn show_file(&self, path: &Path, stamp: Option<DiskStamp>, name: &str) {
        self.with(|state| {
            state.name = name.to_owned();
            let wanted = Shown::File {
                path: path.to_path_buf(),
                stamp,
            };
            if state.shown.as_ref() == Some(&wanted)
                && (state.image.is_some() || state.loading || state.error.is_some())
            {
                return;
            }
            let same_file =
                matches!(&state.shown, Some(Shown::File { path: shown, .. }) if shown == path);
            if !same_file {
                state.zoom = Zoom::Fit;
                state.offset = (0.0, 0.0);
                state.image = None;
                state.bitmap = None;
            }
            state.shown = Some(wanted);
            state.svg_error = false;
            start_decode(state, Source::File(path.to_path_buf()));
        });
        finish(self.hwnd);
    }

    /// Renders SVG source text. The previous render stays on screen until the new one lands.
    pub fn show_svg(&self, text: Arc<str>, name: &str) {
        self.with(|state| {
            state.name = name.to_owned();
            if matches!(&state.shown, Some(Shown::Svg(shown)) if **shown == *text) {
                return;
            }
            if !matches!(state.shown, Some(Shown::Svg(_))) {
                state.zoom = Zoom::Fit;
                state.offset = (0.0, 0.0);
                state.image = None;
                state.bitmap = None;
            }
            let width = svg_width(state);
            state.shown = Some(Shown::Svg(Arc::clone(&text)));
            start_decode(state, Source::Svg { text, width });
        });
        finish(self.hwnd);
    }

    /// Shows the failed state for `error` without decoding.
    pub fn show_error(&self, error: ImageError) {
        self.with(|state| {
            // A deleted file's tab shows the stamp "no file"; asking for that again keeps the notice.
            if error == ImageError::Missing
                && let Some(Shown::File { stamp, .. }) = &mut state.shown
            {
                *stamp = None;
            }
            if error == ImageError::SvgTooLarge {
                state.shown = None;
            }
            state.generation += 1;
            stop_loading(state);
            state.image = None;
            state.bitmap = None;
            state.svg_error = false;
            state.error = Some(error);
        });
        finish(self.hwnd);
    }

    /// Frees the device resources while the view is hidden. The decoded pixels stay, so showing
    /// the same image again paints without decoding (spec §3).
    pub fn release(&self) {
        self.with(|state| {
            drop_device_resources(state);
            state.title_format = None;
            state.body_format = None;
            state.drag = None;
            unsafe {
                KillTimer(state.hwnd, SVG_TIMER);
            }
        });
    }

    pub fn zoom_in(&self) {
        self.zoom_by(zoom::step_in);
    }

    pub fn zoom_out(&self) {
        self.zoom_by(zoom::step_out);
    }

    fn zoom_by(&self, step: fn(f32) -> f32) {
        self.with(|state| {
            let (width, height) = client_size(state.hwnd);
            let new = step(state.scale);
            set_zoom(state, Zoom::Scale(new), (width / 2.0, height / 2.0));
        });
        finish(self.hwnd);
    }

    pub fn zoom_reset(&self) {
        self.with(|state| {
            let (width, height) = client_size(state.hwnd);
            set_zoom(state, Zoom::Fit, (width / 2.0, height / 2.0));
        });
        finish(self.hwnd);
    }

    /// Double-click: fit ↔ 100%, keeping the image point under `anchor` still.
    pub fn toggle_actual_size(&self, anchor: (f32, f32)) {
        self.with(|state| toggle_actual_size(state, anchor));
        finish(self.hwnd);
    }

    pub fn set_appearance(&self, colors: PreviewColors, high_contrast: bool) {
        self.with(|state| {
            if state.colors != colors || state.high_contrast != high_contrast {
                state.colors = colors;
                state.high_contrast = high_contrast;
                state.checker = None;
            }
        });
        invalidate(self.hwnd);
    }

    pub fn status(&self) -> ImageViewStatus {
        self.with(|state| status_of(state)).unwrap_or_default()
    }

    pub fn stats(&self) -> ImageStats {
        self.with(|state| ImageStats {
            phase: phase(state),
            scale: state.scale,
            offset: state.offset,
            decodes: state.decodes,
            has_target: state.target.is_some(),
            svg_error: state.svg_error,
        })
        .unwrap_or_default()
    }

    /// The accessible name and value (spec §6.6).
    #[cfg(test)]
    pub fn accessible_text(&self) -> (String, String) {
        self.with(|state| {
            state
                .accessible
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        })
        .unwrap_or_default()
    }
}

/// Kept out of `ImageView::create` so the public function never hands a raw `HWND` straight to an
/// unsafe call (`clippy::not_unsafe_ptr_arg_deref`); Windows validates the handle itself.
fn create_window(parent: HWND) -> Result<HWND> {
    register_class()?;
    let class = wide_null(CLASS_NAME);
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP | WS_HSCROLL | WS_VSCROLL,
            0,
            0,
            0,
            0,
            parent,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(last_error());
    }
    Ok(hwnd)
}

fn register_class() -> Result<()> {
    let class = wide_null(CLASS_NAME);
    let window_class = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(image_proc),
        hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
        hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    if unsafe { RegisterClassW(&window_class) } == 0
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(last_error());
    }
    Ok(())
}

fn with_state<R>(hwnd: HWND, action: impl FnOnce(&mut ViewState) -> R) -> Option<R> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ViewState;
    // SAFETY: the pointer is owned by this window until WM_NCDESTROY, and every entry point runs on
    // the window's thread without sending messages while the borrow is live.
    (!pointer.is_null()).then(|| action(unsafe { &mut *pointer }))
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

fn dpi_scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0
}

/// The client area in device pixels (the render target uses 96 DPI, so these are also DIPs).
fn client_size(hwnd: HWND) -> (f32, f32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) };
    (
        (rect.right - rect.left).max(0) as f32,
        (rect.bottom - rect.top).max(0) as f32,
    )
}

fn phase(state: &ViewState) -> Phase {
    if state.error.is_some() {
        Phase::Failed
    } else if state.image.is_some() {
        Phase::Ready
    } else if state.loading {
        Phase::Loading
    } else {
        Phase::Empty
    }
}

fn status_of(state: &ViewState) -> ImageViewStatus {
    let ready = phase(state) == Phase::Ready;
    ImageViewStatus {
        size: state
            .image
            .as_ref()
            .map(|image| (image.natural_width, image.natural_height)),
        format: state.image.as_ref().map(|image| image.format),
        zoom_percent: ready.then(|| (state.scale * 100.0).round() as u32),
        failed: state.error.is_some(),
    }
}

/// The SVG render width for the current zoom: natural size × scale, or natural size (0) before
/// the first render.
fn svg_width(state: &ViewState) -> u32 {
    state.image.as_ref().map_or(0, |image| {
        (image.natural_width as f32 * state.scale).round().max(1.0) as u32
    })
}

fn start_decode(state: &mut ViewState, source: Source) {
    state.generation += 1;
    state.error = None;
    state.loading = true;
    state.loading_visible = false;
    unsafe { SetTimer(state.hwnd, LOADING_TIMER, LOADING_DELAY_MS, None) };
    decode::spawn(
        state.generation,
        source,
        state.max_side,
        state.hwnd,
        WM_FASTPAD_IMAGE_DECODED,
    );
}

fn stop_loading(state: &mut ViewState) {
    unsafe { KillTimer(state.hwnd, LOADING_TIMER) };
    state.loading = false;
    state.loading_visible = false;
}

fn drop_device_resources(state: &mut ViewState) {
    state.target = None;
    state.bitmap = None;
    state.checker = None;
}

fn accept_decoded(state: &mut ViewState, decoded: Decoded) {
    if decoded.generation != state.generation {
        return;
    }
    stop_loading(state);
    state.decodes += 1;
    let svg = matches!(state.shown, Some(Shown::Svg(_)));
    match decoded.result {
        Ok(image) => {
            // A reload whose pixel size changed starts at fit again (spec §6.5); an SVG re-render at
            // another zoom keeps its natural size, so its zoom stays.
            let resized = state.image.as_ref().is_some_and(|old| {
                (old.natural_width, old.natural_height)
                    != (image.natural_width, image.natural_height)
            });
            if resized && !svg {
                state.zoom = Zoom::Fit;
            }
            state.image = Some(image);
            state.bitmap = None;
            state.error = None;
            state.svg_error = false;
        }
        Err(_) if svg && state.image.is_some() => state.svg_error = true,
        Err(error) => {
            state.error = Some(error);
            state.image = None;
            state.bitmap = None;
        }
    }
    recompute(state);
}

/// Applies the zoom to the current client size: sets `scale` and clamps `offset`.
fn recompute(state: &mut ViewState) {
    let Some(image) = &state.image else {
        return;
    };
    let natural = (image.natural_width as f32, image.natural_height as f32);
    let (width, height) = client_size(state.hwnd);
    state.scale = match state.zoom {
        Zoom::Fit => zoom::fit_scale(natural, (width, height)),
        Zoom::Scale(scale) => scale,
    };
    state.offset = (
        zoom::clamp_axis(state.offset.0, natural.0 * state.scale, width),
        zoom::clamp_axis(state.offset.1, natural.1 * state.scale, height),
    );
}

fn set_zoom(state: &mut ViewState, zoom: Zoom, anchor: (f32, f32)) {
    if state.image.is_none() {
        return;
    }
    let old = state.scale;
    state.zoom = zoom;
    recompute_scale_only(state);
    if old > 0.0 {
        state.offset = zoom::zoom_about(state.offset, old, state.scale, anchor);
    }
    recompute(state);
    if matches!(state.shown, Some(Shown::Svg(_))) {
        unsafe {
            SetTimer(
                state.hwnd,
                SVG_TIMER,
                crate::preview::PREVIEW_UPDATE_DELAY_MS,
                None,
            )
        };
    }
}

/// `recompute`'s scale half, so `set_zoom` can anchor the offset before it is clamped.
fn recompute_scale_only(state: &mut ViewState) {
    let Some(image) = &state.image else {
        return;
    };
    let natural = (image.natural_width as f32, image.natural_height as f32);
    state.scale = match state.zoom {
        Zoom::Fit => zoom::fit_scale(natural, client_size(state.hwnd)),
        Zoom::Scale(scale) => scale,
    };
}

fn toggle_actual_size(state: &mut ViewState, anchor: (f32, f32)) {
    let zoom = match state.zoom {
        Zoom::Fit => Zoom::Scale(1.0),
        Zoom::Scale(_) => Zoom::Fit,
    };
    set_zoom(state, zoom, anchor);
}

/// The image's displayed size in client pixels, or `None` with nothing to show.
fn shown_size(state: &ViewState) -> Option<(f32, f32)> {
    state.image.as_ref().map(|image| {
        (
            image.natural_width as f32 * state.scale,
            image.natural_height as f32 * state.scale,
        )
    })
}

/// Whether the image is larger than the view on either axis, so it can be dragged.
fn draggable(state: &ViewState) -> bool {
    let (width, height) = client_size(state.hwnd);
    shown_size(state).is_some_and(|(w, h)| w > width + 0.5 || h > height + 0.5)
}

fn pan(state: &mut ViewState, dx: f32, dy: f32) {
    state.offset = (state.offset.0 + dx, state.offset.1 + dy);
    recompute(state);
}

fn scroll_ranges(state: &ViewState) -> (ScrollRange, ScrollRange) {
    let (width, height) = client_size(state.hwnd);
    let axis = |image_len: f32, view_len: f32, offset: f32| {
        if image_len <= view_len + 0.5 {
            ScrollRange::default()
        } else {
            ScrollRange {
                max: image_len.round() as i32 - 1,
                page: view_len.round() as u32,
                pos: (-offset).round() as i32,
            }
        }
    };
    match shown_size(state) {
        Some((w, h)) => (
            axis(w, width, state.offset.0),
            axis(h, height, state.offset.1),
        ),
        None => (ScrollRange::default(), ScrollRange::default()),
    }
}

fn accessible_texts(state: &ViewState) -> (String, String) {
    let name = &state.name;
    match (&state.error, &state.image) {
        (Some(error), _) => (
            format!("{name}, image, can't display: {}", error.message()),
            String::new(),
        ),
        (None, Some(image)) => (
            format!(
                "{name}, image, {} by {} pixels",
                image.natural_width, image.natural_height
            ),
            format!("Zoom {} percent", (state.scale * 100.0).round() as u32),
        ),
        (None, None) if state.loading => (format!("{name}, image, loading"), String::new()),
        (None, None) => (format!("{name}, image"), String::new()),
    }
}

/// Publishes a state change: the scroll bars, the accessible text and its events, the status
/// message to the parent, and a repaint. Runs with no state borrowed, because `SetScrollInfo` and
/// `NotifyWinEvent` can call back into this window.
fn finish(hwnd: HWND) {
    let Some((scroll, name_changed, value_changed, status_changed)) = with_state(hwnd, |state| {
        let ranges = scroll_ranges(state);
        let scroll = (state.last_scroll != Some(ranges)).then_some(ranges);
        state.last_scroll = Some(ranges);
        let (name, value) = accessible_texts(state);
        let (name_changed, value_changed) = {
            let mut shared = state
                .accessible
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let changed = (shared.0 != name, shared.1 != value);
            *shared = (name, value);
            changed
        };
        let status = status_of(state);
        let status_changed = status != state.last_status;
        state.last_status = status;
        (scroll, name_changed, value_changed, status_changed)
    }) else {
        return;
    };
    if let Some((horizontal, vertical)) = scroll {
        for (bar, range) in [(SB_HORZ, horizontal), (SB_VERT, vertical)] {
            let info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: range.max,
                nPage: range.page,
                nPos: range.pos,
                nTrackPos: 0,
            };
            unsafe { SetScrollInfo(hwnd, bar, &info, 1) };
        }
    }
    if name_changed {
        unsafe {
            NotifyWinEvent(
                EVENT_OBJECT_NAMECHANGE,
                hwnd,
                OBJID_CLIENT,
                CHILDID_SELF as i32,
            )
        };
    }
    if value_changed {
        unsafe {
            NotifyWinEvent(
                EVENT_OBJECT_VALUECHANGE,
                hwnd,
                OBJID_CLIENT,
                CHILDID_SELF as i32,
            )
        };
    }
    if status_changed {
        unsafe { PostMessageW(GetParent(hwnd), WM_FASTPAD_IMAGE_STATUS, 0, 0) };
    }
    invalidate(hwnd);
}

fn point_from(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam & 0xFFFF) as u16 as i16 as i32,
        ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

fn handle_scroll(hwnd: HWND, bar: i32, request: i32) {
    let track = if request == SB_THUMBTRACK || request == SB_THUMBPOSITION {
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_TRACKPOS,
            ..Default::default()
        };
        unsafe { GetScrollInfo(hwnd, bar, &mut info) };
        Some(info.nTrackPos as f32)
    } else {
        None
    };
    let line = PAN_STEP * dpi_scale(hwnd);
    with_state(hwnd, |state| {
        let (width, height) = client_size(hwnd);
        let page = if bar == SB_HORZ { width } else { height };
        let current = if bar == SB_HORZ {
            state.offset.0
        } else {
            state.offset.1
        };
        let target = match request {
            SB_LINEUP => current + line,
            SB_LINEDOWN => current - line,
            SB_PAGEUP => current + page,
            SB_PAGEDOWN => current - page,
            SB_TOP => 0.0,
            SB_BOTTOM => f32::MIN,
            _ => match track {
                Some(position) => -position,
                None => return,
            },
        };
        if bar == SB_HORZ {
            state.offset.0 = target;
        } else {
            state.offset.1 = target;
        }
        recompute(state);
    });
    finish(hwnd);
}

unsafe extern "system" fn image_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut ViewState;
            if !pointer.is_null() {
                drop(unsafe { Box::from_raw(pointer) });
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let (repaint, shrank) = with_state(hwnd, |state| {
                let old_max = state.max_side;
                match paint::paint(state) {
                    Ok(()) => state.paint_retried = false,
                    Err(error) => {
                        drop_device_resources(state);
                        if error.code() != D2DERR_RECREATE_TARGET {
                            // Retry a failed frame once; a device that keeps failing must not spin.
                            return (!std::mem::replace(&mut state.paint_retried, true), false);
                        }
                        return (true, false);
                    }
                }
                // The first target can hold smaller bitmaps than the decode assumed: decode again.
                let shrank = state.max_side < old_max
                    && state
                        .image
                        .as_ref()
                        .is_some_and(|image| image.width.max(image.height) > state.max_side);
                (false, shrank)
            })
            .unwrap_or_default();
            unsafe { ValidateRect(hwnd, std::ptr::null()) };
            if shrank {
                with_state(hwnd, |state| match state.shown.clone() {
                    Some(Shown::File { path, .. }) => start_decode(state, Source::File(path)),
                    Some(Shown::Svg(text)) => {
                        let width = svg_width(state);
                        start_decode(state, Source::Svg { text, width });
                    }
                    None => {}
                });
            }
            if repaint {
                invalidate(hwnd);
            }
            0
        }
        WM_SIZE => {
            let width = (lparam & 0xFFFF) as u32;
            let height = ((lparam >> 16) & 0xFFFF) as u32;
            with_state(hwnd, |state| {
                if let Some(target) = &state.target
                    && unsafe {
                        target.Resize(&D2D_SIZE_U {
                            width: width.max(1),
                            height: height.max(1),
                        })
                    }
                    .is_err()
                {
                    drop_device_resources(state);
                }
                recompute(state);
            });
            finish(hwnd);
            0
        }
        WM_DPICHANGED_AFTERPARENT => {
            with_state(hwnd, |state| {
                state.title_format = None;
                state.body_format = None;
                recompute(state);
            });
            finish(hwnd);
            0
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            invalidate(hwnd);
            0
        }
        WM_TIMER => {
            match wparam {
                LOADING_TIMER => {
                    with_state(hwnd, |state| {
                        unsafe { KillTimer(hwnd, LOADING_TIMER) };
                        if state.loading {
                            state.loading_visible = true;
                        }
                    });
                    invalidate(hwnd);
                }
                SVG_TIMER => {
                    with_state(hwnd, |state| {
                        unsafe { KillTimer(hwnd, SVG_TIMER) };
                        let Some(Shown::Svg(text)) = state.shown.clone() else {
                            return;
                        };
                        let Some(rendered) = state.image.as_ref().map(|image| image.width) else {
                            return;
                        };
                        let wanted = svg_width(state);
                        let drift =
                            (wanted as f32 - rendered as f32).abs() / rendered.max(1) as f32;
                        if drift > 0.01 {
                            start_decode(
                                state,
                                Source::Svg {
                                    text,
                                    width: wanted,
                                },
                            );
                        }
                    });
                }
                _ => {}
            }
            0
        }
        WM_FASTPAD_IMAGE_DECODED => {
            if lparam != 0 {
                let decoded = unsafe { Box::from_raw(lparam as *mut Decoded) };
                with_state(hwnd, |state| accept_decoded(state, *decoded));
                finish(hwnd);
            }
            0
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as f32;
            let keys = (wparam & 0xFFFF) as u32;
            let step = delta / 120.0 * PAN_STEP * dpi_scale(hwnd);
            if message == WM_MOUSEWHEEL && keys & MK_CONTROL != 0 {
                let (x, y) = point_from(lparam);
                let mut point = POINT { x, y };
                unsafe { ScreenToClient(hwnd, &mut point) };
                with_state(hwnd, |state| {
                    let new = if delta > 0.0 {
                        zoom::step_in(state.scale)
                    } else {
                        zoom::step_out(state.scale)
                    };
                    set_zoom(state, Zoom::Scale(new), (point.x as f32, point.y as f32));
                });
            } else if message == WM_MOUSEHWHEEL {
                with_state(hwnd, |state| pan(state, -step, 0.0));
            } else if keys & MK_SHIFT != 0 {
                with_state(hwnd, |state| pan(state, step, 0.0));
            } else {
                with_state(hwnd, |state| pan(state, 0.0, step));
            }
            finish(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            unsafe { SetFocus(hwnd) };
            let point = point_from(lparam);
            let dragging = with_state(hwnd, |state| {
                let dragging = draggable(state);
                if dragging {
                    state.drag = Some((point, state.offset));
                }
                dragging
            })
            .unwrap_or(false);
            if dragging {
                unsafe { SetCapture(hwnd) };
            }
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = point_from(lparam);
            let moved = with_state(hwnd, |state| {
                let Some(((start_x, start_y), start)) = state.drag else {
                    return false;
                };
                state.offset = (
                    start.0 + (x - start_x) as f32,
                    start.1 + (y - start_y) as f32,
                );
                recompute(state);
                true
            })
            .unwrap_or(false);
            if moved {
                finish(hwnd);
            }
            0
        }
        WM_LBUTTONUP => {
            let dragging = with_state(hwnd, |state| state.drag.take().is_some()).unwrap_or(false);
            if dragging {
                unsafe { ReleaseCapture() };
            }
            0
        }
        WM_CAPTURECHANGED => {
            with_state(hwnd, |state| state.drag = None);
            0
        }
        WM_LBUTTONDBLCLK => {
            let (x, y) = point_from(lparam);
            with_state(hwnd, |state| {
                toggle_actual_size(state, (x as f32, y as f32))
            });
            finish(hwnd);
            0
        }
        WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
            let cursor = if with_state(hwnd, |state| draggable(state)).unwrap_or(false) {
                IDC_SIZEALL
            } else {
                IDC_ARROW
            };
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
            1
        }
        WM_KEYDOWN => {
            let line = PAN_STEP * dpi_scale(hwnd);
            let key = wparam as u16;
            let handled = with_state(hwnd, |state| {
                let (_, height) = client_size(hwnd);
                match key {
                    VK_LEFT => pan(state, line, 0.0),
                    VK_RIGHT => pan(state, -line, 0.0),
                    VK_UP => pan(state, 0.0, line),
                    VK_DOWN => pan(state, 0.0, -line),
                    VK_PRIOR => pan(state, 0.0, height * 0.9),
                    VK_NEXT => pan(state, 0.0, -height * 0.9),
                    VK_HOME => {
                        state.offset.1 = 0.0;
                        recompute(state);
                    }
                    VK_END => {
                        state.offset.1 = f32::MIN;
                        recompute(state);
                    }
                    _ => return false,
                }
                true
            })
            .unwrap_or(false);
            if handled {
                finish(hwnd);
                0
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_GETDLGCODE => DLGC_WANTARROWS as LRESULT,
        WM_HSCROLL => {
            handle_scroll(hwnd, SB_HORZ, (wparam & 0xFFFF) as i32);
            0
        }
        WM_VSCROLL => {
            handle_scroll(hwnd, SB_VERT, (wparam & 0xFFFF) as i32);
            0
        }
        WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => {
            // Clone the snapshot inside the borrow; `LresultFromObject` runs after it ends.
            match with_state(hwnd, |state| Arc::clone(&state.accessible)) {
                Some(text) => accessible::object_result(hwnd, text, wparam),
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Whether `hwnd` holds the keyboard focus, for the focus ring.
fn focused(hwnd: HWND) -> bool {
    (unsafe { GetFocus() }) == hwnd
}
