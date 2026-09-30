//! Link and disclosure targets in the preview: hit testing, hover underlines, activation,
//! `<details>` toggles, posting links to the parent, and keyboard focus between links.

use super::*;

pub(super) fn client_point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam & 0xFFFF) as u16 as i16 as i32,
        ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

pub(super) fn target_at(state: &mut ViewState, x: i32, y: i32) -> Option<(usize, usize)> {
    let scale = dpi_scale(state.hwnd);
    let (view_width, _) = view_size(state.hwnd);
    let (content_left, _) = content_frame(view_width, state.centered);
    let document_y = y as f32 / scale + state.scroll_y - bar_height(state);
    let index = state.heights.index_at(document_y);
    let top = state.heights.top(index);
    let laid = state.layouts.get(index)?.as_ref()?;
    let block_x = x as f32 / scale - content_left;
    let block_y = document_y - top;
    let offset = state.h_scroll.get(&index).copied().unwrap_or(0.0);
    let hit = |target: &Target| {
        target
            .visible_rects(offset)
            .iter()
            .any(|rect| rect.contains(block_x, block_y))
    };
    // A link inside a summary wins over the row's disclosure.
    laid.targets
        .iter()
        .position(|target| matches!(target.kind, TargetKind::Link(_)) && hit(target))
        .or_else(|| laid.targets.iter().position(hit))
        .map(|target| (index, target))
}

pub(super) fn target_dest(state: &ViewState, (block, target): (usize, usize)) -> Option<String> {
    state
        .layouts
        .get(block)?
        .as_ref()?
        .targets
        .get(target)?
        .dest()
        .map(str::to_owned)
}

pub(super) fn set_underline(state: &ViewState, target: Option<(usize, usize)>, underline: bool) {
    if let Some((block, index)) = target
        && let Some(Some(laid)) = state.layouts.get(block)
        && let Some(target) = laid.targets.get(index)
        && matches!(target.kind, TargetKind::Link(_))
    {
        let _ = unsafe { target.layout.SetUnderline(underline, target.range) };
    }
}

/// Follows a link or toggles a section.
pub(super) fn activate(state: &mut ViewState, (block, index): (usize, usize)) {
    let Some(kind) = state
        .layouts
        .get(block)
        .and_then(Option::as_ref)
        .and_then(|laid| laid.targets.get(index))
        .map(|target| target.kind.clone())
    else {
        return;
    };
    match kind {
        TargetKind::Link(dest) => post_link(state.hwnd, dest),
        TargetKind::Disclosure { key, .. } => toggle_details(state, &key),
    }
}

pub(super) fn set_details_open(state: &mut ViewState, key: &DetailsKey, default: bool, open: bool) {
    if open == default {
        state.details_overrides.remove(key);
    } else {
        state.details_overrides.insert(key.clone(), open);
    }
}

/// Expands or collapses a section. Only its top-level block is laid out again, and the block at the
/// top of the view stays where it is.
pub(super) fn toggle_details(state: &mut ViewState, key: &DetailsKey) {
    let found = outline(state)
        .details
        .iter()
        .enumerate()
        .find_map(|(block, keys)| {
            keys.iter()
                .position(|candidate| candidate == key)
                .map(|index| (block, index))
        });
    // A stale key (its summary was edited since) toggles nothing.
    let Some((block, index)) = found else {
        return;
    };
    let default =
        details_open_attribute(&state.document.blocks[block].kind, index).unwrap_or(false);
    let open = state.details_overrides.get(key).copied().unwrap_or(default);
    set_details_open(state, key, default, !open);
    set_underline(state, state.hover, false);
    state.hover = None;
    state.pressed = None;
    let reading = state.heights.anchor(state.scroll_y);
    state.layouts[block] = None;
    if matches!(ensure_layouts(state, &[block]), Ok(true)) {
        state.scroll_y = state.heights.scroll_for_anchor(reading);
    }
    state.state_change = Some(key.clone());
    invalidate(state.hwnd);
}

pub(super) fn post_link(hwnd: HWND, dest: String) {
    let payload = Box::into_raw(Box::new(dest));
    if unsafe {
        PostMessageW(
            crate::platform::win32::root_window(hwnd),
            WM_FASTPAD_PREVIEW_LINK,
            hwnd as usize,
            payload as isize,
        )
    } == 0
    {
        drop(unsafe { Box::from_raw(payload) });
    }
}

pub(super) fn post_hover(hwnd: HWND, dest: Option<String>) {
    let payload = Box::into_raw(Box::new(dest));
    if unsafe {
        PostMessageW(
            crate::platform::win32::root_window(hwnd),
            WM_FASTPAD_PREVIEW_HOVER,
            hwnd as usize,
            payload as isize,
        )
    } == 0
    {
        drop(unsafe { Box::from_raw(payload) });
    }
}

pub(super) fn visible_links(state: &mut ViewState) -> Vec<VisibleLink> {
    let scale = dpi_scale(state.hwnd);
    let (view_width, view_height) = view_size(state.hwnd);
    let (content_left, _) = content_frame(view_width, state.centered);
    let bar = bar_height(state);
    let mut links = Vec::new();
    for index in visible_indices(state, view_height - bar) {
        let top = state.heights.top(index) - state.scroll_y + bar;
        let offset = state.h_scroll.get(&index).copied().unwrap_or(0.0);
        let Some(Some(laid)) = state.layouts.get(index) else {
            continue;
        };
        for (target_index, target) in laid.targets.iter().enumerate() {
            // Targets scrolled out of a wide table's clip are not on screen.
            let Some(rect) = target.visible_rects(offset).first().copied() else {
                continue;
            };
            let rect = rect.offset(content_left, top);
            links.push(VisibleLink {
                text: target.text.clone(),
                dest: target.dest().unwrap_or_default().to_owned(),
                rect: RECT {
                    left: (rect.left * scale) as i32,
                    top: (rect.top * scale) as i32,
                    right: (rect.right * scale) as i32,
                    bottom: (rect.bottom * scale) as i32,
                },
                disclosure: match &target.kind {
                    TargetKind::Link(_) => None,
                    TargetKind::Disclosure { key, expanded } => Some(Disclosure {
                        key: key.clone(),
                        expanded: *expanded,
                    }),
                },
                focused: state.focus == Some((index, target_index)),
            });
        }
    }
    links
}

/// Moves keyboard focus to the next (or previous) link or disclosure in document order and scrolls it into view.
/// Blocks without links are skipped using the model alone; only the block that receives focus is
/// laid out.
pub(super) fn move_focus(state: &mut ViewState, forward: bool) {
    let count = state.document.blocks.len();
    if count == 0 {
        return;
    }
    let (mut block, mut link) = match state.focus {
        Some((block, link)) => (block.min(count - 1), Some(link)),
        None => (state.heights.index_at(state.scroll_y).min(count - 1), None),
    };
    // One extra step lets the search wrap back into the block it started from.
    for _ in 0..=count {
        if block_has_target(&state.document.blocks[block].kind) {
            let anchor = state.heights.anchor(state.scroll_y);
            match ensure_layouts(state, &[block]) {
                Err(_) => return,
                // A block above the reading position changed height: keep the view still.
                Ok(true) => state.scroll_y = state.heights.scroll_for_anchor(anchor),
                Ok(false) => {}
            }
            let links = state.layouts[block]
                .as_ref()
                .map_or(0, |laid| laid.targets.len());
            let next = match (link, forward) {
                (None, true) if links > 0 => Some(0),
                (None, false) if links > 0 => Some(links - 1),
                (Some(current), true) if current + 1 < links => Some(current + 1),
                (Some(current), false) if current > 0 && current <= links => Some(current - 1),
                _ => None,
            };
            if let Some(next) = next {
                focus_link(state, block, next);
                return;
            }
        }
        link = None;
        block = if forward {
            (block + 1) % count
        } else {
            (block + count - 1) % count
        };
    }
}

/// Focuses a laid-out link and reveals it: horizontally inside a wide table's clip, then
/// vertically in the view.
pub(super) fn focus_link(state: &mut ViewState, block: usize, link: usize) {
    state.focus = Some((block, link));
    let Some(laid) = state.layouts.get(block).and_then(Option::as_ref) else {
        return;
    };
    let Some(hit) = laid.targets.get(link) else {
        return;
    };
    let rect = hit.rects.first().copied().unwrap_or_default();
    if hit.scrolls
        && let Some(clip) = hit.clip
    {
        let current = state.h_scroll.get(&block).copied().unwrap_or(0.0);
        let limit = (laid.scroll_width - clip.width()).max(0.0);
        let mut offset = current;
        if rect.right - offset > clip.right {
            offset = rect.right - clip.right;
        }
        if rect.left - offset < clip.left {
            offset = rect.left - clip.left;
        }
        let offset = offset.clamp(0.0, limit);
        if (offset - current).abs() > 0.01 {
            state.h_scroll.insert(block, offset);
        }
    }
    let top = state.heights.top(block);
    let (_, view_height) = view_size(state.hwnd);
    let visible_height = view_height - bar_height(state);
    if top + rect.top < state.scroll_y || top + rect.bottom > state.scroll_y + visible_height {
        set_scroll(state, top + rect.top - visible_height / 3.0, true);
    }
    invalidate(state.hwnd);
}
