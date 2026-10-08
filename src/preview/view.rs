//! The `FastPadPreview` child window. It owns the preview model, lays out only the blocks that
//! scroll into view, and paints them with Direct2D. User actions are posted to the parent (never
//! sent), so the parent can call back into the view without re-entering a borrowed state.

use crate::Result;
use crate::platform::{last_error, wide_null};
use crate::preview::colors::{ColorRole, PreviewColors};
use crate::preview::dwrite::Graphics;
use crate::preview::heights::{HeightIndex, estimate_height, line_for_offset, offset_for_line};
use crate::preview::images::ImageCache;
use crate::preview::incremental::{Edit, PreviewDocument, SourceText, Update};
use crate::preview::layout::{
    LaidBlock, LayoutContext, PreviewFonts, Target, TargetKind, block_has_target, layout_block,
};
use crate::preview::outline::{self, DetailsKey, Outline, details_open_attribute, has_details};
use crate::preview::render::{Brushes, color_f, create_hwnd_target, draw_ops};
use crate::window::{
    WM_FASTPAD_PREVIEW_ACTIVATE, WM_FASTPAD_PREVIEW_ESCAPE, WM_FASTPAD_PREVIEW_HOVER,
    WM_FASTPAD_PREVIEW_IMAGE, WM_FASTPAD_PREVIEW_LINK, WM_FASTPAD_PREVIEW_REFRESH,
    WM_FASTPAD_PREVIEW_SCROLLED,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows::Win32::Graphics::Direct2D::ID2D1HwndRenderTarget;
use windows::Win32::Graphics::DirectWrite::{DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL};
use windows_numerics::{Matrix3x2, Vector2};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, ValidateRect};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{SetScrollInfo, SetWindowTheme, WM_MOUSELEAVE};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END, VK_ESCAPE,
    VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DLGC_WANTALLKEYS, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetScrollInfo, GetWindowLongPtrW, HTCLIENT, IDC_ARROW, IDC_HAND, LoadCursorW, PostMessageW,
    RegisterClassW, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBTRACK,
    SB_TOP, SB_VERT, SCROLLINFO, SIF_ALL, SIF_TRACKPOS, SetCursor, SetWindowLongPtrW,
    WM_DPICHANGED_AFTERPARENT, WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY,
    WM_PAINT, WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WM_VSCROLL, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_TABSTOP, WS_VSCROLL,
};

mod painting;
use painting::*;
mod targets;
use targets::*;
mod window_proc;
use window_proc::*;
const CLASS_NAME: &str = "FastPadPreview";
/// `WM_FASTPAD_PREVIEW_ACTIVATE` `wparam` for a disclosure: `lparam` is a `Box<DetailsKey>`. Links use
/// `wparam` 0 with a `Box<String>`.
pub const ACTIVATE_DISCLOSURE: usize = 1;
/// `MK_SHIFT` from WinUser.h; its windows-sys home needs a feature FastPad does not enable.
const MK_SHIFT: u32 = 0x0004;
const PADDING: f32 = 16.0;
const MAX_CENTERED_WIDTH: f32 = 980.0;
const PAUSED_BAR_HEIGHT: f32 = 32.0;
const PAUSED_TEXT: &str =
    "Live preview is paused for this large file. Click here to refresh the preview.";

#[derive(Clone, Copy, Debug, Default)]
pub struct PreviewStats {
    pub block_count: usize,
    pub revision: u64,
    pub first_frame_micros: u64,
    pub last_update_micros: u64,
    /// Updates whose frame has been painted; `last_update_micros` belongs to the latest one.
    pub painted_updates: u64,
    /// The document's laid-out height in DIPs as of the last paint.
    pub content_height: f32,
}

/// A disclosure's identity and state, as a snapshot for accessibility clients.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disclosure {
    pub key: DetailsKey,
    pub expanded: bool,
}

/// A link or disclosure on screen. `Debug`, `PartialEq`, and `Eq` are written by hand: windows-sys
/// `RECT` derives none of them.
#[derive(Clone)]
pub struct VisibleLink {
    pub text: String,
    /// Empty for a disclosure.
    pub dest: String,
    /// Client pixels of the target's first line.
    pub rect: RECT,
    pub disclosure: Option<Disclosure>,
    /// Whether keyboard focus is on this target.
    pub focused: bool,
}

impl std::fmt::Debug for VisibleLink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RECT {
            left,
            top,
            right,
            bottom,
        } = self.rect;
        formatter
            .debug_struct("VisibleLink")
            .field("text", &self.text)
            .field("dest", &self.dest)
            .field("rect", &(left, top, right, bottom))
            .field("disclosure", &self.disclosure)
            .field("focused", &self.focused)
            .finish()
    }
}

impl PartialEq for VisibleLink {
    fn eq(&self, other: &Self) -> bool {
        let rect = |rect: &RECT| (rect.left, rect.top, rect.right, rect.bottom);
        self.text == other.text
            && self.dest == other.dest
            && rect(&self.rect) == rect(&other.rect)
            && self.disclosure == other.disclosure
            && self.focused == other.focused
    }
}

impl Eq for VisibleLink {}

pub fn content_frame(view_width: f32, centered: bool) -> (f32, f32) {
    let available = (view_width - 2.0 * PADDING).max(1.0);
    if centered && available > MAX_CENTERED_WIDTH {
        (
            PADDING + (available - MAX_CENTERED_WIDTH) / 2.0,
            MAX_CENTERED_WIDTH,
        )
    } else {
        (PADDING, available)
    }
}

struct ViewState {
    hwnd: HWND,
    target: Option<ID2D1HwndRenderTarget>,
    brushes: Option<Brushes>,
    colors: PreviewColors,
    fonts: PreviewFonts,
    document: PreviewDocument,
    document_dir: Option<PathBuf>,
    layouts: Vec<Option<LaidBlock>>,
    heights: HeightIndex,
    layout_width: f32,
    scroll_y: f32,
    h_scroll: HashMap<usize, f32>,
    hover: Option<(usize, usize)>,
    pressed: Option<(usize, usize)>,
    focus: Option<(usize, usize)>,
    tracking_mouse: bool,
    images: ImageCache,
    centered: bool,
    paused: bool,
    live_resize: bool,
    /// Set after a failed paint schedules its one retry; cleared by the next successful paint.
    paint_retried: bool,
    /// Section keys and heading anchors; `None` until something needs them after the document
    /// changed.
    outline: Option<Outline>,
    /// `<details>` sections the user toggled away from their `open` attribute.
    details_overrides: HashMap<DetailsKey, bool>,
    /// Per block, whether it holds a `<details>` section. Kept in step with the document so an
    /// edit can tell whether section keys after it changed without walking every block.
    sectioned: Vec<bool>,
    /// A section toggled since the last paint; its accessible child raises a state change once the
    /// snapshot shows the new state.
    state_change: Option<DetailsKey>,
    stats: PreviewStats,
    opened_at: Option<Instant>,
    update_started: Option<Instant>,
    /// On-screen links as of the last paint, shared with the accessibility provider.
    accessible: Arc<RwLock<Vec<VisibleLink>>>,
    /// Declared last so it drops last: the target, brushes, text layouts, and bitmaps above all
    /// live in d2d1.dll and dwrite.dll, which the final `Graphics` reference unloads.
    graphics: Rc<Graphics>,
}

/// A handle to a preview window; copies are cheap and all refer to the same window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewView {
    hwnd: HWND,
}

impl PreviewView {
    pub fn create(
        parent: HWND,
        graphics: Rc<Graphics>,
        colors: PreviewColors,
        fonts: PreviewFonts,
    ) -> Result<Self> {
        let hwnd = create_window(parent)?;
        let state = Box::new(ViewState {
            hwnd,
            graphics,
            target: None,
            brushes: None,
            colors,
            fonts,
            document: PreviewDocument::default(),
            document_dir: None,
            layouts: Vec::new(),
            heights: HeightIndex::default(),
            layout_width: 0.0,
            scroll_y: 0.0,
            h_scroll: HashMap::new(),
            hover: None,
            pressed: None,
            focus: None,
            tracking_mouse: false,
            images: ImageCache::new(hwnd, WM_FASTPAD_PREVIEW_IMAGE),
            centered: false,
            paused: false,
            live_resize: false,
            paint_retried: false,
            outline: None,
            details_overrides: HashMap::new(),
            sectioned: Vec::new(),
            state_change: None,
            stats: PreviewStats::default(),
            opened_at: None,
            update_started: None,
            accessible: Arc::new(RwLock::new(Vec::new())),
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

    pub fn replace_document(
        &self,
        document: PreviewDocument,
        document_dir: Option<PathBuf>,
        started: Instant,
    ) {
        self.with(|state| state.details_overrides.clear());
        self.install_parse(document, document_dir, started);
    }

    /// Installs a worker parse of the document already shown. Unlike `replace_document`, the
    /// sections the user expanded or collapsed stay that way.
    pub fn install_parse(
        &self,
        document: PreviewDocument,
        document_dir: Option<PathBuf>,
        started: Instant,
    ) {
        self.with(|state| {
            state.document = document;
            state.document_dir = document_dir;
            state.scroll_y = 0.0;
            // The snapshot describes the old document until the next paint; accessibility clients
            // must not see its links in the meantime.
            state
                .accessible
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            accept_update(state, Update::Full, started);
        });
    }

    pub fn apply_edits<S: SourceText + ?Sized>(
        &self,
        source: &S,
        edits: &[Edit],
        started: Instant,
        allow_full_parse: bool,
    ) -> Option<Update> {
        self.with(|state| {
            let update = if allow_full_parse {
                Some(state.document.apply(source, edits))
            } else {
                state.document.try_apply(source, edits)
            }?;
            accept_update(state, update.clone(), started);
            Some(update)
        })
        .flatten()
    }

    pub fn reparse<S: SourceText + ?Sized>(&self, source: &S, started: Instant) -> Update {
        self.with(|state| {
            let update = state.document.reparse(source);
            accept_update(state, update.clone(), started);
            update
        })
        .unwrap_or(Update::Unchanged)
    }

    pub fn set_appearance(&self, colors: PreviewColors, fonts: PreviewFonts, dark_scrollbar: bool) {
        self.with(|state| {
            state.colors = colors;
            state.fonts = fonts;
            state.brushes = None;
            relayout(state);
        });
        let theme = wide_null(if dark_scrollbar {
            "DarkMode_Explorer"
        } else {
            "Explorer"
        });
        unsafe { SetWindowTheme(self.hwnd, theme.as_ptr(), std::ptr::null()) };
        invalidate(self.hwnd);
    }

    pub fn set_document_dir(&self, document_dir: Option<PathBuf>) {
        self.with(|state| {
            if state.document_dir != document_dir {
                state.document_dir = document_dir;
                relayout(state);
            }
        });
        invalidate(self.hwnd);
    }

    pub fn set_centered(&self, centered: bool) {
        self.with(|state| state.centered = centered);
        invalidate(self.hwnd);
    }

    pub fn set_paused(&self, paused: bool) {
        self.with(|state| state.paused = paused);
        invalidate(self.hwnd);
    }

    pub fn is_paused(&self) -> bool {
        self.with(|state| state.paused).unwrap_or(false)
    }

    pub fn colors(&self) -> PreviewColors {
        self.with(|state| state.colors).unwrap_or_else(|| {
            crate::preview::colors::preview_colors(crate::platform::theme::Theme::Light, false)
        })
    }

    pub fn set_live_resize(&self, live: bool) {
        self.with(|state| state.live_resize = live);
        invalidate(self.hwnd);
    }

    pub fn mark_opened(&self, at: Instant) {
        self.with(|state| state.opened_at = Some(at));
    }

    pub fn scroll_to_line(&self, line: usize) {
        self.with(|state| {
            let ViewState {
                document, heights, ..
            } = state;
            let offset = offset_for_line(&document.blocks, heights, line);
            set_scroll(state, offset, false);
        });
    }

    pub fn top_line(&self) -> usize {
        self.with(|state| {
            let ViewState {
                document,
                heights,
                scroll_y,
                ..
            } = state;
            line_for_offset(&document.blocks, heights, *scroll_y)
        })
        .unwrap_or(0)
    }

    pub fn scroll_to_anchor(&self, anchor: &str) -> bool {
        self.with(|state| {
            let Some(found) = outline(state)
                .anchors
                .iter()
                .find(|entry| entry.slug == anchor)
                .cloned()
            else {
                return false;
            };
            // Chrome and Edge open every section around the target before scrolling to it.
            let keys = outline(state).details[found.block].clone();
            let mut opened = false;
            for &index in &found.enclosing {
                let default =
                    details_open_attribute(&state.document.blocks[found.block].kind, index)
                        .unwrap_or(false);
                if !state
                    .details_overrides
                    .get(&keys[index])
                    .copied()
                    .unwrap_or(default)
                {
                    set_details_open(state, &keys[index], default, true);
                    state.state_change = Some(keys[index].clone());
                    opened = true;
                }
            }
            if opened {
                state.layouts[found.block] = None;
            }
            let reading = state.heights.anchor(state.scroll_y);
            if matches!(ensure_layouts(state, &[found.block]), Ok(true)) {
                state.scroll_y = state.heights.scroll_for_anchor(reading);
            }
            let within = state.layouts[found.block]
                .as_ref()
                .and_then(|laid| {
                    laid.headings
                        .iter()
                        .find(|(heading, _)| *heading == found.heading)
                })
                .map_or(0.0, |(_, y)| *y);
            let top = state.heights.top(found.block) + within;
            set_scroll(state, top, true);
            true
        })
        .unwrap_or(false)
    }

    /// Frees what a hidden preview does not need: the block model, layouts, image cache, and the
    /// render target. Showing the preview again loads the document afresh.
    pub fn release(&self) {
        self.with(|state| {
            state.document = PreviewDocument::default();
            state.document_dir = None;
            state.layouts = Vec::new();
            state.heights = HeightIndex::default();
            state.scroll_y = 0.0;
            state.h_scroll = HashMap::new();
            state.hover = None;
            state.pressed = None;
            state.focus = None;
            state.outline = None;
            state.details_overrides = HashMap::new();
            state.sectioned = Vec::new();
            state.state_change = None;
            state.update_started = None;
            state.stats.block_count = 0;
            state.images.clear();
            drop_device_resources(state);
            *state
                .accessible
                .write()
                .unwrap_or_else(|error| error.into_inner()) = Vec::new();
        });
    }

    pub fn stats(&self) -> PreviewStats {
        self.with(|state| state.stats).unwrap_or_default()
    }

    pub fn visible_links(&self) -> Vec<VisibleLink> {
        self.with(visible_links).unwrap_or_default()
    }

    /// Each laid-out image with a local path, and whether its pixels are decoded.
    #[cfg(test)]
    pub fn image_states(&self) -> Vec<(PathBuf, bool)> {
        self.with(|state| {
            state
                .layouts
                .iter()
                .flatten()
                .flat_map(|laid| &laid.images)
                .filter_map(|slot| {
                    let path = slot.path.clone()?;
                    let ready = state.images.size(&path).is_some();
                    Some((path, ready))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    pub fn accessible_links(&self) -> Arc<RwLock<Vec<VisibleLink>>> {
        self.with(|state| Arc::clone(&state.accessible))
            .unwrap_or_default()
    }
}

/// Kept out of `PreviewView::create` so the public function never hands a raw `HWND` straight to an
/// unsafe call (`clippy::not_unsafe_ptr_arg_deref`); Windows validates the handle itself.
fn create_window(parent: HWND) -> Result<HWND> {
    register_class()?;
    let class = wide_null(CLASS_NAME);
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP | WS_VSCROLL,
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
        lpfnWndProc: Some(preview_proc),
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

fn view_size(hwnd: HWND) -> (f32, f32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) };
    let scale = dpi_scale(hwnd);
    (
        (rect.right - rect.left) as f32 / scale,
        (rect.bottom - rect.top) as f32 / scale,
    )
}

fn bar_height(state: &ViewState) -> f32 {
    if state.paused { PAUSED_BAR_HEIGHT } else { 0.0 }
}

/// Estimated heights for `range` of the document's blocks.
fn estimates(state: &ViewState, range: std::ops::Range<usize>) -> Vec<f32> {
    let line_height = state.fonts.line_pitch();
    let gap = 16.0 * state.fonts.unit();
    state.document.blocks[range]
        .iter()
        .map(|block| estimate_height(block.lines.len(), line_height, gap))
        .collect()
}

fn outline(state: &mut ViewState) -> &Outline {
    state
        .outline
        .get_or_insert_with(|| outline::build(&state.document.blocks))
}

/// Forgets every layout for a new document: heights fall back to estimates.
fn reset_layouts(state: &mut ViewState) {
    state.layouts = (0..state.document.blocks.len()).map(|_| None).collect();
    let estimates = estimates(state, 0..state.document.blocks.len());
    state.heights.reset(estimates);
    state.hover = None;
    state.pressed = None;
    state.focus = None;
}

/// Forgets every layout of the current document (width, fonts, brushes, or folder changed) while
/// keeping the reading position: the block at the top stays at the top.
fn relayout(state: &mut ViewState) {
    let anchor = (!state.heights.is_empty()).then(|| state.heights.anchor(state.scroll_y));
    reset_layouts(state);
    if let Some(anchor) = anchor
        && anchor.0 < state.heights.len()
    {
        state.scroll_y = state.heights.scroll_for_anchor(anchor);
    }
}

/// Everything tied to the Direct2D device. Layouts hold brushes as drawing effects, so the next
/// paint lays out again once brushes are recreated; decoded pixels survive for new bitmaps.
fn drop_device_resources(state: &mut ViewState) {
    state.target = None;
    state.brushes = None;
    state.images.release_bitmaps();
}

fn accept_update(state: &mut ViewState, update: Update, started: Instant) {
    let repaint = match update {
        Update::Unchanged => return,
        Update::Full => {
            state.sectioned = state
                .document
                .blocks
                .iter()
                .map(|block| has_details(&block.kind))
                .collect();
            reset_layouts(state);
            state.h_scroll.clear();
            true
        }
        Update::Replaced { old, new } => {
            let (_, view_height) = view_size(state.hwnd);
            let view_top = state.scroll_y;
            let view_bottom = view_top + view_height;
            // Only the replaced range is visited: running sums past it stay stale until paint
            // needs them, so an edit costs its own size, not the document's.
            let old_top = state.heights.top(old.start);
            let old_height = old
                .clone()
                .map(|index| state.heights.height(index))
                .sum::<f32>();
            let old_bottom = old_top + old_height;
            state.layouts.splice(old.clone(), new.clone().map(|_| None));
            let new_sectioned = state.document.blocks[new.clone()]
                .iter()
                .map(|block| has_details(&block.kind))
                .collect::<Vec<_>>();
            let keys_shift = new_sectioned.contains(&true)
                || state
                    .sectioned
                    .get(old.clone())
                    .is_some_and(|old| old.contains(&true));
            state.sectioned.splice(old.clone(), new_sectioned);
            if keys_shift {
                // Section keys count repeated summaries in document order, so a section added,
                // removed, or renamed here renumbers the sections after it. Their kept layouts
                // hold the old keys and would toggle the wrong section.
                for index in new.end..state.layouts.len() {
                    if state.sectioned.get(index).copied().unwrap_or(false) {
                        state.layouts[index] = None;
                    }
                }
            }
            let new_estimates = estimates(state, new.clone());
            let new_height = new_estimates.iter().sum::<f32>();
            state.heights.splice(old.clone(), &new_estimates);
            let delta = new.len() as isize - old.len() as isize;
            state.h_scroll = state
                .h_scroll
                .drain()
                .filter_map(|(index, offset)| {
                    if index < old.start {
                        Some((index, offset))
                    } else if index >= old.end {
                        Some(((index as isize + delta) as usize, offset))
                    } else {
                        None
                    }
                })
                .collect();
            state.hover = None;
            state.pressed = None;
            state.focus = None;
            let on_screen = old_top < view_bottom && old_bottom >= view_top;
            let total_changed = (new_height - old_height).abs() > 0.01;
            on_screen || old.len() != new.len() || total_changed
        }
    };
    // Rebuilt on the next lookup: walking every block on every keystroke's update is O(document).
    state.outline = None;
    state.stats.block_count = state.document.blocks.len();
    state.stats.revision = state.document.revision;
    if repaint {
        state.update_started = Some(started);
        invalidate(state.hwnd);
    }
}

/// Lays out `indices` that have no layout yet; true when any height changed.
fn ensure_layouts(state: &mut ViewState, indices: &[usize]) -> Result<bool> {
    // Section keys need a walk of the whole document, so only blocks holding a section pay for it.
    let sectioned = indices
        .iter()
        .copied()
        .filter(|&index| {
            state.layouts.get(index).is_some_and(Option::is_none)
                && state.sectioned.get(index).copied().unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let keys = sectioned
        .into_iter()
        .map(|index| (index, outline(state).details[index].clone()))
        .collect::<HashMap<_, _>>();
    let dark = state.colors.is_dark();
    let ViewState {
        hwnd,
        graphics,
        brushes,
        fonts,
        document,
        document_dir,
        images,
        layouts,
        heights,
        layout_width,
        details_overrides,
        ..
    } = state;
    let Some(brushes) = brushes.as_ref() else {
        return Ok(false);
    };
    let mut requests = Vec::new();
    let mut changed = false;
    {
        let sizes = |path: &Path| images.size(path);
        let context = LayoutContext::new(
            graphics,
            brushes,
            fonts,
            document_dir.as_deref(),
            &sizes,
            dark,
            details_overrides,
        )?;
        for &index in indices {
            if index >= document.blocks.len() || layouts[index].is_some() {
                continue;
            }
            let block_keys = keys.get(&index).map_or(&[][..], Vec::as_slice);
            let laid = layout_block(
                &context,
                &document.blocks[index].kind,
                *layout_width,
                block_keys,
            )?;
            for slot in &laid.images {
                if let Some(path) = &slot.path
                    && !images.is_failed(path)
                {
                    requests.push(path.clone());
                }
            }
            heights.set_measured(index, laid.height);
            layouts[index] = Some(laid);
            changed = true;
        }
    }
    // Decode at the content width in device pixels. The cache caps that at the natural width and
    // decodes again only when a wider pane needs more pixels than the last decode has.
    let pixel_width = (*layout_width * dpi_scale(*hwnd)).ceil().max(1.0) as u32;
    for path in requests {
        images.request(&path, pixel_width);
    }
    Ok(changed)
}

fn visible_indices(state: &mut ViewState, view_height: f32) -> Vec<usize> {
    let bottom = state.scroll_y + view_height;
    let mut index = state.heights.index_at(state.scroll_y);
    let mut indices = Vec::new();
    while index < state.document.blocks.len() && state.heights.top(index) < bottom {
        indices.push(index);
        index += 1;
    }
    indices
}

fn max_scroll(state: &mut ViewState, view_height: f32) -> f32 {
    (state.heights.total() - (view_height - bar_height(state))).max(0.0)
}

fn set_scroll(state: &mut ViewState, y: f32, user: bool) {
    let (_, view_height) = view_size(state.hwnd);
    let clamped = y.clamp(0.0, max_scroll(state, view_height));
    if (clamped - state.scroll_y).abs() < 0.01 {
        return;
    }
    state.scroll_y = clamped;
    invalidate(state.hwnd);
    if user {
        let ViewState {
            document,
            heights,
            scroll_y,
            hwnd,
            ..
        } = state;
        let line = line_for_offset(&document.blocks, heights, *scroll_y);
        unsafe {
            PostMessageW(
                crate::platform::win32::root_window(*hwnd),
                WM_FASTPAD_PREVIEW_SCROLLED,
                line,
                *hwnd as isize,
            )
        };
    }
}

#[cfg(test)]
mod tests;
