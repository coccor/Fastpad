//! Dragging a tab (split editors spec §6): a press on a tab arms a drag; movement past the drag
//! distance starts it, with the capture on the group window, the tab's label following the
//! pointer and the drop overlay over where it would land. `group_drop` decides; this module
//! resolves the pointer, shows the feedback and carries the drop out. Open Editors row drags,
//! which the notebook panel runs, use `target_at`, `show_feedback` and `apply` too.

use crate::document::DocumentId;
use crate::window::drag_label::DragLabel;
use crate::window::drop_overlay::{BAR_ALPHA, DropOverlay, TINT_ALPHA};
use crate::window::group_drop::{self, Action, Source, Target};
use crate::window::main_window::{self, app_ptr};
use crate::window::split_tree::{Direction, GroupId};
use crate::window::titlebar::{Point, scale};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, MapWindowPoints, ScreenToClient};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, IDC_ARROW, IDC_NO, LoadCursorW, MSG, SM_CXDRAG, SM_CYDRAG, SetCursor,
    WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

/// `wParam` bits of a mouse message: the left button and Ctrl are down.
const MK_LBUTTON: u32 = 0x0001;
const MK_CONTROL: u32 = 0x0008;

/// A tab drag: armed by the press, `started` once past the drag distance.
#[derive(Clone, Copy)]
pub(crate) struct TabDrag {
    pub(crate) source: Source,
    /// The group window the press went down in; it has the capture once the drag starts.
    pub(crate) window: HWND,
    /// Where the press went down, in that window's client coordinates.
    pub(crate) origin: (i32, i32),
    pub(crate) started: bool,
    /// What a release where the pointer last was would do.
    pub(crate) action: Option<Action>,
    /// The last pointer position, on the screen, so Ctrl can re-target without a move.
    pub(crate) pointer: POINT,
    pub(crate) label: Option<DragLabel>,
    /// A right press cancelled the drag; the capture stays until its release (amendment 8).
    pub(crate) eat_right_up: bool,
}

impl std::fmt::Debug for TabDrag {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TabDrag")
            .field("source", &self.source)
            .field("started", &self.started)
            .field("action", &self.action)
            .field("eat_right_up", &self.eat_right_up)
            .finish_non_exhaustive()
    }
}

fn with_drag<R>(hwnd: HWND, f: impl FnOnce(&mut Option<TabDrag>) -> R) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    Some(f(&mut unsafe { app.as_mut() }.tab_drag))
}

/// The dragged view as it is now: its index and its group's tab count. `None` once it has
/// closed or left `group`.
pub(crate) fn source_of(hwnd: HWND, group: GroupId, id: DocumentId) -> Option<Source> {
    let app = unsafe { app_ptr(hwnd) }?;
    let tabs = unsafe { app.as_ref() }.tabs.group(group)?;
    let ids = tabs.document_ids();
    Some(Source {
        group,
        document: id,
        index: ids.iter().position(|document| *document == id)?,
        group_len: ids.len(),
    })
}

/// What screen point `point` is over: a group's strip insertion point, or a zone of its content.
/// `None` over the band, a find bar, a caption button or anything outside the groups.
pub(crate) fn target_at(hwnd: HWND, point: POINT) -> Option<Target> {
    let (group, window) = main_window::group_at(hwnd, point)?;
    let mut local = point;
    unsafe { ScreenToClient(window, &mut local) };
    let layout = main_window::strip_layout_of(hwnd, group)?;
    if local.y >= 0 && local.y < layout.height {
        return (local.x < layout.bounds().right).then(|| Target::Strip {
            group,
            index: layout.insertion_index(local.x),
        });
    }
    let content = main_window::with_group_id(hwnd, group, |state| state.content)?;
    content
        .contains(Point::new(local.x, local.y))
        .then(|| Target::Content {
            group,
            zone: group_drop::zone(content, Point::new(local.x, local.y)),
        })
}

/// The screen rectangle and alpha the overlay shows for a drop of `action` on `target`.
fn feedback(hwnd: HWND, target: Target, action: Action) -> Option<(RECT, u8)> {
    let (group, local) = match (target, action) {
        (Target::Strip { group, index }, Action::Reorder { .. } | Action::Place { .. }) => {
            let layout = main_window::strip_layout_of(hwnd, group)?;
            let window = main_window::with_group_id(hwnd, group, |state| state.hwnd)?;
            let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window) }.max(96);
            let x = layout.insertion_x(index);
            let half = scale(2, dpi) / 2;
            (
                group,
                RECT {
                    left: x - half,
                    top: 0,
                    right: x - half + scale(2, dpi).max(1),
                    bottom: layout.height,
                },
            )
        }
        (Target::Content { group, .. }, _) => {
            let content = main_window::with_group_id(hwnd, group, |state| state.content)?;
            let content = RECT {
                left: content.left,
                top: content.top,
                right: content.right,
                bottom: content.bottom,
            };
            let rect = match action {
                Action::Split { direction, .. } => half_towards(content, direction),
                _ => content,
            };
            (group, rect)
        }
        (Target::Strip { .. }, Action::Split { .. }) => return None,
    };
    let window = main_window::with_group_id(hwnd, group, |state| state.hwnd)?;
    let mut corners = [
        POINT {
            x: local.left,
            y: local.top,
        },
        POINT {
            x: local.right,
            y: local.bottom,
        },
    ];
    unsafe { MapWindowPoints(window, std::ptr::null_mut(), corners.as_mut_ptr(), 2) };
    let alpha = if matches!(target, Target::Strip { .. }) {
        BAR_ALPHA
    } else {
        TINT_ALPHA
    };
    Some((
        RECT {
            left: corners[0].x,
            top: corners[0].y,
            right: corners[1].x,
            bottom: corners[1].y,
        },
        alpha,
    ))
}

/// The half of `rect` towards `direction`: where the new group of an edge drop goes.
fn half_towards(rect: RECT, direction: Direction) -> RECT {
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    match direction {
        Direction::Left => RECT {
            right: rect.left + width / 2,
            ..rect
        },
        Direction::Right => RECT {
            left: rect.right - width / 2,
            ..rect
        },
        Direction::Up => RECT {
            bottom: rect.top + height / 2,
            ..rect
        },
        Direction::Down => RECT {
            top: rect.bottom - height / 2,
            ..rect
        },
    }
}

/// Shows the overlay for `action` on `target`, or hides it when the drop would do nothing; sets
/// the arrow or no-drop cursor to match. Call it with nothing of the App borrowed.
pub(crate) fn show_feedback(hwnd: HWND, target: Option<Target>, action: Option<Action>) {
    let wanted = target
        .zip(action)
        .and_then(|(target, action)| feedback(hwnd, target, action));
    let Some((rect, alpha)) = wanted else {
        hide_feedback(hwnd);
        set_cursor(false);
        return;
    };
    set_cursor(true);
    let palette = main_window::current_palette(hwnd);
    let color = if alpha == BAR_ALPHA {
        palette.editor_foreground
    } else {
        palette.selection_background
    };
    let existing = unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.drop_overlay);
    match existing {
        Some(overlay) => overlay.place(rect, color, alpha),
        None => {
            if let Some(overlay) = DropOverlay::show(hwnd, rect, color, alpha)
                && let Some(mut app) = unsafe { app_ptr(hwnd) }
            {
                unsafe { app.as_mut() }.drop_overlay = Some(overlay);
            }
        }
    }
}

/// Destroys the overlay, if one shows. Call it with nothing of the App borrowed.
pub(crate) fn hide_feedback(hwnd: HWND) {
    let overlay =
        unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.drop_overlay.take());
    if let Some(overlay) = overlay {
        overlay.destroy();
    }
}

fn set_cursor(accepted: bool) {
    let cursor = if accepted { IDC_ARROW } else { IDC_NO };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
}

/// A left press on tab `index` of `group`'s strip arms a drag of its view.
pub(crate) fn arm(hwnd: HWND, group: GroupId, window: HWND, index: usize, x: i32, y: i32) {
    let document = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(group)?
            .document_ids()
            .get(index)
            .copied()
    });
    let Some(source) = document.and_then(|id| source_of(hwnd, group, id)) else {
        return;
    };
    with_drag(hwnd, |drag| {
        *drag = Some(TabDrag {
            source,
            window,
            origin: (x, y),
            started: false,
            action: None,
            pointer: POINT { x, y },
            label: None,
            eat_right_up: false,
        })
    });
}

/// `WM_MOUSEMOVE` on `window`: starts an armed drag past the drag distance, then follows the
/// pointer. True while a started drag has the move, so the strip's hover code leaves it alone.
pub(crate) fn mouse_move(hwnd: HWND, window: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some(drag) = with_drag(hwnd, |drag| *drag).flatten() else {
        return false;
    };
    if drag.window != window || drag.eat_right_up {
        return drag.started;
    }
    if buttons & MK_LBUTTON as usize == 0 {
        // The release went elsewhere: a menu, a dialog, another window.
        cancel(hwnd);
        return drag.started;
    }
    if !drag.started {
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        if !crate::window::tree_drag::past_threshold(drag.origin, (x, y), cx, cy) {
            return false;
        }
        start(hwnd, window, x, y);
    }
    let mut screen = POINT { x, y };
    unsafe { ClientToScreen(window, &mut screen) };
    if let Some(label) = with_drag(hwnd, |drag| drag.as_ref().and_then(|drag| drag.label)).flatten()
    {
        label.move_to(screen);
    }
    retarget(hwnd, screen, buttons & MK_CONTROL as usize != 0);
    true
}

/// The drag goes past the drag distance: the capture, the strip's press cleared, the label.
fn start(hwnd: HWND, window: HWND, x: i32, y: i32) {
    let Some(source) = with_drag(hwnd, |drag| {
        let drag = drag.as_mut()?;
        drag.started = true;
        Some(drag.source)
    })
    .flatten() else {
        return;
    };
    main_window::update_strip_pointer(hwnd, source.group, |pointer| {
        pointer.press(None).hover(None)
    });
    unsafe { SetCapture(window) };
    let named = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let document = unsafe { app.as_ref() }.tabs.document(source.document)?;
        let item = crate::window::icon_sets::TreeItem::Note(document.path.as_deref().map_or(
            crate::window::file_icons::NoteKind::Text,
            crate::window::file_icons::note_kind,
        ));
        Some((item, document.title()))
    });
    let Some((item, name)) = named else {
        return;
    };
    let Some(image) = crate::window::notebook_view::tab_label_image(hwnd, window, item, &name)
    else {
        return;
    };
    let mut screen = POINT { x, y };
    unsafe { ClientToScreen(window, &mut screen) };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window) }.max(96);
    let label = DragLabel::show(hwnd, &image, &name, screen, dpi);
    let orphan = with_drag(hwnd, |drag| match drag.as_mut() {
        Some(drag) => {
            drag.label = label;
            None
        }
        None => label,
    })
    .flatten();
    if let Some(orphan) = orphan {
        orphan.destroy();
    }
}

/// Resolves the pointer at screen `point` and shows what a release there would do.
fn retarget(hwnd: HWND, point: POINT, copy: bool) {
    let Some(drag) = with_drag(hwnd, |drag| *drag).flatten() else {
        return;
    };
    let target = target_at(hwnd, point);
    let over_tree = target.is_none()
        && tab_path(hwnd, drag.source.document).is_some_and(|path| {
            super::notebook_view::strip_tab_over(hwnd, point, drag.source.document, &path)
        });
    if target.is_some() || !over_tree {
        super::notebook_view::strip_tab_leave(hwnd);
    }
    let source = source_of(hwnd, drag.source.group, drag.source.document);
    let action = source
        .zip(target)
        .and_then(|(source, target)| group_drop::decide(source, target, copy));
    with_drag(hwnd, |drag| {
        if let Some(drag) = drag.as_mut() {
            drag.pointer = point;
            drag.action = action;
        }
    });
    if over_tree {
        // The tree draws its own band over the folder that takes the file.
        hide_feedback(hwnd);
        set_cursor(true);
        return;
    }
    show_feedback(hwnd, target, action);
}

/// `WM_LBUTTONUP` on `window`: a started drag drops where the button went up. An armed one was
/// a click and is dropped here, leaving the release to the strip. True when a drag was under way.
pub(crate) fn release(hwnd: HWND, window: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some(drag) = with_drag(hwnd, Option::take).flatten() else {
        return false;
    };
    if !drag.started || drag.window != window {
        return false;
    }
    let mut screen = POINT { x, y };
    unsafe { ClientToScreen(window, &mut screen) };
    let target = target_at(hwnd, screen);
    if target.is_none()
        && let Some(path) = tab_path(hwnd, drag.source.document)
        && super::notebook_view::strip_tab_drop(hwnd, screen, drag.source.document, &path)
    {
        end_feedback(hwnd, drag);
        return true;
    }
    let action = source_of(hwnd, drag.source.group, drag.source.document)
        .zip(target)
        .and_then(|(source, target)| {
            group_drop::decide(source, target, buttons & MK_CONTROL as usize != 0)
                .map(|action| (source, action))
        });
    end_feedback(hwnd, drag);
    if let Some((source, action)) = action {
        apply(hwnd, source, action);
    }
    true
}

/// Document `id`'s file, while it has one.
fn tab_path(hwnd: HWND, id: DocumentId) -> Option<std::path::PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.tabs.document(id)?.path.clone()
}

/// Takes the label, the overlay and the capture down after `drag` was taken out of the App.
fn end_feedback(hwnd: HWND, drag: TabDrag) {
    if let Some(label) = drag.label {
        label.destroy();
    }
    hide_feedback(hwnd);
    super::notebook_view::strip_tab_leave(hwnd);
    set_cursor(true);
    if unsafe { GetCapture() } == drag.window {
        unsafe { ReleaseCapture() };
    }
}

/// Ends a drag without dropping: Esc, a lost capture, its group closing. An armed drag just
/// goes. True when a drag was under way.
pub(crate) fn cancel(hwnd: HWND) -> bool {
    let Some(drag) = with_drag(hwnd, Option::take).flatten() else {
        return false;
    };
    if drag.started {
        end_feedback(hwnd, drag);
    }
    drag.started
}

/// A right press cancels a started drag but keeps the capture until its own release reaches the
/// strip, which `right_release` swallows (plan amendment 8). True when a drag was under way.
pub(crate) fn cancel_for_right_press(hwnd: HWND) -> bool {
    let Some(drag) = with_drag(hwnd, |slot| {
        let drag = slot
            .as_mut()
            .filter(|drag| drag.started && !drag.eat_right_up)?;
        drag.eat_right_up = true;
        let label = drag.label.take();
        Some(label)
    })
    .flatten() else {
        return false;
    };
    if let Some(label) = drag {
        label.destroy();
    }
    hide_feedback(hwnd);
    set_cursor(true);
    true
}

/// `WM_RBUTTONUP` after `cancel_for_right_press`: the capture goes, and the release does
/// nothing else. True when it was that release.
pub(crate) fn right_release(hwnd: HWND) -> bool {
    let Some(drag) = with_drag(hwnd, |slot| slot.take_if(|drag| drag.eat_right_up)).flatten()
    else {
        return false;
    };
    if unsafe { GetCapture() } == drag.window {
        unsafe { ReleaseCapture() };
    }
    true
}

/// Keys during a started drag (plan amendment 7): Esc cancels, Ctrl re-targets (move and copy
/// swap), and nothing else reaches the editor until the drag ends. True when the key was taken.
pub(crate) fn keeps_key(hwnd: HWND, message: &MSG) -> bool {
    if !matches!(
        message.message,
        WM_KEYDOWN | WM_KEYUP | WM_CHAR | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_SYSCHAR
    ) {
        return false;
    }
    let Some(drag) = with_drag(hwnd, |drag| *drag)
        .flatten()
        .filter(|drag| drag.started && !drag.eat_right_up)
    else {
        return false;
    };
    let key = message.wParam as u16;
    if message.message == WM_KEYDOWN && key == VK_ESCAPE {
        cancel(hwnd);
    } else if key == VK_CONTROL {
        let copy = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
        retarget(hwnd, drag.pointer, copy);
    }
    true
}

/// Carries out `action` for `source`, if its view is still where the drag found it (a tab can
/// close mid-drag).
pub(crate) fn apply(hwnd: HWND, source: Source, action: Action) {
    let Some(current) = source_of(hwnd, source.group, source.document) else {
        return;
    };
    match action {
        Action::Reorder { group, to, .. } => {
            let reordered = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
                unsafe { app.as_mut() }.tabs.reorder(
                    group,
                    current.index,
                    to.min(current.group_len - 1),
                )
            });
            if reordered {
                main_window::focus_view(hwnd, group, source.document);
                main_window::refresh_tabs(hwnd);
                main_window::focus_content(hwnd);
            }
        }
        Action::Place { group, index, copy } => {
            main_window::place_view(hwnd, source.group, source.document, group, index, copy);
        }
        Action::Split {
            group,
            direction,
            copy,
        } => {
            if let Some(new) = main_window::split_group(hwnd, group, direction) {
                main_window::place_view(hwnd, source.group, source.document, new, None, copy);
            }
        }
    }
}
