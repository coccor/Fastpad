//! The `FastPadLive` child window: the Live view of one editor group's Markdown tab. The group's
//! Scintilla stays the document (text, selection, undo); this window draws it with Direct2D and,
//! turns keys and clicks into Scintilla input and selections (phase 1 plan,
//! Architecture). Like the preview, nothing here sends messages while the state is borrowed.
//!
//! The view renders the document's Markdown (phase 2 plan, Task 6): edits are logged as they
//! arrive and applied to the document's spans (`flush`) before anything measures or paints, and
//! the lines the selection touches show as source (`sync_reveal`), keeping the caret where it was
//! on screen when lines change between rendered and source.
//!
//! Block constructs (phase 3 plan, Task 6) are drawn from each line's decorations; images decode
//! on the view's own `ImageCache` worker, relative to the document's folder, and an image line is
//! laid out again when its picture arrives.

use crate::Result;
use crate::editor::Editor;
use crate::live::blocks::LiveDocument;
use crate::live::layout::{LiveFonts, LiveLayout};
use crate::live::reveal;
use crate::live::scroll;
use crate::live::styles::{ImageInfo, Paint, Styled, resolve_image};
use crate::live::text::{EditorText, LiveText};
use crate::platform::{last_error, wide_null};
use crate::preview::dwrite::Graphics;
use crate::preview::images::ImageCache;
use crate::preview::incremental::{Edit, EditLog, Pending, SourceText};
use std::ops::Range;
use std::path::PathBuf;
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

mod drop;
mod ime;
mod keys;
mod mouse;
pub use mouse::MenuRequest;
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
    /// The Markdown roles, as the source view colours them (`languages/markdown.rs`).
    pub heading: u32,
    pub emphasis: u32,
    pub marker: u32,
    pub link: u32,
    pub code: u32,
    pub code_background: u32,
    pub dim: u32,
    /// Behind the block being edited (the revealed lines): the editor's current-line colour.
    pub active_block: u32,
}

struct Brushes {
    text: ID2D1SolidColorBrush,
    selection: ID2D1SolidColorBrush,
    inactive_selection: ID2D1SolidColorBrush,
    caret: ID2D1SolidColorBrush,
    line_number: ID2D1SolidColorBrush,
    heading: ID2D1SolidColorBrush,
    emphasis: ID2D1SolidColorBrush,
    marker: ID2D1SolidColorBrush,
    link: ID2D1SolidColorBrush,
    code: ID2D1SolidColorBrush,
    code_background: ID2D1SolidColorBrush,
    dim: ID2D1SolidColorBrush,
    active_block: ID2D1SolidColorBrush,
}

impl Brushes {
    /// The brush for a colour role of a styled line.
    fn paint(&self, paint: Paint) -> &ID2D1SolidColorBrush {
        match paint {
            Paint::Text => &self.text,
            Paint::Heading => &self.heading,
            Paint::Emphasis => &self.emphasis,
            Paint::Marker => &self.marker,
            Paint::Link => &self.link,
            Paint::Code => &self.code,
            Paint::CodeBackground => &self.code_background,
            // Rules, quote bars and quote text use the comment colour, as dimmed text does.
            Paint::Dim | Paint::Rule | Paint::QuoteBar | Paint::Quote => &self.dim,
        }
    }
}

/// The pictures image lines show: decoded on a worker, found relative to the document's folder.
struct Pictures {
    cache: ImageCache,
    folder: Option<PathBuf>,
}

impl Pictures {
    fn resolve(&self, dest: &str) -> ImageInfo {
        resolve_image(
            dest,
            self.folder.as_deref(),
            |path| self.cache.size(path),
            |path| self.cache.is_failed(path),
        )
    }
}

struct ViewState {
    hwnd: HWND,
    editor: Editor,
    target: Option<ID2D1HwndRenderTarget>,
    brushes: Option<Brushes>,
    /// Bumped whenever the brushes are made again: a layout bound for an older generation
    /// re-binds its colours before it is drawn.
    brush_generation: u64,
    /// Code backgrounds drawn by the last frame, for the tests.
    #[cfg(test)]
    painted_backgrounds: usize,
    /// Decorations drawn by the last frame, by kind, for the tests.
    #[cfg(test)]
    painted: PaintedDecorations,
    pictures: Pictures,
    colors: LiveColors,
    layout: LiveLayout,
    /// The document's blocks and spans, as of the last `flush`.
    document: LiveDocument,
    /// Edits not yet applied to `document`.
    edits: EditLog,
    /// Edited lines whose layout height may have changed, with their height before the edit.
    height_checks: Vec<(usize, f32)>,
    /// The source lines showing as source: the ones the selection touches.
    revealed: Range<usize>,
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
    /// Whether the pointer shows the hand: over a link with Ctrl held.
    hand: bool,
    /// Where dragged text would land: drawn as a second caret while a drag is over the view.
    drop_at: Option<usize>,
    /// A press inside the selection, which a move turns into a text drag.
    text_drag: Option<mouse::TextDrag>,
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

/// What the last frame drew besides text, for the tests: fills (code blocks, table headers),
/// lines (rules, table borders), bullets, numbers, checkboxes and images.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintedDecorations {
    pub fills: usize,
    pub lines: usize,
    pub bullets: usize,
    pub numbers: usize,
    pub checkboxes: usize,
    pub images: usize,
    /// Revealed lines drawn on the edited block's highlight.
    pub active_block: usize,
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
            brush_generation: 0,
            #[cfg(test)]
            painted_backgrounds: 0,
            #[cfg(test)]
            painted: PaintedDecorations::default(),
            pictures: Pictures {
                cache: ImageCache::new(hwnd, crate::window::WM_FASTPAD_LIVE_IMAGE),
                folder: None,
            },
            colors,
            layout,
            document: LiveDocument::default(),
            edits: EditLog::default(),
            height_checks: Vec::new(),
            revealed: 0..1,
            scroll_y: 0.0,
            line_numbers,
            gutter: 0.0,
            focused: false,
            caret_on: true,
            goal: None,
            drag: None,
            clicks: None,
            hand: false,
            drop_at: None,
            text_drag: None,
            caret_px: None,
            marks: None,
            caret_span: None,
            paint_retried: false,
            graphics,
        });
        load_document(&mut state);
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize) };
        drop::register(hwnd);
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
            load_document(state);
            state.scroll_y = state.layout.line_top(top);
        });
        invalidate(self.hwnd);
    }

    /// The folder relative image paths resolve against (the document's). A change forgets the
    /// decoded images and lays the image lines out again; nothing else depends on it.
    pub fn set_document_dir(&self, folder: Option<PathBuf>) {
        let changed = self.with(|state| {
            if state.pictures.folder == folder {
                return false;
            }
            state.pictures.folder = folder;
            state.pictures.cache.clear();
            state.layout.drop_image_lines();
            true
        });
        if changed == Some(true) {
            invalidate(self.hwnd);
        }
    }

    pub fn set_appearance(&self, colors: LiveColors, fonts: LiveFonts, line_numbers: bool) {
        self.with(|state| {
            state.line_numbers = line_numbers;
            if state.colors != colors {
                state.colors = colors;
                state.brushes = None;
            }
            let anchor = state.layout.anchor(state.scroll_y);
            let ViewState {
                editor,
                layout,
                document,
                revealed,
                pictures,
                ..
            } = state;
            let source = EditorText(editor);
            let text = Styled {
                text: &source,
                document,
                revealed: revealed.clone(),
                images: &|dest: &str| pictures.resolve(dest),
            };
            if layout.set_fonts(&text, fonts).is_ok() {
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
            prepare(state);
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

    /// The document changed: `edit` starts on `first_line`. The edit is logged for the next
    /// `flush`, the touched lines lose their layouts, and the first visible line stays where it
    /// was.
    pub fn edited(&self, edit: Edit, first_line: usize) {
        #[cfg(test)]
        let started = std::time::Instant::now();
        let lines_added = edit.lines_delta;
        let painted = self.with(|state| {
            state.edits.record(edit);
            let scroll = state.scroll_y;
            let anchor = state.layout.anchor(scroll);
            let top = state.layout.line_top(first_line);
            let old_height = state.layout.line_top(first_line + 1) - top;
            state.revealed = reveal::shift(state.revealed.clone(), first_line, lines_added);
            let ViewState {
                editor,
                layout,
                document,
                revealed,
                pictures,
                ..
            } = state;
            let source = EditorText(editor);
            let text = Styled {
                text: &source,
                document,
                revealed: revealed.clone(),
                images: &|dest: &str| pictures.resolve(dest),
            };
            layout.edit(&text, first_line, lines_added);
            state.scroll_y = state.layout.scroll_for_anchor(anchor);
            if (state.scroll_y - scroll).abs() > 0.01 {
                return false;
            }
            if lines_added == 0 {
                // `flush` checks the line's new height once its spans are current.
                state.height_checks.push((first_line, old_height));
                invalidate_span(state, top, top + old_height);
            } else {
                // Everything below moved.
                invalidate_span(state, top, f32::MAX);
            }
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
            if prepare(state) {
                // Lines changed between rendered and source: `sync_reveal` repaints everything.
                scroll_to_caret(state);
                return true;
            }
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

    /// Line `line` as laid out now: its layout text, whether it is revealed, and its height.
    #[cfg(test)]
    pub fn debug_line(&self, line: usize) -> Option<(String, bool, f32)> {
        self.with(|state| {
            prepare(state);
            let ViewState {
                editor,
                layout,
                document,
                revealed,
                pictures,
                ..
            } = state;
            let source = EditorText(editor);
            let text = Styled {
                text: &source,
                document,
                revealed: revealed.clone(),
                images: &|dest: &str| pictures.resolve(dest),
            };
            let laid = layout.line(&text, line).ok()?;
            Some((
                String::from_utf16_lossy(laid.offsets.layout_text()),
                laid.style.revealed,
                laid.height,
            ))
        })
        .flatten()
    }

    /// Code backgrounds the last frame drew, and the brush generation it drew with.
    #[cfg(test)]
    pub fn paint_stats(&self) -> (usize, u64) {
        self.with(|state| (state.painted_backgrounds, state.brush_generation))
            .unwrap_or((0, 0))
    }

    /// The brush generation line `line`'s colours are bound for (0 when not laid out).
    #[cfg(test)]
    pub fn line_binding(&self, line: usize) -> u64 {
        self.with(|state| state.layout.laid_line(line).map_or(0, |laid| laid.bound()))
            .unwrap_or(0)
    }

    /// The client point (x, middle of its row y) where the caret at `position` is drawn.
    #[cfg(test)]
    pub fn client_point_of(&self, position: usize) -> Option<(i32, i32)> {
        self.with(|state| {
            prepare(state);
            let scale = dpi_scale(state.hwnd);
            let left = text_left(state);
            let ViewState {
                editor,
                layout,
                document,
                revealed,
                scroll_y,
                pictures,
                ..
            } = state;
            let source = EditorText(editor);
            let text = Styled {
                text: &source,
                document,
                revealed: revealed.clone(),
                images: &|dest: &str| pictures.resolve(dest),
            };
            let rect = layout.caret_rect(&text, position).ok()?;
            Some((
                ((left + rect.left) * scale).round() as i32,
                (((rect.top + rect.bottom) / 2.0 - *scroll_y) * scale).round() as i32,
            ))
        })
        .flatten()
    }

    /// The client point at the centre of the first checkbox drawn for `line`.
    #[cfg(test)]
    pub fn checkbox_point(&self, line: usize) -> Option<(i32, i32)> {
        self.with(|state| {
            prepare(state);
            let scale = dpi_scale(state.hwnd);
            let left = text_left(state);
            let top = state.layout.line_top(line) - state.scroll_y;
            state
                .layout
                .laid_line(line)?
                .decorations
                .iter()
                .find_map(|decoration| match decoration {
                    crate::live::layout::Decoration::Checkbox { rect, .. } => Some((
                        ((left + (rect.left + rect.right) / 2.0) * scale).round() as i32,
                        ((top + (rect.top + rect.bottom) / 2.0) * scale).round() as i32,
                    )),
                    _ => None,
                })
        })
        .flatten()
    }

    /// What the last frame drew besides text.
    #[cfg(test)]
    pub fn painted_decorations(&self) -> PaintedDecorations {
        self.with(|state| state.painted).unwrap_or_default()
    }

    /// Whether the pointer last showed the hand.
    #[cfg(test)]
    pub fn hand_cursor(&self) -> bool {
        self.with(|state| state.hand).unwrap_or(false)
    }

    /// Drops the render target, as a lost device does.
    #[cfg(test)]
    pub fn drop_target(&self) {
        self.with(|state| {
            state.target = None;
            state.brushes = None;
        });
        invalidate(self.hwnd);
    }

    /// Where the drop caret is drawn, while dragged text is over the view.
    #[cfg(test)]
    pub fn drop_caret(&self) -> Option<usize> {
        self.with(|state| state.drop_at).flatten()
    }

    /// Drops `text` at client point `client` through the window's OLE drop target.
    #[cfg(test)]
    pub fn drop_text(&self, text: &str, client: (i32, i32)) -> u32 {
        drop::test_drop_text(self.hwnd, text, client)
    }

    /// The source lines showing as source.
    #[cfg(test)]
    pub fn revealed(&self) -> Range<usize> {
        self.with(|state| state.revealed.clone()).unwrap_or(0..0)
    }

    /// The caret's top in client pixels, as last painted.
    #[cfg(test)]
    pub fn caret_client_y(&self) -> Option<i32> {
        self.with(|state| state.caret_px.map(|(_, y, _)| y))
            .flatten()
    }

    /// How many times the document applied edits or reparsed.
    #[cfg(test)]
    pub fn document_updates(&self) -> usize {
        self.with(|state| state.document.updates()).unwrap_or(0)
    }

    /// How many blocks the document has parsed for spans.
    #[cfg(test)]
    pub fn parsed_blocks(&self) -> usize {
        self.with(|state| state.document.parsed_blocks())
            .unwrap_or(0)
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
    prepare(state);
    let (_, height) = view_size(state.hwnd);
    let ViewState {
        editor,
        layout,
        document,
        revealed,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
    let (_, caret) = text.selection();
    let Ok(rect) = layout.caret_rect(&text, caret) else {
        return;
    };
    let wanted = scroll::reveal(state.scroll_y, height, rect.top, rect.bottom, CARET_MARGIN);
    state.scroll_y = scroll::clamp(wanted, height, state.layout.total_height());
}

/// Parses the whole document again and lays it out from scratch, revealing the selection's
/// lines: entering Live, or a change the view was not told about.
fn load_document(state: &mut ViewState) {
    let width = text_width(state.hwnd, state.gutter);
    let ViewState {
        editor,
        layout,
        document,
        edits,
        height_checks,
        revealed,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    *document = LiveDocument::parse(&SourceText::slice(&source, 0..SourceText::len(&source)));
    *edits = EditLog::default();
    height_checks.clear();
    let (anchor, caret) = source.selection();
    *revealed = revealed_for(&source, document, anchor, caret);
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
    layout.set_width(&text, width);
    layout.reset(&text);
}

/// The lines shown as source for the selection `anchor..caret`: the blocks it touches, whole.
fn revealed_for<T: LiveText + SourceText>(
    source: &T,
    document: &LiveDocument,
    anchor: usize,
    caret: usize,
) -> Range<usize> {
    reveal::revealed_blocks(
        anchor,
        caret,
        |at| LiveText::line_of(source, at),
        |line| block_lines(source, document, line),
    )
}

/// The lines of the block holding `line`: its top-level block (a paragraph, a heading, a whole
/// code block or table, the front matter), narrowed inside a list or quote to the lines between
/// the blank lines around it. A blank line is a block of its own.
fn block_lines<T: LiveText + SourceText>(
    source: &T,
    document: &LiveDocument,
    line: usize,
) -> Range<usize> {
    let blank = |line: usize| LiveText::line(source, line).trim().is_empty();
    if blank(line) {
        return line..line + 1;
    }
    let start = source.line_start(line);
    let end = source.line_end(line);
    // A byte of the line itself: an indented block starts after the line's start.
    let Some(bytes) = document.block_bytes(end.saturating_sub(1).max(start)) else {
        return line..line + 1;
    };
    let first = LiveText::line_of(source, bytes.start).min(line);
    let last = LiveText::line_of(source, bytes.end.saturating_sub(1).max(bytes.start)).max(line);
    let nested = document
        .line_block(source, start..end)
        .is_some_and(|block| !block.containers.is_empty());
    if !nested {
        return first..last + 1;
    }
    let mut top = line;
    while top > first && !blank(top - 1) {
        top -= 1;
    }
    let mut bottom = line;
    while bottom < last && !blank(bottom + 1) {
        bottom += 1;
    }
    top..bottom + 1
}

/// Brings the view up to date before anything measures or paints: applies pending edits to the
/// document (`flush`), then reveals the selection's lines (`sync_reveal`). True when the revealed
/// lines changed (everything is then invalidated).
fn prepare(state: &mut ViewState) -> bool {
    flush(state);
    sync_reveal(state)
}

/// Applies the logged edits to the document and drops the layouts of lines whose spans changed,
/// invalidating what may have moved. Nothing to do when no edit is pending.
fn flush(state: &mut ViewState) {
    let pending = state.edits.take();
    if pending == Pending::Nothing {
        return;
    }
    let checks = std::mem::take(&mut state.height_checks);
    let (_, view_height) = view_size(state.hwnd);
    let scroll = state.scroll_y;
    let ViewState {
        editor,
        layout,
        document,
        revealed,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    let all = 0..layout.line_count();
    #[cfg(test)]
    let started = std::time::Instant::now();
    let restyle = match pending {
        Pending::Edits(edits) => {
            let changed = document.apply(&source, &edits);
            let first = LiveText::line_of(&source, changed.start);
            let last = LiveText::line_of(&source, changed.end);
            first..last + 1
        }
        _ => {
            document.reparse(&source);
            all
        }
    };
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
    #[cfg(test)]
    record("flush: parse", started);
    #[cfg(test)]
    let started = std::time::Instant::now();
    let dropped = layout.restyle(&text, restyle);
    #[cfg(test)]
    record("flush: restyle", started);
    #[cfg(test)]
    let started = std::time::Instant::now();
    // Restyled lines on screen, and the one edited line, are laid out again now: only a line
    // whose height changed repaints everything below it, the others just their own rows (typing
    // in a table cell restyles a row, not the screen). Restyled lines off screen keep their
    // heights until they are laid out, so nothing on screen moves for them.
    let first_visible = layout.line_at_y(scroll);
    let last_visible = layout.line_at_y(scroll + view_height);
    let mut relay = dropped
        .iter()
        .filter(|&&line| (first_visible..=last_visible).contains(&line))
        .map(|&line| (line, layout.line_top(line + 1) - layout.line_top(line)))
        .collect::<Vec<_>>();
    let mut moved_from = (checks.len() > 1).then_some(0);
    if let [(line, old_height)] = checks[..] {
        // Its height in the index is an estimate since the edit; the one recorded is its own.
        relay.retain(|(other, _)| *other != line);
        relay.push((line, old_height));
    }
    let mut rows = Vec::new();
    for (line, old_height) in relay {
        let top = layout.line_top(line);
        if layout.ensure(&text, line..line + 1).is_err() {
            moved_from = Some(moved_from.map_or(line, |from| from.min(line)));
            continue;
        }
        let height = layout.line_top(line + 1) - top;
        if (height - old_height).abs() > 0.01 {
            moved_from = Some(moved_from.map_or(line, |from| from.min(line)));
        } else {
            rows.push((top, top + height));
        }
    }
    #[cfg(test)]
    record("flush: lay out", started);
    if let Some(line) = moved_from {
        let top = state.layout.line_top(line);
        invalidate_span(state, top, f32::MAX);
    }
    for (top, bottom) in rows {
        invalidate_span(state, top, bottom);
    }
}

/// Reveals the lines the selection touches, if they changed (Decision 2 of the phase 2 plan):
/// the caret's rectangle is taken before and after the change and the view scrolls by the
/// difference, so the caret keeps its place on screen however heights above or under it change.
fn sync_reveal(state: &mut ViewState) -> bool {
    let (_, height) = view_size(state.hwnd);
    let ViewState {
        editor,
        layout,
        document,
        revealed,
        scroll_y,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    let (anchor, caret) = source.selection();
    let new = revealed_for(&source, document, anchor, caret);
    if new == *revealed {
        return false;
    }
    let before = layout
        .caret_rect(
            &Styled {
                text: &source,
                document,
                revealed: revealed.clone(),
                images: &|dest: &str| pictures.resolve(dest),
            },
            caret,
        )
        .ok();
    let old = std::mem::replace(revealed, new.clone());
    let text = Styled {
        text: &source,
        document,
        revealed: new.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
    // Lines that changed state between the top of the view and the caret are laid out again
    // now: until then they keep their old heights, and the caret's new top would not show the
    // change. Lines above the view are held steady by the paint's anchor instead.
    let caret_line = LiveText::line_of(&source, caret);
    let first_visible = layout.line_at_y(*scroll_y);
    for lines in reveal::changed(old, new) {
        layout.restyle(&text, lines.clone());
        let relay = lines.start.max(first_visible)..lines.end.min(caret_line + 1);
        if relay.start < relay.end {
            let _ = layout.ensure(&text, relay);
        }
    }
    if let (Some(before), Ok(after)) = (before, layout.caret_rect(&text, caret)) {
        *scroll_y += after.top - before.top;
    }
    *scroll_y = scroll::clamp(*scroll_y, height, layout.total_height());
    invalidate(state.hwnd);
    true
}

/// `WM_FASTPAD_LIVE_IMAGE`: decodes finished. Image lines are laid out again with their pictures
/// (or their alt text, for a picture that failed).
fn images_ready(hwnd: HWND) {
    let changed = with_state(hwnd, |state| {
        if !state.pictures.cache.drain() {
            return false;
        }
        let dropped = state.layout.drop_image_lines();
        if let Some(&first) = dropped.first() {
            let top = state.layout.line_top(first);
            invalidate_span(state, top, f32::MAX);
        }
        true
    });
    if changed == Some(true) {
        post_changed(hwnd, false);
    }
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
        document,
        revealed,
        scroll_y,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
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
