//! The `FastPadLive` child window: the Live view of one editor group's Markdown tab. The group's
//! Scintilla stays the document (text, selection, undo); this window draws it with Direct2D and,
//! turns keys and clicks into Scintilla input and selections (phase 1 plan,
//! Architecture). Like the preview, nothing here sends messages while the state is borrowed.

use crate::Result;
use crate::editor::Editor;
use crate::live::layout::{LiveFonts, LiveLayout};
use crate::live::scroll;
use crate::live::text::{EditorText, LiveText};
use crate::platform::{last_error, wide_null};
use crate::preview::dwrite::Graphics;
use std::rc::Rc;
use windows::Win32::Graphics::Direct2D::{ID2D1HwndRenderTarget, ID2D1SolidColorBrush};
use windows_sys::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, IDC_IBEAM,
    LoadCursorW, RegisterClassW, SetWindowLongPtrW, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
    WS_TABSTOP, WS_VSCROLL,
};

mod ime;
mod keys;
mod mouse;
mod paint;
mod scrolling;
mod window_proc;

pub const CLASS_NAME: &str = "FastPadLive";
/// Space between the window's edges and the text, in DIPs.
const PADDING: f32 = 16.0;
/// Rows kept visible above and below the caret when scrolling to it, in DIPs.
const CARET_MARGIN: f32 = 8.0;
/// Space between the line numbers and the text, in DIPs.
const GUTTER_GAP: f32 = 12.0;

/// The colours the view draws with, as `COLORREF`s: the editor's own, so Live and source look
/// like one editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveColors {
    pub background: u32,
    pub text: u32,
    pub selection: u32,
    pub inactive_selection: u32,
    pub caret: u32,
    pub line_number: u32,
}

struct Brushes {
    text: ID2D1SolidColorBrush,
    selection: ID2D1SolidColorBrush,
    inactive_selection: ID2D1SolidColorBrush,
    caret: ID2D1SolidColorBrush,
    line_number: ID2D1SolidColorBrush,
}

struct ViewState {
    hwnd: HWND,
    editor: Editor,
    target: Option<ID2D1HwndRenderTarget>,
    brushes: Option<Brushes>,
    colors: LiveColors,
    layout: LiveLayout,
    /// Top of the viewport in document DIPs.
    scroll_y: f32,
    /// Whether line numbers show (the editor's `line_numbers` setting), and the gutter's width in
    /// DIPs as of the last frame (0 without them).
    line_numbers: bool,
    gutter: f32,
    focused: bool,
    /// The blinking caret's phase.
    caret_on: bool,
    /// The x a run of Up/Down keeps aiming for, and the caret it left: any other caret move
    /// leaves the two apart, which ends the run.
    goal: Option<(f32, usize)>,
    /// A button drag selecting text, from `WM_LBUTTONDOWN` to its release.
    drag: Option<mouse::Drag>,
    /// The last press, for counting double and triple clicks.
    clicks: Option<mouse::Click>,
    /// Where the caret was drawn last, in client pixels: x, y and height.
    caret_px: Option<(i32, i32, i32)>,
    /// The document rows (top, bottom in DIPs) the last frame drew the selection and caret on,
    /// and the caret's own: what a selection move or a blink repaints.
    marks: Option<(f32, f32)>,
    caret_span: Option<(f32, f32)>,
    paint_retried: bool,
    /// Last, so the target, brushes and layouts above are released before it can unload
    /// Direct2D and DirectWrite.
    graphics: Rc<Graphics>,
}

/// A handle to a Live window; copies are cheap and all refer to the same window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveView {
    hwnd: HWND,
}

impl LiveView {
    /// A hidden Live window in `parent` showing `editor`'s document.
    pub fn create(
        parent: HWND,
        editor: Editor,
        graphics: Rc<Graphics>,
        colors: LiveColors,
        fonts: LiveFonts,
        line_numbers: bool,
    ) -> Result<Self> {
        let hwnd = create_window(parent)?;
        let layout = LiveLayout::new(Rc::clone(&graphics), fonts, 1.0);
        let layout = match layout {
            Ok(layout) => layout,
            Err(error) => {
                unsafe { DestroyWindow(hwnd) };
                return Err(error);
            }
        };
        let mut state = Box::new(ViewState {
            hwnd,
            editor,
            target: None,
            brushes: None,
            colors,
            layout,
            scroll_y: 0.0,
            line_numbers,
            gutter: 0.0,
            focused: false,
            caret_on: true,
            goal: None,
            drag: None,
            clicks: None,
            caret_px: None,
            marks: None,
            caret_span: None,
            paint_retried: false,
            graphics,
        });
        let width = text_width(hwnd, 0.0);
        let ViewState { editor, layout, .. } = &mut *state;
        let text = EditorText(editor);
        layout.set_width(&text, width);
        layout.reset(&text);
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

    /// Lays the whole document out again: it was replaced, or edited while the view was hidden.
    /// The first visible line stays where it was.
    pub fn reload(&self) {
        self.with(|state| {
            let top = state.layout.line_at_y(state.scroll_y);
            let ViewState { editor, layout, .. } = state;
            layout.reset(&EditorText(editor));
            state.scroll_y = state.layout.line_top(top);
        });
        invalidate(self.hwnd);
    }

    pub fn set_appearance(&self, colors: LiveColors, fonts: LiveFonts, line_numbers: bool) {
        self.with(|state| {
            state.line_numbers = line_numbers;
            if state.colors != colors {
                state.colors = colors;
                state.brushes = None;
            }
            let anchor = state.layout.anchor(state.scroll_y);
            let ViewState { editor, layout, .. } = state;
            if layout.set_fonts(&EditorText(editor), fonts).is_ok() {
                state.scroll_y = state.layout.scroll_for_anchor(anchor);
            }
        });
        invalidate(self.hwnd);
    }

    /// The first source line at least partly visible.
    pub fn top_line(&self) -> usize {
        self.with(|state| state.layout.line_at_y(state.scroll_y))
            .unwrap_or(0)
    }

    /// Scrolls so `line` is at the top.
    pub fn set_top_line(&self, line: usize) {
        self.with(|state| {
            let line = line.min(state.layout.line_count().saturating_sub(1));
            state.scroll_y = state.layout.line_top(line);
        });
        invalidate(self.hwnd);
    }

    /// Scrolls the caret into view, if it is not.
    pub fn scroll_to_caret(&self) {
        self.with(scroll_to_caret);
        invalidate(self.hwnd);
    }

    /// The document changed: an edit starting on `first_line` added `lines_added` lines
    /// (negative when it removed some). Only the touched lines are laid out again; the first
    /// visible line stays where it was.
    pub fn edited(&self, first_line: usize, lines_added: isize) {
        #[cfg(test)]
        let started = std::time::Instant::now();
        let painted = self.with(|state| {
            let scroll = state.scroll_y;
            let anchor = state.layout.anchor(scroll);
            let top = state.layout.line_top(first_line);
            let old_height = state.layout.line_top(first_line + 1) - top;
            let ViewState { editor, layout, .. } = state;
            let text = EditorText(editor);
            layout.edit(&text, first_line, lines_added);
            if lines_added == 0 {
                // Laid out now, so a line that kept its height repaints alone.
                let _ = layout.ensure(&text, first_line..first_line + 1);
            }
            state.scroll_y = state.layout.scroll_for_anchor(anchor);
            if (state.scroll_y - scroll).abs() > 0.01 {
                return false;
            }
            let new_height = state.layout.line_top(first_line + 1) - top;
            let bottom = if lines_added == 0 && (new_height - old_height).abs() < 0.01 {
                top + new_height
            } else {
                // Everything below moved.
                f32::MAX
            };
            invalidate_span(state, top, bottom);
            true
        });
        #[cfg(test)]
        record("edited", started);
        if painted != Some(true) {
            invalidate(self.hwnd);
        }
    }

    /// The editor's selection moved: the caret shows, is scrolled into view, and a run of Up/Down
    /// ends unless the move was its own.
    pub fn selection_changed(&self) {
        let painted = self.with(|state| {
            state.caret_on = true;
            let caret = state
                .editor
                .anchor_and_caret()
                .map_or(0, |(_, caret)| caret);
            if state.goal.is_some_and(|(_, at)| at != caret) {
                state.goal = None;
            }
            let scroll = state.scroll_y;
            scroll_to_caret(state);
            if (state.scroll_y - scroll).abs() > 0.01 {
                return false;
            }
            // Where the selection and caret were drawn, and where they go now.
            if let Some((top, bottom)) = state.marks {
                invalidate_span(state, top, bottom);
            }
            if let Some((top, bottom)) = selection_span(state) {
                invalidate_span(state, top, bottom);
            }
            true
        });
        if painted != Some(true) {
            invalidate(self.hwnd);
        }
    }

    /// The text of each laid-out line, `None` for lines not laid out yet.
    #[cfg(test)]
    pub fn debug_lines(&self) -> Vec<Option<String>> {
        self.with(|state| {
            (0..state.layout.line_count())
                .map(|line| {
                    state
                        .layout
                        .laid_line(line)
                        .map(|laid| String::from_utf16_lossy(laid.offsets.layout_text()))
                })
                .collect()
        })
        .unwrap_or_default()
    }

    /// A row's height and the text's left edge, in client pixels.
    #[cfg(test)]
    pub fn row_metrics(&self) -> (i32, i32) {
        self.with(|state| {
            let scale = dpi_scale(state.hwnd);
            (
                (state.layout.fonts().line_height * scale).round() as i32,
                (text_left(state) * scale).round() as i32,
            )
        })
        .unwrap_or((0, 0))
    }

    /// The line-number gutter's width in DIPs as of the last frame.
    #[cfg(test)]
    pub fn gutter(&self) -> f32 {
        self.with(|state| state.gutter).unwrap_or(0.0)
    }

    #[cfg(test)]
    pub fn font_size(&self) -> f32 {
        self.with(|state| state.layout.fonts().size).unwrap_or(0.0)
    }

    #[cfg(test)]
    pub fn scroll_y(&self) -> f32 {
        self.with(|state| state.scroll_y).unwrap_or(0.0)
    }
}

fn scroll_to_caret(state: &mut ViewState) {
    let (_, height) = view_size(state.hwnd);
    let ViewState { editor, layout, .. } = state;
    let text = EditorText(editor);
    let (_, caret) = text.selection();
    let Ok(rect) = layout.caret_rect(&text, caret) else {
        return;
    };
    let wanted = scroll::reveal(state.scroll_y, height, rect.top, rect.bottom, CARET_MARGIN);
    state.scroll_y = scroll::clamp(wanted, height, state.layout.total_height());
}

/// Kept out of `LiveView::create` so the public function never hands a raw `HWND` straight to an
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
        lpfnWndProc: Some(window_proc::live_proc),
        hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
        hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_IBEAM) },
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

/// Invalidates the document rows from `top` to `bottom` (DIPs) across the view's width, so the
/// next frame repaints only them. Sends no message.
fn invalidate_span(state: &ViewState, top: f32, bottom: f32) {
    let scale = dpi_scale(state.hwnd);
    let mut client = RECT::default();
    unsafe { GetClientRect(state.hwnd, &mut client) };
    let to_pixels = |y: f32| ((y - state.scroll_y) * scale).clamp(-1.0, client.bottom as f32 + 1.0);
    let rect = RECT {
        left: 0,
        top: to_pixels(top).floor() as i32 - 1,
        right: client.right,
        bottom: to_pixels(bottom).ceil() as i32 + 1,
    };
    if rect.bottom > 0 && rect.top < client.bottom {
        unsafe { InvalidateRect(state.hwnd, &rect, 0) };
    }
}

/// The rows the selection (or caret) covers on screen now, in document DIPs.
fn selection_span(state: &mut ViewState) -> Option<(f32, f32)> {
    let (_, height) = view_size(state.hwnd);
    let ViewState {
        editor,
        layout,
        scroll_y,
        ..
    } = state;
    let text = EditorText(editor);
    let (anchor, caret) = text.selection();
    let first_visible = layout.line_at_y(*scroll_y);
    let last_visible = layout.line_at_y(*scroll_y + height);
    let first = text.line_of(anchor.min(caret)).max(first_visible);
    let last = text.line_of(anchor.max(caret)).min(last_visible);
    if first > last {
        return None;
    }
    layout.ensure(&text, first..last + 1).ok()?;
    Some((layout.line_top(first), layout.line_top(last + 1)))
}

fn dpi_scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0
}

/// The client area in DIPs.
fn view_size(hwnd: HWND) -> (f32, f32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) };
    let scale = dpi_scale(hwnd);
    (
        (rect.right - rect.left) as f32 / scale,
        (rect.bottom - rect.top) as f32 / scale,
    )
}

/// The width the text wraps to, in DIPs.
fn text_width(hwnd: HWND, gutter: f32) -> f32 {
    (view_size(hwnd).0 - 2.0 * PADDING - gutter).max(1.0)
}

/// Where the text starts, in client DIPs: after the padding and the line-number gutter.
fn text_left(state: &ViewState) -> f32 {
    PADDING + state.gutter
}

/// The gutter's width for `state`'s document: room for its largest line number (two digits at
/// least, so it does not widen at line 10) and the gap before the text; 0 without line numbers.
fn gutter_width(state: &ViewState) -> f32 {
    if !state.line_numbers {
        return 0.0;
    }
    let lines = state.layout.line_count().max(1);
    let digits = (lines.ilog10() + 1).max(2) as f32;
    (digits * state.layout.digit_width()).ceil() + GUTTER_GAP
}

/// Tells the main window that input here may have moved the caret, or scrolled the view when
/// `scrolled`: a hidden Scintilla reports neither.
fn post_changed(hwnd: HWND, scrolled: bool) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            crate::platform::win32::root_window(hwnd),
            crate::window::WM_FASTPAD_LIVE_CHANGED,
            hwnd as usize,
            isize::from(scrolled),
        )
    };
}

#[cfg(test)]
thread_local! {
    /// Phase timings in microseconds, for the perf tests' breakdown.
    static TIMINGS: std::cell::RefCell<Vec<(&'static str, u64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Records how long phase `name` took since `started` (tests only).
#[cfg(test)]
fn record(name: &'static str, started: std::time::Instant) {
    TIMINGS.with(|timings| {
        timings
            .borrow_mut()
            .push((name, started.elapsed().as_micros() as u64))
    });
}

/// Takes the phase timings recorded so far.
#[cfg(test)]
pub fn take_timings() -> Vec<(&'static str, u64)> {
    TIMINGS.with(|timings| std::mem::take(&mut *timings.borrow_mut()))
}
