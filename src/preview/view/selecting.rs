//! Text selection in the preview (2026-10-22 preview selection spec §5): hit testing a point to
//! a text point, mouse selection by character, word or run with autoscroll, select all, the
//! context menu request, and copying.

use super::*;
use crate::preview::render::RectF;
use crate::preview::selection::{Clip, Selection, TextPoint, copy, text_runs, word_range};

/// The autoscroll timer while a drag selection is outside the view.
pub(super) const AUTOSCROLL_TIMER: usize = 0x5345_4C31;
pub(super) const AUTOSCROLL_MS: u32 = 30;

/// What a drag extends by: characters, or the whole word or run it started on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unit {
    Character,
    Word,
    Run,
}

/// A press of the left button that may become a selection drag.
#[derive(Clone, Copy, Debug)]
pub(super) struct Press {
    /// Client pixels where it started.
    origin: (i32, i32),
    unit: Unit,
    /// The word or run first selected, in document order; for characters, the press point twice.
    base: (TextPoint, TextPoint),
    /// The pointer moved past the drag threshold (or Shift extended): a click no longer follows
    /// a link.
    pub(super) dragging: bool,
}

/// The click count for double and triple click: the time, place and count of the last press.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Clicks {
    time: u32,
    at: (i32, i32),
    count: u32,
}

/// What a context menu acts on: where to show it and the link it was opened on.
#[derive(Debug)]
pub struct MenuRequest {
    /// Screen coordinates.
    pub screen: (i32, i32),
    pub link: Option<String>,
}

/// Whether block `block`'s `index`-th section is open, given its `open` attribute.
fn section_open(
    state: &ViewState,
    keys: Option<&Vec<DetailsKey>>,
    index: usize,
    attribute: bool,
) -> bool {
    keys.and_then(|keys| keys.get(index))
        .and_then(|key| state.details_overrides.get(key))
        .copied()
        .unwrap_or(attribute)
}

/// Section keys for the sectioned blocks in `blocks`.
fn section_keys(
    state: &mut ViewState,
    blocks: std::ops::RangeInclusive<usize>,
) -> HashMap<usize, Vec<DetailsKey>> {
    let sectioned = blocks
        .filter(|&block| state.sectioned.get(block).copied().unwrap_or(false))
        .collect::<Vec<_>>();
    if sectioned.is_empty() {
        return HashMap::new();
    }
    let outline = outline(state);
    sectioned
        .into_iter()
        .filter_map(|block| Some((block, outline.details.get(block)?.clone())))
        .collect()
}

/// Block `block`'s runs as UTF-16 text.
fn run_units(state: &mut ViewState, block: usize) -> Vec<Vec<u16>> {
    let keys = section_keys(state, block..=block);
    let Some(kind) = state.document.blocks.get(block).map(|block| &block.kind) else {
        return Vec::new();
    };
    text_runs(kind, &mut |index, attribute| {
        section_open(state, keys.get(&block), index, attribute)
    })
    .iter()
    .map(|run| run.text.units())
    .collect()
}

/// The point after the last character of the document.
fn document_end(state: &mut ViewState) -> TextPoint {
    let Some(last) = state.document.blocks.len().checked_sub(1) else {
        return TextPoint::default();
    };
    let runs = run_units(state, last);
    match runs.last() {
        Some(units) => TextPoint::new(last, runs.len() as u32 - 1, units.len() as u32),
        None => TextPoint::new(last, 0, 0),
    }
}

/// The text point nearest client point (`x`, `y`): in the block under it (the first or last
/// block above or below the document), the run containing it or else the nearest run, then the
/// nearest place between two characters.
pub(super) fn point_at(state: &mut ViewState, x: i32, y: i32) -> Option<TextPoint> {
    let count = state.document.blocks.len();
    if count == 0 {
        return None;
    }
    let scale = dpi_scale(state.hwnd);
    let (view_width, _) = view_size(state.hwnd);
    let (content_left, _) = content_frame(view_width, state.centered);
    let document_y = y as f32 / scale + state.scroll_y - bar_height(state);
    if document_y < 0.0 {
        return Some(TextPoint::default());
    }
    if document_y >= state.heights.total() {
        return Some(document_end(state));
    }
    let index = state.heights.index_at(document_y).min(count - 1);
    if state.layouts.get(index).is_some_and(Option::is_none) {
        let _ = ensure_layouts(state, &[index]);
    }
    let block_x = x as f32 / scale - content_left;
    let block_y = document_y - state.heights.top(index);
    let offset = state.h_scroll.get(&index).copied().unwrap_or(0.0);
    let laid = state.layouts.get(index)?.as_ref()?;
    let distance = |rect: &RectF| {
        let dy = (rect.top - block_y).max(block_y - rect.bottom).max(0.0);
        let dx = (rect.left - block_x).max(block_x - rect.right).max(0.0);
        (dy, dx)
    };
    let Some((run_index, run)) = laid.runs.iter().enumerate().min_by(|(_, a), (_, b)| {
        distance(&a.visible_bounds(offset))
            .partial_cmp(&distance(&b.visible_bounds(offset)))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        // A block without text (a rule) sits between the text around it.
        return Some(TextPoint::new(index, 0, 0));
    };
    let shift = if run.scrolls { offset } else { 0.0 };
    let position = crate::preview::dwrite::hit_test_point(
        &run.layout,
        block_x + shift - run.x,
        block_y - run.y,
    )
    .unwrap_or(0)
    .min(run.len);
    Some(TextPoint::new(index, run_index as u32, position))
}

/// Whether client point (`x`, `y`) is over text, for the I-beam cursor.
pub(super) fn over_text(state: &mut ViewState, x: i32, y: i32) -> bool {
    let scale = dpi_scale(state.hwnd);
    let (view_width, _) = view_size(state.hwnd);
    let (content_left, _) = content_frame(view_width, state.centered);
    let document_y = y as f32 / scale + state.scroll_y - bar_height(state);
    if document_y < 0.0 || document_y >= state.heights.total() {
        return false;
    }
    let index = state.heights.index_at(document_y);
    let Some(Some(laid)) = state.layouts.get(index) else {
        return false;
    };
    let block_x = x as f32 / scale - content_left;
    let block_y = document_y - state.heights.top(index);
    let offset = state.h_scroll.get(&index).copied().unwrap_or(0.0);
    laid.runs
        .iter()
        .any(|run| run.visible_bounds(offset).contains(block_x, block_y))
}

/// The word or the whole run around `point`.
fn unit_range(state: &mut ViewState, point: TextPoint, unit: Unit) -> (TextPoint, TextPoint) {
    let units = run_units(state, point.block)
        .into_iter()
        .nth(point.run as usize)
        .unwrap_or_default();
    let (start, end) = match unit {
        Unit::Character => (point.offset, point.offset),
        Unit::Word => word_range(&units, point.offset),
        Unit::Run => (0, units.len() as u32),
    };
    (
        TextPoint::new(point.block, point.run, start),
        TextPoint::new(point.block, point.run, end),
    )
}

/// `WM_LBUTTONDOWN` at client (`x`, `y`): a first click starts a selection there (or, with
/// Shift, extends the current one); a second selects the word; a third the run. Returns whether
/// the press may become a drag, so the caller captures the mouse.
pub(super) fn press(state: &mut ViewState, x: i32, y: i32, shift: bool, time: u32) -> bool {
    let (double_x, double_y) = unsafe {
        (
            windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows_sys::Win32::UI::WindowsAndMessaging::SM_CXDOUBLECLK,
            ),
            windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                windows_sys::Win32::UI::WindowsAndMessaging::SM_CYDOUBLECLK,
            ),
        )
    };
    let near = (x - state.clicks.at.0).abs() <= double_x / 2
        && (y - state.clicks.at.1).abs() <= double_y / 2;
    let quick = time.wrapping_sub(state.clicks.time)
        <= unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
    let count = if near && quick && !shift {
        state.clicks.count % 3 + 1
    } else {
        1
    };
    state.clicks = Clicks {
        time,
        at: (x, y),
        count,
    };
    let Some(point) = point_at(state, x, y) else {
        return false;
    };
    if shift && let Some(selection) = state.selection {
        state.selection = Some(Selection {
            anchor: selection.anchor,
            focus: point,
        });
        state.press = Some(Press {
            origin: (x, y),
            unit: Unit::Character,
            base: (selection.anchor, selection.anchor),
            dragging: true,
        });
        invalidate(state.hwnd);
        return true;
    }
    let unit = match count {
        1 => Unit::Character,
        2 => Unit::Word,
        _ => Unit::Run,
    };
    let base = unit_range(state, point, unit);
    state.selection = Some(Selection {
        anchor: base.0,
        focus: base.1,
    });
    state.press = Some(Press {
        origin: (x, y),
        unit,
        base,
        dragging: unit != Unit::Character,
    });
    invalidate(state.hwnd);
    true
}

/// `WM_MOUSEMOVE` with the button down: past the drag threshold, the selection follows the
/// pointer; outside the view, autoscroll starts. Returns whether autoscroll should run.
pub(super) fn drag(state: &mut ViewState, x: i32, y: i32) -> bool {
    let Some(mut press) = state.press else {
        return false;
    };
    if !press.dragging {
        let (threshold_x, threshold_y) = unsafe {
            (
                windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows_sys::Win32::UI::WindowsAndMessaging::SM_CXDRAG,
                ),
                windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
                    windows_sys::Win32::UI::WindowsAndMessaging::SM_CYDRAG,
                ),
            )
        };
        if (x - press.origin.0).abs() < threshold_x && (y - press.origin.1).abs() < threshold_y {
            return false;
        }
        press.dragging = true;
        state.press = Some(press);
        // A drag that started on a link selects instead of following it.
        state.pressed = None;
    }
    extend(state, press, x, y);
    let mut client = RECT::default();
    unsafe { GetClientRect(state.hwnd, &mut client) };
    y < client.top || y >= client.bottom
}

fn extend(state: &mut ViewState, press: Press, x: i32, y: i32) {
    let Some(point) = point_at(state, x, y) else {
        return;
    };
    let (start, end) = unit_range(state, point, press.unit);
    let selection = if point < press.base.0 {
        Selection {
            anchor: press.base.1,
            focus: start,
        }
    } else {
        Selection {
            anchor: press.base.0,
            focus: end.max(press.base.1),
        }
    };
    if state.selection != Some(selection) {
        state.selection = Some(selection);
        invalidate(state.hwnd);
    }
}

/// An autoscroll tick at client point (`x`, `y`): scrolls toward it, faster the further outside
/// the view it is, and extends the selection to it. Returns whether to keep scrolling.
pub(super) fn autoscroll(state: &mut ViewState, x: i32, y: i32) -> bool {
    let Some(press) = state.press.filter(|press| press.dragging) else {
        return false;
    };
    let mut client = RECT::default();
    unsafe { GetClientRect(state.hwnd, &mut client) };
    let scale = dpi_scale(state.hwnd);
    let outside = if y < client.top {
        (y - client.top) as f32
    } else if y >= client.bottom {
        (y - client.bottom + 1) as f32
    } else {
        return false;
    } / scale;
    let step = outside.signum() * (outside.abs().min(200.0) * 0.5 + 4.0);
    set_scroll(state, state.scroll_y + step, true);
    extend(state, press, x, y);
    true
}

/// The button was released or the capture lost: the drag ends.
pub(super) fn release_press(state: &mut ViewState) -> Option<Press> {
    let press = state.press.take();
    if state
        .selection
        .is_some_and(|selection| selection.is_empty())
    {
        state.selection = None;
    }
    press
}

pub(super) fn select_all(state: &mut ViewState) {
    if state.document.blocks.is_empty() {
        return;
    }
    let end = document_end(state);
    state.selection = Some(Selection {
        anchor: TextPoint::default(),
        focus: end,
    })
    .filter(|selection| !selection.is_empty());
    invalidate(state.hwnd);
}

pub(super) fn clear(state: &mut ViewState) -> bool {
    let had = state.selection.take().is_some();
    state.press = None;
    if had {
        invalidate(state.hwnd);
    }
    had
}

/// The selection's text as plain text and HTML, read from the model.
pub(super) fn clip(state: &mut ViewState) -> Option<Clip> {
    let selection = state.selection.filter(|selection| !selection.is_empty())?;
    let (start, end) = selection.ordered();
    let keys = section_keys(
        state,
        start.block..=end.block.min(state.document.blocks.len().saturating_sub(1)),
    );
    let state = &*state;
    Some(copy(
        &state.document.blocks,
        &selection,
        &mut |block, index, attribute| section_open(state, keys.get(&block), index, attribute),
    ))
}

/// The context menu request for client point (`x`, `y`), or for the keyboard (`None`): at the
/// selection's start when it is on screen, else at the view's top-left corner.
pub(super) fn menu_request(state: &mut ViewState, at: Option<(i32, i32)>) -> MenuRequest {
    let client = at.unwrap_or_else(|| {
        let scale = dpi_scale(state.hwnd);
        (
            (8.0 * scale) as i32,
            ((8.0 + bar_height(state)) * scale) as i32,
        )
    });
    let link = at
        .and_then(|(x, y)| target_at(state, x, y))
        .and_then(|target| target_dest(state, target));
    let mut screen = windows_sys::Win32::Foundation::POINT {
        x: client.0,
        y: client.1,
    };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(state.hwnd, &mut screen) };
    MenuRequest {
        screen: (screen.x, screen.y),
        link,
    }
}

/// Client pixels of text point `point`, a little inside its line, for tests that click there.
#[cfg(test)]
pub(super) fn client_of(state: &mut ViewState, point: TextPoint) -> Option<(i32, i32)> {
    let _ = ensure_layouts(state, &[point.block]);
    let scale = dpi_scale(state.hwnd);
    let (view_width, _) = view_size(state.hwnd);
    let (content_left, _) = content_frame(view_width, state.centered);
    let offset = state.h_scroll.get(&point.block).copied().unwrap_or(0.0);
    let top = state.heights.top(point.block) - state.scroll_y + bar_height(state);
    let run = state
        .layouts
        .get(point.block)?
        .as_ref()?
        .runs
        .get(point.run as usize)?;
    let (mut x, mut y) = (0.0, 0.0);
    let mut hit = windows::Win32::Graphics::DirectWrite::DWRITE_HIT_TEST_METRICS::default();
    unsafe {
        run.layout
            .HitTestTextPosition(point.offset, false, &mut x, &mut y, &mut hit)
    }
    .ok()?;
    let shift = if run.scrolls { offset } else { 0.0 };
    Some((
        ((content_left + run.x - shift + x + 1.0) * scale) as i32,
        ((top + run.y + y + hit.height / 2.0) * scale) as i32,
    ))
}
