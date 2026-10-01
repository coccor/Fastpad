use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

/// Wrap-once-then-stop search progression: bookkeeping only, independent of what actually looks
/// for the query in a given range. `next_range` is a pure, dependency-free reference
/// implementation driven by a plain string, used for unit testing this algorithm; live plain-mode
/// navigation drives the same shape of search via `Editor::search_in_target`, never materializing
/// the full document text. Regex mode doesn't use it (`regex_match`).
#[derive(Debug)]
pub struct SearchState {
    query: String,
    direction: SearchDirection,
    origin: usize,
    cursor: usize,
    wrapped: bool,
}

impl SearchState {
    pub fn new(query: &str, direction: SearchDirection, start: usize) -> Self {
        Self {
            query: query.to_owned(),
            direction,
            origin: start,
            cursor: start,
            wrapped: false,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn direction(&self) -> SearchDirection {
        self.direction
    }

    /// Bounds for a leftmost-match backend (`str::find`/`str::rfind` over a normally-ordered
    /// slice): always `start <= end`. Used only by the pure `next_range` reference below.
    fn str_bounds(&self, len: usize) -> Range<usize> {
        match self.direction {
            SearchDirection::Forward if !self.wrapped => self.cursor..len,
            SearchDirection::Forward => self.cursor..self.origin,
            SearchDirection::Backward if !self.wrapped => 0..self.cursor,
            SearchDirection::Backward => self.origin..self.cursor,
        }
    }

    /// Bounds for `Editor::search_in_target`: Scintilla treats a target range with `start > end`
    /// as a backward search from `start` toward `end`, so `Backward` deliberately builds a
    /// reversed range here (unlike `str_bounds` above, which normal-orders it for `rfind`).
    fn scintilla_bounds(&self, doc_len: usize) -> Range<usize> {
        match self.direction {
            SearchDirection::Forward if !self.wrapped => self.cursor..doc_len,
            SearchDirection::Forward => self.cursor..self.origin,
            SearchDirection::Backward if !self.wrapped => self.cursor..0,
            SearchDirection::Backward => self.cursor..self.origin,
        }
    }

    fn record_match(&mut self, found: Range<usize>) {
        match self.direction {
            SearchDirection::Forward => self.cursor = found.end,
            SearchDirection::Backward => self.cursor = found.start,
        }
    }

    fn record_miss(&mut self, len: usize) {
        if self.wrapped {
            return;
        }
        self.wrapped = true;
        self.cursor = match self.direction {
            SearchDirection::Forward => 0,
            SearchDirection::Backward => len,
        };
    }

    /// Pure reference implementation of the wrap progression over an in-memory string. Never used
    /// for live editor navigation (see the struct docs); exists for testing.
    pub fn next_range(&mut self, haystack: &str) -> Option<Range<usize>> {
        let query = self.query.clone();
        let direction = self.direction;
        let find = |range: Range<usize>| -> Option<Range<usize>> {
            let slice = haystack.get(range.clone())?;
            let relative = match direction {
                SearchDirection::Forward => slice.find(&query),
                SearchDirection::Backward => slice.rfind(&query),
            }?;
            let start = range.start + relative;
            Some(start..start + query.len())
        };

        let bounds = self.str_bounds(haystack.len());
        if let Some(found) = find(bounds) {
            self.record_match(found.clone());
            return Some(found);
        }
        if self.wrapped {
            return None;
        }
        self.record_miss(haystack.len());
        let bounds = self.str_bounds(haystack.len());
        let found = find(bounds)?;
        self.record_match(found.clone());
        Some(found)
    }

    /// Drives the same wrap-once progression against a live Scintilla document via
    /// `Editor::search_in_target`, never materializing the full document text. For plain mode.
    pub(crate) fn next_editor_match(
        &mut self,
        editor: &Editor,
        flags: u32,
        doc_len: usize,
    ) -> crate::Result<Option<Range<usize>>> {
        if self.query.is_empty() {
            return Ok(None);
        }
        let bounds = self.scintilla_bounds(doc_len);
        let found = editor.search_in_target(&self.query, bounds, flags)?;
        if let Some(found) = found {
            self.record_match(found.clone());
            return Ok(Some(found));
        }
        if self.wrapped {
            return Ok(None);
        }
        self.record_miss(doc_len);
        let bounds = self.scintilla_bounds(doc_len);
        let found = editor.search_in_target(&self.query, bounds, flags)?;
        if let Some(found) = &found {
            self.record_match(found.clone());
        }
        Ok(found)
    }
}

use crate::editor::Editor;
use crate::editor::scintilla_constants::{SCFIND_MATCHCASE, SCFIND_NONE, SCFIND_WHOLEWORD};
use crate::search::{MatchOptions, Matcher, SearchOption};

/// The Scintilla search flags for plain mode (spec §8). Regex mode doesn't search with Scintilla
/// but with `Matcher` (`regex_matcher`).
pub(crate) fn search_flags(options: MatchOptions) -> u32 {
    let mut flags = SCFIND_NONE;
    if options.case {
        flags |= SCFIND_MATCHCASE;
    }
    if options.whole_word {
        flags |= SCFIND_WHOLEWORD;
    }
    flags
}

/// The find bar's regex mode: the same `Matcher` Search uses, so a result opens to the match
/// Search showed (spec §8). `None` for a pattern error or a pattern that matches empty text,
/// which the bar shows as its no-match state, as Search shows its pattern error.
pub(crate) fn regex_matcher(query: &str, options: MatchOptions) -> Option<Matcher> {
    Matcher::new(
        query,
        MatchOptions {
            regex: true,
            ..options
        },
    )
    .ok()
}

/// Forward, the first match starting at or after `origin`, else (wrapping once) the first in the
/// text. Backward, the last match ending at or before `origin`, else the last in the text.
pub(crate) fn regex_match(
    matcher: &Matcher,
    text: &str,
    origin: usize,
    direction: SearchDirection,
) -> Option<Range<usize>> {
    match direction {
        SearchDirection::Forward => matcher
            .find_at(text, origin)
            .or_else(|| matcher.find_at(text, 0)),
        SearchDirection::Backward => matcher
            .last_before(text, origin)
            .or_else(|| matcher.last_before(text, text.len())),
    }
}

/// The match of `query` under `options` that find next (or previous) selects from `origin`,
/// wrapping once. Plain mode searches with Scintilla; regex mode runs `Matcher` over the
/// document's text, borrowed without a copy. A pattern error is `None`, a miss.
pub(crate) fn find_in_editor(
    editor: &Editor,
    query: &str,
    options: MatchOptions,
    origin: usize,
    direction: SearchDirection,
) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    if options.regex {
        let matcher = regex_matcher(query, options)?;
        return editor
            .with_document_text(|text| regex_match(&matcher, text, origin, direction))
            .ok()
            .flatten();
    }
    let doc_len = editor.length().ok()?;
    SearchState::new(query, direction, origin)
        .next_editor_match(editor, search_flags(options), doc_len)
        .ok()
        .flatten()
}

/// The text Replace puts in place of `selection` when the selection is exactly a match of
/// `query` under `options`, else `None`, as Replace checks before it replaces the selection.
///
/// - Plain mode: the match is Scintilla's, found where the selection starts, and the text is
///   `replacement` as it is.
/// - Regex mode: the match is one of `Matcher::find_iter`'s over the document, and the text is
///   `replacement` expanded with that match's captures (`$1`, `${name}`, `$$`). Only that match
///   is expanded (`Matcher::replacement_at`), not every match in the document.
pub(crate) fn replacement_for(
    editor: &Editor,
    query: &str,
    replacement: &str,
    options: MatchOptions,
    selection: Range<usize>,
) -> Option<String> {
    if query.is_empty() || selection.is_empty() {
        return None;
    }
    if options.regex {
        let matcher = regex_matcher(query, options)?;
        return editor
            .with_document_text(|text| matcher.replacement_at(text, selection, replacement))
            .ok()
            .flatten();
    }
    let found = editor
        .search_in_target(query, selection.clone(), search_flags(options))
        .ok()
        .flatten();
    (found == Some(selection)).then(|| replacement.to_owned())
}

/// Replaces every match of `query` under `options` as one undo action, and returns how many.
/// Plain mode puts `replacement` in as it is, through Scintilla's search. Regex mode replaces
/// `Matcher`'s matches from the end backwards, each with `replacement` expanded with its own
/// captures (`Matcher::replacements`).
pub(crate) fn replace_all(
    editor: &Editor,
    query: &str,
    replacement: &str,
    options: MatchOptions,
) -> usize {
    if query.is_empty() {
        return 0;
    }
    if !options.regex {
        return editor
            .replace_all(query, replacement, search_flags(options))
            .unwrap_or(0);
    }
    let Some(matcher) = regex_matcher(query, options) else {
        return 0;
    };
    let Ok(edits) = editor.with_document_text(|text| matcher.replacements(text, replacement))
    else {
        return 0;
    };
    editor.replace_ranges_with(&edits).unwrap_or(0)
}

mod count;
pub(crate) use count::{MatchCount, count_matches};

// --- Window integration: native child controls hosting Find/Replace ---

use crate::platform::{last_error, wide_null};
use crate::window::design::metrics::scale;
use crate::window::option_toggles;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, create_panel, fill, inset, text_height};
use crate::window::side_panel::draw_text;
use crate::window::sidebar_accessibility::{self, AccessibleItem, AccessibleSource};
use crate::window::tooltip::Tooltip;
use std::cell::Cell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, EndPaint, HBRUSH, HDC, HFONT, InvalidateRect, PAINTSTRUCT,
    RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RedrawWindow, SelectObject, SetBkColor, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_TOOLBAR;
use windows_sys::Win32::UI::Controls::{
    EM_GETMARGINS, EM_REPLACESEL, EM_SETSEL, EM_UNDO, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
    VK_RETURN, VK_SHIFT,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetParent, GetWindowTextLengthW, GetWindowTextW,
    HWND_TOP, MoveWindow, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW,
    SetWindowPos, SetWindowTextW, ShowWindow, WM_CHAR, WM_CLEAR, WM_CUT, WM_GETFONT, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_PASTE, WM_SETFOCUS,
    WM_SETFONT, WM_SETTEXT, WM_UNDO, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

const BAR_HEIGHT_AT_96_DPI: i32 = 36;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const PADDING_AT_96_DPI: i32 = 6;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
/// The match counter's room inside the query field, left of the toggles: "3 of 1000+" fits.
const COUNTER_WIDTH_AT_96_DPI: i32 = 88;
/// The chevron buttons' glyphs (Segoe MDL2 Assets, as the close button's).
const GLYPH_PREVIOUS: &str = "\u{E70E}";
const GLYPH_NEXT: &str = "\u{E70D}";
const PREVIOUS_TIP: &str = "Previous match (Shift+Enter)";
const NEXT_TIP: &str = "Next match (Enter)";

/// Height of the bar, reserved above the editor whenever it's visible.
pub(crate) const fn find_bar_height(dpi: u32) -> i32 {
    scale(BAR_HEIGHT_AT_96_DPI, dpi)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FindBarMode {
    Find,
    Replace,
}

/// One painted field box and the borderless `Edit` centered inside it, in bar coordinates.
#[derive(Clone, Copy)]
struct FieldLayout {
    field: RECT,
    edit: RECT,
}

/// The two buttons that step through the matches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavButton {
    Previous,
    Next,
}

impl NavButton {
    fn tooltip(self) -> &'static str {
        match self {
            Self::Previous => PREVIOUS_TIP,
            Self::Next => NEXT_TIP,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Previous => "Previous match",
            Self::Next => "Next match",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Previous => GLYPH_PREVIOUS,
            Self::Next => GLYPH_NEXT,
        }
    }
}

/// The bar's parts in bar coordinates: the query field, in Replace mode the replacement field
/// beside it (each half the width), the previous and next match buttons, and a square close
/// button at the right end. `counter` is the match counter's room inside the query field, between
/// its text and the toggles.
struct BarLayout {
    query: FieldLayout,
    replace: Option<FieldLayout>,
    counter: RECT,
    previous: RECT,
    next: RECT,
    close: RECT,
}

fn bar_layout(width: i32, dpi: u32, text_height: i32, mode: FindBarMode) -> BarLayout {
    let padding = scale(PADDING_AT_96_DPI, dpi);
    let field_height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
    let top = (find_bar_height(dpi) - 1 - field_height) / 2;
    let button = |left: i32| RECT {
        left: left.max(0),
        top,
        right: (left + field_height).max(0),
        bottom: top + field_height,
    };
    let close = button(width - padding - field_height);
    let next = button(close.left - field_height);
    let previous = button(next.left - field_height);
    let (query, replace) = field_layouts(previous.left, dpi, text_height, mode, top);
    let counter_left = query.edit.right;
    let counter = RECT {
        left: counter_left,
        top: query.field.top,
        right: (query.field.right - option_toggles::reserved_width(dpi)).max(counter_left),
        bottom: query.field.bottom,
    };
    BarLayout {
        query,
        replace,
        counter,
        previous,
        next,
        close,
    }
}

/// The fields across `width` (the right padding included), starting `top` pixels down the bar.
/// The query field's right end holds the three option toggles (spec §8), and the match counter
/// sits left of them.
fn field_layouts(
    width: i32,
    dpi: u32,
    text_height: i32,
    mode: FindBarMode,
    top: i32,
) -> (FieldLayout, Option<FieldLayout>) {
    let padding = scale(PADDING_AT_96_DPI, dpi);
    let field_height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    let text_height = text_height.clamp(1, (field_height - 2).max(1));
    let toggles = option_toggles::reserved_width(dpi);
    let query_reserve = toggles + scale(COUNTER_WIDTH_AT_96_DPI, dpi);
    // `reserve` is how far the Edit stops short of the field's right edge.
    let field = |left: i32, right: i32, reserve: i32| {
        let field = RECT {
            left,
            top,
            right: right.max(left),
            bottom: top + field_height,
        };
        let edit_top = top + (field_height - text_height) / 2;
        FieldLayout {
            field,
            edit: RECT {
                left: left + inset_x,
                top: edit_top,
                right: (field.right - reserve).max(left + inset_x),
                bottom: edit_top + text_height,
            },
        }
    };
    match mode {
        FindBarMode::Find => (field(padding, width - padding, query_reserve), None),
        FindBarMode::Replace => {
            let half = (width - 3 * padding) / 2;
            (
                field(padding, padding + half, query_reserve),
                // An odd leftover pixel stays at the right edge so both fields match.
                Some(field(2 * padding + half, 2 * padding + 2 * half, inset_x)),
            )
        }
    }
}

/// A painted band in the strip colors hosting two borderless `Edit` controls (query, replacement),
/// shown/hidden/positioned by the main window. Each field sits in a box in the editor's colors,
/// outlined with the accent while it has the focus.
#[derive(Debug)]
pub(crate) struct FindBar {
    panel: HWND,
    query_edit: HWND,
    replace_edit: HWND,
    mode: FindBarMode,
    visible: bool,
    colors: Palette,
    field_brush: HBRUSH,
    close_hovered: Cell<bool>,
    /// Match case, whole word and regex (spec §8). They last for the session, and opening a
    /// Search result replaces them with Search's.
    options: MatchOptions,
    /// The last search found nothing. Cleared when the query changes or a search finds a match.
    no_match: Cell<bool>,
    hovered_toggle: Cell<Option<SearchOption>>,
    hovered_nav: Cell<Option<NavButton>>,
    /// The counter's last count, `None` while there is nothing to show (no query, or not counted
    /// yet). Set by the main window's debounced recount.
    count: Cell<Option<MatchCount>>,
    /// The toggles' and navigation buttons' tooltip, made the first time the pointer moves over the bar.
    tooltip: Cell<Option<Tooltip>>,
    tooltip_failed: Cell<bool>,
    /// The fields' text, kept at each `EN_CHANGE` (`field_changed`), so screen readers read it
    /// without a `WM_GETTEXT` under the App borrow.
    query_value: String,
    replace_value: String,
}

/// What a click released on the bar hit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BarClick {
    Close,
    Toggle(SearchOption),
    Nav(NavButton),
}

/// Text to put in the query field once nothing of the `App` is borrowed. Setting it sends
/// `EN_CHANGE`, and the main window handles that by borrowing the find bar again.
#[must_use = "the text only reaches the field when applied"]
pub(crate) struct PendingText {
    edit: HWND,
    text: String,
}

impl PendingText {
    pub(crate) fn apply(self) {
        set_control_text(self.edit, &self.text);
    }
}

impl FindBar {
    pub(crate) fn create(parent: HWND) -> crate::Result<Self> {
        let panel = create_panel(parent)?;
        let fields = (|| {
            let query_edit = create_edit_child(panel)?;
            let replace_edit = create_edit_child(panel)?;
            // The field hooks call into the main window, whichever window holds the bar.
            let main = crate::platform::win32::root_window(parent);
            install_field_hook(query_edit, main, FindField::Query)?;
            install_field_hook(replace_edit, main, FindField::Replace)?;
            Ok((query_edit, replace_edit))
        })();
        let (query_edit, replace_edit) = match fields {
            Ok(fields) => fields,
            Err(error) => {
                unsafe {
                    DestroyWindow(panel);
                }
                return Err(error);
            }
        };
        let colors = Palette::neutral();
        Ok(Self {
            panel,
            query_edit,
            replace_edit,
            mode: FindBarMode::Find,
            visible: false,
            colors,
            field_brush: unsafe { CreateSolidBrush(colors.editor_background) },
            close_hovered: Cell::new(false),
            options: MatchOptions::default(),
            no_match: Cell::new(false),
            hovered_toggle: Cell::new(None),
            hovered_nav: Cell::new(None),
            count: Cell::new(None),
            tooltip: Cell::new(None),
            tooltip_failed: Cell::new(false),
            query_value: String::new(),
            replace_value: String::new(),
        })
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn owns(&self, hwnd: HWND) -> bool {
        !hwnd.is_null()
            && (hwnd == self.panel || hwnd == self.query_edit || hwnd == self.replace_edit)
    }

    /// Whether `hwnd` is the query field, whose edits clear the no-match state.
    pub(crate) fn is_query(&self, hwnd: HWND) -> bool {
        !hwnd.is_null() && hwnd == self.query_edit
    }

    /// Whether the query field holds text, by the value kept at its last `EN_CHANGE`.
    pub(crate) fn has_query(&self) -> bool {
        !self.query_value.is_empty()
    }

    pub(crate) fn query_text(&self) -> String {
        control_text(self.query_edit)
    }

    pub(crate) fn replace_text(&self) -> String {
        control_text(self.replace_edit)
    }

    /// Keeps `text`, read from `control` with nothing borrowed at its `EN_CHANGE`, as that
    /// field's accessible value.
    pub(crate) fn field_changed(&mut self, control: HWND, text: String) {
        if control == self.query_edit {
            self.query_value = text;
        } else if control == self.replace_edit {
            self.replace_value = text;
        }
    }

    /// The bar's MSAA children, in order: the Find field, the three toggles, the Replace field
    /// in Replace mode, the previous and next match buttons, and the close button.
    pub(crate) fn accessible_items(&self) -> Vec<AccessibleItem> {
        let (layout, dpi) = self.current_layout();
        let focus = unsafe { GetFocus() };
        let mut items = vec![sidebar_accessibility::field_item(
            "Find",
            self.query_value.clone(),
            focus == self.query_edit,
            layout.query.field,
            self.query_edit,
        )];
        let rects = option_toggles::toggle_rects(layout.query.field, dpi);
        for (option, rect) in SearchOption::ALL.into_iter().zip(rects) {
            items.push(sidebar_accessibility::check_item(
                option_toggles::label(option),
                self.options.get(option),
                rect,
            ));
        }
        if let Some(replace) = layout.replace {
            items.push(sidebar_accessibility::field_item(
                "Replace",
                self.replace_value.clone(),
                focus == self.replace_edit,
                replace.field,
                self.replace_edit,
            ));
        }
        for (button, rect) in [
            (NavButton::Previous, layout.previous),
            (NavButton::Next, layout.next),
        ] {
            items.push(sidebar_accessibility::button_item(
                button.label(),
                false,
                false,
                rect,
            ));
        }
        items.push(sidebar_accessibility::button_item(
            "Close",
            false,
            false,
            layout.close,
        ));
        items
    }

    /// Shows the bar in `mode`. The caller applies the returned prefill once it holds no `App`
    /// borrow.
    pub(crate) fn show(
        &mut self,
        mode: FindBarMode,
        prefill: Option<&str>,
        colors: Palette,
    ) -> Option<PendingText> {
        self.set_colors(colors);
        self.mode = mode;
        self.visible = true;
        self.no_match.set(false);
        self.count.set(None);
        unsafe {
            ShowWindow(
                self.replace_edit,
                if mode == FindBarMode::Replace {
                    SW_SHOWNA
                } else {
                    SW_HIDE
                },
            );
        }
        prefill.map(|text| PendingText {
            edit: self.query_edit,
            text: text.to_owned(),
        })
    }

    /// Shows the bar with `query` and `options`, as opening a Search result does (spec §8).
    pub(crate) fn show_with(
        &mut self,
        mode: FindBarMode,
        query: &str,
        options: MatchOptions,
        colors: Palette,
    ) -> PendingText {
        self.options = options;
        let _ = self.show(mode, None, colors);
        PendingText {
            edit: self.query_edit,
            text: query.to_owned(),
        }
    }

    pub(crate) fn options(&self) -> MatchOptions {
        self.options
    }

    pub(crate) fn toggle_option(&mut self, option: SearchOption) {
        self.options = self.options.toggled(option);
        self.no_match.set(false);
        unsafe {
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    #[cfg_attr(not(test), allow(dead_code, reason = "read by the window tests"))]
    pub(crate) fn no_match(&self) -> bool {
        self.no_match.get()
    }

    /// Shows or clears the no-match outline on the query field.
    pub(crate) fn set_no_match(&self, no_match: bool) {
        if self.no_match.replace(no_match) != no_match {
            unsafe {
                InvalidateRect(self.panel, std::ptr::null(), 0);
            }
        }
    }

    /// Shows `count` in the counter, or nothing for `None`.
    pub(crate) fn set_count(&self, count: Option<MatchCount>) {
        if self.count.replace(count) != count {
            let (layout, _) = self.current_layout();
            unsafe {
                InvalidateRect(self.panel, &layout.counter, 0);
            }
        }
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "consumed by the in-process window tests, not the source-linked targets"
    )]
    pub(crate) fn count(&self) -> Option<MatchCount> {
        self.count.get()
    }

    /// The three toggles, in bar coordinates, in `SearchOption::ALL` order.
    pub(crate) fn toggle_rects(&self) -> [RECT; 3] {
        let (layout, dpi) = self.current_layout();
        option_toggles::toggle_rects(layout.query.field, dpi)
    }

    /// Whether the pointer's first move over the bar should make the toggles' tooltip.
    pub(crate) fn wants_tooltip(&self) -> bool {
        self.tooltip.get().is_none() && !self.tooltip_failed.get()
    }

    /// Keeps the tooltip made for the bar. `None` means it couldn't be made, and that is not
    /// tried again.
    pub(crate) fn set_tooltip(&self, tooltip: Option<Tooltip>) {
        self.tooltip.set(tooltip);
        self.tooltip_failed.set(tooltip.is_none());
    }

    /// The previous and next match buttons, in bar coordinates.
    pub(crate) fn nav_rects(&self) -> [(NavButton, RECT); 2] {
        let (layout, _) = self.current_layout();
        [
            (NavButton::Previous, layout.previous),
            (NavButton::Next, layout.next),
        ]
    }

    /// The tooltip and each tool (the three toggles, then previous and next), to set with
    /// nothing of the `App` borrowed.
    pub(crate) fn tooltip_tools(&self) -> Option<(Tooltip, [(RECT, &'static str); 5])> {
        let tooltip = self.tooltip.get()?;
        let toggles = self.toggle_rects();
        let nav = self.nav_rects();
        Some((
            tooltip,
            std::array::from_fn(|index| match index {
                0..3 => (
                    toggles[index],
                    option_toggles::tooltip(SearchOption::ALL[index]),
                ),
                _ => {
                    let (button, rect) = nav[index - 3];
                    (rect, button.tooltip())
                }
            }),
        ))
    }

    pub(crate) fn hide(&mut self) {
        self.visible = false;
        unsafe {
            ShowWindow(self.panel, SW_HIDE);
        }
    }

    /// Recolors for a theme change; the caller repaints with `invalidate`.
    pub(crate) fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        unsafe {
            DeleteObject(self.field_brush);
            self.field_brush = CreateSolidBrush(colors.editor_background);
        }
        self.colors = colors;
    }

    pub(crate) fn invalidate(&self) {
        unsafe {
            RedrawWindow(
                self.panel,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }

    /// Places the bar across `width` from `left`, at `top`, and its fields inside it.
    pub(crate) fn layout(&self, left: i32, width: i32, top: i32, dpi: u32, font: HFONT) {
        if !self.visible {
            return;
        }
        unsafe {
            if !font.is_null() {
                SendMessageW(self.query_edit, WM_SETFONT, font as WPARAM, 0);
                SendMessageW(self.replace_edit, WM_SETFONT, font as WPARAM, 0);
            }
        }
        let BarLayout { query, replace, .. } =
            bar_layout(width, dpi, text_height(self.query_edit, font), self.mode);
        let move_to = |hwnd, rect: RECT| unsafe {
            MoveWindow(
                hwnd,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                1,
            );
        };
        move_to(self.query_edit, query.edit);
        if let Some(replace) = replace {
            move_to(self.replace_edit, replace.edit);
        }
        unsafe {
            SetWindowPos(
                self.panel,
                HWND_TOP,
                left,
                top,
                width.max(0),
                find_bar_height(dpi),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
        // Keeps the tooltip's tools on the toggles and buttons. The tooltip is a control of this
        // thread, and setting its tools calls nothing back in the main window.
        if let Some((tooltip, tools)) = self.tooltip_tools() {
            for (index, (rect, text)) in tools.into_iter().enumerate() {
                tooltip.set_tool(index, rect, text);
            }
        }
    }

    pub(crate) fn focus_query(&self) {
        unsafe {
            SetFocus(self.query_edit);
            select_all(self.query_edit);
        }
    }

    /// `WM_CTLCOLOREDIT` for either field.
    pub(crate) fn control_color(&self, dc: HDC) -> HBRUSH {
        unsafe {
            SetTextColor(dc, self.colors.editor_foreground);
            SetBkColor(dc, self.colors.editor_background);
        }
        self.field_brush
    }

    /// `WM_MOUSEMOVE`, `WM_MOUSELEAVE` and `WM_LBUTTONUP` on the bar: tracks hovering over the
    /// close button and the toggles, and reports a click released on one of them.
    pub(crate) fn pointer(&self, message: u32, lparam: LPARAM) -> Option<BarClick> {
        let (layout, dpi) = self.current_layout();
        let point = POINT {
            x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
            y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
        };
        let leaving = message == WM_MOUSELEAVE;
        let over_close = !leaving && contains(&layout.close, point);
        let over_toggle = if leaving {
            None
        } else {
            option_toggles::hit(
                &option_toggles::toggle_rects(layout.query.field, dpi),
                point,
            )
        };
        let over_nav = if leaving {
            None
        } else {
            [
                (NavButton::Previous, layout.previous),
                (NavButton::Next, layout.next),
            ]
            .into_iter()
            .find(|(_, rect)| contains(rect, point))
            .map(|(button, _)| button)
        };
        if message == WM_MOUSEMOVE {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
        }
        if self.close_hovered.replace(over_close) != over_close {
            unsafe {
                InvalidateRect(self.panel, &layout.close, 0);
            }
        }
        if self.hovered_toggle.replace(over_toggle) != over_toggle {
            unsafe {
                InvalidateRect(self.panel, &layout.query.field, 0);
            }
        }
        if self.hovered_nav.replace(over_nav) != over_nav {
            unsafe {
                InvalidateRect(self.panel, &layout.previous, 0);
                InvalidateRect(self.panel, &layout.next, 0);
            }
        }
        if message != WM_LBUTTONUP {
            return None;
        }
        if over_close {
            Some(BarClick::Close)
        } else if let Some(button) = over_nav {
            Some(BarClick::Nav(button))
        } else {
            over_toggle.map(BarClick::Toggle)
        }
    }

    fn current_layout(&self) -> (BarLayout, u32) {
        let mut client = RECT::default();
        unsafe {
            GetClientRect(self.panel, &mut client);
        }
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(self.panel) }.max(96);
        (bar_layout(client.right, dpi, 0, self.mode), dpi)
    }

    /// `WM_PAINT` for an empty field: its placeholder in the muted color where typed text starts.
    /// Returns false for a field that is not this bar's, which then paints normally.
    pub(crate) fn paint_placeholder(&self, edit: HWND) -> bool {
        let Some(placeholder) = self.placeholder(edit) else {
            return false;
        };
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(edit, &mut paint) };
        if dc.is_null() {
            return true;
        }
        unsafe {
            let mut client = RECT::default();
            GetClientRect(edit, &mut client);
            fill(dc, client, self.colors.editor_background);
            let font = SendMessageW(edit, WM_GETFONT, 0, 0);
            let previous = (font != 0).then(|| SelectObject(dc, font as _));
            // Typed text starts after the Edit's left margin (the low word).
            client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, self.colors.muted_foreground);
            let mut text = placeholder.encode_utf16().collect::<Vec<_>>();
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                text.len() as i32,
                &mut client,
                DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            if let Some(previous) = previous {
                SelectObject(dc, previous);
            }
            EndPaint(edit, &paint);
        }
        true
    }

    /// What an empty field shows, or `None` for a control that is not one of this bar's fields.
    pub(crate) fn placeholder(&self, edit: HWND) -> Option<&'static str> {
        if edit == self.query_edit {
            Some("Find")
        } else if edit == self.replace_edit {
            Some("Replace")
        } else {
            None
        }
    }

    /// `WM_PAINT` for the bar: strip background, a hairline above the editor, each visible
    /// field's box (outlined with the accent while it has the focus), the match counter, the toggles,
    /// and the previous, next and close buttons.
    pub(crate) fn paint_panel(&self, panel: HWND, glyph_font: HFONT, text_font: HFONT) {
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(panel, &mut paint) };
        if dc.is_null() {
            return;
        }
        let mut client = RECT::default();
        unsafe {
            GetClientRect(panel, &mut client);
        }
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
        let colors = self.colors;
        let BarLayout {
            query,
            replace,
            counter,
            previous,
            next,
            close,
        } = bar_layout(client.right, dpi, 0, self.mode);
        let focus = unsafe { GetFocus() };
        unsafe {
            fill(dc, client, colors.strip_background);
            fill(
                dc,
                RECT {
                    top: client.bottom - 1,
                    ..client
                },
                colors.pressed_background,
            );
            for (layout, edit) in [(Some(query), self.query_edit), (replace, self.replace_edit)] {
                let Some(layout) = layout else {
                    continue;
                };
                let outline = if edit == self.query_edit && self.no_match.get() {
                    // A miss is outlined in the Search view's error color.
                    colors.error_foreground
                } else if focus == edit {
                    colors.selection_background
                } else {
                    colors.pressed_background
                };
                fill(dc, layout.field, outline);
                fill(dc, inset(layout.field, 1), colors.editor_background);
            }
            if !text_font.is_null() {
                option_toggles::paint(
                    dc,
                    &option_toggles::toggle_rects(query.field, dpi),
                    self.options,
                    self.hovered_toggle.get(),
                    &colors,
                    text_font,
                    dpi,
                );
            }
            if !text_font.is_null()
                && let Some(count) = self.count.get()
            {
                let color = if count.total == 0 {
                    colors.error_foreground
                } else {
                    colors.muted_foreground
                };
                draw_text(
                    dc,
                    &count.label(),
                    counter,
                    text_font,
                    color,
                    DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
            }
            let hovered_nav = self.hovered_nav.get();
            let buttons = [
                (
                    previous,
                    NavButton::Previous.glyph(),
                    hovered_nav == Some(NavButton::Previous),
                ),
                (
                    next,
                    NavButton::Next.glyph(),
                    hovered_nav == Some(NavButton::Next),
                ),
                (
                    close,
                    crate::window::titlebar::GLYPH_CLOSE,
                    self.close_hovered.get(),
                ),
            ];
            for (rect, _, hovered) in buttons {
                if hovered {
                    fill(dc, rect, colors.hover_background);
                }
            }
            if !glyph_font.is_null() {
                let previous_font = SelectObject(dc, glyph_font as _);
                SetBkMode(dc, TRANSPARENT as i32);
                for (mut rect, glyph, hovered) in buttons {
                    SetTextColor(
                        dc,
                        if hovered {
                            colors.hover_foreground
                        } else {
                            colors.muted_foreground
                        },
                    );
                    let mut glyph = glyph.encode_utf16().collect::<Vec<_>>();
                    DrawTextW(
                        dc,
                        glyph.as_mut_ptr(),
                        glyph.len() as i32,
                        &mut rect,
                        DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX,
                    );
                }
                SelectObject(dc, previous_font);
            }
            EndPaint(panel, &paint);
        }
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "consumed by the source-linked editing integration target"
    )]
    pub(crate) fn query_hwnd(&self) -> HWND {
        self.query_edit
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "consumed by the source-linked editing integration target"
    )]
    pub(crate) fn replace_hwnd(&self) -> HWND {
        self.replace_edit
    }

    pub(crate) fn panel_hwnd(&self) -> HWND {
        self.panel
    }
}

impl Drop for FindBar {
    fn drop(&mut self) {
        // The tooltip's popup is owned by the main window, not by the bar, so it goes by hand.
        if let Some(tooltip) = self.tooltip.get() {
            tooltip.destroy();
        }
        unsafe {
            DeleteObject(self.field_brush);
        }
    }
}

fn contains(rect: &RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

/// The 0-based MSAA child of `option`'s toggle: right after the Find field.
pub(crate) fn toggle_child(option: SearchOption) -> usize {
    1 + SearchOption::ALL
        .iter()
        .position(|shown| *shown == option)
        .unwrap_or(0)
}

/// Runs `f` on the find bar, in whichever group, whose panel is `panel`. Called on the window's own thread
/// (`sidebar_accessibility` sends every query there), under a shared App borrow: `f` reads kept
/// state and sends no messages.
fn with_bar<R>(panel: HWND, f: impl FnOnce(&FindBar) -> R) -> Option<R> {
    let main = crate::platform::win32::root_window(panel);
    let app = unsafe { super::main_window::app_ptr(main) }?;
    let bar = unsafe { app.as_ref() }
        .groups
        .iter()
        .filter_map(|group| group.find_bar.as_ref())
        .find(|bar| bar.panel == panel)?;
    Some(f(bar))
}

fn accessible_container(panel: HWND) -> (String, u32) {
    let replace = with_bar(panel, |bar| bar.mode == FindBarMode::Replace).unwrap_or(false);
    let name = if replace { "Find and replace" } else { "Find" };
    (name.to_owned(), ROLE_SYSTEM_TOOLBAR)
}

fn accessible_count(panel: HWND) -> usize {
    with_bar(panel, |bar| bar.accessible_items().len()).unwrap_or(0)
}

fn accessible_item(panel: HWND, index: usize) -> Option<AccessibleItem> {
    with_bar(panel, |bar| bar.accessible_items().into_iter().nth(index)).flatten()
}

/// The toggles sit inside the Find field, so the last child under the point wins.
fn accessible_hit(panel: HWND, point: POINT) -> Option<usize> {
    with_bar(panel, |bar| {
        bar.accessible_items().iter().rposition(|item| {
            point.x >= item.rect.left
                && point.x < item.rect.right
                && point.y >= item.rect.top
                && point.y < item.rect.bottom
        })
    })
    .flatten()
}

fn accessible_current(_panel: HWND) -> Option<usize> {
    None
}

fn accessible_select(_panel: HWND, _index: usize) {}

/// A field's default action focuses it. A toggle's or the close button's is a click on its
/// center, which `main_window::panel_pointer` handles as the mouse's. Both run with nothing of
/// the App borrowed.
fn accessible_activate(panel: HWND, index: usize) {
    let Some(item) = accessible_item(panel, index) else {
        return;
    };
    if item.window.is_null() {
        sidebar_accessibility::click_item(panel, item.rect);
    } else {
        unsafe {
            SetFocus(item.window);
        }
    }
}

fn accessible_identity(_panel: HWND, index: usize) -> Option<u64> {
    Some(index as u64)
}

fn accessible_generation(_panel: HWND) -> u64 {
    0
}

pub(crate) static FIND_BAR_ACCESSIBLE: AccessibleSource = AccessibleSource {
    container: accessible_container,
    count: accessible_count,
    item: accessible_item,
    hit: accessible_hit,
    current: accessible_current,
    select: accessible_select,
    activate: accessible_activate,
    identity: accessible_identity,
    generation: accessible_generation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FindField {
    Query,
    Replace,
}

struct FindFieldHook {
    parent: HWND,
    field: FindField,
}
const FIND_FIELD_HOOK_ID: usize = 0x4650_4644;

fn install_field_hook(field_hwnd: HWND, parent: HWND, field: FindField) -> crate::Result<()> {
    let data = Rc::into_raw(Rc::new(FindFieldHook { parent, field })) as usize;
    if unsafe { SetWindowSubclass(field_hwnd, Some(find_field_proc), FIND_FIELD_HOOK_ID, data) }
        == 0
    {
        unsafe {
            drop(Rc::from_raw(data as *const FindFieldHook));
        }
        return Err(last_error());
    }
    Ok(())
}

unsafe extern "system" fn find_field_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> isize {
    let raw = ref_data as *const FindFieldHook;
    unsafe {
        Rc::increment_strong_count(raw);
    }
    let hook = unsafe { Rc::from_raw(raw) };
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(find_field_proc), FIND_FIELD_HOOK_ID);
            Rc::decrement_strong_count(raw);
        }
    }
    // A single-line Edit beeps at Enter and Escape characters; both are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b) {
        return 0;
    }
    // Alt+C, Alt+W and Alt+R flip the options while a field has the focus (spec §8). The key
    // down flips; its WM_SYSCHAR is swallowed, so the menu band never sees the letter.
    if let Some(option) = option_toggles::alt_option(message, wparam, lparam) {
        super::main_window::toggle_find_option(hook.parent, option);
        return 0;
    }
    if option_toggles::is_toggle_char(message, wparam, lparam) {
        return 0;
    }
    if message == WM_PAINT
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
        && super::main_window::paint_find_placeholder(hook.parent, hwnd)
    {
        return 0;
    }
    // The Edit repaints only the text it changes; the placeholder must go (or come back) whole.
    let edits_text = matches!(
        message,
        WM_CHAR
            | WM_KEYDOWN
            | WM_PASTE
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_SETTEXT
            | EM_UNDO
            | EM_REPLACESEL
    );
    let was_empty = edits_text && unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if edits_text && was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    match message {
        WM_KEYDOWN => {
            let shift = unsafe { GetAsyncKeyState(VK_SHIFT as i32) } < 0;
            let key = wparam as u16;
            if key == VK_RETURN {
                match hook.field {
                    FindField::Query if shift => super::main_window::find_previous(hook.parent),
                    FindField::Query => super::main_window::find_next(hook.parent),
                    FindField::Replace if shift => {
                        super::main_window::replace_all_matches(hook.parent)
                    }
                    FindField::Replace => super::main_window::replace_current(hook.parent),
                }
            } else if key == VK_ESCAPE {
                super::main_window::close_find_bar(hook.parent);
            }
        }
        // The focused field carries the accent outline the bar paints.
        WM_SETFOCUS | WM_KILLFOCUS => {
            if message == WM_SETFOCUS {
                super::main_window::post_content_focus(hwnd);
            }
            unsafe { InvalidateRect(GetParent(hwnd), std::ptr::null(), 0) };
        }
        _ => {}
    }
    result
}

fn create_edit_child(panel: HWND) -> crate::Result<HWND> {
    create_child(
        panel,
        &wide_null("Edit"),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | (ES_AUTOHSCROLL as u32),
    )
}

pub(crate) fn control_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(copied.max(0) as usize);
        String::from_utf16_lossy(&buffer)
    }
}

fn set_control_text(hwnd: HWND, text: &str) {
    let wide = wide_null(text);
    unsafe {
        SetWindowTextW(hwnd, wide.as_ptr());
    }
}

fn select_all(hwnd: HWND) {
    unsafe {
        SendMessageW(hwnd, EM_SETSEL, 0, -1);
    }
}

#[cfg(test)]
mod tests;
