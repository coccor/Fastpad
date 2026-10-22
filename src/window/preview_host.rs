//! The main window's half of the Markdown preview: each tab's view mode (2026-10-22 view modes
//! spec), the split layout and its divider,
//! loading documents into the preview window, and (in later tasks) the debounced update pipeline,
//! scroll sync, and link actions. Every entry point takes the main window handle. No function holds
//! an `App` borrow while calling Win32 APIs that can send messages back to the main window
//! (`SetFocus`, `ShowWindow`, `MoveWindow`, `ShellExecuteW`).

use crate::Result;
use crate::document::{DocumentId, Language};
use crate::editor::scintilla_constants::SC_MOD_INSERTTEXT;
use crate::editor::{Editor, ScintillaNotification};
use crate::image_view::ImageView;
use crate::platform::wide_null;
use crate::preview::colors::{PreviewColors, preview_colors};
use crate::preview::dwrite::Graphics;
use crate::preview::incremental::{Edit, EditLog, Pending, PreviewDocument, SourceText};
use crate::preview::layout::PreviewFonts;
use crate::preview::links::{LinkAction, classify_link};
use crate::preview::view::PreviewView;
use crate::preview::{
    LIVE_UPDATE_LIMIT, PREVIEW_UPDATE_DELAY_MS, ViewMode, WORKER_PARSE_THRESHOLD,
};
use crate::window::WM_FASTPAD_PREVIEW_PARSED;
use crate::window::commands::CommandId;
use crate::window::design::metrics::scale;
use crate::window::main_window as host_window;
use crate::window::palette::Palette;
use crate::window::split_tree::GroupId;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetFocus, ReleaseCapture, SetCapture, SetFocus,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetMessageTime, KillTimer, MoveWindow, PostMessageW, SW_HIDE, SW_SHOWNA,
    SW_SHOWNORMAL, SetTimer, ShowWindow,
};

pub(crate) const PREVIEW_TIMER_ID: usize = 0x4650_5056;
const DIVIDER_WIDTH_AT_96_DPI: i32 = 4;
const MIN_RATIO: f32 = 0.2;
const MAX_RATIO: f32 = 0.8;
const NOT_MARKDOWN_NOTICE: &str = "Markdown preview is available for Markdown and SVG documents. \
                                   Choose View > Markdown to treat this tab as Markdown.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScrollOrigin {
    #[default]
    None,
    Preview,
}

pub(crate) struct PreviewHost {
    /// The view mode chosen for each of the group's tabs; absent follows the default.
    pub(crate) modes: HashMap<DocumentId, ViewMode>,
    /// Each tab's last mode other than Preview, where Escape in Preview goes back to.
    last_editing: HashMap<DocumentId, ViewMode>,
    pub(crate) view: Option<PreviewView>,
    /// The SVG preview (image preview spec §8): an image view in the preview slot of an SVG tab.
    pub(crate) svg_view: Option<ImageView>,
    /// The SVG tab `svg_view` shows; `None` forces a reload when next shown.
    pub(crate) svg_document: Option<DocumentId>,
    ratio: f32,
    pub(crate) edits: EditLog,
    /// The document the preview currently shows; `None` forces a reload when next shown.
    pub(crate) document: Option<DocumentId>,
    pub(crate) parse_generation: u64,
    /// Set while a worker parse for `parse_generation` is outstanding. The view's block model is
    /// stale until it lands, so edits must not be applied incrementally on top of it.
    pub(crate) full_parse_pending: bool,
    dragging: bool,
    last_divider_click: Option<u32>,
    area: Option<RECT>,
    divider: Option<RECT>,
    pub(crate) scroll_origin: ScrollOrigin,
    pub(crate) sync_count: u64,
    pub(crate) hover_text: Option<String>,
    button_hint: Option<String>,
    /// Set while this preview has edits for the next `PREVIEW_TIMER_ID` flush.
    flush_pending: bool,
}

/// Written by hand: windows-sys `RECT` implements no `Debug`.
impl std::fmt::Debug for PreviewHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreviewHost")
            .field("modes", &self.modes)
            .field("last_editing", &self.last_editing)
            .field("view", &self.view)
            .field("svg_view", &self.svg_view)
            .field("svg_document", &self.svg_document)
            .field("ratio", &self.ratio)
            .field("edits", &self.edits)
            .field("document", &self.document)
            .field("parse_generation", &self.parse_generation)
            .field("full_parse_pending", &self.full_parse_pending)
            .field("dragging", &self.dragging)
            .field("area", &self.area.map(rect_tuple))
            .field("divider", &self.divider.map(rect_tuple))
            .field("scroll_origin", &self.scroll_origin)
            .field("sync_count", &self.sync_count)
            .field("hover_text", &self.hover_text)
            .field("button_hint", &self.button_hint)
            .finish_non_exhaustive()
    }
}

impl Default for PreviewHost {
    fn default() -> Self {
        Self {
            modes: HashMap::new(),
            last_editing: HashMap::new(),
            view: None,
            svg_view: None,
            svg_document: None,
            ratio: 0.5,
            edits: EditLog::default(),
            document: None,
            parse_generation: 0,
            full_parse_pending: false,
            dragging: false,
            last_divider_click: None,
            area: None,
            divider: None,
            scroll_origin: ScrollOrigin::None,
            sync_count: 0,
            hover_text: None,
            button_hint: None,
            flush_pending: false,
        }
    }
}

impl PreviewHost {
    pub(crate) fn status_hint(&self) -> Option<String> {
        self.button_hint.clone().or_else(|| self.hover_text.clone())
    }
}

const fn rect_tuple(rect: RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

/// `Debug`, `PartialEq`, and `Eq` are written by hand: windows-sys `RECT` derives none of them.
#[derive(Clone, Copy)]
pub(crate) struct ContentRects {
    pub editor: Option<RECT>,
    pub divider: Option<RECT>,
    pub preview: Option<RECT>,
}

impl std::fmt::Debug for ContentRects {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContentRects")
            .field("editor", &self.editor.map(rect_tuple))
            .field("divider", &self.divider.map(rect_tuple))
            .field("preview", &self.preview.map(rect_tuple))
            .finish()
    }
}

impl PartialEq for ContentRects {
    fn eq(&self, other: &Self) -> bool {
        self.editor.map(rect_tuple) == other.editor.map(rect_tuple)
            && self.divider.map(rect_tuple) == other.divider.map(rect_tuple)
            && self.preview.map(rect_tuple) == other.preview.map(rect_tuple)
    }
}

impl Eq for ContentRects {}

pub(crate) fn content_rects(
    area: RECT,
    mode: ViewMode,
    shown: bool,
    ratio: f32,
    dpi: u32,
) -> ContentRects {
    match (mode, shown) {
        (ViewMode::Preview, true) => ContentRects {
            editor: None,
            divider: None,
            preview: Some(area),
        },
        (ViewMode::Split, true) => {
            let divider = scale(DIVIDER_WIDTH_AT_96_DPI, dpi);
            let usable = (area.right - area.left - divider).max(0);
            let editor_right =
                area.left + (usable as f32 * ratio.clamp(MIN_RATIO, MAX_RATIO)).round() as i32;
            ContentRects {
                editor: Some(RECT {
                    right: editor_right,
                    ..area
                }),
                divider: Some(RECT {
                    left: editor_right,
                    right: editor_right + divider,
                    ..area
                }),
                preview: Some(RECT {
                    left: editor_right + divider,
                    ..area
                }),
            }
        }
        _ => ContentRects {
            editor: Some(area),
            divider: None,
            preview: None,
        },
    }
}

pub(crate) fn ratio_for_x(area: RECT, x: i32, dpi: u32) -> f32 {
    let divider = scale(DIVIDER_WIDTH_AT_96_DPI, dpi);
    let usable = (area.right - area.left - divider).max(1);
    ((x - area.left - divider / 2) as f32 / usable as f32).clamp(MIN_RATIO, MAX_RATIO)
}

/// Document text read straight from Scintilla's buffer without copying. A slice borrows
/// Scintilla's gap buffer and is valid only until the next document change: use it inside one
/// parse call and never across anything that can modify the editor or pump messages.
pub(crate) struct ScintillaSource<'a>(pub &'a Editor);

impl SourceText for ScintillaSource<'_> {
    fn len(&self) -> usize {
        self.0.length().unwrap_or(0)
    }

    fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        match self.0.range_bytes(range) {
            Ok(bytes) => String::from_utf8_lossy(bytes),
            Err(_) => Cow::Borrowed(""),
        }
    }

    fn line_of(&self, byte: usize) -> usize {
        self.0.line_from_position(byte).unwrap_or(0)
    }
}

thread_local! {
    /// The group the preview functions act on while `in_group` runs; the active group otherwise.
    static TARGET: std::cell::Cell<Option<GroupId>> = const { std::cell::Cell::new(None) };
}

/// Runs `action` with this module's functions acting on group `id`'s preview, editor and active
/// tab instead of the active group's: a group's notifications, posts and parse results reach its
/// own preview (split editors spec §12).
pub(crate) fn in_group<R>(id: GroupId, action: impl FnOnce() -> R) -> R {
    let previous = TARGET.with(|target| target.replace(Some(id)));
    let result = action();
    TARGET.with(|target| target.set(previous));
    result
}

/// The group this module acts on now.
fn target_group(hwnd: HWND) -> Option<GroupId> {
    TARGET.with(std::cell::Cell::get).or_else(|| {
        unsafe { host_window::app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    })
}

pub(crate) fn with_host<R>(hwnd: HWND, action: impl FnOnce(&mut PreviewHost) -> R) -> Option<R> {
    let id = target_group(hwnd)?;
    with_group_host(hwnd, id, action)
}

/// `with_host` for the group `id`, which need not be the target.
pub(crate) fn with_group_host<R>(
    hwnd: HWND,
    id: GroupId,
    action: impl FnOnce(&mut PreviewHost) -> R,
) -> Option<R> {
    // SAFETY: the App pointer is used only for the immediate field access inside `action`, which
    // never calls back into Win32.
    unsafe { host_window::app_ptr(hwnd) }
        .and_then(|mut app| Some(action(&mut unsafe { app.as_mut() }.group_mut(id)?.preview)))
}

/// The target group's active tab's view mode.
pub(crate) fn mode(hwnd: HWND) -> ViewMode {
    target_group(hwnd).map_or(ViewMode::Source, |id| group_mode(hwnd, id))
}

/// Group `id`'s active tab's view mode: its choice, else the default for its language, as far as
/// the tab can show it. A tab that is neither Markdown nor SVG is always Source.
pub(crate) fn group_mode(hwnd: HWND, id: GroupId) -> ViewMode {
    let Some((document, language, _)) = group_document(hwnd, id) else {
        return ViewMode::Source;
    };
    let chosen = with_group_host(hwnd, id, |host| host.modes.get(&document).copied()).flatten();
    let default = match language {
        Language::Markdown => unsafe { host_window::app_ptr(hwnd) }.map_or(
            ViewMode::Source,
            |app| unsafe { app.as_ref() }.settings.markdown_view,
        ),
        _ => ViewMode::Source,
    };
    let mode = chosen.unwrap_or(default);
    // The default skips a file over the Live limit silently; a choice was checked when made.
    if available(hwnd, id, language, mode) {
        mode
    } else {
        ViewMode::Source
    }
}

/// Whether group `id`'s active tab, in `language`, can show `mode`.
fn available(hwnd: HWND, id: GroupId, language: Language, mode: ViewMode) -> bool {
    match language {
        Language::Markdown => {
            mode != ViewMode::Live
                || host_window::group_editor(hwnd, id)
                    .and_then(|editor| editor.length().ok())
                    .unwrap_or(0)
                    <= crate::live::LIVE_MAX_BYTES
        }
        Language::Svg => mode != ViewMode::Live,
        _ => mode == ViewMode::Source,
    }
}

/// Records `mode` for group `id`'s active tab without showing it: the caller syncs.
pub(crate) fn choose(hwnd: HWND, id: GroupId, mode: ViewMode) {
    let Some((document, ..)) = group_document(hwnd, id) else {
        return;
    };
    with_group_host(hwnd, id, |host| {
        host.modes.insert(document, mode);
        if mode != ViewMode::Preview {
            host.last_editing.insert(document, mode);
        }
    });
}

/// Group `id`'s active tab's last mode other than Preview; Source when it had none.
fn last_editing(hwnd: HWND, id: GroupId) -> ViewMode {
    group_document(hwnd, id)
        .and_then(|(document, ..)| {
            with_group_host(hwnd, id, |host| host.last_editing.get(&document).copied()).flatten()
        })
        .unwrap_or(ViewMode::Source)
}

/// Document `document` was moved or copied from group `from` to group `to`: its mode goes too
/// (view modes spec §2).
pub(crate) fn carry_mode(hwnd: HWND, from: GroupId, document: DocumentId, to: GroupId) {
    let Some((mode, last)) = with_group_host(hwnd, from, |host| {
        (
            host.modes.get(&document).copied(),
            host.last_editing.get(&document).copied(),
        )
    }) else {
        return;
    };
    with_group_host(hwnd, to, |host| {
        if let Some(mode) = mode {
            host.modes.insert(document, mode);
        }
        if let Some(last) = last {
            host.last_editing.insert(document, last);
        }
    });
}

/// Drops the modes of documents no longer among group `id`'s tabs.
fn prune_modes(hwnd: HWND, id: GroupId) {
    let Some(mut app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return;
    };
    let app = unsafe { app.as_mut() };
    let Some(open) = app.tabs.group(id).map(|tabs| tabs.document_ids()) else {
        return;
    };
    if let Some(group) = app.group_mut(id) {
        group
            .preview
            .modes
            .retain(|document, _| open.contains(document));
        group
            .preview
            .last_editing
            .retain(|document, _| open.contains(document));
    }
}

pub(crate) fn view(hwnd: HWND) -> Option<PreviewView> {
    with_host(hwnd, |host| host.view).flatten()
}

/// The target group's editor.
pub(crate) fn editor(hwnd: HWND) -> Option<Editor> {
    host_window::group_editor(hwnd, target_group(hwnd)?)
}

fn editor_hwnd(hwnd: HWND) -> Option<HWND> {
    editor(hwnd).map(|editor| editor.hwnd())
}

/// The window showing the target group's text: its Live view for a Live tab, else its editor.
fn text_hwnd(hwnd: HWND) -> Option<HWND> {
    crate::window::live_host::text_view_hwnd(hwnd, target_group(hwnd)?)
}

/// The target group's window, where its preview windows go.
fn content_parent(hwnd: HWND) -> HWND {
    target_group(hwnd)
        .and_then(|id| host_window::with_group_id(hwnd, id, |group| group.hwnd))
        .unwrap_or(hwnd)
}

/// Whether the target group's active tab is a text tab.
fn shows_text(hwnd: HWND) -> bool {
    let Some(id) = target_group(hwnd) else {
        return false;
    };
    unsafe { host_window::app_ptr(hwnd) }.is_some_and(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.group(id)
            .and_then(|group| group.active_document())
            .and_then(|document| tabs.document(document))
            .is_some_and(|document| !document.is_image())
    })
}

/// The target group's active tab's id, language, and folder.
pub(crate) fn active_document(hwnd: HWND) -> Option<(DocumentId, Language, Option<PathBuf>)> {
    group_document(hwnd, target_group(hwnd)?)
}

/// Group `id`'s active tab's id, language, and folder.
pub(crate) fn group_document(
    hwnd: HWND,
    id: GroupId,
) -> Option<(DocumentId, Language, Option<PathBuf>)> {
    let app = unsafe { host_window::app_ptr(hwnd) }?;
    let tabs = &unsafe { app.as_ref() }.tabs;
    let document = tabs.document(tabs.group(id)?.active_document()?)?;
    let folder = document
        .path
        .as_ref()
        .and_then(|path| path.parent().map(PathBuf::from));
    Some((document.id, document.language, folder))
}

pub(crate) fn buttons_visible(hwnd: HWND) -> bool {
    active_document(hwnd)
        .is_some_and(|(_, language, _)| matches!(language, Language::Markdown | Language::Svg))
}

fn active_is_svg(hwnd: HWND) -> bool {
    active_document(hwnd).is_some_and(|(_, language, _)| language == Language::Svg)
}

fn svg_view(hwnd: HWND) -> Option<ImageView> {
    with_host(hwnd, |host| host.svg_view).flatten()
}

pub(crate) fn preview_shown(hwnd: HWND) -> bool {
    mode(hwnd).shows_preview()
        && buttons_visible(hwnd)
        && if active_is_svg(hwnd) {
            svg_view(hwnd).is_some()
        } else {
            view(hwnd).is_some()
        }
}

pub(crate) fn full_view_hwnd(hwnd: HWND) -> Option<HWND> {
    if mode(hwnd) != ViewMode::Preview || !preview_shown(hwnd) {
        return None;
    }
    if active_is_svg(hwnd) {
        svg_view(hwnd).map(|view| view.hwnd())
    } else {
        view(hwnd).map(|view| view.hwnd())
    }
}

fn ensure_svg_view(hwnd: HWND) -> Result<ImageView> {
    if let Some(view) = svg_view(hwnd) {
        return Ok(view);
    }
    let graphics = shared_graphics(hwnd)?;
    let (colors, high_contrast) = image_colors(hwnd);
    let view = ImageView::create(content_parent(hwnd), graphics, colors, high_contrast)?;
    with_host(hwnd, |host| {
        host.svg_view = Some(view);
        host.svg_document = None;
    });
    Ok(view)
}

/// Sends the SVG tab's current text to the SVG preview.
fn load_svg(hwnd: HWND) {
    let (Some(view), Some(editor), Some((id, ..))) =
        (svg_view(hwnd), editor(hwnd), active_document(hwnd))
    else {
        return;
    };
    let previous = with_host(hwnd, |host| host.svg_document.replace(id)).flatten();
    if editor.length().unwrap_or(0) as u64 > crate::preview::svg::MAX_SVG_BYTES {
        view.show_error(crate::image_view::decode::ImageError::SvgTooLarge);
        return;
    }
    let name = unsafe { host_window::app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.document(id)?.title()))
        .unwrap_or_default();
    if let Ok(text) = editor.text() {
        view.show_svg(std::sync::Arc::from(text), &name, previous != Some(id));
    }
}

/// Whether `window` is the Markdown or the SVG preview window.
pub(crate) fn owns_view(hwnd: HWND, window: HWND) -> bool {
    !window.is_null()
        && (view(hwnd).is_some_and(|view| view.hwnd() == window)
            || svg_view(hwnd).is_some_and(|view| view.hwnd() == window))
}

/// Zooms the SVG preview while it has the keyboard focus; false otherwise, so the editor zooms.
pub(crate) fn zoom_svg(hwnd: HWND, command: CommandId) -> bool {
    let Some(view) = svg_view(hwnd).filter(|view| {
        active_is_svg(hwnd) && mode(hwnd).shows_preview() && unsafe { GetFocus() } == view.hwnd()
    }) else {
        return false;
    };
    match command {
        CommandId::ZoomIn => view.zoom_in(),
        CommandId::ZoomOut => view.zoom_out(),
        CommandId::ZoomReset => view.zoom_reset(),
        _ => return false,
    }
    true
}

pub(crate) fn run_command(hwnd: HWND, command: CommandId) {
    let next = match command {
        CommandId::MarkdownViewCycle => {
            let (Some(id), Some((_, language, _))) = (target_group(hwnd), active_document(hwnd))
            else {
                return;
            };
            if !buttons_visible(hwnd) {
                host_window::push_notice(hwnd, NOT_MARKDOWN_NOTICE.to_owned());
                return;
            }
            mode(hwnd).next(|mode| available(hwnd, id, language, mode))
        }
        CommandId::MarkdownViewSource => ViewMode::Source,
        CommandId::MarkdownViewLive => ViewMode::Live,
        CommandId::MarkdownViewSplit => ViewMode::Split,
        CommandId::MarkdownViewPreview => ViewMode::Preview,
        _ => return,
    };
    set_mode(hwnd, next);
}

/// Escape in group `id`'s Preview goes back to the tab's last editing mode.
pub(crate) fn escape(hwnd: HWND, id: GroupId) {
    in_group(id, || {
        if mode(hwnd) == ViewMode::Preview {
            set_mode(hwnd, last_editing(hwnd, id));
        }
    });
}

/// `set_mode` for group `id`.
pub(crate) fn set_group_mode(hwnd: HWND, id: GroupId, next: ViewMode) {
    in_group(id, || set_mode(hwnd, next));
}

/// Shows the target group's active tab in `next` (a menu item, a key, a segment of the floating
/// control), with a notice when it cannot be shown so.
pub(crate) fn set_mode(hwnd: HWND, next: ViewMode) {
    let (Some(id), Some((_, language, _))) = (target_group(hwnd), active_document(hwnd)) else {
        return;
    };
    match (next, language) {
        (ViewMode::Source, _) => {}
        (ViewMode::Live, Language::Markdown) if !available(hwnd, id, language, next) => {
            host_window::push_notice(hwnd, crate::window::live_host::TOO_LARGE_NOTICE.to_owned());
            return;
        }
        (ViewMode::Live, Language::Markdown) => {}
        (ViewMode::Live, _) => {
            host_window::push_notice(
                hwnd,
                crate::window::live_host::NOT_MARKDOWN_NOTICE.to_owned(),
            );
            return;
        }
        (_, Language::Markdown | Language::Svg) => {}
        _ => {
            host_window::push_notice(hwnd, NOT_MARKDOWN_NOTICE.to_owned());
            return;
        }
    }
    let previous = mode(hwnd);
    choose(hwnd, id, next);
    if !next.shows_preview() {
        close_view(hwnd);
    }
    sync_visibility(hwnd);
    let focus = match (previous, next) {
        (_, ViewMode::Preview) => full_view_hwnd(hwnd),
        (ViewMode::Preview, _) => text_hwnd(hwnd),
        _ => None,
    };
    if let Some(target) = focus {
        unsafe { SetFocus(target) };
    }
}

fn close_view(hwnd: HWND) {
    let closed = with_host(hwnd, |host| {
        host.edits = EditLog::default();
        host.document = None;
        host.full_parse_pending = false;
        host.divider = None;
        host.hover_text = None;
        host.svg_document = None;
        host.flush_pending = false;
        (host.view.take(), host.svg_view.take())
    })
    .unwrap_or((None, None));
    stop_timer_when_idle(hwnd);
    let focus = unsafe { GetFocus() };
    let mut had_focus = false;
    if let Some(view) = closed.0 {
        had_focus |= focus == view.hwnd();
        view.destroy();
    }
    if let Some(view) = closed.1 {
        had_focus |= focus == view.hwnd();
        view.destroy();
    }
    if had_focus && let Some(editor) = text_hwnd(hwnd) {
        unsafe { SetFocus(editor) };
    }
    host_window::invalidate_status_bar(hwnd);
}

fn appearance(hwnd: HWND) -> (PreviewColors, PreviewFonts, bool) {
    let theme = host_window::effective_theme(hwnd);
    unsafe { host_window::app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            let high_contrast = app.theme.is_some_and(|system| system.high_contrast);
            let dark = Palette::for_cached_theme(app.theme, app.settings.theme).dark_frame;
            (
                preview_colors(theme, high_contrast),
                PreviewFonts::from_settings(
                    &app.settings.preview_font,
                    &app.settings.font_face,
                    app.settings.preview_font_size,
                    app.settings.preview_line_height,
                ),
                dark,
            )
        })
        .unwrap_or_else(|| {
            (
                preview_colors(theme, false),
                PreviewFonts::from_settings(
                    crate::window::design::faces::current().text,
                    "Consolas",
                    crate::config::defaults::DEFAULT_PREVIEW_FONT_SIZE,
                    crate::config::defaults::DEFAULT_PREVIEW_LINE_HEIGHT,
                ),
                false,
            )
        })
}

pub(crate) fn refresh_appearance(hwnd: HWND) {
    if let Some(view) = view(hwnd) {
        let (colors, fonts, dark) = appearance(hwnd);
        view.set_appearance(colors, fonts, dark);
    }
    if let Some(view) = svg_view(hwnd) {
        let (colors, high_contrast) = image_colors(hwnd);
        view.set_appearance(colors, high_contrast);
    }
}

/// Loads Direct2D and DirectWrite once per window; the Markdown preview, the SVG preview and image
/// tabs share them.
pub(crate) fn shared_graphics(hwnd: HWND) -> Result<Rc<Graphics>> {
    // SAFETY: the App pointer is used only for these field accesses.
    let app = unsafe { host_window::app_ptr(hwnd) };
    if let Some(graphics) = app.and_then(|app| unsafe { app.as_ref() }.graphics.clone()) {
        return Ok(graphics);
    }
    let graphics = Rc::new(Graphics::load()?);
    if let Some(mut app) = app {
        unsafe { app.as_mut() }.graphics = Some(Rc::clone(&graphics));
    }
    Ok(graphics)
}

/// The image view's colours: the preview palette for the current theme, and high contrast.
pub(crate) fn image_colors(hwnd: HWND) -> (PreviewColors, bool) {
    let (colors, ..) = appearance(hwnd);
    let high_contrast = unsafe { host_window::app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .theme
            .is_some_and(|theme| theme.high_contrast)
    });
    (colors, high_contrast)
}

fn ensure_view(hwnd: HWND) -> Result<PreviewView> {
    if let Some(view) = view(hwnd) {
        return Ok(view);
    }
    let started = Instant::now();
    let graphics = shared_graphics(hwnd)?;
    let (colors, fonts, dark) = appearance(hwnd);
    let view = PreviewView::create(content_parent(hwnd), graphics, colors, fonts.clone())?;
    view.set_appearance(colors, fonts, dark);
    view.mark_opened(started);
    with_host(hwnd, |host| {
        host.view = Some(view);
        host.document = None;
    });
    Ok(view)
}

/// Shows or hides the preview and editor for the current mode and active tab, loading the active
/// document when the preview shows something else. Called after mode changes, tab activation or
/// closing, language changes, and file loads.
pub(crate) fn sync_visibility(hwnd: HWND) {
    let svg = active_is_svg(hwnd);
    let markdown = buttons_visible(hwnd) && !svg;
    let target = target_group(hwnd);
    if let Some(id) = target {
        prune_modes(hwnd, id);
    }
    let wanted = mode(hwnd);
    let mut view = view(hwnd);
    if wanted.shows_preview() && markdown && view.is_none() {
        match ensure_view(hwnd) {
            Ok(created) => view = Some(created),
            Err(error) => {
                if let Some(id) = target {
                    choose(hwnd, id, last_editing(hwnd, id));
                }
                host_window::push_notice(
                    hwnd,
                    format!("FastPad could not open the Markdown preview: {error}"),
                );
            }
        }
    }
    let shown = wanted.shows_preview() && markdown && view.is_some();
    if let Some(view) = view {
        view.set_centered(wanted == ViewMode::Preview);
        if !shown {
            with_host(hwnd, |host| {
                host.document = None;
                host.edits = EditLog::default();
                host.full_parse_pending = false;
                host.hover_text = None;
                host.flush_pending = false;
            });
            stop_timer_when_idle(hwnd);
            // A hidden preview keeps only its window and the factories; the document (up to
            // LIVE_UPDATE_LIMIT of text) is parsed again when the preview shows.
            view.release();
        }
    }
    if shown {
        let active = active_document(hwnd).map(|(id, ..)| id);
        if with_host(hwnd, |host| host.document).flatten() != active {
            load_active_document(hwnd, false);
        }
    }
    let mut shown_svg = None;
    if wanted.shows_preview() && svg {
        match ensure_svg_view(hwnd) {
            Ok(view) => shown_svg = Some(view),
            Err(error) => {
                if let Some(id) = target {
                    choose(hwnd, id, last_editing(hwnd, id));
                }
                host_window::push_notice(
                    hwnd,
                    format!("FastPad could not open the SVG preview: {error}"),
                );
            }
        }
    }
    if let Some(view) = svg_view(hwnd) {
        if shown_svg.is_some() {
            unsafe { ShowWindow(view.hwnd(), SW_SHOWNA) };
            let active = active_document(hwnd).map(|(id, ..)| id);
            if with_host(hwnd, |host| host.svg_document).flatten() != active {
                load_svg(hwnd);
            }
        } else {
            with_host(hwnd, |host| host.svg_document = None);
            unsafe { ShowWindow(view.hwnd(), SW_HIDE) };
            view.release();
        }
    }
    let editor_hwnd = editor_hwnd(hwnd);
    let hide_editor = (shown || shown_svg.is_some()) && wanted.hides_text();
    let focus_view = shown_svg
        .map(|view| view.hwnd())
        .or(view.map(|view| view.hwnd()));
    let live_hwnd = target.and_then(|id| crate::window::live_host::view_hwnd(hwnd, id));
    let focus = unsafe { GetFocus() };
    if let (Some(editor), Some(target)) = (editor_hwnd, focus_view)
        && hide_editor
        && (focus == editor || live_hwnd.is_some_and(|live| live == focus))
    {
        unsafe { SetFocus(target) };
    }
    if let Some(view) = view {
        unsafe { ShowWindow(view.hwnd(), if shown { SW_SHOWNA } else { SW_HIDE }) };
    }
    // The text shows in the Live view or the editor; Live decides which.
    if let Some(id) = target
        && shows_text(hwnd)
    {
        crate::window::live_host::show_text(hwnd, id, !hide_editor);
    }
    if let (Some(app), Some(id)) = (unsafe { host_window::app_ptr(hwnd) }, target) {
        let buttons = crate::window::tabs::ViewButtons {
            segments: crate::window::editor_toolbar::segments_for(hwnd, id),
            current: group_mode(hwnd, id),
        };
        unsafe { app.as_ref() }
            .tabs
            .set_view_buttons_in(id, buttons);
    }
    host_window::layout_editor_and_find_bar(hwnd);
    let parent = content_parent(hwnd);
    if parent != hwnd {
        host_window::layout_group(hwnd, parent);
    }
    host_window::invalidate_title_strip(hwnd);
}

pub(crate) fn load_active_document(hwnd: HWND, force: bool) {
    let (Some(view), Some(editor), Some((id, _, folder))) =
        (view(hwnd), editor(hwnd), active_document(hwnd))
    else {
        return;
    };
    let started = Instant::now();
    let mode = mode(hwnd);
    let previous = with_host(hwnd, |host| {
        let previous = host.document.replace(id);
        host.edits = EditLog::default();
        host.flush_pending = false;
        // Any worker parse still running describes older text.
        host.parse_generation += 1;
        host.full_parse_pending = false;
        previous
    })
    .flatten();
    stop_timer_when_idle(hwnd);
    let length = editor.length().unwrap_or(0);
    // Full mode hides the editor, so its top line is stale: reloading the document the preview
    // already shows keeps the preview's own position there.
    let top_line = if mode == ViewMode::Preview && previous == Some(id) {
        view.top_line()
    } else {
        editor_top_line(&editor)
    };
    if length > LIVE_UPDATE_LIMIT && !force {
        view.set_paused(true);
        view.replace_document(PreviewDocument::default(), folder, started);
    } else if length > WORKER_PARSE_THRESHOLD {
        view.set_paused(length > LIVE_UPDATE_LIMIT);
        // Until the worker lands, show nothing rather than the previous document: its blocks,
        // links, and accessibility snapshot must never stand in for this one.
        view.replace_document(PreviewDocument::default(), folder, started);
        spawn_parse(hwnd, &editor, id, started, Some(top_line));
    } else {
        view.set_paused(false);
        view.replace_document(PreviewDocument::default(), folder, started);
        view.reparse(&ScintillaSource(&editor), started);
        view.scroll_to_line(top_line);
    }
}

fn editor_top_line(editor: &Editor) -> usize {
    editor
        .first_visible_line()
        .and_then(|line| editor.doc_line_from_visible(line))
        .unwrap_or(0)
}

/// A finished worker parse, posted to the main window as `WM_FASTPAD_PREVIEW_PARSED`.
pub(crate) struct ParsedPreview {
    /// The group whose preview asked for it.
    group: GroupId,
    document: DocumentId,
    generation: u64,
    parsed: PreviewDocument,
    started: Instant,
    /// Where a Full-mode preview goes when this parse lands; `None` keeps its current position.
    full_mode_line: Option<usize>,
}

/// Copies the text (the worker must not touch Scintilla's buffer) and parses it on a new thread.
/// Bumping the generation retires any parse still running.
fn spawn_parse(
    hwnd: HWND,
    editor: &Editor,
    document: DocumentId,
    started: Instant,
    full_mode_line: Option<usize>,
) -> bool {
    let Ok(text) = editor.text() else {
        return false;
    };
    let Some(group) = target_group(hwnd) else {
        return false;
    };
    let generation = with_host(hwnd, |host| {
        host.parse_generation += 1;
        host.full_parse_pending = true;
        host.parse_generation
    })
    .unwrap_or(0);
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let parsed = PreviewDocument::parse(&text);
        drop(text);
        let payload = Box::into_raw(Box::new(ParsedPreview {
            group,
            document,
            generation,
            parsed,
            started,
            full_mode_line,
        }));
        if unsafe {
            PostMessageW(
                target as HWND,
                WM_FASTPAD_PREVIEW_PARSED,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
    true
}

/// `WM_FASTPAD_PREVIEW_PARSED`: installs a worker parse in the group that asked for it, unless
/// newer text superseded it.
pub(crate) fn parsed(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let payload = unsafe { Box::from_raw(lparam as *mut ParsedPreview) };
    in_group(payload.group, || install_parse(hwnd, *payload));
}

fn install_parse(hwnd: HWND, payload: ParsedPreview) {
    let (current, edits_waiting) = with_host(hwnd, |host| {
        let current = host.view.is_some()
            && host.document == Some(payload.document)
            && host.parse_generation == payload.generation;
        if current {
            host.full_parse_pending = false;
            host.flush_pending |= !host.edits.is_empty();
        }
        (current, !host.edits.is_empty())
    })
    .unwrap_or((false, false));
    let (Some(view), true) = (view(hwnd), current) else {
        return;
    };
    // Edits made while the worker ran were deferred; they apply on top of the parsed snapshot.
    if edits_waiting {
        unsafe { SetTimer(hwnd, PREVIEW_TIMER_ID, PREVIEW_UPDATE_DELAY_MS, None) };
    }
    let folder = active_document(hwnd).and_then(|(_, _, folder)| folder);
    let ParsedPreview {
        parsed,
        started,
        full_mode_line,
        ..
    } = payload;
    // Split follows the editor; Full mode hides the editor, so the preview keeps its own position.
    let line = if mode(hwnd) == ViewMode::Preview {
        Some(full_mode_line.unwrap_or_else(|| view.top_line()))
    } else {
        editor(hwnd).map(|editor| editor_top_line(&editor))
    };
    view.install_parse(parsed, folder, started);
    if let Some(line) = line {
        view.scroll_to_line(line);
    }
}

/// `SCN_MODIFIED` for group `id`'s preview: O(1) bookkeeping only; the timer does the work.
pub(crate) fn record_edit(hwnd: HWND, id: GroupId, notification: &ScintillaNotification) {
    let inserted = notification.modification_type as u32 & SC_MOD_INSERTTEXT != 0;
    let length = notification.length.max(0) as usize;
    let edit = Edit::from_change(
        notification.position.max(0) as usize,
        length,
        inserted,
        notification.lines_added,
    );
    let recorded = with_group_host(hwnd, id, |host| {
        // The SVG preview renders the whole text again after the pause; it keeps no edit log.
        if host.svg_document.is_some() {
            host.flush_pending = true;
            return true;
        }
        if host.view.is_none() || host.document.is_none() {
            return false;
        }
        host.edits.record(edit);
        host.flush_pending = true;
        true
    })
    .unwrap_or(false);
    if recorded {
        unsafe { SetTimer(hwnd, PREVIEW_TIMER_ID, PREVIEW_UPDATE_DELAY_MS, None) };
    }
}

/// What a debounced flush does with the pending edits.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FlushPlan {
    Nothing,
    /// A worker parse is still outstanding: keep these edits for when it lands. Spawning another
    /// would copy the whole text again on every typing pause.
    Defer(Pending),
    /// The document is over `LIVE_UPDATE_LIMIT`: show the refresh bar and drop the edits.
    Pause,
    /// Reparse everything on the UI thread.
    Reparse,
    /// Reparse everything on a worker.
    WorkerParse,
    /// Reparse around the edits; `allow_full_parse` lets that fall back to a full parse inline.
    Incremental {
        edits: Vec<Edit>,
        allow_full_parse: bool,
    },
}

/// A paused preview needs the whole document parsed again. While a worker parse is outstanding the
/// block model is stale, so edits wait for it instead of being applied on top.
pub(crate) fn plan_flush(
    pending: Pending,
    length: usize,
    paused: bool,
    full_parse_pending: bool,
) -> FlushPlan {
    if length > LIVE_UPDATE_LIMIT {
        return FlushPlan::Pause;
    }
    let pending = match pending {
        Pending::Nothing if !paused || full_parse_pending => return FlushPlan::Nothing,
        pending if full_parse_pending => return FlushPlan::Defer(pending),
        _ if paused => Pending::Full,
        pending => pending,
    };
    let large = length > WORKER_PARSE_THRESHOLD;
    match pending {
        Pending::Nothing => FlushPlan::Nothing,
        Pending::Full if large => FlushPlan::WorkerParse,
        Pending::Full => FlushPlan::Reparse,
        Pending::Edits(edits) => FlushPlan::Incremental {
            edits,
            allow_full_parse: !large,
        },
    }
}

/// `WM_TIMER` for `PREVIEW_TIMER_ID`: the typing pause has elapsed. Flushes every group whose
/// preview has edits waiting.
pub(crate) fn flush(hwnd: HWND) {
    unsafe { KillTimer(hwnd, PREVIEW_TIMER_ID) };
    if host_window::input_pending() {
        unsafe { SetTimer(hwnd, PREVIEW_TIMER_ID, PREVIEW_UPDATE_DELAY_MS, None) };
        return;
    }
    let waiting = unsafe { host_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .groups
                .iter()
                .filter(|group| group.preview.flush_pending)
                .map(|group| group.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for id in waiting {
        with_group_host(hwnd, id, |host| host.flush_pending = false);
        in_group(id, || flush_group(hwnd));
    }
}

/// Kills the preview timer unless some group's preview still has edits waiting.
fn stop_timer_when_idle(hwnd: HWND) {
    let waiting = unsafe { host_window::app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .groups
            .iter()
            .any(|group| group.preview.flush_pending)
    });
    if !waiting {
        unsafe { KillTimer(hwnd, PREVIEW_TIMER_ID) };
    }
}

fn flush_group(hwnd: HWND) {
    if active_is_svg(hwnd) {
        if with_host(hwnd, |host| host.svg_document)
            .flatten()
            .is_some()
        {
            load_svg(hwnd);
        }
        return;
    }
    let (Some(view), Some(editor), Some((id, ..))) =
        (view(hwnd), editor(hwnd), active_document(hwnd))
    else {
        return;
    };
    let Some((pending, full_parse_pending)) = with_host(hwnd, |host| {
        (host.document == Some(id)).then(|| (host.edits.take(), host.full_parse_pending))
    })
    .flatten() else {
        return;
    };
    let length = editor.length().unwrap_or(0);
    let started = Instant::now();
    match plan_flush(pending, length, view.is_paused(), full_parse_pending) {
        FlushPlan::Nothing => {}
        FlushPlan::Defer(pending) => {
            with_host(hwnd, |host| {
                host.edits.restore(pending);
                host.flush_pending = true;
            });
            unsafe { SetTimer(hwnd, PREVIEW_TIMER_ID, PREVIEW_UPDATE_DELAY_MS, None) };
        }
        FlushPlan::Pause => view.set_paused(true),
        FlushPlan::Reparse => {
            view.set_paused(false);
            // Retire any worker parse still running: this parse is newer.
            with_host(hwnd, |host| {
                host.parse_generation += 1;
                host.full_parse_pending = false;
            });
            view.reparse(&ScintillaSource(&editor), started);
        }
        FlushPlan::WorkerParse => {
            view.set_paused(false);
            if !spawn_parse(hwnd, &editor, id, started, None) {
                // Keep the work for the next flush instead of dropping it.
                with_host(hwnd, |host| host.edits.request_full());
            }
        }
        FlushPlan::Incremental {
            edits,
            allow_full_parse,
        } => {
            let applied =
                view.apply_edits(&ScintillaSource(&editor), &edits, started, allow_full_parse);
            // `try_apply` declining has already shifted the model's ranges for the edits without
            // reparsing, so nothing may be applied incrementally on top of it: a full parse must
            // come first. The worker parse marks one pending (later edits wait for it); if it
            // cannot start, the edits survive as a full reparse request.
            if applied.is_none() && !spawn_parse(hwnd, &editor, id, started, None) {
                with_host(hwnd, |host| host.edits.request_full());
            }
        }
    }
}

/// `WM_FASTPAD_PREVIEW_REFRESH`: group `id`'s paused bar was clicked.
pub(crate) fn refresh(hwnd: HWND, id: GroupId) {
    in_group(id, || load_active_document(hwnd, true));
}

/// A file finished loading into the active tab: its text replaced whatever the preview showed.
pub(crate) fn document_reloaded(hwnd: HWND) {
    with_host(hwnd, |host| {
        host.document = None;
        host.svg_document = None;
    });
    sync_visibility(hwnd);
}

/// `WM_FASTPAD_PREVIEW_LINK`: a link in group `id`'s preview was clicked or activated with Enter.
pub(crate) fn follow_link(hwnd: HWND, id: GroupId, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let dest = *unsafe { Box::from_raw(lparam as *mut String) };
    in_group(id, || follow(hwnd, dest));
}

fn follow(hwnd: HWND, dest: String) {
    let folder = active_document(hwnd).and_then(|(_, _, folder)| folder);
    match classify_link(&dest, folder.as_deref()) {
        LinkAction::External(url) => {
            if !shell_open(hwnd, &url) {
                host_window::push_notice(hwnd, format!("FastPad could not open {url}."));
            }
        }
        LinkAction::Anchor(anchor) => {
            // A link clicked in the Live view goes to the heading in the editor.
            let live = target_group(hwnd)
                .filter(|&id| crate::window::live_host::view_hwnd(hwnd, id).is_some());
            if let Some(id) = live {
                if !crate::window::live_host::go_to_anchor(hwnd, id, &anchor) {
                    host_window::push_notice(
                        hwnd,
                        format!("No heading in this document matches #{anchor}."),
                    );
                }
                return;
            }
            if !view(hwnd).is_some_and(|view| view.scroll_to_anchor(&anchor)) {
                host_window::push_notice(
                    hwnd,
                    format!("No heading in this document matches #{anchor}."),
                );
            }
        }
        LinkAction::LocalFile(path) => {
            if let Err(error) = crate::window::open_path(hwnd, &path) {
                host_window::report_open_failure(hwnd, &path, &error);
            }
        }
        LinkAction::Wiki { target, heading } => {
            let from = unsafe { host_window::app_ptr(hwnd) }.and_then(|app| {
                let tabs = &unsafe { app.as_ref() }.tabs;
                tabs.document(active_document(hwnd)?.0)?.path.clone()
            });
            if target.is_empty() {
                // `[[#heading]]`: a heading of this note.
                if let Some(heading) = heading
                    && !view(hwnd).is_some_and(|view| {
                        view.scroll_to_anchor(&crate::preview::links::slug(&heading))
                    })
                {
                    host_window::push_notice(
                        hwnd,
                        format!("No heading in this document matches #{heading}."),
                    );
                }
            } else {
                crate::window::library_host::follow_wiki(
                    hwnd,
                    from.as_deref(),
                    &target,
                    heading.as_deref(),
                );
            }
        }
        LinkAction::Tag(tag) => {
            if crate::window::library_host::folder(hwnd).is_some() {
                crate::window::tags_view::reveal(hwnd, &tag);
            } else {
                host_window::push_notice(hwnd, "Open a notebook to browse its tags.".to_owned());
            }
        }
        LinkAction::Ignored => {
            host_window::push_notice(
                hwnd,
                format!("FastPad does not open this kind of link: {dest}"),
            );
        }
    }
}

type ShellExecuteFn = unsafe extern "system" fn(
    HWND,
    *const u16,
    *const u16,
    *const u16,
    *const u16,
    i32,
) -> *mut core::ffi::c_void;

/// `ShellExecuteW`, resolved from shell32.dll on the first external link. A static import would
/// load shell32 into every launch. The module stays loaded for the life of the process, since
/// ShellExecute can leave work running inside it after returning.
fn shell_execute() -> Option<ShellExecuteFn> {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };
    static RESOLVED: OnceLock<Option<usize>> = OnceLock::new();
    let address = RESOLVED.get_or_init(|| {
        let name = wide_null("shell32.dll");
        let module = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return None;
        }
        unsafe { GetProcAddress(module, c"ShellExecuteW".as_ptr().cast()) }
            .map(|proc| proc as usize)
    });
    // SAFETY: the address is the `ShellExecuteW` export, whose signature `ShellExecuteFn` matches.
    address.map(|address| unsafe { std::mem::transmute::<usize, ShellExecuteFn>(address) })
}

pub(crate) fn shell_open(hwnd: HWND, url: &str) -> bool {
    let Some(execute) = shell_execute() else {
        return false;
    };
    let operation = wide_null("open");
    let target = wide_null(url);
    let result = unsafe {
        execute(
            hwnd,
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    result as isize > 32
}

/// `WM_FASTPAD_PREVIEW_HOVER`: the link under the pointer in group `id`'s preview changed.
pub(crate) fn hover_link(hwnd: HWND, id: GroupId, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let dest = *unsafe { Box::from_raw(lparam as *mut Option<String>) };
    with_group_host(hwnd, id, |host| host.hover_text = dest);
    host_window::invalidate_status_bar(hwnd);
}

/// `SCN_UPDATEUI` with a vertical scroll in group `id`'s editor: move its preview to the editor's
/// top line, unless this scroll is the echo of a preview-initiated one.
pub(crate) fn editor_scrolled(hwnd: HWND, id: GroupId) {
    in_group(id, || follow_editor(hwnd));
}

fn follow_editor(hwnd: HWND) {
    // Taken before the mode check so a guard can never outlive the scroll that set it.
    let echo = with_host(hwnd, |host| {
        std::mem::take(&mut host.scroll_origin) == ScrollOrigin::Preview
    })
    .unwrap_or(false);
    if echo || mode(hwnd) != ViewMode::Split || !preview_shown(hwnd) {
        return;
    }
    let (Some(view), Some(editor)) = (view(hwnd), editor(hwnd)) else {
        return;
    };
    if let Ok(line) = editor
        .first_visible_line()
        .and_then(|line| editor.doc_line_from_visible(line))
    {
        view.scroll_to_line(line);
        with_host(hwnd, |host| host.sync_count += 1);
    }
}

/// `WM_FASTPAD_PREVIEW_SCROLLED`: the user scrolled group `id`'s preview; move its editor.
pub(crate) fn preview_scrolled(hwnd: HWND, id: GroupId, line: usize) {
    in_group(id, || follow_preview(hwnd, line));
}

fn follow_preview(hwnd: HWND, line: usize) {
    if mode(hwnd) != ViewMode::Split || !preview_shown(hwnd) {
        return;
    }
    let Some(editor) = editor(hwnd) else {
        return;
    };
    let Ok(target) = editor.visible_from_doc_line(line) else {
        return;
    };
    let before = editor.first_visible_line().ok();
    if before == Some(target) {
        return;
    }
    with_host(hwnd, |host| host.scroll_origin = ScrollOrigin::Preview);
    let _ = editor.set_first_visible_line(target);
    // Scintilla clamps near the end of the document; with no scroll there is no echo to clear the
    // guard, and it would swallow the next real editor scroll.
    let scrolled = editor.first_visible_line().ok() != before;
    with_host(hwnd, |host| {
        if scrolled {
            host.sync_count += 1;
        } else {
            host.scroll_origin = ScrollOrigin::None;
        }
    });
}

/// `WM_FASTPAD_DIAGNOSTIC_PREVIEW` (only under `--diagnostic`).
pub(crate) fn diagnostic(hwnd: HWND, selector: usize) -> isize {
    let view = view(hwnd);
    let stats = view.map(|view| view.stats()).unwrap_or_default();
    match selector {
        0 => match mode(hwnd) {
            ViewMode::Source => 0,
            ViewMode::Split => 1,
            ViewMode::Preview => 2,
            ViewMode::Live => 3,
        },
        1 => stats.block_count as isize,
        2 => stats.revision as isize,
        3 => stats.first_frame_micros as isize,
        4 => stats.last_update_micros as isize,
        5 => view.map_or(0, |view| view.top_line() as isize),
        6 => with_host(hwnd, |host| host.sync_count as isize).unwrap_or(0),
        7 => isize::from(preview_shown(hwnd)),
        _ => -1,
    }
}

/// Positions the preview for `area` and returns where the editor goes.
/// Lays out group `id`'s content area: its editor and its preview beside or over it.
pub(crate) fn layout(hwnd: HWND, id: GroupId, area: RECT, dpi: u32) -> ContentRects {
    let language = group_document(hwnd, id).map(|(_, language, _)| language);
    let previewable = matches!(language, Some(Language::Markdown | Language::Svg));
    let svg = language == Some(Language::Svg);
    let mode = group_mode(hwnd, id);
    let (ratio, slot) = with_group_host(hwnd, id, |host| {
        let slot = if svg {
            host.svg_view.map(|view| view.hwnd())
        } else {
            host.view.map(|view| view.hwnd())
        };
        (host.ratio, slot)
    })
    .unwrap_or((0.5, None));
    let shown = mode.shows_preview() && previewable && slot.is_some();
    let rects = content_rects(area, mode, shown, ratio, dpi);
    with_group_host(hwnd, id, |host| {
        host.area = Some(area);
        host.divider = rects.divider;
    });
    if let (Some(view), Some(rect)) = (slot, rects.preview) {
        unsafe {
            MoveWindow(
                view,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                1,
            )
        };
    }
    rects
}

impl PreviewHost {
    /// The divider between the editor and a side preview, while one shows.
    pub(crate) fn divider_rect(&self) -> Option<RECT> {
        self.divider
    }
}

pub(crate) fn divider_rect(hwnd: HWND) -> Option<RECT> {
    with_host(hwnd, |host| host.divider).flatten()
}

fn over_divider(hwnd: HWND, x: i32, y: i32) -> bool {
    divider_rect(hwnd)
        .is_some_and(|rect| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
}

/// A press at `x`, `y` in `group`'s client area, where the content area and divider are laid out.
pub(crate) fn begin_divider_drag(hwnd: HWND, group: HWND, x: i32, y: i32) -> bool {
    if host_window::menu_mode(hwnd).is_some() || !over_divider(hwnd, x, y) {
        return false;
    }
    let now = unsafe { GetMessageTime() } as u32;
    let double_click_time = unsafe { GetDoubleClickTime() };
    let double = with_host(hwnd, |host| {
        let double = host
            .last_divider_click
            .is_some_and(|last| now.wrapping_sub(last) <= double_click_time);
        host.last_divider_click = if double { None } else { Some(now) };
        if double {
            host.ratio = 0.5;
        } else {
            host.dragging = true;
        }
        double
    })
    .unwrap_or(false);
    if double {
        host_window::layout_editor_and_find_bar(hwnd);
        host_window::invalidate_title_strip(hwnd);
        return true;
    }
    if let Some(view) = view(hwnd) {
        view.set_live_resize(true);
    }
    unsafe { SetCapture(group) };
    true
}

pub(crate) fn drag_divider(hwnd: HWND, x: i32) -> bool {
    let Some(area) = with_host(hwnd, |host| host.dragging.then_some(host.area).flatten()).flatten()
    else {
        return false;
    };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    with_host(hwnd, |host| host.ratio = ratio_for_x(area, x, dpi));
    host_window::layout_editor_and_find_bar(hwnd);
    host_window::invalidate_title_strip(hwnd);
    true
}

/// `WM_CAPTURECHANGED`: capture was taken away mid-drag (task switch, a dialog), so no button-up
/// will arrive.
pub(crate) fn cancel_divider_drag(hwnd: HWND) {
    if with_host(hwnd, |host| std::mem::replace(&mut host.dragging, false)).unwrap_or(false)
        && let Some(view) = view(hwnd)
    {
        view.set_live_resize(false);
    }
}

pub(crate) fn end_divider_drag(hwnd: HWND) -> bool {
    if !with_host(hwnd, |host| std::mem::replace(&mut host.dragging, false)).unwrap_or(false) {
        return false;
    }
    unsafe { ReleaseCapture() };
    if let Some(view) = view(hwnd) {
        view.set_live_resize(false);
    }
    true
}

pub(crate) fn cursor_over_divider(hwnd: HWND, group: HWND) -> bool {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) } == 0 || unsafe { ScreenToClient(group, &mut point) } == 0
    {
        return false;
    }
    with_host(hwnd, |host| host.dragging).unwrap_or(false) || over_divider(hwnd, point.x, point.y)
}

/// The status bar's hint for a preview button, naming `cycle`, the key that cycles the preview
/// modes, when there is one.
/// Copy while a preview has the keyboard focus copies its selection, not the hidden or neighbouring
/// editor's (2026-10-22 preview selection spec §2). True when a preview had the focus, whether or
/// not it had anything selected.
pub(crate) fn copy_focused(hwnd: HWND) -> bool {
    let focus = unsafe { GetFocus() };
    let Some(view) = focused_view(hwnd, focus) else {
        return false;
    };
    view.copy();
    true
}

/// The Markdown preview with the keyboard focus, in any group.
fn focused_view(hwnd: HWND, focus: HWND) -> Option<PreviewView> {
    if focus.is_null() {
        return None;
    }
    let app = unsafe { host_window::app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .groups
        .iter()
        .filter_map(|group| group.preview.view)
        .find(|view| view.hwnd() == focus)
}

/// A preview's context menu (preview selection spec §2): Copy and Select all, and over a link
/// Open link and Copy link address. Menu-only entries reuse command ids, as Live's menu does:
/// Select all is `SelectAllOccurrences`, Open link `NotesFollowLink` and Copy link address
/// `MarkdownLink`.
pub(crate) fn context_menu(
    hwnd: HWND,
    id: GroupId,
    window: HWND,
    request: &crate::preview::view::MenuRequest,
) {
    use crate::window::menus::{self, MenuEntry};
    let Some(view) = with_group_host(hwnd, id, |host| host.view)
        .flatten()
        .filter(|view| view.hwnd() == window)
    else {
        return;
    };
    let mut entries = vec![
        MenuEntry::local("&Copy\tCtrl+C", CommandId::Copy).enabled(view.has_selection()),
        MenuEntry::local("Select &All\tCtrl+A", CommandId::SelectAllOccurrences),
    ];
    if request.link.is_some() {
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::local("&Open link", CommandId::NotesFollowLink),
            MenuEntry::local("Copy link &address", CommandId::MarkdownLink),
        ]);
    }
    // `track_popup` takes main-window client coordinates.
    let mut at = POINT {
        x: request.screen.0,
        y: request.screen.1,
    };
    unsafe { ScreenToClient(hwnd, &mut at) };
    match menus::track_popup(hwnd, &entries, at) {
        Some(CommandId::Copy) => {
            view.copy();
        }
        Some(CommandId::SelectAllOccurrences) => view.select_all(),
        Some(CommandId::NotesFollowLink) => {
            if let Some(dest) = request.link.clone() {
                in_group(id, || follow(hwnd, dest));
            }
        }
        Some(CommandId::MarkdownLink) => {
            if let Some(dest) = &request.link {
                let _ = crate::platform::clipboard::set_text(hwnd, dest);
            }
        }
        _ => {}
    }
}

/// A segment's tooltip and status-bar text: the mode, and its own key or else the cycle key.
fn button_hint(mode: ViewMode, own: Option<&str>, cycle: Option<&str>) -> String {
    let action = match mode {
        ViewMode::Source => "Source",
        ViewMode::Live => "Live",
        ViewMode::Split => "Split: source and preview",
        ViewMode::Preview => "Preview",
    };
    match (own, cycle) {
        (Some(key), _) => format!("{action} ({key})"),
        (None, Some(key)) => format!("{action} ({key} cycles views)"),
        (None, None) => action.to_owned(),
    }
}

/// The command that shows `mode`.
pub(crate) const fn mode_command(mode: ViewMode) -> CommandId {
    match mode {
        ViewMode::Source => CommandId::MarkdownViewSource,
        ViewMode::Live => CommandId::MarkdownViewLive,
        ViewMode::Split => CommandId::MarkdownViewSplit,
        ViewMode::Preview => CommandId::MarkdownViewPreview,
    }
}

/// `mode`'s tooltip and status-bar text. The keys come from the keymap, so a rebound key shows
/// here too.
pub(crate) fn button_text(hwnd: HWND, mode: ViewMode) -> String {
    let own = super::main_window::first_key_text(hwnd, mode_command(mode));
    let cycle = super::main_window::first_key_text(hwnd, CommandId::MarkdownViewCycle);
    button_hint(mode, own.as_deref(), cycle.as_deref())
}

pub(crate) fn button_hover(hwnd: HWND, mode: Option<ViewMode>) {
    let hint = mode.map(|mode| button_text(hwnd, mode));
    let changed = with_host(hwnd, |host| {
        let changed = host.button_hint != hint;
        host.button_hint = hint;
        changed
    })
    .unwrap_or(false);
    if changed {
        host_window::invalidate_status_bar(hwnd);
    }
}

#[cfg(test)]
mod tests;
