//! The Notebook view's input: the panel's message handler, pointer moves and presses,
//! tooltips, focus, activation of rows and header buttons, and keys and type-ahead.

use super::*;
use crate::library::tree::{self, RowKind, TreeRow};
use crate::window::commands::CommandId;
use crate::window::main_window::OpenMode;
use crate::window::menus::MenuEntry;
use crate::window::notebook_layout::{self};
use crate::window::panel_cursor::{self, Cursor};
use crate::window::row_list::{self, ListKey};
use crate::window::side_panel::point_of;
use crate::window::tooltip::Tooltip;
use crate::window::tree_drag::DragSource;
use std::path::Path;
use std::time::Instant;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, VK_DELETE, VK_F2, VK_LEFT, VK_RETURN, VK_RIGHT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, WM_CAPTURECHANGED, WM_CHAR, WM_CONTEXTMENU, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
};

/// The panel's input while the Notebook view is shown (`side_panel::view_mouse` and `view_key`).
/// `None` leaves the message to `DefWindowProcW`. The panel handles its resize edge itself.
pub(crate) fn handle(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_MOUSEMOVE => {
            let (x, y) = point_of(lparam);
            if !drag_move(hwnd, x, y, wparam) {
                mouse_move(hwnd, x, y);
            }
            Some(0)
        }
        // The panel class has CS_DBLCLKS: the second press of a double-click comes as this.
        WM_LBUTTONDBLCLK => {
            let (x, y) = point_of(lparam);
            double_click(hwnd, x, y);
            Some(0)
        }
        WM_MOUSELEAVE => {
            with_view(hwnd, |view| {
                view.tracking_leave = false;
                view.list.hover = None;
                view.editors.list.hover = None;
                view.editors.hover_close = false;
                view.hover = None;
                view.hover_pin = false;
                view.invalidate();
            });
            Some(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            left_down(hwnd, x, y);
            Some(0)
        }
        WM_MBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            let pressed = with_view(hwnd, |view| match view.hit_test(x, y) {
                Hit::Editor { index, .. } => view.editors.row(index).map(|row| row.id),
                _ => None,
            })
            .flatten();
            with_view(hwnd, |view| view.middle_press = pressed);
            Some(0)
        }
        WM_MBUTTONUP => {
            let (x, y) = point_of(lparam);
            let pressed = with_view(hwnd, |view| view.middle_press.take()).flatten();
            let released = with_view(hwnd, |view| match view.hit_test(x, y) {
                Hit::Editor { index, .. } => view.editors.row(index).map(|row| (row.id, row.group)),
                _ => None,
            })
            .flatten();
            if let Some((id, group)) = released.filter(|(id, _)| pressed == Some(*id)) {
                // The row's own group loses the tab.
                crate::window::main_window::activate_group(hwnd, group);
                crate::window::main_window::close_document_tab(hwnd, id);
            }
            Some(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_of(lparam);
            if drag_release(hwnd, x, y, wparam) {
                return Some(0);
            }
            // Released after the borrow ends: ReleaseCapture sends WM_CAPTURECHANGED here.
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| {
                view.thumb_grab = None;
                // Taken by someone else before our own release arrived: nothing to eat now.
                view.eat_right_up = false;
            });
            // Capture taken away mid-drag (a task switch, a dialog): nothing moves.
            cancel_drag(hwnd);
            Some(0)
        }
        WM_RBUTTONDOWN => {
            // A right press cancels a drag and does nothing else (tree drag spec §3.3). The
            // capture stays until its own release reaches the panel (spec §10).
            if cancel_drag_for_right_press(hwnd) {
                with_view(hwnd, |view| view.eat_right_up = true);
                return Some(0);
            }
            // An earlier cancel's release may never have come here (it went to another window):
            // this press is an ordinary one, so it opens the menu as usual.
            drop_right_release_wait(hwnd);
            // Selects the row; DefWindowProc turns the button-up into WM_CONTEXTMENU.
            let (x, y) = point_of(lparam);
            let hit = hit_after_commit(hwnd, x, y);
            focus_panel_for(hwnd, hit.as_ref());
            if let Some(Hit::Row { index, .. }) = hit {
                with_view(hwnd, |view| {
                    view.cursor = Cursor::Tree;
                    view.select(index);
                });
            }
            Some(0)
        }
        WM_RBUTTONUP => {
            // The release of a right press that cancelled a drag opens no menu; its capture,
            // kept until now, is released here (spec §10).
            drop_right_release_wait(hwnd).then_some(0)
        }
        WM_CONTEXTMENU => {
            context_menu(hwnd, lparam);
            Some(0)
        }
        WM_TIMER if wparam == DRAG_TIMER => {
            drag_tick(hwnd, Instant::now());
            Some(0)
        }
        // A started drag owns the keyboard until it ends: Esc already cancels it, before this
        // (side_panel routes it to cancel_drag first), so nothing here needs to (tree drag spec
        // §3.3).
        WM_KEYDOWN if drag_started(hwnd) => Some(0),
        WM_CHAR if drag_started(hwnd) => Some(0),
        WM_KEYDOWN => key_down(hwnd, wparam as u16).then_some(0),
        WM_CHAR => {
            let ch = char::from_u32(wparam as u32).filter(|ch| !ch.is_control())?;
            typed(hwnd, ch);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            // Over the Open Editors rows the section scrolls; anywhere else the tree does.
            let mut pointer = POINT::default();
            unsafe { GetCursorPos(&mut pointer) };
            let scrolled = with_view(hwnd, |view| {
                unsafe { ScreenToClient(view.panel, &mut pointer) };
                let editors = view.layout(view.client(), view.dpi()).editors_list;
                if contains(editors, pointer.x, pointer.y) {
                    if view.editors.list.wheel(delta, lines, height(editors)) {
                        view.invalidate();
                    }
                    return false;
                }
                let height = view.list_height();
                let scrolled = view.list.wheel(delta, lines, height);
                if scrolled {
                    view.invalidate();
                }
                scrolled
            })
            .unwrap_or(false);
            // The field moves with its row (inline naming spec §5.4).
            if scrolled {
                crate::window::inline_name::place(hwnd);
                // A started drag's band and cursor follow the rows a wheel scroll moved under
                // its pointer (tree drag spec §3.2, §3.3).
                retarget_drag(hwnd, Instant::now());
            }
            Some(0)
        }
        _ => None,
    }
}

pub(super) fn mouse_move(hwnd: HWND, x: i32, y: i32) {
    // Read before the view is borrowed: `ui_fonts` borrows the App itself.
    let fonts = crate::window::main_window::ui_fonts(hwnd);
    let (tools, scrolled) = with_view(hwnd, |view| {
        if let Some(grab) = view.thumb_grab {
            let list = view.list_rect(view.client());
            let scrolled = view.list.drag_thumb(grab, y - list.top, height(list));
            if scrolled {
                view.invalidate();
            }
            return (None, scrolled);
        }
        view.track_leave();
        let hit = view.hit_test(x, y);
        let (row, pin) = match hit {
            Hit::Row { index, part } => (Some(index), part == RowPart::Pin),
            _ => (None, false),
        };
        let (editor, close) = match hit {
            Hit::Editor { index, close } => (Some(index), close),
            _ => (None, false),
        };
        let hot = matches!(
            hit,
            Hit::Header(_) | Hit::StateButton | Hit::SecondButton | Hit::EditorsHeader | Hit::Root
        )
        .then_some(hit);
        let row_changed = view.list.set_hover(row);
        let editor_changed = view.editors.list.set_hover(editor);
        if row_changed
            || editor_changed
            || view.hover_pin != pin
            || view.editors.hover_close != close
            || view.hover != hot
        {
            view.hover_pin = pin;
            view.editors.hover_close = close;
            view.hover = hot;
            view.invalidate();
            return (Some(view.tooltip_tools(fonts)), false);
        }
        (None, false)
    })
    .unwrap_or((None, false));
    if let Some(tools) = tools {
        apply_tooltips(hwnd, &tools);
    }
    // The field moves with its row while the thumb is dragged (inline naming spec §5.4).
    if scrolled {
        crate::window::inline_name::place(hwnd);
    }
}

/// Gives the tooltip `tools`, making the tooltip first if the view has none yet. Runs with
/// nothing of the App borrowed: creating the control and adding tools send messages.
pub(super) fn apply_tooltips(hwnd: HWND, tools: &[(usize, RECT, String)]) {
    let Some((existing, failed, panel)) =
        with_view(hwnd, |view| (view.tooltip, view.tooltip_failed, view.panel))
    else {
        return;
    };
    let tooltip = match existing {
        Some(tooltip) => tooltip,
        None if failed => return,
        None => {
            let created = Tooltip::create(panel);
            let kept = with_view(hwnd, |view| {
                view.tooltip = created;
                view.tooltip_failed = created.is_none();
            });
            match (created, kept) {
                (Some(tooltip), Some(())) => tooltip,
                (Some(tooltip), None) => {
                    tooltip.destroy();
                    return;
                }
                (None, _) => return,
            }
        }
    };
    for (id, rect, text) in tools {
        tooltip.set_tool(*id, *rect, text);
    }
}

/// Gives the panel the keyboard focus with nothing of the App borrowed: SetFocus sends
/// WM_KILLFOCUS and WM_SETFOCUS, whose handlers borrow it again.
pub(super) fn focus_panel(hwnd: HWND) {
    let Some(panel) = with_view(hwnd, |view| view.panel) else {
        return;
    };
    unsafe {
        if GetFocus() != panel {
            SetFocus(panel);
        }
    }
}

/// What a press at `x`, `y` hits. An inline edit open when the press comes ends first, as if
/// focus had left it (inline naming spec §5.3), and a row hit follows the row it landed on to
/// wherever the commit moved it. `None` when there is no view, or that row went with the commit
/// (the draft row, or the row just renamed). A press on the scroll thumb leaves the edit open.
pub(super) fn hit_after_commit(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    let hit = with_view(hwnd, |view| view.hit_test(x, y))?;
    // The scroll thumb only scrolls, and the field scrolls with its row (spec §5.4).
    if !crate::window::inline_name::is_open(hwnd) || matches!(hit, Hit::Thumb(_)) {
        return Some(hit);
    }
    let clicked = match hit {
        Hit::Row { index, .. } => with_view(hwnd, |view| {
            view.rows.get(index).map(|row| row.kind.clone())
        })
        .flatten(),
        _ => None,
    };
    crate::window::inline_name::commit(hwnd, crate::window::inline_name::How::FocusLeft);
    match (hit, clicked) {
        (Hit::Row { part, .. }, Some(kind)) => {
            let index = with_view(hwnd, |view| tree::row_index(&view.rows, &kind)).flatten()?;
            Some(Hit::Row { index, part })
        }
        (Hit::Row { .. }, None) => None,
        (hit, _) => Some(hit),
    }
}

/// Moves the keyboard focus to the panel for a press that `hit`, unless it is on the scroll
/// thumb while an inline edit is open: the field keeps the focus and goes on editing while the
/// drag scrolls (inline naming spec §5.4).
pub(super) fn focus_panel_for(hwnd: HWND, hit: Option<&Hit>) {
    if !(matches!(hit, Some(Hit::Thumb(_))) && crate::window::inline_name::is_open(hwnd)) {
        focus_panel(hwnd);
    }
}

pub(super) fn left_down(hwnd: HWND, x: i32, y: i32) {
    // A drag armed by an earlier press whose release never came here. `cancel_drag` also ends
    // one that had started, label and all, though its capture should have ended it already.
    cancel_drag(hwnd);
    // A right press's cancel whose own release has not come yet: this press ends the wait, and
    // the capture kept for it (the release then opens the menu as any other would).
    drop_right_release_wait(hwnd);
    let hit = hit_after_commit(hwnd, x, y);
    focus_panel_for(hwnd, hit.as_ref());
    let Some(hit) = hit else {
        return;
    };
    // The keyboard selection follows the click (open editors spec §3.5).
    let cursor = match hit {
        Hit::EditorsHeader => Some(Cursor::EditorsHeader),
        Hit::Editor { index, .. } => Some(Cursor::Editor(index)),
        Hit::Root => Some(Cursor::Root),
        Hit::Row { .. } => Some(Cursor::Tree),
        // A root row button acts as the header's did (open editors spec §3.3): New note and New
        // folder still go to the tree's selected folder.
        Hit::Header(_) | Hit::StateButton | Hit::SecondButton | Hit::Thumb(_) | Hit::Empty => None,
    };
    if let Some(cursor) = cursor {
        with_view(hwnd, |view| {
            view.cursor = cursor;
            view.invalidate();
        });
    }
    match hit {
        Hit::EditorsHeader => {
            let expanded = with_view(hwnd, |view| view.editors_expanded).unwrap_or(true);
            crate::window::main_window::set_open_editors_expanded(hwnd, !expanded);
            rebuild(hwnd);
        }
        Hit::Editor { index, close } => {
            // A header row does nothing.
            let Some(row) = with_view(hwnd, |view| view.editors.row(index).cloned()).flatten()
            else {
                return;
            };
            if close {
                crate::window::main_window::activate_group(hwnd, row.group);
                crate::window::main_window::close_document_tab(hwnd, row.id);
            } else {
                crate::window::main_window::focus_view(hwnd, row.group, row.id);
                // The name and path are taken now, so the drag outlives its tab closing (open
                // editors spec §4.3). An untitled tab drags onto groups only.
                arm_drag(
                    hwnd,
                    DragSource::Tab {
                        id: row.id,
                        group: row.group,
                        name: row.name.clone(),
                        path: row.path.clone(),
                    },
                    x,
                    y,
                );
            }
        }
        Hit::Root => {
            let expanded = crate::window::library_host::root_expanded(hwnd);
            crate::window::library_host::set_root_expanded(hwnd, !expanded);
            rebuild(hwnd);
        }
        Hit::Header(button) => header_clicked(hwnd, button),
        Hit::StateButton => state_button(hwnd),
        Hit::SecondButton => crate::window::library_host::choose_and_open_folder(hwnd),
        Hit::Thumb(grab) => {
            // Captured after the borrow ends: SetCapture sends WM_CAPTURECHANGED to the window
            // that held the capture before.
            if let Some(panel) = with_view(hwnd, |view| {
                view.thumb_grab = Some(grab);
                view.panel
            }) {
                unsafe {
                    SetCapture(panel);
                }
            }
        }
        Hit::Row { index, part } => {
            with_view(hwnd, |view| view.select(index));
            // Read before the click acts: opening a note or toggling a folder can move rows.
            let source = (part == RowPart::Body)
                .then(|| {
                    with_view(hwnd, |view| {
                        view.rows.get(index).map(|row| row.kind.clone())
                    })
                })
                .flatten()
                .flatten();
            row_clicked(hwnd, index, part, false);
            if let Some(source) = source {
                arm_drag(hwnd, DragSource::Row(source), x, y);
            }
        }
        Hit::Empty => {}
    }
}

/// `WM_LBUTTONDBLCLK`: a row's second press opens it as a normal tab, which also promotes its
/// preview (spec §6.4). Anywhere else it is one more click.
pub(super) fn double_click(hwnd: HWND, x: i32, y: i32) {
    match with_view(hwnd, |view| view.hit_test(x, y)) {
        Some(Hit::Row { index, part }) => row_clicked(hwnd, index, part, true),
        Some(_) => left_down(hwnd, x, y),
        None => {}
    }
}

/// A press on row `index`. `double` is the second press of a double-click, whose first press
/// already toggled a pin or a folder, or started opening a recent notebook.
pub(super) fn row_clicked(hwnd: HWND, index: usize, part: RowPart, double: bool) {
    let Some(target) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    match target {
        Target::Row(TreeRow {
            kind: RowKind::Note(relative),
            ..
        }) if part == RowPart::Pin => {
            if !double && let Some(root) = crate::window::library_host::folder(hwnd) {
                crate::window::library_host::toggle_pin(hwnd, &root.join(relative));
            }
        }
        Target::Row(TreeRow {
            kind: RowKind::Folder(_),
            ..
        })
        | Target::Recent(_)
            if double => {}
        _ => activate(
            hwnd,
            index,
            if double {
                Activation::Permanent
            } else {
                Activation::Click
            },
        ),
    }
}

pub(super) fn set_folder_expanded(hwnd: HWND, relative: &Path, expanded: bool) {
    crate::window::library_host::set_expanded(hwnd, relative, expanded);
    rebuild(hwnd);
}

/// Opens or toggles row `index` (spec §6.4). A folder toggles, a note opens, and a recent
/// notebook opens.
pub(crate) fn activate(hwnd: HWND, index: usize, how: Activation) {
    let Some(target) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    match target {
        Target::Recent(folder) => crate::window::library_host::open_listed_notebook(hwnd, &folder),
        Target::Row(row) => match row.kind {
            RowKind::Folder(relative) => set_folder_expanded(hwnd, &relative, !row.expanded),
            RowKind::Note(relative) => {
                let Some(root) = crate::window::library_host::folder(hwnd) else {
                    return;
                };
                let path = root.join(relative);
                let (mode, focus) = match how {
                    // A click keeps the keyboard in the tree, as VS Code's explorer does, so F2
                    // and Del act on the row just clicked.
                    Activation::Click => (OpenMode::Preview, false),
                    Activation::Permanent => (OpenMode::Permanent, true),
                };
                if let Err(error) = crate::window::main_window::open_note(hwnd, &path, mode, focus)
                {
                    crate::window::main_window::report_open_failure(hwnd, &path, &error);
                }
            }
            RowKind::Draft => {}
        },
        Target::Truncated | Target::Nothing => {}
    }
}

/// The header's buttons (spec §6.5).
pub(crate) fn header_clicked(hwnd: HWND, button: HeaderButton) {
    match button {
        HeaderButton::Favorite => crate::window::library_host::toggle_notebook_favorite(hwnd),
        HeaderButton::NewNote => run(hwnd, CommandId::NoteNew),
        HeaderButton::NewFolder => run(hwnd, CommandId::NoteNewFolder),
        HeaderButton::More => more_menu(hwnd),
    }
}

/// "…": the notebook's own actions.
pub(super) fn more_menu(hwnd: HWND) {
    let Some(at) = with_view(hwnd, |view| {
        let dpi = view.dpi();
        let root = view.layout(view.client(), dpi).root;
        let rect = notebook_layout::root_parts(root, dpi).buttons[3].1;
        view.to_main(POINT {
            x: rect.left,
            y: rect.bottom,
        })
    }) else {
        return;
    };
    let entries = [
        MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
        MenuEntry::command("Close notebook", CommandId::CloseNotebook),
    ];
    match crate::window::menus::track_popup(hwnd, &entries, at) {
        Some(CommandId::NoteRevealInExplorer) => {
            if let Some(root) = crate::window::library_host::folder(hwnd) {
                crate::window::library_host::reveal(hwnd, &root);
            }
        }
        Some(command) => run(hwnd, command),
        None => {}
    }
}

/// `pub(crate)` for the tests that click the state button directly.
pub(crate) fn state_button(hwnd: HWND) {
    match with_view(hwnd, |view| view.mode) {
        Some(Mode::NoNotebook) => crate::window::library_host::choose_and_open_folder(hwnd),
        // The empty notebook's own "New note" puts a draft row in the tree, not an untitled tab
        // (inline naming spec §3.1): there is no tree yet, but `apply` makes one for a draft.
        Some(Mode::Empty) => run(hwnd, CommandId::NoteNew),
        Some(Mode::Failed) => crate::window::library_host::retry_load(hwnd),
        _ => {}
    }
}

/// The panel's keys (spec §10, open editors spec §3.5): the arrows run one selection through
/// the header rows, the Open Editors rows and the tree. Returns false for keys it leaves to the
/// panel.
pub(crate) fn key_down(hwnd: HWND, key: u16) -> bool {
    if let Some(list_key) = ListKey::from_virtual_key(u32::from(key)) {
        with_view(hwnd, |view| {
            let shape = view.shape();
            // A list with nothing selected yet moves as one list does: the first Up or Down
            // selects a row in view.
            let own_list = view.cursor == Cursor::Tree
                && shape.tree > 0
                && view.list.selected.is_none()
                && matches!(list_key, ListKey::Up | ListKey::Down);
            if own_list {
                let height = view.list_height();
                view.list.move_selection(list_key, height);
                view.invalidate();
                return;
            }
            let mut stepped = panel_cursor::step(view.cursor, view.list.selected, list_key, shape);
            // A group's header row is passed over, as a separator is.
            for _ in 0..view.editors.rows.len() {
                match stepped {
                    Some((Cursor::Editor(index), tree)) if view.editors.is_header(index) => {
                        let next = panel_cursor::step(Cursor::Editor(index), tree, list_key, shape);
                        if next == stepped {
                            break;
                        }
                        stepped = next;
                    }
                    _ => break,
                }
            }
            match stepped {
                Some((cursor, tree)) => {
                    view.cursor = cursor;
                    if let Some(index) = tree {
                        view.select(index);
                    }
                    if let Cursor::Editor(index) = cursor {
                        let height = height(view.layout(view.client(), view.dpi()).editors_list);
                        view.editors.list.ensure_visible(index, height);
                    }
                }
                None if view.cursor == Cursor::Tree => {
                    let height = view.list_height();
                    view.list.move_selection(list_key, height);
                }
                // Page Up and Page Down move within the Open Editors rows (open editors spec
                // §3.5). The list's own selection is the active tab's row, so it is put back.
                None => {
                    if let Cursor::Editor(index) = view.cursor {
                        let height = height(view.layout(view.client(), view.dpi()).editors_list);
                        let list = &mut view.editors.list;
                        let active = list.selected.replace(index);
                        list.move_selection(list_key, height);
                        let moved = list.selected.unwrap_or(index);
                        list.selected = active;
                        view.cursor = Cursor::Editor(moved);
                    }
                }
            }
            view.invalidate();
        });
        return true;
    }
    let (cursor, hidden) =
        with_view(hwnd, |view| (view.cursor, view.tree_hidden())).unwrap_or((Cursor::Tree, false));
    if cursor != Cursor::Tree {
        return section_key(hwnd, cursor, key);
    }
    // A row the collapsed root hides is not acted on.
    let selected = with_view(hwnd, |view| view.list.selected)
        .flatten()
        .filter(|_| !hidden);
    let Some(selected) = selected else {
        return matches!(key, VK_RETURN | VK_LEFT | VK_RIGHT | VK_F2 | VK_DELETE);
    };
    match key {
        // Enter opens a normal tab: the preview tab is the mouse's (spec §6.4).
        VK_RETURN => {
            activate(hwnd, selected, Activation::Permanent);
            true
        }
        VK_RIGHT => {
            right(hwnd, selected);
            true
        }
        VK_LEFT => {
            left(hwnd, selected);
            true
        }
        VK_F2 | VK_DELETE => {
            let kind = with_view(hwnd, |view| match view.target(selected) {
                Target::Row(row) => Some(row.kind),
                _ => None,
            })
            .flatten();
            let Some(root) = crate::window::library_host::folder(hwnd) else {
                return true;
            };
            match kind {
                Some(kind @ (RowKind::Note(_) | RowKind::Folder(_))) if key == VK_F2 => {
                    crate::window::inline_name::rename(hwnd, &kind);
                }
                Some(RowKind::Note(relative))
                    if crate::window::library_host::ready_library(hwnd) =>
                {
                    crate::window::library_host::delete_file(hwnd, &root.join(relative));
                }
                Some(RowKind::Folder(relative))
                    if crate::window::library_host::ready_library(hwnd) =>
                {
                    crate::window::library_host::delete_folder(hwnd, &relative);
                }
                _ => {}
            }
            true
        }
        _ => false,
    }
}

/// A key on a header row or an Open Editors row. F2 and Del do nothing there: they act on tree
/// rows only.
pub(super) fn section_key(hwnd: HWND, cursor: Cursor, key: u16) -> bool {
    match (cursor, key) {
        (Cursor::Editor(index), VK_RETURN) => {
            let Some((group, id)) = with_view(hwnd, |view| {
                view.editors.row(index).map(|row| (row.group, row.id))
            })
            .flatten() else {
                return true;
            };
            crate::window::main_window::focus_view(hwnd, group, id);
            crate::window::main_window::focus_content(hwnd);
        }
        (Cursor::EditorsHeader, VK_RETURN | VK_LEFT | VK_RIGHT) => {
            let expanded = crate::window::main_window::open_editors_expanded(hwnd);
            let wanted = expanded_after(key, expanded);
            if wanted != expanded {
                crate::window::main_window::set_open_editors_expanded(hwnd, wanted);
                rebuild(hwnd);
            }
        }
        (Cursor::Root, VK_RETURN | VK_LEFT | VK_RIGHT) => {
            if crate::window::library_host::folder(hwnd).is_none() {
                return true;
            }
            let expanded = crate::window::library_host::root_expanded(hwnd);
            let wanted = expanded_after(key, expanded);
            if wanted != expanded {
                crate::window::library_host::set_root_expanded(hwnd, wanted);
                rebuild(hwnd);
            }
        }
        (_, VK_RETURN | VK_LEFT | VK_RIGHT | VK_F2 | VK_DELETE) => {}
        _ => return false,
    }
    true
}

/// Whether a header row is expanded after `key`: Left collapses it, Right expands it, Enter
/// toggles it.
pub(super) fn expanded_after(key: u16, expanded: bool) -> bool {
    match key {
        VK_LEFT => false,
        VK_RIGHT => true,
        _ => !expanded,
    }
}

/// Right expands a folder, or moves into an expanded one.
pub(super) fn right(hwnd: HWND, index: usize) {
    let Some(Target::Row(row)) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    let RowKind::Folder(relative) = &row.kind else {
        return;
    };
    if !row.expanded {
        set_folder_expanded(hwnd, relative, true);
        return;
    }
    with_view(hwnd, |view| {
        if view
            .rows
            .get(index + 1)
            .is_some_and(|child| child.depth > row.depth)
        {
            view.select(index + 1);
        }
    });
}

/// Left collapses an expanded folder, or moves to the parent folder.
pub(super) fn left(hwnd: HWND, index: usize) {
    let Some(Target::Row(row)) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    if let RowKind::Folder(relative) = &row.kind
        && row.expanded
    {
        set_folder_expanded(hwnd, relative, false);
        return;
    }
    with_view(hwnd, |view| {
        match tree::parent_index(&view.rows, index) {
            Some(parent) => view.select(parent),
            // A top-level row: the notebook's root row is its parent (open editors spec §3.5).
            None if view.mode == Mode::Tree => {
                view.cursor = Cursor::Root;
                view.invalidate();
            }
            None => {}
        }
    });
}

/// Type-ahead: the next row whose name starts with what was typed in the last second. A single
/// letter searches from the row after the selection, so repeating it steps through matches.
pub(super) fn typed(hwnd: HWND, ch: char) {
    with_view(hwnd, |view| {
        if !view.tree_shown() {
            return;
        }
        let prefix = view.typed.push(ch, Instant::now()).to_owned();
        let from = match view.list.selected {
            Some(selected) if prefix.chars().count() == 1 => selected + 1,
            Some(selected) => selected,
            None => 0,
        };
        if let Some(index) = tree::type_ahead(&view.rows, from, &prefix) {
            view.cursor = Cursor::Tree;
            view.select(index);
        }
    });
}
