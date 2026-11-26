//! The image view (image preview spec §6): a lazily created Direct2D child window showing one
//! image with fit, zoom and pan. Image tabs (`window::image_host`) and the SVG preview
//! (`window::preview_host`) each own one.
//!
//! Calls that can send a message back to this window (`SetScrollInfo`, `SetFocus`,
//! `ReleaseCapture`, `NotifyWinEvent`) run only after the state borrow ends, in `finish`.

pub mod accessible;
mod cursor;
pub mod decode;
pub(crate) mod effects;
mod paint;
pub mod toolbar;
mod vector;
pub mod zoom;

use crate::Result;
use crate::library::DiskStamp;
use crate::platform::{last_error, wide_null};
use crate::preview::colors::PreviewColors;
use crate::preview::dwrite::Graphics;
use crate::window::{
    WM_FASTPAD_DIAGRAM_CLICKED, WM_FASTPAD_IMAGE_DECODED, WM_FASTPAD_IMAGE_STATUS,
};
use accessible::AccessibleText;
use decode::{Decoded, Delivery, FullImage, ImageError, Rendered, Source, Tile, TileJob};
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
use windows_sys::Win32::UI::Controls::{SetScrollInfo, WM_MOUSELEAVE};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_CONTROL, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT,
    VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CS_DBLCLKS, CreateWindowExW, DLGC_WANTARROWS, DefWindowProcW, DestroyWindow,
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_VALUECHANGE, GWLP_USERDATA, GetClientRect, GetCursorPos,
    GetScrollInfo, GetSystemMetrics, GetWindowLongPtrW, HCURSOR, HTCLIENT, IDC_ARROW, IDC_HAND,
    IDC_SIZEALL, KillTimer, LoadCursorW, MSG, OBJID_CLIENT, PM_REMOVE, PeekMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, SB_BOTTOM, SB_HORZ, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN,
    SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_PAGE, SIF_POS,
    SIF_RANGE, SIF_TRACKPOS, SM_CXDRAG, SM_CYDRAG, SetCursor, SetTimer, SetWindowLongPtrW,
    WM_CAPTURECHANGED, WM_COMMAND, WM_DPICHANGED_AFTERPARENT, WM_ERASEBKGND, WM_GETDLGCODE,
    WM_GETOBJECT, WM_HSCROLL, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_QUIT,
    WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_HSCROLL, WS_TABSTOP, WS_VSCROLL, WindowFromPoint,
};
use zoom::Zoom;

const CLASS_NAME: &str = "FastPadImageView";
const LOADING_TIMER: usize = 1;
const SVG_TIMER: usize = 2;
const TILE_TIMER: usize = 3;
const LOADING_DELAY_MS: u32 = 150;
/// A pan that uncovers blurred picture renders a sharp tile this long after it starts.
const TILE_DELAY_MS: u32 = 40;
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageStats {
    pub phase: Phase,
    pub scale: f32,
    pub offset: (f32, f32),
    pub decodes: u32,
    pub has_target: bool,
    pub svg_error: bool,
    /// The text the error band shows, when it shows.
    pub band: Option<String>,
    /// The pixel size of the whole-picture bitmap.
    pub whole: Option<(u32, u32)>,
    /// The zoom and area of the sharp tile over a deeply zoomed vector picture.
    pub tile: Option<(f32, decode::TileArea)>,
    /// The picture is drawn by Direct2D at every paint, at any zoom, with no pixels rendered.
    pub vector: bool,
    /// The diagram line whose elements are outlined (the caret's, in Split).
    pub marked_line: Option<u32>,
    /// How many elements that outline draws.
    pub marked: usize,
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
    FastDraw {
        text: Arc<str>,
        file: Option<PathBuf>,
    },
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
    /// The picture as Direct2D geometry while `image.vector`, made at its first paint.
    scene: Option<vector::Scene>,
    /// Direct2D draws pictures it can; cleared when making a scene fails, so resvg draws them.
    gpu: bool,
    /// The visible part of a vector picture zoomed past `decode::WHOLE_PIXEL_BUDGET`, rendered
    /// sharp from the image's tree and drawn over the enlarged whole picture.
    tile: Option<Tile>,
    tile_bitmap: Option<ID2D1Bitmap>,
    /// Numbers the image's tree, so a render from a replaced tree is dropped.
    tree_id: u64,
    /// A render from the tree is under way; one at a time, and its landing looks for more work.
    rendering: bool,
    /// `TILE_TIMER` is set.
    tile_timer: bool,
    error: Option<ImageError>,
    /// The last SVG render failed while an earlier image is still shown.
    svg_error: bool,
    /// The band's text for a diagram error; an SVG error uses the generic text.
    band: Option<String>,
    loading: bool,
    loading_visible: bool,
    zoom: Zoom,
    scale: f32,
    offset: (f32, f32),
    /// Pointer position and offset when a drag started.
    drag: Option<((i32, i32), (f32, f32))>,
    /// Where the left button went down, until it comes up: released without moving past the drag
    /// distance, the press is a click: it zooms a picture, and on a diagram selects the line of the
    /// element under it.
    press: Option<(i32, i32)>,
    /// The press moved past the drag distance, so it pans instead of zooming.
    moved: bool,
    /// The shown diagram's hits are those of the text last asked for: false from an edit until its
    /// drawing lands, so a click on the old picture never selects a line of the new text.
    hits_fresh: bool,
    /// The diagram's text was edited since it was last asked to draw it.
    edited: bool,
    /// The 1-based line whose diagram elements are outlined.
    marked_line: Option<u32>,
    /// The pointer over the diagram toolbar, and its tooltip.
    bar: toolbar::Pointer,
    /// The toolbar's icon font, made at its first paint.
    icon_format: Option<IDWriteTextFormat>,
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
            scene: None,
            gpu: true,
            tile: None,
            tile_bitmap: None,
            tree_id: 0,
            rendering: false,
            tile_timer: false,
            error: None,
            svg_error: false,
            band: None,
            loading: false,
            loading_visible: false,
            zoom: Zoom::Fit,
            scale: 1.0,
            offset: (0.0, 0.0),
            drag: None,
            press: None,
            moved: false,
            hits_fresh: false,
            edited: false,
            marked_line: None,
            bar: toolbar::Pointer::default(),
            icon_format: None,
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

    /// Forgets the diagram toolbar's pointer state and destroys its tooltip, whose tips would
    /// otherwise go on showing over the picture that replaces the diagram.
    fn drop_toolbar(&self) {
        let tooltip = self
            .with(|state| std::mem::take(&mut state.bar).tooltip)
            .flatten();
        // Destroyed with nothing of the state borrowed: it sends messages.
        if let Some(tooltip) = tooltip {
            tooltip.destroy();
        }
    }

    /// The text of the diagram shown changed in its editor: until it is drawn again, the drawing's
    /// lines are not the text's, so clicks select nothing and nothing is outlined.
    pub fn diagram_edited(&self) {
        let changed = self
            .with(|state| !std::mem::replace(&mut state.edited, true))
            .unwrap_or(false);
        if changed {
            invalidate(self.hwnd);
        }
    }

    /// Shows the image file at `path`. Asking again for the same file and stamp keeps what is
    /// shown; a new stamp decodes again and keeps the zoom, a new file starts at fit.
    pub fn show_file(&self, path: &Path, stamp: Option<DiskStamp>, name: &str) {
        self.drop_toolbar();
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
                clear_picture(state);
            }
            state.shown = Some(wanted);
            state.svg_error = false;
            state.band = None;
            start_decode(state, Source::File(path.to_path_buf()));
        });
        finish(self.hwnd);
    }

    /// Renders SVG source text. An edit of the same document keeps the previous render on screen
    /// until the new one lands; a `new_document` starts at fit with nothing shown, so its first
    /// failure shows the failed state rather than another document's picture.
    pub fn show_svg(&self, text: Arc<str>, name: &str, new_document: bool) {
        self.drop_toolbar();
        self.with(|state| {
            state.name = name.to_owned();
            if !new_document && matches!(&state.shown, Some(Shown::Svg(shown)) if **shown == *text)
            {
                return;
            }
            if new_document || !matches!(state.shown, Some(Shown::Svg(_))) {
                state.zoom = Zoom::Fit;
                state.offset = (0.0, 0.0);
                clear_picture(state);
            }
            state.shown = Some(Shown::Svg(text));
            start_svg_decode(state);
        });
        finish(self.hwnd);
    }

    /// Draws FastDraw source `text` (from `file`, the base of a relative look path) in the view's
    /// light or dark look. Edits and new documents behave as for `show_svg`.
    pub fn show_fastdraw(
        &self,
        text: Arc<str>,
        file: Option<PathBuf>,
        name: &str,
        new_document: bool,
    ) {
        self.with(|state| {
            state.name = name.to_owned();
            state.edited = false;
            if !new_document
                && matches!(&state.shown, Some(Shown::FastDraw { text: shown, file: shown_file })
                    if **shown == *text && *shown_file == file)
            {
                return;
            }
            if new_document || !matches!(state.shown, Some(Shown::FastDraw { .. })) {
                state.zoom = Zoom::Fit;
                state.offset = (0.0, 0.0);
                clear_picture(state);
                state.svg_error = false;
                state.band = None;
            }
            state.shown = Some(Shown::FastDraw { text, file });
            state.hits_fresh = false;
            start_svg_decode(state);
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
            clear_picture(state);
            state.svg_error = false;
            state.band = None;
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
            state.icon_format = None;
            state.drag = None;
            state.press = None;
            state.tile_timer = false;
            unsafe {
                KillTimer(state.hwnd, SVG_TIMER);
                KillTimer(state.hwnd, TILE_TIMER);
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
            let new = step(zoom_level(state));
            set_zoom(state, Zoom::Scale(new), (width / 2.0, height / 2.0));
        });
        finish(self.hwnd);
    }

    /// The zoom asked for: fit, or a fixed scale.
    pub fn zoom(&self) -> Zoom {
        self.with(|state| state.zoom).unwrap_or(Zoom::Fit)
    }

    /// Zooms to `zoom` about the middle of the view, as a level picked from a list.
    pub fn set_zoom(&self, zoom: Zoom) {
        self.with(|state| {
            let (width, height) = client_size(state.hwnd);
            set_zoom(state, zoom, (width / 2.0, height / 2.0));
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

    pub fn set_appearance(&self, colors: PreviewColors, high_contrast: bool) {
        self.with(|state| {
            if state.colors != colors || state.high_contrast != high_contrast {
                let relook = state.colors.is_dark() != colors.is_dark()
                    && matches!(state.shown, Some(Shown::FastDraw { .. }));
                state.colors = colors;
                state.high_contrast = high_contrast;
                state.checker = None;
                // A diagram draws in the look that matches the theme.
                if relook {
                    start_svg_decode(state);
                }
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
            band: state.svg_error.then(|| {
                state
                    .band
                    .clone()
                    .unwrap_or_else(|| paint::SVG_BAND_TEXT.to_owned())
            }),
            whole: state
                .image
                .as_ref()
                .map(|image| (image.width, image.height)),
            tile: state.tile.as_ref().map(|tile| (tile.scale, tile.area)),
            vector: drawn_by_gpu(state),
            marked_line: state.marked_line,
            marked: marked_shapes(state).count(),
        })
        .unwrap_or_default()
    }

    /// The 1-based line of the diagram element under client point `client`, if the shown drawing
    /// is the current text's.
    pub fn hit_line(&self, client: (i32, i32)) -> Option<u32> {
        let tolerance = HIT_TOLERANCE_DIP * dpi_scale(self.hwnd);
        self.with(|state| hit_line(state, client, tolerance))
            .flatten()
    }

    /// Outlines the diagram elements written on 1-based `line`, or none.
    pub fn set_marked_line(&self, line: Option<u32>) {
        let changed = self
            .with(|state| std::mem::replace(&mut state.marked_line, line) != line)
            .unwrap_or(false);
        if changed {
            invalidate(self.hwnd);
        }
    }

    /// The diagram text the view was last asked to draw; `None` when it shows no diagram.
    pub fn diagram_text(&self) -> Option<Arc<str>> {
        self.with(|state| match &state.shown {
            Some(Shown::FastDraw { text, .. }) => Some(Arc::clone(text)),
            _ => None,
        })
        .flatten()
    }

    /// Whether the view holds the diagram toolbar's tooltip control.
    #[cfg(test)]
    pub fn has_toolbar_tooltip(&self) -> bool {
        self.with(|state| state.bar.tooltip.is_some())
            .unwrap_or(false)
    }

    /// The centre of the diagram toolbar's `button`, in client pixels.
    #[cfg(test)]
    pub fn button_point(&self, button: toolbar::Button) -> (i32, i32) {
        let (width, _) = client_size(self.hwnd);
        toolbar::layout(width, dpi_scale(self.hwnd))
            .into_iter()
            .find(|(b, _)| *b == button)
            .map(|(_, rect)| {
                (
                    ((rect.left + rect.right) / 2.0) as i32,
                    ((rect.top + rect.bottom) / 2.0) as i32,
                )
            })
            .unwrap_or_default()
    }

    /// Where scene point `scene` of the shown diagram is drawn, in client pixels.
    #[cfg(test)]
    pub fn client_point(&self, scene: (f64, f64)) -> (i32, i32) {
        self.with(|state| {
            let origin = diagram_hits(state).map_or((0.0, 0.0), |hits| hits.origin);
            let (x, y) = zoom::from_scene(scene, state.offset, state.scale, origin);
            (x.round() as i32, y.round() as i32)
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
        zoom_percent: ready.then(|| (zoom_level(state) * 100.0).round() as u32),
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

/// The size the whole SVG picture renders at for the current zoom: the zoomed size, up to
/// `decode::WHOLE_PIXEL_BUDGET`.
fn whole_size(image: &FullImage, state: &ViewState) -> (u32, u32) {
    decode::svg_render_size(
        (image.natural_width, image.natural_height),
        svg_width(state),
        state.max_side,
        decode::WHOLE_PIXEL_BUDGET,
    )
}

/// Whether Direct2D draws the shown picture at every paint, so no zoom renders pixels.
fn drawn_by_gpu(state: &ViewState) -> bool {
    state.image.as_ref().is_some_and(|image| image.vector)
}

/// Whether the shown SVG bitmap is more than 1% off the size the current zoom renders at, so it
/// is drawn stretched and should be rendered again.
fn svg_needs_render(state: &ViewState) -> bool {
    let Some(image) = state.image.as_ref().filter(|image| !image.vector) else {
        return false;
    };
    let (wanted, _) = whole_size(image, state);
    (wanted as f32 - image.width as f32).abs() / image.width.max(1) as f32 > 0.01
}

/// The tile the current zoom and position show sharp, when the vector picture is zoomed past
/// what renders whole.
fn tile_job(state: &ViewState) -> Option<TileJob> {
    let image = state.image.as_ref().filter(|image| !image.vector)?;
    image.tree.as_ref()?;
    let natural = (image.natural_width, image.natural_height);
    if !decode::tiled(natural, svg_width(state), state.max_side) {
        return None;
    }
    let area = decode::tile_area(
        shown_size(state)?,
        state.offset,
        client_size(state.hwnd),
        decode::TILE_MARGIN,
        state.max_side,
    )?;
    Some((state.scale, area))
}

/// `tile_job` when the shown tile does not already cover the view at this zoom.
fn wanted_tile(state: &ViewState) -> Option<TileJob> {
    let (scale, area) = tile_job(state)?;
    if let Some(tile) = &state.tile
        && tile.scale == scale
    {
        let visible = decode::tile_area(
            shown_size(state)?,
            state.offset,
            client_size(state.hwnd),
            0.0,
            state.max_side,
        )?;
        if tile.area.contains(&visible) || tile.area == area {
            return None;
        }
    }
    Some((scale, area))
}

/// Renders from the kept tree what the zoom and position want and the view lacks: the whole
/// picture at another size, a sharp tile, or both. A document decode under way brings its own.
fn refresh_vector(state: &mut ViewState) {
    if !shows_vector(state) || state.loading || state.rendering || drawn_by_gpu(state) {
        return;
    }
    let Some(image) = &state.image else {
        return;
    };
    let Some(tree) = image.tree.clone() else {
        if svg_needs_render(state) {
            start_svg_decode(state);
        }
        return;
    };
    let natural = (image.natural_width, image.natural_height);
    let whole = svg_needs_render(state).then(|| whole_size(image, state));
    let tile = wanted_tile(state);
    if whole.is_none() && tile.is_none() {
        return;
    }
    state.rendering = true;
    decode::spawn_render(
        state.tree_id,
        tree,
        natural,
        whole,
        tile,
        state.hwnd,
        WM_FASTPAD_IMAGE_DECODED,
    );
}

/// Asks for a tile shortly when the view shows blurred picture, unless one is already asked for.
fn schedule_tile(state: &mut ViewState) {
    if !state.tile_timer && shows_vector(state) && wanted_tile(state).is_some() {
        state.tile_timer = true;
        unsafe { SetTimer(state.hwnd, TILE_TIMER, TILE_DELAY_MS, None) };
    }
}

/// Whether the view shows the diagram toolbar: on a FastDraw diagram, never on an image or SVG.
fn shows_toolbar(state: &ViewState) -> bool {
    matches!(state.shown, Some(Shown::FastDraw { .. }))
}

/// Whether client point `point` is on the diagram toolbar, while it shows.
fn on_toolbar(state: &ViewState, point: (i32, i32)) -> bool {
    shows_toolbar(state)
        && toolbar::on_strip(
            client_size(state.hwnd).0,
            dpi_scale(state.hwnd),
            (point.0 as f32, point.1 as f32),
        )
}

/// The toolbar button under client point `point`, while the toolbar shows.
fn toolbar_button(state: &ViewState, point: (i32, i32)) -> Option<toolbar::Button> {
    shows_toolbar(state)
        .then(|| {
            toolbar::button_at(
                client_size(state.hwnd).0,
                dpi_scale(state.hwnd),
                (point.0 as f32, point.1 as f32),
            )
        })
        .flatten()
}

/// Does what toolbar `button` does. Open in FastDraw is the main window's command, which reads the
/// diagram's text from its editor.
fn press_toolbar(hwnd: HWND, button: toolbar::Button) {
    use toolbar::Button;
    let zoom = match button {
        Button::Fit => Some(Zoom::Fit),
        Button::Fill => Some(Zoom::FillWidth),
        Button::Percent => Some(Zoom::Scale(1.0)),
        Button::ZoomIn | Button::ZoomOut => None,
        Button::OpenInFastDraw => {
            unsafe {
                PostMessageW(
                    crate::platform::win32::root_window(hwnd),
                    WM_COMMAND,
                    crate::window::commands::CommandId::OpenInFastDraw as usize,
                    0,
                )
            };
            return;
        }
    };
    let view = ImageView { hwnd };
    match (zoom, button) {
        (Some(zoom), _) => view.set_zoom(zoom),
        (None, Button::ZoomIn) => view.zoom_in(),
        (None, _) => view.zoom_out(),
    }
}

/// Follows the pointer over the toolbar: the hovered button, the leave tracking that clears it,
/// and the tooltip, made at the first hover and laid out again when the view's width changes.
fn track_toolbar(hwnd: HWND, point: (i32, i32), message: u32, wparam: WPARAM, lparam: LPARAM) {
    let Some((changed, track, tooltip)) = with_state(hwnd, |state| {
        let hovered = if state.press.is_some() {
            None
        } else {
            toolbar_button(state, point)
        };
        let changed = std::mem::replace(&mut state.bar.hovered, hovered) != hovered;
        let track = shows_toolbar(state) && !state.bar.tracking;
        state.bar.tracking |= track;
        let width = client_size(hwnd).0;
        let wanted = shows_toolbar(state)
            && !state.bar.tooltip_failed
            && (state.bar.tooltip.is_none() || state.bar.tooltip_width != width);
        (changed, track, wanted.then_some(width))
    }) else {
        return;
    };
    if track {
        let mut leave = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        unsafe { TrackMouseEvent(&mut leave) };
    }
    if let Some(width) = tooltip {
        // Made with no state borrowed: creating the control sends messages.
        let existing = with_state(hwnd, |state| state.bar.tooltip).flatten();
        let made = existing.or_else(|| crate::window::tooltip::Tooltip::create(hwnd));
        with_state(hwnd, |state| {
            state.bar.tooltip = made;
            state.bar.tooltip_failed = made.is_none();
            state.bar.tooltip_width = width;
        });
        if let Some(tip) = made {
            for (index, (button, rect)) in toolbar::layout(width, dpi_scale(hwnd))
                .into_iter()
                .enumerate()
            {
                let rect = windows_sys::Win32::Foundation::RECT {
                    left: rect.left as i32,
                    top: rect.top as i32,
                    right: rect.right as i32,
                    bottom: rect.bottom as i32,
                };
                tip.set_tool(index, rect, toolbar::tooltip(button));
            }
            if existing.is_none() {
                tip.relay(message, wparam, lparam);
            }
        }
    }
    if changed {
        invalidate(hwnd);
    }
}

/// The virtual keys of F (Fit) and W (Fill the width) on a diagram.
const KEY_FIT: u16 = b'F' as u16;
const KEY_FILL: u16 = b'W' as u16;

/// How near a connector a click counts as on it, in device-independent pixels (VS Code's 6).
const HIT_TOLERANCE_DIP: f32 = 6.0;

/// The shown diagram's elements, whatever text they were drawn from.
fn diagram_hits(state: &ViewState) -> Option<&crate::preview::fastdraw_hits::Hits> {
    if !matches!(state.shown, Some(Shown::FastDraw { .. })) {
        return None;
    }
    state
        .image
        .as_ref()?
        .diagram
        .as_ref()
        .map(|facts| &*facts.hits)
}

/// The outlines of the elements on the marked line, while the drawing is the current text's.
fn marked_shapes(
    state: &ViewState,
) -> impl Iterator<Item = &crate::preview::fastdraw_hits::HitShape> {
    let hits = (state.hits_fresh && !state.edited)
        .then(|| diagram_hits(state))
        .flatten();
    let line = state.marked_line.unwrap_or(0);
    hits.into_iter()
        .flat_map(move |hits| crate::preview::fastdraw_hits::on_line(hits, line))
}

/// The line under `client` (pixels), with connectors hit within `tolerance` pixels; `None` while
/// the picture is an older text's.
fn hit_line(state: &ViewState, client: (i32, i32), tolerance: f32) -> Option<u32> {
    if !state.hits_fresh || state.edited || state.scale <= 0.0 {
        return None;
    }
    let hits = diagram_hits(state)?;
    let point = zoom::to_scene(
        (client.0 as f32, client.1 as f32),
        state.offset,
        state.scale,
        hits.origin,
    );
    crate::preview::fastdraw_hits::hit_at(hits, point, f64::from(tolerance / state.scale))
}

fn clear_picture(state: &mut ViewState) {
    state.image = None;
    state.scene = None;
    state.bitmap = None;
    state.tile = None;
    state.tile_bitmap = None;
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

/// Whether the view shows source that is drawn at the zoom (an SVG or a FastDraw diagram).
fn shows_vector(state: &ViewState) -> bool {
    matches!(state.shown, Some(Shown::Svg(_) | Shown::FastDraw { .. }))
}

/// Decodes the shown SVG or diagram again at the size the zoom wants.
fn start_svg_decode(state: &mut ViewState) {
    let width = svg_width(state);
    let tile = tile_job(state);
    let source = match state.shown.clone() {
        Some(Shown::Svg(text)) => Source::Svg {
            text,
            width,
            tile,
            gpu: state.gpu,
        },
        Some(Shown::FastDraw { text, file }) => Source::FastDraw {
            text,
            file,
            dark: state.colors.is_dark(),
            width,
            tile,
            gpu: state.gpu,
        },
        _ => return,
    };
    start_decode(state, source);
}

fn stop_loading(state: &mut ViewState) {
    unsafe { KillTimer(state.hwnd, LOADING_TIMER) };
    state.loading = false;
    state.loading_visible = false;
}

fn drop_device_resources(state: &mut ViewState) {
    state.target = None;
    if let Some(scene) = &mut state.scene {
        scene.release_target();
    }
    state.bitmap = None;
    state.tile_bitmap = None;
    state.checker = None;
}

fn accept_decoded(state: &mut ViewState, decoded: Decoded) {
    if decoded.generation != state.generation {
        return;
    }
    stop_loading(state);
    state.decodes += 1;
    let svg = shows_vector(state);
    match decoded.result {
        Ok(image) => {
            let band = image.diagram.as_ref().and_then(|facts| facts.band.clone());
            // A reload whose pixel size changed starts at fit again (spec §6.5); an SVG re-render at
            // another zoom keeps its natural size, so its zoom stays.
            let resized = state.image.as_ref().is_some_and(|old| {
                (old.natural_width, old.natural_height)
                    != (image.natural_width, image.natural_height)
            });
            if resized && !svg {
                state.zoom = Zoom::Fit;
            }
            state.hits_fresh = image.diagram.is_some();
            state.image = Some(image);
            state.scene = None;
            state.bitmap = None;
            state.tile = decoded.tile;
            state.tile_bitmap = None;
            state.tree_id += 1;
            state.error = None;
            state.svg_error = band.is_some();
            state.band = band;
        }
        Err(error) if svg && state.image.is_some() => {
            state.svg_error = true;
            state.band = matches!(error, ImageError::Diagram(_)).then(|| error.message());
        }
        Err(error) => {
            state.error = Some(error);
            clear_picture(state);
            state.svg_error = false;
            state.band = None;
        }
    }
    recompute(state);
    // An SVG render asked for with the previous document's size (an edit changed its dimensions,
    // or the first render at natural size fits smaller) is drawn stretched: render it again at
    // the size this zoom wants, from the tree just parsed.
    if svg {
        refresh_vector(state);
    }
}

fn accept_rendered(state: &mut ViewState, rendered: Rendered) {
    state.rendering = false;
    if rendered.tree_id == state.tree_id {
        if let Some((width, height, pixels)) = rendered.whole
            && let Some(image) = &mut state.image
        {
            (image.width, image.height, image.pixels) = (width, height, pixels);
            state.bitmap = None;
        }
        if rendered.tile.is_some() {
            state.tile = rendered.tile;
            state.tile_bitmap = None;
        }
        // Back within the budget, the whole picture is sharp on its own.
        if tile_job(state).is_none() && !svg_needs_render(state) {
            state.tile = None;
            state.tile_bitmap = None;
        }
    }
    refresh_vector(state);
}

/// The device pixels one picture unit takes at 100%: a photo's 100% is its own pixels, while an
/// SVG's or a diagram's units are points, so it is the display scale, as a browser draws them.
fn unit(state: &ViewState) -> f32 {
    if shows_vector(state) {
        dpi_scale(state.hwnd)
    } else {
        1.0
    }
}

/// The zoom as people read it (1.0 = 100%): `scale` over `unit`.
fn zoom_level(state: &ViewState) -> f32 {
    state.scale / unit(state)
}

/// Applies the zoom to the current client size: sets `scale` and clamps `offset`.
fn recompute(state: &mut ViewState) {
    let Some(image) = &state.image else {
        return;
    };
    let natural = (image.natural_width as f32, image.natural_height as f32);
    let (width, height) = client_size(state.hwnd);
    state.scale = zoom::device_scale(state.zoom, natural, (width, height), unit(state));
    state.offset = (
        zoom::clamp_axis(state.offset.0, natural.0 * state.scale, width),
        zoom::clamp_axis(state.offset.1, natural.1 * state.scale, height),
    );
    schedule_tile(state);
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
    if shows_vector(state) && !drawn_by_gpu(state) {
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
    state.scale = zoom::device_scale(state.zoom, natural, client_size(state.hwnd), unit(state));
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
            format!(
                "Zoom {} percent",
                (zoom_level(state) * 100.0).round() as u32
            ),
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
    if scroll.is_some() {
        // Showing or hiding one bar resizes the client area, and the nested WM_SIZE publishes
        // ranges for the new size; the other bar must not then be set from the old one. So each
        // bar is set from ranges read just before it.
        for bar in [SB_HORZ, SB_VERT] {
            let Some(range) = with_state(hwnd, |state| {
                let ranges = scroll_ranges(state);
                state.last_scroll = Some(ranges);
                if bar == SB_HORZ { ranges.0 } else { ranges.1 }
            }) else {
                return;
            };
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
        unsafe {
            PostMessageW(
                crate::platform::win32::root_window(hwnd),
                WM_FASTPAD_IMAGE_STATUS,
                0,
                0,
            )
        };
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
                let state = unsafe { Box::from_raw(pointer) };
                // A child window's tooltip popup is not destroyed with it.
                if let Some(tooltip) = state.bar.tooltip {
                    tooltip.destroy();
                }
                drop(state);
            }
            // A decode that landed after the last message was read would leak its pixels.
            let mut queued = MSG::default();
            while unsafe {
                PeekMessageW(
                    &mut queued,
                    hwnd,
                    WM_FASTPAD_IMAGE_DECODED,
                    WM_FASTPAD_IMAGE_DECODED,
                    PM_REMOVE,
                )
            } != 0
            {
                // PeekMessage hands out WM_QUIT whatever the filter. When the view goes with a
                // closing window, the quit is already posted: put it back for the message loop,
                // or the process never exits.
                if queued.message == WM_QUIT {
                    unsafe { PostQuitMessage(queued.wParam as i32) };
                    break;
                }
                if queued.lParam != 0 {
                    drop(unsafe { Box::from_raw(queued.lParam as *mut Delivery) });
                }
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
                    Some(Shown::Svg(_) | Shown::FastDraw { .. }) => start_svg_decode(state),
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
                state.icon_format = None;
                recompute(state);
            });
            finish(hwnd);
            0
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            if message == WM_SETFOCUS {
                crate::window::post_content_focus(hwnd);
            }
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
                SVG_TIMER | TILE_TIMER => {
                    with_state(hwnd, |state| {
                        unsafe { KillTimer(hwnd, wparam) };
                        if wparam == TILE_TIMER {
                            state.tile_timer = false;
                        }
                        refresh_vector(state);
                    });
                }
                _ => {}
            }
            0
        }
        WM_FASTPAD_IMAGE_DECODED => {
            if lparam != 0 {
                let delivery = unsafe { Box::from_raw(lparam as *mut Delivery) };
                with_state(hwnd, |state| match *delivery {
                    Delivery::Decoded(decoded) => accept_decoded(state, decoded),
                    Delivery::Rendered(rendered) => accept_rendered(state, rendered),
                });
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
                        zoom::step_in(zoom_level(state))
                    } else {
                        zoom::step_out(zoom_level(state))
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
        // A click zooms in about the pointer and Ctrl+click zooms out (on a diagram, a click selects
        // the line of the element under it instead); a press that moves past the
        // drag distance pans a picture larger than the view instead. The second press of a
        // double-click is another click.
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            unsafe { SetFocus(hwnd) };
            let point = point_from(lparam);
            with_state(hwnd, |state| {
                // A press on the diagram toolbar is the button's alone: it starts no click or pan.
                if on_toolbar(state, point) {
                    state.bar.pressed = toolbar_button(state, point);
                    return;
                }
                // On a diagram the first press of a double-click already selected its line and laid
                // the tab out again: the second, landing on the new layout, is no click.
                let second = message == WM_LBUTTONDBLCLK && shows_toolbar(state);
                state.press = (!second).then_some(point);
                state.moved = false;
                state.drag = draggable(state).then_some((point, state.offset));
            });
            unsafe { SetCapture(hwnd) };
            invalidate(hwnd);
            0
        }
        WM_MOUSELEAVE => {
            with_state(hwnd, |state| {
                state.bar.tracking = false;
                state.bar.hovered = None;
            });
            invalidate(hwnd);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = point_from(lparam);
            track_toolbar(hwnd, (x, y), message, wparam, lparam);
            let slop = drag_slop();
            let (panned, started) = with_state(hwnd, |state| {
                let Some((press_x, press_y)) = state.press else {
                    return (false, false);
                };
                let started =
                    !state.moved && ((x - press_x).abs() > slop.0 || (y - press_y).abs() > slop.1);
                state.moved |= started;
                let Some(((start_x, start_y), start)) = state.drag.filter(|_| state.moved) else {
                    return (false, started);
                };
                state.offset = (
                    start.0 + (x - start_x) as f32,
                    start.1 + (y - start_y) as f32,
                );
                recompute(state);
                (true, started)
            })
            .unwrap_or((false, false));
            if started {
                set_pointer(hwnd);
            }
            if panned {
                finish(hwnd);
            }
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from(lparam);
            // A toolbar button acts when released over the button it was pressed on.
            let pressed =
                with_state(hwnd, |state| {
                    state.bar.pressed.take().map(|button| {
                        (toolbar_button(state, (x, y)) == Some(button)).then_some(button)
                    })
                })
                .flatten();
            if let Some(released) = pressed {
                unsafe { ReleaseCapture() };
                if let Some(button) = released {
                    press_toolbar(hwnd, button);
                }
                invalidate(hwnd);
                return 0;
            }
            let clicked = with_state(hwnd, |state| {
                let clicked = state.press.take().is_some() && !state.moved;
                state.drag = None;
                state.moved = false;
                clicked
            })
            .unwrap_or(false);
            unsafe { ReleaseCapture() };
            let diagram = with_state(hwnd, |state| {
                matches!(state.shown, Some(Shown::FastDraw { .. }))
            })
            .unwrap_or(false);
            if clicked && diagram {
                // A diagram click finds the line that writes the element under it; empty canvas
                // does nothing.
                if let Some(line) = (ImageView { hwnd }).hit_line((x, y)) {
                    unsafe {
                        PostMessageW(
                            crate::platform::win32::root_window(hwnd),
                            WM_FASTPAD_DIAGRAM_CLICKED,
                            hwnd as usize,
                            line as isize,
                        )
                    };
                }
            } else if clicked {
                let out = (wparam as u32) & MK_CONTROL != 0;
                with_state(hwnd, |state| {
                    let new = if out {
                        zoom::step_out(zoom_level(state))
                    } else {
                        zoom::step_in(zoom_level(state))
                    };
                    set_zoom(state, Zoom::Scale(new), (x as f32, y as f32));
                });
                finish(hwnd);
            }
            set_pointer(hwnd);
            0
        }
        WM_CAPTURECHANGED => {
            with_state(hwnd, |state| {
                state.drag = None;
                state.press = None;
                state.moved = false;
                state.bar.pressed = None;
            });
            0
        }
        WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
            unsafe { SetCursor(pointer(hwnd)) };
            1
        }
        // Ctrl turns the magnifier's plus into a minus while the pointer is over the picture.
        WM_KEYDOWN | WM_KEYUP if wparam as u16 == VK_CONTROL => {
            if pointer_over(hwnd) {
                set_pointer(hwnd);
            }
            if message == WM_KEYDOWN {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            } else {
                0
            }
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
                    // A diagram's toolbar keys: F fits the whole diagram, W fills the width.
                    KEY_FIT | KEY_FILL
                        if matches!(state.shown, Some(Shown::FastDraw { .. }))
                            && unsafe { GetKeyState(i32::from(VK_CONTROL)) } >= 0 =>
                    {
                        let (width, _) = client_size(hwnd);
                        let zoom = if key == KEY_FIT {
                            Zoom::Fit
                        } else {
                            Zoom::FillWidth
                        };
                        set_zoom(state, zoom, (width / 2.0, height / 2.0));
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

/// The pointer over the view: the move arrows while a press pans, the magnifier over a picture
/// (a minus while Ctrl is down), else the arrow.
fn pointer(hwnd: HWND) -> HCURSOR {
    let (panning, ready, diagram) = with_state(hwnd, |state| {
        (
            state.moved && state.drag.is_some(),
            phase(state) == Phase::Ready,
            shows_toolbar(state),
        )
    })
    .unwrap_or((false, false, false));
    let stock = |id| unsafe { LoadCursorW(std::ptr::null_mut(), id) };
    if panning {
        return stock(IDC_SIZEALL);
    }
    if !ready {
        return stock(IDC_ARROW);
    }
    // A diagram's click finds an element's line instead of zooming: a hand over an element, an
    // arrow elsewhere and over the toolbar.
    if diagram {
        let mut point = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut point);
            ScreenToClient(hwnd, &mut point);
        }
        let over = with_state(hwnd, |state| on_toolbar(state, (point.x, point.y))).unwrap_or(true);
        let element = !over && (ImageView { hwnd }).hit_line((point.x, point.y)).is_some();
        return stock(if element { IDC_HAND } else { IDC_ARROW });
    }
    let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    cursor::magnifier(dpi, !ctrl).unwrap_or_else(|| stock(IDC_ARROW))
}

fn set_pointer(hwnd: HWND) {
    unsafe { SetCursor(pointer(hwnd)) };
}

/// Whether the mouse pointer is over the view itself.
fn pointer_over(hwnd: HWND) -> bool {
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) }) != 0 && unsafe { WindowFromPoint(point) } == hwnd
}

/// How far a press moves before it pans instead of clicking.
fn drag_slop() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) }
}

/// Whether `hwnd` holds the keyboard focus, for the focus ring.
fn focused(hwnd: HWND) -> bool {
    (unsafe { GetFocus() }) == hwnd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::theme::Theme;
    use crate::preview::colors::preview_colors;
    use crate::preview::render::TestWindow;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, TranslateMessage};

    #[test]
    fn destroying_the_view_keeps_a_posted_quit_for_the_message_loop() {
        // Break caught: the drain of late decode results taking WM_QUIT too, which PeekMessage
        // hands out whatever the filter, so a window closed with an SVG preview never exits.
        let parent = crate::preview::render::TestWindow::new(200, 200);
        let view = super::create_window(parent.0).unwrap();
        unsafe {
            PostQuitMessage(7);
            DestroyWindow(view);
        }
        let mut message = MSG::default();
        let found = unsafe {
            PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                WM_QUIT,
                WM_QUIT,
                PM_REMOVE,
            )
        };
        assert_ne!(found, 0, "the quit message was swallowed");
        assert_eq!(message.wParam, 7);
    }

    const GOOD: &str = "a: rect \"A\" at 0,0
";
    const REFUSED: &str = "look \"//server/x.look\"
a: rect \"A\" at 0,0
";
    const BROKEN: &str = "a: nosuchkind \"A\" at 0,0
";

    fn view_in(parent: &TestWindow) -> ImageView {
        let graphics = Rc::new(Graphics::load().unwrap());
        let view = ImageView::create(
            parent.0,
            graphics,
            preview_colors(Theme::Light, false),
            false,
        )
        .unwrap();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MoveWindow(view.hwnd, 0, 0, 400, 300, 0);
        }
        view
    }

    fn pump_until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut msg = MSG::default();
            while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            if done() {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn settle(view: &ImageView, decodes: u32) {
        pump_until("a decode", || {
            let stats = view.stats();
            stats.decodes >= decodes && stats.phase != Phase::Loading
        });
    }

    /// The part of the picture the view shows, in zoomed pixels.
    fn visible(view: &ImageView) -> decode::TileArea {
        view.with(|state| {
            decode::tile_area(
                shown_size(state).unwrap(),
                state.offset,
                client_size(state.hwnd),
                0.0,
                state.max_side,
            )
            .unwrap()
        })
        .unwrap()
    }

    #[test]
    fn a_picture_direct2d_draws_zooms_without_rendering_anything_again() {
        // Break caught: a zoom step waiting on a raster render (a blurred picture sharpening a
        // moment later) for an SVG Direct2D can draw at any zoom.
        let parent = TestWindow::new(400, 300);
        let view = view_in(&parent);
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1044" height="752">
            <rect x="10" y="10" width="1024" height="732" fill="#48c" stroke="#000"/></svg>"##;
        view.show_svg(svg.into(), "big.svg", true);
        settle(&view, 1);
        assert!(view.stats().vector);
        assert_eq!(view.stats().whole, Some((0, 0)), "no pixels rendered");
        while view.stats().scale < 16.0 {
            view.zoom_in();
        }
        // A hidden test window gets no WM_PAINT of its own: paint it, as showing it would.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(view.hwnd, WM_PAINT, 0, 0)
        };
        std::thread::sleep(Duration::from_millis(200));
        pump_until("any render the zoom asked for", || true);
        let stats = view.stats();
        assert_eq!((stats.decodes, stats.tile), (1, None));
        assert!(
            view.with(|state| !state.rendering && state.scene.is_some())
                .unwrap()
        );
    }

    #[test]
    fn a_deep_zoom_renders_the_view_sharp_from_a_whole_picture_within_the_budget() {
        // Break caught: 800% rasterizing the whole 1044×752 picture (50 MP, 0.7 s a step in a
        // release build), or panning there leaving the uncovered part blurred.
        let parent = TestWindow::new(400, 300);
        let view = view_in(&parent);
        // The filter keeps it on the raster path Direct2D leaves such pictures to.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1044" height="752">
            <filter id="f"><feComponentTransfer/></filter>
            <rect x="10" y="10" width="1024" height="732" fill="#48c" stroke="#000" filter="url(#f)"/></svg>"##;
        view.show_svg(svg.into(), "big.svg", true);
        settle(&view, 1);
        while view.stats().scale < 8.0 {
            view.zoom_in();
        }
        pump_until("a sharp tile at 800%", || {
            view.stats().tile.is_some_and(|(scale, _)| scale == 8.0)
        });
        let stats = view.stats();
        let (width, height) = stats.whole.unwrap();
        assert!(u64::from(width) * u64::from(height) <= decode::WHOLE_PIXEL_BUDGET);
        assert!(width > 1044, "the whole picture renders past 100%: {width}");
        let (_, area) = stats.tile.unwrap();
        assert!(area.contains(&visible(&view)), "{area:?}");
        assert!(u64::from(area.width) * u64::from(area.height) < 2_000_000);
        // Panning past the tile's margin renders the newly shown part sharp.
        for _ in 0..30 {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                    view.hwnd,
                    WM_KEYDOWN,
                    VK_RIGHT as usize,
                    0,
                )
            };
        }
        pump_until("a tile after the pan", || {
            view.stats()
                .tile
                .is_some_and(|(_, area)| area.contains(&visible(&view)))
        });
        assert_eq!(
            view.stats().whole,
            Some((width, height)),
            "a pan re-rendered the whole"
        );
        assert_eq!(view.stats().decodes, 1, "zoom and pan parsed the SVG again");
        // Back at 100%, the whole picture is sharp on its own.
        view.zoom_reset();
        while view.stats().scale > 1.0 {
            view.zoom_out();
        }
        while view.stats().scale < 1.0 {
            view.zoom_in();
        }
        pump_until("the whole picture at 100%", || {
            let stats = view.stats();
            stats.whole == Some((1044, 752)) && stats.tile.is_none()
        });
    }

    #[test]
    fn a_diagram_with_a_refused_look_shows_with_a_band() {
        // Break caught: the refusal blanking the drawing, or going unreported.
        let parent = TestWindow::new(400, 300);
        let view = view_in(&parent);
        view.show_fastdraw(REFUSED.into(), None, "d.fdraw", true);
        settle(&view, 1);
        let stats = view.stats();
        assert_eq!(stats.phase, Phase::Ready);
        assert!(stats.svg_error);
        assert!(
            stats
                .band
                .unwrap()
                .starts_with("Line 1: FastPad reads look files")
        );
        view.show_fastdraw(GOOD.into(), None, "d.fdraw", false);
        settle(&view, 2);
        let stats = view.stats();
        assert!(!stats.svg_error && stats.band.is_none());
    }

    #[test]
    fn a_broken_edit_keeps_the_last_drawing_under_a_band() {
        // Break caught: typing through a half-written line blanking the view.
        let parent = TestWindow::new(400, 300);
        let view = view_in(&parent);
        view.show_fastdraw(GOOD.into(), None, "d.fdraw", true);
        settle(&view, 1);
        let size = view.status().size;
        assert!(size.is_some());
        view.show_fastdraw(BROKEN.into(), None, "d.fdraw", false);
        settle(&view, 2);
        let stats = view.stats();
        assert_eq!(stats.phase, Phase::Ready);
        assert!(stats.band.unwrap().starts_with("Line 1: "));
        assert_eq!(view.status().size, size);
    }

    #[test]
    fn a_broken_first_diagram_shows_the_failed_state() {
        // Break caught: a new document with an error showing another document's picture or nothing.
        let parent = TestWindow::new(400, 300);
        let view = view_in(&parent);
        view.show_fastdraw(GOOD.into(), None, "d.fdraw", true);
        settle(&view, 1);
        view.show_fastdraw(BROKEN.into(), None, "e.fdraw", true);
        settle(&view, 2);
        let stats = view.stats();
        assert_eq!(stats.phase, Phase::Failed);
        assert!(!stats.svg_error && stats.band.is_none());
    }
}
