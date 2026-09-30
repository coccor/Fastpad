//! Dragging in the Notebook view (tree drag spec §3): arming and starting a row or Open
//! Editors drag, the drag label and cursor, hover and auto-expand, drops, and drags coming from
//! Explorer or from a tab strip.

use super::*;
use crate::document::DocumentId;
use crate::window::drag_label::DragLabel;
use crate::window::sidebar_accessibility::{MK_CONTROL, MK_LBUTTON};
use crate::window::tree_drag::{self, Drag, DragSource};
use std::path::{Path, PathBuf};
use std::time::Instant;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetSystemMetrics, IDC_ARROW, IDC_NO, KillTimer, LoadCursorW, SM_CXDRAG,
    SM_CYDRAG, SetCursor, SetTimer,
};

/// The cursor a drag shows: the arrow over a folder that takes the item, "no" elsewhere
/// (tree drag spec §3.2). Called with nothing of the App borrowed.
pub(super) fn set_drag_cursor(accepted: bool) {
    let cursor = if accepted { IDC_ARROW } else { IDC_NO };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
}

/// Whether a drag is under way (armed does not count): the keyboard is its while it lasts (tree
/// drag spec §3.3).
pub(super) fn drag_started(hwnd: HWND) -> bool {
    with_view(hwnd, |view| {
        view.drag.as_ref().is_some_and(|drag| drag.started)
    })
    .unwrap_or(false)
}

/// Re-targets a started drag from its last pointer position and updates the cursor to match
/// (tree drag spec §3.2, §3.3): after the rows moved under it without the pointer moving — a
/// rebuild, a mouse-wheel scroll — the band and cursor should still match what is now under the
/// pointer. Does nothing without a started drag.
pub(super) fn retarget_drag(hwnd: HWND, now: Instant) {
    let Some((pointer, external)) = with_view(hwnd, |view| {
        view.drag
            .as_ref()
            .filter(|drag| drag.started)
            .map(|drag| (drag.pointer, is_external(&drag.source)))
    })
    .flatten() else {
        return;
    };
    let accepted = with_view(hwnd, |view| view.drag_to(pointer.0, pointer.1, now)).unwrap_or(false);
    // OLE owns the cursor during an Explorer drag.
    if !external {
        set_drag_cursor(accepted);
    }
}

/// Whether a drag of `source` came from outside FastPad, through OLE.
pub(super) fn is_external(source: &DragSource) -> bool {
    matches!(source, DragSource::Files(_) | DragSource::GroupTab { .. })
}

/// Ends a drag's timer, with nothing of the App borrowed. Leaves the capture alone: most cancels
/// release it here too (`end_drag_input`), but a right-press cancel keeps it until its own
/// release reaches the panel (tree drag spec §3.3, §10).
pub(super) fn end_drag_timer(panel: HWND) {
    unsafe {
        KillTimer(panel, DRAG_TIMER);
    }
}

/// Ends a drag's timer, capture and cursor, with nothing of the App borrowed: ReleaseCapture
/// sends WM_CAPTURECHANGED here.
pub(super) fn end_drag_input(panel: HWND) {
    end_drag_timer(panel);
    unsafe {
        if GetCapture() == panel {
            ReleaseCapture();
        }
    }
    set_drag_cursor(true);
}

/// Panel point (`x`, `y`) on the screen.
pub(super) fn screen_point(panel: HWND, x: i32, y: i32) -> POINT {
    let mut point = POINT { x, y };
    unsafe { ClientToScreen(panel, &mut point) };
    point
}

/// Shows the label of the drag that just started, next to panel point (`x`, `y`) (tree drag spec
/// §3.2). Called with nothing of the App borrowed: it makes a window. A label that can't be made
/// leaves the drag without one.
pub(super) fn show_drag_label(hwnd: HWND, panel: HWND, x: i32, y: i32) {
    let paint =
        crate::window::side_panel::view_paint(hwnd, panel, std::ptr::null_mut(), RECT::default());
    let Some((image, name)) = with_view(hwnd, |view| view.drag_label_image(&paint)).flatten()
    else {
        return;
    };
    let pointer = screen_point(panel, x, y);
    let Some(label) = DragLabel::show(hwnd, &image, &name, pointer, paint.dpi) else {
        return;
    };
    match with_view(hwnd, |view| view.drag_label.replace(label)) {
        // A label no drag end took (none is known): it must not stay on screen.
        Some(Some(stale)) => stale.destroy(),
        Some(None) => {}
        None => label.destroy(),
    }
}

/// Moves the drag's label, if it has one, next to panel point (`x`, `y`). Called with nothing of
/// the App borrowed.
pub(super) fn move_drag_label(hwnd: HWND, panel: HWND, x: i32, y: i32) {
    if let Some(label) = with_view(hwnd, |view| view.drag_label).flatten() {
        label.move_to(screen_point(panel, x, y));
    }
}

/// Destroys the drag's label, if it has one: every way a drag ends comes here. Called with
/// nothing of the App borrowed.
pub(super) fn end_drag_label(hwnd: HWND) {
    if let Some(label) = with_view(hwnd, |view| view.drag_label.take()).flatten() {
        label.destroy();
    }
}

/// A press on a row's body or an Open Editors row arms a drag of `source` (tree drag spec §3.1,
/// open editors spec §4.3), unless an inline edit is still open.
pub(super) fn arm_drag(hwnd: HWND, source: DragSource, x: i32, y: i32) {
    if crate::window::inline_name::is_open(hwnd) {
        return;
    }
    with_view(hwnd, |view| view.drag = Drag::armed(source, x, y));
}

/// `WM_MOUSEMOVE` with a drag armed or under way (tree drag spec §3.1, §3.2). False leaves the
/// move to the hover code: no drag, or one that has not started.
pub(super) fn drag_move(hwnd: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some((started, origin, panel)) = with_view(hwnd, |view| {
        view.drag
            .as_ref()
            .map(|drag| (drag.started, drag.origin, view.panel))
    })
    .flatten() else {
        return false;
    };
    if buttons & MK_LBUTTON == 0 {
        // The release went elsewhere: a menu, a dialog, another window.
        if started {
            cancel_drag(hwnd);
        } else {
            with_view(hwnd, |view| view.drag = None);
        }
        return started;
    }
    if !started {
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        if !tree_drag::past_threshold(origin, (x, y), cx, cy) {
            return false;
        }
        with_view(hwnd, |view| {
            if let Some(drag) = view.drag.as_mut() {
                drag.started = true;
            }
            view.list.hover = None;
            view.hover = None;
            view.hover_pin = false;
            view.invalidate();
        });
        unsafe {
            SetCapture(panel);
            SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None);
        }
        show_drag_label(hwnd, panel, x, y);
    } else {
        move_drag_label(hwnd, panel, x, y);
    }
    let accepted = with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false);
    let source = with_view(hwnd, |view| {
        view.drag.as_ref().map(|drag| drag.source.clone())
    })
    .flatten();
    let over_group = !accepted
        && source.is_some_and(|source| {
            group_drop_at(hwnd, panel, &source, x, y, buttons & MK_CONTROL != 0).is_some()
        });
    if accepted {
        crate::window::tab_drag::hide_feedback(hwnd);
    }
    // Last: `show_feedback` sets the cursor too.
    set_drag_cursor(accepted || over_group);
    true
}

/// `WM_LBUTTONUP`: a drag under way drops where the button went up (tree drag spec §3.4). An
/// armed drag was a click. True when a drag was under way.
/// A started Open Editors drag of `source` at panel point `x`, `y`, off the panel: the group
/// drop there, if any, and its feedback (split editors spec §6.2). `None` for any other drag or
/// point, which also hides the overlay.
pub(super) fn group_drop_at(
    hwnd: HWND,
    panel: HWND,
    source: &DragSource,
    x: i32,
    y: i32,
    copy: bool,
) -> Option<(
    crate::window::group_drop::Source,
    crate::window::group_drop::Action,
)> {
    let DragSource::Tab { id, group, .. } = source else {
        crate::window::tab_drag::hide_feedback(hwnd);
        return None;
    };
    let screen = screen_point(panel, x, y);
    let target = crate::window::tab_drag::target_at(hwnd, screen);
    let found = crate::window::tab_drag::source_of(hwnd, *group, *id)
        .zip(target)
        .and_then(|(from, target)| {
            crate::window::group_drop::decide(from, target, copy).map(|action| (from, action))
        });
    crate::window::tab_drag::show_feedback(hwnd, target, found.map(|(_, action)| action));
    found
}

pub(super) fn drag_release(hwnd: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some((drag, panel)) = with_view(hwnd, |view| {
        if view.drag.as_ref().is_some_and(|drag| drag.started) {
            view.drag_to(x, y, Instant::now());
            view.invalidate();
        }
        (view.drag.take(), view.panel)
    }) else {
        return false;
    };
    let Some(drag) = drag.filter(|drag| drag.started) else {
        return false;
    };
    let group_drop = drag
        .target
        .is_none()
        .then(|| group_drop_at(hwnd, panel, &drag.source, x, y, buttons & MK_CONTROL != 0))
        .flatten();
    end_drag_input(panel);
    end_drag_label(hwnd);
    crate::window::tab_drag::hide_feedback(hwnd);
    if let Some((from, action)) = group_drop {
        crate::window::tab_drag::apply(hwnd, from, action);
    }
    if let Some(folder) = drag.target {
        match &drag.source {
            DragSource::Row(kind) => crate::window::tree_move::drop_into(hwnd, kind, &folder),
            DragSource::Tab {
                id,
                path: Some(path),
                ..
            } => {
                crate::window::copy_host::copy_tab_into(hwnd, *id, path, &folder);
            }
            DragSource::Tab { path: None, .. } | DragSource::GroupTab { .. } => {}
            DragSource::Files(_) => {}
        }
    }
    true
}

/// Takes a started drag, invalidating the row it painted over. `None` when there was no drag, or
/// it had not started (an armed one just goes, with nothing left to undo).
pub(super) fn take_started_drag(hwnd: HWND) -> Option<HWND> {
    let (drag, panel) = with_view(hwnd, |view| {
        let drag = view.drag.take();
        if drag.as_ref().is_some_and(|drag| drag.started) {
            view.invalidate();
        }
        (drag, view.panel)
    })?;
    drag.is_some_and(|drag| drag.started).then_some(panel)
}

/// Ends a drag without moving or copying anything (tree drag spec §3.3): Esc, a lost capture, another view,
/// the sidebar hiding, or the dragged row gone. An armed drag just goes. True when a drag was
/// under way.
pub(crate) fn cancel_drag(hwnd: HWND) -> bool {
    let Some(panel) = take_started_drag(hwnd) else {
        return false;
    };
    end_drag_input(panel);
    end_drag_label(hwnd);
    crate::window::tab_drag::hide_feedback(hwnd);
    true
}

/// A right press cancels a drag too, but keeps the capture until its own `WM_RBUTTONUP` reaches
/// the panel (tree drag spec §3.3, spec §10): releasing it immediately would let that release,
/// even over the editor, fall through to `DefWindowProc` there and open its context menu. True
/// when a drag was under way.
pub(super) fn cancel_drag_for_right_press(hwnd: HWND) -> bool {
    let Some(panel) = take_started_drag(hwnd) else {
        return false;
    };
    end_drag_timer(panel);
    end_drag_label(hwnd);
    crate::window::tab_drag::hide_feedback(hwnd);
    set_drag_cursor(true);
    true
}

/// Ends a right press's wait for its own release (`cancel_drag_for_right_press`) and releases
/// the capture kept for it, if it still has it. Every place that drops the wait comes here: a
/// wait dropped without releasing would leave the panel with the mouse until another window
/// took it. No drag or thumb grab owns the capture while the wait lasts: a left press, which
/// starts either, ends the wait first. True when there was a wait. Called with nothing of the
/// App borrowed: ReleaseCapture sends WM_CAPTURECHANGED here.
pub(crate) fn drop_right_release_wait(hwnd: HWND) -> bool {
    let Some((true, panel)) = with_view(hwnd, |view| {
        (std::mem::take(&mut view.eat_right_up), view.panel)
    }) else {
        return false;
    };
    unsafe {
        if GetCapture() == panel {
            ReleaseCapture();
        }
    }
    true
}

/// The drag timer (tree drag spec §3.3): near the list's top or bottom edge the list scrolls,
/// and a collapsed folder the pointer has rested on long enough expands. `now` comes in so the
/// tests need not wait.
pub(crate) fn drag_tick(hwnd: HWND, now: Instant) {
    let Some((scrolled, expand, pointer, external)) = with_view(hwnd, |view| {
        let drag = view.drag.as_ref().filter(|drag| drag.started)?;
        let pointer = drag.pointer;
        let external = is_external(&drag.source);
        let expand = tree_drag::expand_due(drag.resting.as_ref(), now);
        let list = view.list_rect(view.client());
        let lines = tree_drag::scroll_step(pointer.1, list.top, list.bottom, view.list.row_height);
        let scrolled = lines != 0 && view.list.scroll_lines(lines, height(list));
        if scrolled {
            view.invalidate();
        }
        Some((scrolled, expand, pointer, external))
    })
    .flatten() else {
        return;
    };
    if let Some(folder) = &expand {
        with_view(hwnd, |view| {
            if let Some(drag) = view.drag.as_mut() {
                drag.resting = None;
            }
        });
        set_folder_expanded(hwnd, folder, true);
    }
    if scrolled || expand.is_some() {
        let accepted =
            with_view(hwnd, |view| view.drag_to(pointer.0, pointer.1, now)).unwrap_or(false);
        if !external {
            set_drag_cursor(accepted);
        }
    }
}

/// What an Explorer drag at panel point `x`, `y` does (open editors spec §4.1, §4.3): over Open
/// Editors, or with no notebook, it opens (COPY, no highlight); over the tree, the root row or
/// the body, it copies into the folder under it when that folder takes one of `paths` (the
/// band shows); elsewhere nothing. The drag is kept as a started `DragSource::Files` drag with
/// no capture and no label, so the tree drag's band, auto-expand and auto-scroll apply. While a
/// modal dialog runs nothing takes it: its posted drop could not run until the dialog closed.
pub(crate) fn external_over(hwnd: HWND, x: i32, y: i32, paths: &[PathBuf]) -> bool {
    if crate::window::modal::modal_active(hwnd) {
        external_leave(hwnd);
        return false;
    }
    let opens = with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(false);
    if opens {
        external_leave(hwnd);
        return true;
    }
    let started = with_view(hwnd, |view| {
        if !view
            .drag
            .as_ref()
            .is_some_and(|drag| is_external(&drag.source))
        {
            view.drag = Drag::armed(DragSource::Files(paths.to_vec()), x, y).map(|mut drag| {
                drag.started = true;
                drag
            });
            return true;
        }
        false
    })
    .unwrap_or(false);
    if started && let Some(panel) = with_view(hwnd, |view| view.panel) {
        unsafe { SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None) };
    }
    with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false)
}

/// The Explorer drag left the panel or was cancelled: its band and timer go.
pub(crate) fn external_leave(hwnd: HWND) {
    let panel = with_view(hwnd, |view| {
        let external = view
            .drag
            .as_ref()
            .is_some_and(|drag| is_external(&drag.source));
        if external {
            view.drag = None;
            view.invalidate();
        }
        external.then_some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        end_drag_timer(panel);
    }
}

/// An Explorer drop at panel point `x`, `y`: posts what to do and returns at once, so Explorer
/// never waits on a prompt (spec §6). False when nothing here takes it, as while a modal dialog
/// runs.
pub(crate) fn external_drop(hwnd: HWND, x: i32, y: i32, paths: Vec<PathBuf>) -> bool {
    if crate::window::modal::modal_active(hwnd) {
        external_leave(hwnd);
        return false;
    }
    let opens = with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(false);
    let folder = if opens {
        None
    } else {
        with_view(hwnd, |view| view.drag_to(x, y, Instant::now()))
            .filter(|&accepted| accepted)
            .and_then(|_| {
                with_view(hwnd, |view| {
                    view.drag.as_ref().and_then(|drag| drag.target.clone())
                })
                .flatten()
            })
    };
    external_leave(hwnd);
    if !opens && folder.is_none() {
        return false;
    }
    crate::window::copy_host::post_panel_drop(hwnd, paths, folder)
}

/// Screen point `point` in the panel's client coordinates, when it is over the panel.
pub(super) fn panel_point_of(hwnd: HWND, point: POINT) -> Option<(i32, i32)> {
    let panel = with_view(hwnd, |view| view.panel)?;
    // Its own style: the main window of a test is never shown.
    let style = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::GWL_STYLE,
        )
    } as u32;
    if style & windows_sys::Win32::UI::WindowsAndMessaging::WS_VISIBLE == 0 {
        return None;
    }
    let mut local = point;
    unsafe { ScreenToClient(panel, &mut local) };
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    contains(client, local.x, local.y).then_some((local.x, local.y))
}

/// A strip tab dragged to screen point `point` (split editors spec §6.1): over a folder that
/// takes `path`'s file the band shows and true is returned; over Open Editors, or anywhere a
/// copy would do nothing, nothing shows. Kept as a started `DragSource::GroupTab` drag with no
/// capture and no label, as an Explorer drag is.
pub(crate) fn strip_tab_over(hwnd: HWND, point: POINT, id: DocumentId, path: &Path) -> bool {
    let Some((x, y)) = panel_point_of(hwnd, point) else {
        strip_tab_leave(hwnd);
        return false;
    };
    if with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(true) {
        strip_tab_leave(hwnd);
        return false;
    }
    let started = with_view(hwnd, |view| {
        if !view
            .drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.source, DragSource::GroupTab { .. }))
        {
            let source = DragSource::GroupTab {
                id,
                path: path.to_path_buf(),
            };
            view.drag = Drag::armed(source, x, y).map(|mut drag| {
                drag.started = true;
                drag
            });
            return true;
        }
        false
    })
    .unwrap_or(false);
    if started && let Some(panel) = with_view(hwnd, |view| view.panel) {
        unsafe { SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None) };
    }
    with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false)
}

/// The strip tab left the panel, or its drag ended: the band and the timer go.
pub(crate) fn strip_tab_leave(hwnd: HWND) {
    let panel = with_view(hwnd, |view| {
        let ours = view
            .drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.source, DragSource::GroupTab { .. }));
        if ours {
            view.drag = None;
            view.invalidate();
        }
        ours.then_some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        end_drag_timer(panel);
    }
}

/// A strip tab dropped at screen point `point`: the folder under it that takes its file, if any.
/// The tree's band goes either way; the caller ends its drag before it copies, so a "Replace?"
/// question never opens under the drag.
pub(crate) fn strip_tab_drop_folder(
    hwnd: HWND,
    point: POINT,
    id: DocumentId,
    path: &Path,
) -> Option<PathBuf> {
    let accepted = strip_tab_over(hwnd, point, id, path);
    let folder = accepted
        .then(|| {
            with_view(hwnd, |view| {
                view.drag.as_ref().and_then(|drag| drag.target.clone())
            })
        })
        .flatten()
        .flatten();
    strip_tab_leave(hwnd);
    folder
}
