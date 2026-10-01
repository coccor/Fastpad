//! Dragging from the tree and from Open Editors.

use super::*;

/// Presses on `from` and moves past the drag distance, still holding the button.
fn start_drag(hwnd: HWND, panel: HWND, from: &RowKind) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
    let lparam = row_lparam(hwnd, from);
    mouse(panel, WM_LBUTTONDOWN, 1, lparam);
    let (x, y) = ((lparam & 0xffff) as i32, (lparam >> 16) as i32);
    mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x + 30, y));
}

/// A point in the list below its last row.
fn below_rows(hwnd: HWND, panel: HWND) -> super::super::LPARAM {
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(panel, &mut client) };
    let last = notebook_view(hwnd).rows.len() - 1;
    let bottom = notebook_view(hwnd).row_rect_at(last).unwrap().bottom;
    assert!(
        bottom + 40 < client.bottom - 40,
        "the panel is tall enough to test with"
    );
    client_lparam(client.right / 2, bottom + 40)
}

#[test]
fn open_editors_drag_onto_a_folder_copies_the_file_and_leaves_the_tab_on_it() {
    // Break caught: the drop moving the file, the tab following the copy, or the copied row
    // not selected (open editors spec §4.1, §4.5).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-drag");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let outside = scratch.root.join("draft.txt");
    std::fs::write(&outside, "draft").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    let panel = sidebar_windows(window.hwnd).1;
    start_tab_drag(window.hwnd, panel, 0);
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    drop_at(panel, work);
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join(r"work\draft.txt")).unwrap(),
        "draft"
    );
    assert!(outside.exists());
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(outside.as_path())
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"work\draft.txt".into())),
        "a .txt is a note type, listed and selected"
    );
}

#[test]
fn open_editors_drag_onto_its_own_folder_copies_nothing_and_asks_nothing() {
    // Break caught (Review Focus 1).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-drag-self");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let b = scratch.note(r"work\b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &b).unwrap();
    let panel = sidebar_windows(window.hwnd).1;
    start_tab_drag(window.hwnd, panel, 0);
    let row = row_lparam(window.hwnd, &RowKind::Note(r"work\b.md".into()));
    drag_over(panel, row);
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        None
    );
    drop_at(panel, row);
    assert!(crate::window::modal::take_last_confirm().is_none());
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");
}

#[test]
fn open_editors_an_untitled_row_drags_but_no_folder_takes_it() {
    // Break caught: an untitled row that cannot reach another group, or one the tree tries
    // to copy with no file behind it (plan amendment 5).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-drag-untitled");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    start_tab_drag(window.hwnd, panel, 0);
    assert!(
        notebook_view(window.hwnd)
            .drag
            .as_ref()
            .is_some_and(|drag| drag.started)
    );
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        None
    );
    drop_at(panel, work);
}

/// The Open Editors entry index (headers counted, as `editor_rect_at` counts them) of
/// document `id`'s first view.
fn open_editors_row_of(hwnd: HWND, id: crate::document::DocumentId) -> usize {
    (0..64)
        .find(|&index| {
            notebook_view(hwnd)
                .editors
                .row(index)
                .is_some_and(|row| row.id == id)
        })
        .expect("an Open Editors row")
}

#[test]
fn open_editors_a_row_dropped_on_another_groups_content_moves_there() {
    // Break caught: row drags stopping at the sidebar's edge (split editors spec §6.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-drag-group");
    let outside = scratch.root.join("draft.txt");
    std::fs::write(&outside, "draft").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    let first = app_mut(window.hwnd).tabs.active_group();
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::New);
    // Close the split's view of `dragged`, so a move is visible.
    super::super::focus_view(window.hwnd, second, dragged);
    execute_command(window.hwnd, CommandId::CloseTab);
    assert!(super::super::activate_group(window.hwnd, first));
    let panel = sidebar_windows(window.hwnd).1;
    let row = open_editors_row_of(window.hwnd, dragged);
    start_tab_drag(window.hwnd, panel, row);
    let target = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let middle = super::super::with_group_id(window.hwnd, second, |state| state.content).unwrap();
    let point = lparam_in(
        panel,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
    );
    drag_over(panel, point);
    assert!(app_mut(window.hwnd).drop_overlay.is_some());
    drop_at(panel, point);
    assert!(
        app_mut(window.hwnd)
            .tabs
            .group(second)
            .unwrap()
            .contains(dragged)
    );
    assert!(
        !app_mut(window.hwnd)
            .tabs
            .group(first)
            .is_some_and(|group| group.contains(dragged))
    );
    assert!(app_mut(window.hwnd).drop_overlay.is_none());
}

#[test]
fn open_editors_a_row_drag_cancelled_over_a_group_leaves_no_overlay() {
    // Break caught: Esc mid-drag leaving the tint over the editor.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-drag-cancel");
    let (window, _editor) = notebook_window(&scratch);
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    let panel = sidebar_windows(window.hwnd).1;
    // With two groups entry 0 is a header: press the first tab row.
    let row = (0..64)
        .find(|&index| notebook_view(window.hwnd).editors.row(index).is_some())
        .unwrap();
    start_tab_drag(window.hwnd, panel, row);
    let target = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let middle = super::super::with_group_id(window.hwnd, second, |state| state.content).unwrap();
    drag_over(
        panel,
        lparam_in(
            panel,
            target,
            middle.left + 40,
            (middle.top + middle.bottom) / 2,
        ),
    );
    assert!(crate::window::notebook_view::cancel_drag(window.hwnd));
    assert!(app_mut(window.hwnd).drop_overlay.is_none());
}

#[test]
fn open_editors_drag_survives_its_tab_closing_mid_drag() {
    // Break caught (Review Focus 5): a stale DocumentId panicking the drop, or the drop
    // copying nothing though the pressed file is still on disk.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-drag-closed");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let outside = scratch.root.join("gone.md");
    std::fs::write(&outside, "g").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    let panel = sidebar_windows(window.hwnd).1;
    start_tab_drag(window.hwnd, panel, 0);
    execute_command(window.hwnd, CommandId::CloseTab);
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    drop_at(panel, work);
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert!(scratch.folder().join(r"work\gone.md").exists());
}

fn drag_cursor_is(cursor: windows_sys::core::PCWSTR) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursor, LoadCursorW};
    unsafe { GetCursor() == LoadCursorW(std::ptr::null_mut(), cursor) }
}

#[test]
fn tree_drag_a_short_move_or_a_missed_release_stays_a_click() {
    // Break caught: a click turned into a drag by a jitter, or a drag started after its
    // release went to another window (tree drag spec §3.1).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-click");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let lparam = row_lparam(window.hwnd, &RowKind::Note("a.md".into()));
    let (x, y) = ((lparam & 0xffff) as i32, (lparam >> 16) as i32);

    mouse(panel, WM_LBUTTONDOWN, 1, lparam);
    mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x + 1, y + 1));
    assert!(unsafe { GetCapture() }.is_null());
    drop_at(panel, client_lparam(x + 1, y + 1));
    assert!(notebook_view(window.hwnd).drag.is_none());
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path()),
        "the press still opened the note"
    );

    mouse(panel, WM_LBUTTONDOWN, 1, lparam);
    // The release went elsewhere: the next move comes without the button.
    mouse(
        panel,
        WM_MOUSEMOVE,
        0,
        row_lparam(window.hwnd, &RowKind::Folder("work".into())),
    );
    assert!(notebook_view(window.hwnd).drag.is_none());
    assert!(unsafe { GetCapture() }.is_null());
    assert!(a.exists());
}

#[test]
fn tree_drag_a_note_dropped_on_a_folder_moves_into_it_and_is_selected() {
    // Break caught: the drag not capturing, the drop not moving, the timer or capture left
    // behind, or the moved row not selected (tree drag spec §3.1, §3.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDC_ARROW, KillTimer};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-note");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    assert_eq!(unsafe { GetCapture() }, panel);
    assert!(notebook_view(window.hwnd).drag.as_ref().unwrap().started);
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        Some("work".into())
    );
    assert!(drag_cursor_is(IDC_ARROW));
    drop_at(panel, work);

    let moved = scratch.folder().join(r"work\a.md");
    assert!(moved.exists() && !a.exists());
    assert!(unsafe { GetCapture() }.is_null());
    assert_eq!(
        unsafe { KillTimer(panel, crate::window::notebook_view::DRAG_TIMER) },
        0
    );
    assert!(notebook_view(window.hwnd).drag.is_none());
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"work\a.md".into()))
    );
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(moved.as_path()),
        "the preview the press opened followed"
    );
}

#[test]
fn tree_drag_a_folder_pressed_then_dragged_to_empty_space_moves_to_the_root() {
    // Break caught: the press's folder toggle shifting the rows so the drag follows the
    // wrong row, or empty space not meaning the root (tree drag spec §3.1, §3.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-folder");
    std::fs::create_dir_all(scratch.folder().join(r"work\inner")).unwrap();
    scratch.note(r"work\inner\c.md", "c");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("work"), true);
    crate::window::notebook_view::rebuild(window.hwnd);
    let panel = sidebar_windows(window.hwnd).1;

    // The press toggles `inner` open, adding c.md's row under it.
    start_drag(window.hwnd, panel, &RowKind::Folder(r"work\inner".into()));
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().source,
        DragSource::Row(RowKind::Folder(r"work\inner".into()))
    );
    let below = below_rows(window.hwnd, panel);
    drag_over(panel, below);
    drop_at(panel, below);

    assert!(scratch.folder().join(r"inner\c.md").exists());
    assert!(!scratch.folder().join(r"work\inner").exists());
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder("inner".into()))
    );
}

#[test]
fn tree_drag_refused_targets_and_a_release_outside_move_nothing() {
    // Break caught: a folder dropped into its own subfolder, a note "moved" into its own
    // folder, the refusal cursor missing, or a release over the editor moving anyway
    // (tree drag spec §3.2).
    use windows_sys::Win32::UI::WindowsAndMessaging::IDC_NO;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-refused");
    std::fs::create_dir_all(scratch.folder().join(r"work\inner")).unwrap();
    scratch.note(r"work\inner\c.md", "c");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    for folder in ["work", r"work\inner"] {
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new(folder), true);
    }
    crate::window::notebook_view::rebuild(window.hwnd);
    let panel = sidebar_windows(window.hwnd).1;

    // `work` collapses on the press; it is expanded again so `inner` is there to hover.
    start_drag(window.hwnd, panel, &RowKind::Folder("work".into()));
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("work"), true);
    crate::window::notebook_view::rebuild(window.hwnd);
    let inner = row_lparam(window.hwnd, &RowKind::Folder(r"work\inner".into()));
    drag_over(panel, inner);
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        None
    );
    assert!(drag_cursor_is(IDC_NO));
    drop_at(panel, inner);
    assert!(scratch.folder().join(r"work\inner\c.md").exists());

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    let below = below_rows(window.hwnd, panel);
    drag_over(panel, below);
    assert!(drag_cursor_is(IDC_NO), "the root is a.md's own folder");
    drop_at(panel, below);

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    drop_at(panel, client_lparam(-50, (work >> 16) as i32));
    assert!(scratch.folder().join("a.md").exists());
    assert!(!scratch.folder().join(r"work\a.md").exists());
    assert!(
        notices(window.hwnd).is_empty(),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn tree_drag_esc_a_right_press_and_a_lost_capture_cancel() {
    // Break caught: Esc sending the focus to the editor mid-drag, a right press opening the
    // menu or leaving the drag on, or a task switch leaving a drag that drops later
    // (tree drag spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetCapture, GetFocus, ReleaseCapture, SetCapture, VK_ESCAPE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-cancel");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let work = || row_lparam(window.hwnd, &RowKind::Folder("work".into()));

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    drag_over(panel, work());
    unsafe { SendMessageW(panel, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert!(notebook_view(window.hwnd).drag.is_none());
    assert!(unsafe { GetCapture() }.is_null());
    assert_eq!(unsafe { GetFocus() }, panel, "the focus stays in the tree");
    drop_at(panel, work());
    assert!(a.exists());

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    drag_over(panel, work());
    mouse(panel, WM_RBUTTONDOWN, 2, work());
    assert!(notebook_view(window.hwnd).drag.is_none());
    // The fix for review round 2 item 1: the capture stays until the right press's own
    // release reaches the panel (otherwise that release, sent while the pointer is over the
    // editor, would fall through to DefWindowProc there and open its context menu).
    assert_eq!(
        unsafe { GetCapture() },
        panel,
        "the capture stays until the right press's own release"
    );
    mouse(panel, WM_RBUTTONUP, 0, work());
    assert!(unsafe { GetCapture() }.is_null());
    drop_at(panel, work());
    assert!(a.exists());

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    drag_over(panel, work());
    unsafe { SetCapture(window.hwnd) };
    assert!(notebook_view(window.hwnd).drag.is_none());
    unsafe { ReleaseCapture() };
    drop_at(panel, work());
    assert!(a.exists());
    assert!(!scratch.folder().join(r"work\a.md").exists());
}

#[test]
fn tree_drag_a_right_press_cancel_frees_the_mouse_when_its_release_cannot_come() {
    // Break caught: after a right press cancelled a drag, a view switch, the sidebar hiding
    // or a left press while the right button was still down leaving the panel with the
    // capture for good, so clicks anywhere in FastPad went to the sidebar (tree drag spec
    // §10).
    use crate::config::SidebarView;
    use crate::window::side_panel::show_view;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-right-capture");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let note = RowKind::Note("a.md".into());
    // Starts a drag of a.md and cancels it with a right press, still held.
    let cancel_with_right_press = || {
        start_drag(window.hwnd, panel, &note);
        mouse(panel, WM_RBUTTONDOWN, 2, row_lparam(window.hwnd, &note));
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert_eq!(unsafe { GetCapture() }, panel, "kept for the right release");
    };

    for (view, name) in [
        (SidebarView::Search, "another view"),
        (SidebarView::Hidden, "the sidebar hiding"),
    ] {
        cancel_with_right_press();
        show_view(window.hwnd, view, false);
        assert!(unsafe { GetCapture() }.is_null(), "{name} frees the mouse");
        show_view(window.hwnd, SidebarView::Notebook, false);
        assert!(!notebook_view(window.hwnd).eat_right_up, "{name}");
    }

    cancel_with_right_press();
    let lparam = row_lparam(window.hwnd, &note);
    mouse(panel, WM_LBUTTONDOWN, 3, lparam);
    assert!(
        unsafe { GetCapture() }.is_null(),
        "a left press frees the mouse"
    );
    mouse(panel, WM_LBUTTONUP, 2, lparam);
    // The wait is gone, so the right release would now open the tree's menu, as any other
    // does: not sent, since the menu's loop would wait for input.
    assert!(!notebook_view(window.hwnd).eat_right_up);
    assert!(a.exists(), "nothing moved");
}

#[test]
fn tree_drag_a_right_press_cancel_eats_only_its_own_release() {
    // Break caught: a right press that cancelled a drag setting a flag that outlives its own
    // release (the release went elsewhere, e.g. the pointer was over the editor), so the next
    // ordinary right-click in the tree opens no menu (tree drag spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-right-cancel");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let work = || row_lparam(window.hwnd, &RowKind::Folder("work".into()));

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    drag_over(panel, work());
    mouse(panel, WM_RBUTTONDOWN, 2, work());
    assert!(
        notebook_view(window.hwnd).eat_right_up,
        "the cancel's own release should be eaten"
    );
    let result = unsafe { SendMessageW(panel, WM_RBUTTONUP, 0, work()) };
    assert_eq!(result, 0, "its own release is eaten, so no menu opens");
    assert!(!notebook_view(window.hwnd).eat_right_up);

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    drag_over(panel, work());
    mouse(panel, WM_RBUTTONDOWN, 2, work());
    assert!(notebook_view(window.hwnd).eat_right_up);
    // The release never came here (it went elsewhere): a later, unrelated right press must
    // not still be eating a release meant for it.
    mouse(panel, WM_RBUTTONDOWN, 2, work());
    assert!(
        !notebook_view(window.hwnd).eat_right_up,
        "a later ordinary right press clears the stale flag"
    );
    // The first press's release never came (simulated above): its capture is still held.
    // Tidy up, since nothing else in this scenario will release it.
    unsafe { ReleaseCapture() };
}

#[test]
fn tree_drag_a_right_press_cancel_over_the_editor_still_gets_its_release() {
    // Break caught: releasing the capture as soon as a right press cancels a drag lets its
    // own WM_RBUTTONUP, sent while the pointer is over the editor, fall through to
    // DefWindowProc there and open the editor's context menu instead of the panel eating its
    // own release (tree drag spec §3.3, spec §10; final review Important 1).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-right-editor");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    drag_over(panel, work);
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    // Beyond the panel's own client width: over the editor. Capture still routes it here.
    let beyond = client_lparam(client.right + 50, (work >> 16) as i32);
    mouse(panel, WM_RBUTTONDOWN, 2, beyond);
    assert!(notebook_view(window.hwnd).drag.is_none());
    assert_eq!(
        unsafe { GetCapture() },
        panel,
        "the capture stays until the right press's own release reaches the panel"
    );

    let result = unsafe { SendMessageW(panel, WM_RBUTTONUP, 0, beyond) };
    assert_eq!(
        result, 0,
        "eaten: DefWindowProc never turns it into a context menu"
    );
    assert!(unsafe { GetCapture() }.is_null());
    assert!(!notebook_view(window.hwnd).eat_right_up);
}

#[test]
fn tree_drag_the_timer_expands_a_resting_folder_and_scrolls_near_the_bottom() {
    // Break caught: a hovered collapsed folder never opening, the list not scrolling at its
    // edge, or no timer while dragging (tree drag spec §3.3).
    use windows_sys::Win32::UI::WindowsAndMessaging::KillTimer;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-timer");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    for index in 0..80 {
        scratch.note(&format!("n{index:02}.md"), "n");
    }
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("work"), false);
    crate::window::notebook_view::rebuild(window.hwnd);

    start_drag(window.hwnd, panel, &RowKind::Note("n00.md".into()));
    let start = std::time::Instant::now();
    drag_over(
        panel,
        row_lparam(window.hwnd, &RowKind::Folder("work".into())),
    );
    crate::window::notebook_view::drag_tick(window.hwnd, start);
    assert!(
        !crate::window::library_host::expanded(window.hwnd)
            .contains(&std::path::PathBuf::from("work"))
    );
    crate::window::notebook_view::drag_tick(
        window.hwnd,
        start + std::time::Duration::from_millis(800),
    );
    assert!(
        crate::window::library_host::expanded(window.hwnd)
            .contains(&std::path::PathBuf::from("work"))
    );

    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(panel, &mut client) };
    drag_over(panel, client_lparam(client.right / 2, client.bottom - 2));
    let top = notebook_view(window.hwnd).list.top;
    crate::window::notebook_view::drag_tick(window.hwnd, start);
    assert!(notebook_view(window.hwnd).list.top > top, "scrolled down");
    assert_ne!(
        unsafe { KillTimer(panel, crate::window::notebook_view::DRAG_TIMER) },
        0,
        "the drag's timer runs"
    );
    crate::window::notebook_view::cancel_drag(window.hwnd);
}

#[test]
fn tree_drag_a_rebuild_or_view_switch_mid_drag_cancels_or_retargets() {
    // Break caught: a drag of a row a rescan removed staying on, a drop into a folder that
    // vanished, or a drag surviving another view (tree drag spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-rebuild");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("other")).unwrap();
    let a = scratch.note("a.md", "a");
    scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;

    start_drag(window.hwnd, panel, &RowKind::Note("b.md".into()));
    drag_over(
        panel,
        row_lparam(window.hwnd, &RowKind::Folder("work".into())),
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        state.remove_folder_for_test(std::path::Path::new("work"));
    });
    crate::window::notebook_view::rebuild(window.hwnd);
    let drag = notebook_view(window.hwnd).drag.clone().unwrap();
    assert_eq!(drag.target, None, "the target folder's row is gone");

    crate::window::library_host::with_state(window.hwnd, |state| state.remove_note(&a));
    crate::window::notebook_view::rebuild(window.hwnd);
    assert!(
        notebook_view(window.hwnd).drag.is_some(),
        "b.md is still there"
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        state.remove_note(&scratch.folder().join("b.md"))
    });
    crate::window::notebook_view::rebuild(window.hwnd);
    assert!(notebook_view(window.hwnd).drag.is_none());
    assert!(unsafe { GetCapture() }.is_null());

    crate::window::library_host::with_state(window.hwnd, |state| {
        let _ = state.add_note(&a);
    });
    crate::window::notebook_view::rebuild(window.hwnd);
    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    assert!(unsafe { GetCapture() }.is_null());
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    assert!(notebook_view(window.hwnd).drag.is_none());
}

#[test]
fn tree_drag_a_press_during_a_refused_edit_ends_the_edit_before_the_drag() {
    // Break caught: a taken name left open under a drag started by the same press, instead
    // of the press committing the refused edit first — closing it with its notice, renaming
    // nothing — and only then arming the drag of the row it actually landed on (spec §8, tree
    // drag spec §3.1; final review Important 3, a controller-pinned ordering).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-refused-edit");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("other")).unwrap();
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;

    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "other");
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("other already exists here.")
    );

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));

    assert!(!inline_open(window.hwnd), "the refused edit closed");
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice == "other already exists here."),
        "{:?}",
        notices(window.hwnd)
    );
    assert!(scratch.folder().join("sub").is_dir(), "nothing was renamed");
    assert!(scratch.folder().join("other").is_dir());
    assert_eq!(
        notebook_view(window.hwnd)
            .drag
            .as_ref()
            .map(|drag| drag.source.clone()),
        Some(DragSource::Row(RowKind::Note("a.md".into()))),
        "the pressed row's drag armed only once the edit had closed"
    );
    assert!(notebook_view(window.hwnd).drag.as_ref().unwrap().started);
    assert_eq!(unsafe { GetCapture() }, panel);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path()),
        "the press still opened the note"
    );

    crate::window::notebook_view::cancel_drag(window.hwnd);
}

#[test]
fn tree_drag_a_notebook_switch_mid_drag_cancels_even_when_the_row_still_resolves() {
    // Break caught: a rebuild that only checks whether the dragged row's RowKind still has a
    // row, so switching to a different notebook that happens to have its own "a.md" reads as
    // "the row is still there" and the drag survives into the wrong notebook (final review
    // Minor 5; tree drag spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("drag-switch-first");
    let a = first.note("a.md", "a");
    let (window, _editor) = notebook_window(&first);
    let panel = sidebar_windows(window.hwnd).1;

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    assert_eq!(unsafe { GetCapture() }, panel);

    let second = LibraryScratch::new("drag-switch-second");
    second.note("a.md", "a2");
    let local = crate::library::local::local_file(&second.data(), &second.folder());
    let state = crate::library::load(&second.folder(), &local, crate::library::now_unix()).unwrap();
    crate::window::library_host::install_for_test(window.hwnd, state);
    crate::window::notebook_view::rebuild(window.hwnd);

    assert!(
        notebook_view(window.hwnd).drag.is_none(),
        "a different notebook's a.md is not the same row"
    );
    assert!(unsafe { GetCapture() }.is_null());
    assert!(a.exists());
}

#[test]
fn tree_drag_a_rebuild_mid_drag_retargets_the_band_from_the_still_pointer() {
    // Break caught: rows shifting under a pointer that has not moved (a folder appearing
    // elsewhere, a rescan) leaving the drag's target and cursor pointing at what used to be
    // there, until the next mouse move (final review Minor 4; tree drag spec §3.2, §3.3).
    use windows_sys::Win32::UI::WindowsAndMessaging::IDC_ARROW;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-rebuild-retarget");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        Some("work".into())
    );

    // A folder appears above "work", sorted before it: "work"'s row shifts down one, so the
    // still pointer is now over the new folder's row instead.
    crate::window::library_host::with_state(window.hwnd, |state| {
        state.add_folder(std::path::Path::new("AAA"));
    });
    crate::window::notebook_view::rebuild(window.hwnd);

    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        Some("AAA".into()),
        "the band followed the row that moved under the still pointer"
    );
    assert!(drag_cursor_is(IDC_ARROW));

    crate::window::notebook_view::cancel_drag(window.hwnd);
}

#[test]
fn tree_drag_a_wheel_scroll_mid_drag_retargets_the_band_from_the_still_pointer() {
    // Break caught: a wheel scroll during a started drag moving the rows under the pointer
    // without re-checking the target, so the band and cursor keep showing the row that used
    // to be there (final review Minor 4; tree drag spec §3.2, §3.3).
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDC_ARROW, IDC_NO, WM_MOUSEWHEEL};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-wheel-retarget");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note("a.md", "a");
    for index in 0..80 {
        scratch.note(&format!("n{index:02}.md"), "n");
    }
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;

    // "a.md" is dragged: the root is its own folder, so it is refused there and accepted in
    // "work", the only folder.
    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    drag_over(panel, work);
    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        Some("work".into())
    );
    assert!(drag_cursor_is(IDC_ARROW));

    // A big scroll: "work" (the list's one folder, at the top) scrolls out of view, so the
    // still pointer, at the same pixel it was over "work" at, now lands on a root note.
    let down = ((-(120_i16 * 20)) as u16 as usize) << 16;
    let top = notebook_view(window.hwnd).list.top;
    mouse(panel, WM_MOUSEWHEEL, down, 0);
    assert!(notebook_view(window.hwnd).list.top > top, "scrolled");

    assert_eq!(
        notebook_view(window.hwnd).drag.as_ref().unwrap().target,
        None,
        "the still pointer is over a root note now: a.md's own folder"
    );
    assert!(drag_cursor_is(IDC_NO));

    crate::window::notebook_view::cancel_drag(window.hwnd);
}

#[test]
fn tree_drag_keys_are_ignored_while_a_drag_is_started() {
    // Break caught: F2 opening a rename field, or a typed letter jumping the selection, on
    // the row a started drag is carrying (final review Minor 6; tree drag spec §3.3). Esc
    // still cancels it: side_panel routes that to cancel_drag before this is ever reached.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-keys-ignored");
    scratch.note("a.md", "a");
    scratch.note("zzz.md", "z");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("a.md".into()))
    );

    mouse(panel, WM_KEYDOWN, VK_F2 as usize, 0);
    assert!(!inline_open(window.hwnd), "F2 opened no rename field");

    mouse(panel, WM_CHAR, 'z' as usize, 0);
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("a.md".into())),
        "a typed letter did not jump the selection"
    );
    assert!(notebook_view(window.hwnd).drag.as_ref().unwrap().started);

    crate::window::notebook_view::cancel_drag(window.hwnd);
}

#[test]
fn tree_drag_a_label_with_the_name_follows_the_pointer_until_the_drag_ends() {
    // Break caught: a drag with nothing following the pointer, a label that takes the focus
    // or clicks, one left on screen after a drop or a cancel, or one shown for a click
    // (tree drag spec §3.2).
    use crate::window::drag_label::{place, work_area};
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetFocus, ReleaseCapture, SetCapture, VK_ESCAPE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongW, GetWindowRect, GetWindowTextW, IsWindow, IsWindowVisible,
        WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WS_EX_NOACTIVATE,
        WS_EX_TRANSPARENT,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-label");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let label = || {
        notebook_view(window.hwnd)
            .drag_label
            .map(|label| label.hwnd())
    };
    let text = |label: HWND| {
        let mut buffer = [0u16; 64];
        let length = unsafe { GetWindowTextW(label, buffer.as_mut_ptr(), 64) };
        String::from_utf16_lossy(&buffer[..length as usize])
    };
    let point = |lparam: super::super::LPARAM| ((lparam & 0xffff) as i32, (lparam >> 16) as i32);
    // Where the label should be for the pointer at panel `lparam`.
    let expected = |label: HWND, lparam: super::super::LPARAM| {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(label, &mut rect) };
        let (x, y) = point(lparam);
        let mut pointer = POINT { x, y };
        unsafe { ClientToScreen(panel, &mut pointer) };
        let size = windows_sys::Win32::Foundation::SIZE {
            cx: rect.right - rect.left,
            cy: rect.bottom - rect.top,
        };
        let at = place(pointer, size, work_area(pointer), unsafe {
            GetDpiForWindow(panel)
        });
        ((rect.left, rect.top), (at.x, at.y))
    };
    let a = row_lparam(window.hwnd, &RowKind::Note("a.md".into()));
    let work = || row_lparam(window.hwnd, &RowKind::Folder("work".into()));

    let (x, y) = point(a);
    mouse(panel, WM_LBUTTONDOWN, 1, a);
    mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x + 1, y + 1));
    assert!(label().is_none(), "a click shows no label");
    drop_at(panel, client_lparam(x + 1, y + 1));

    // The click may have moved the rows: start_drag presses where the row is now.
    let (x, y) = point(row_lparam(window.hwnd, &RowKind::Note("a.md".into())));
    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    let shown = label().expect("the drag shows a label");
    assert!(unsafe { IsWindowVisible(shown) } != 0);
    assert_eq!(text(shown), "a.md");
    let style = unsafe { GetWindowLongW(shown, GWL_EXSTYLE) } as u32;
    assert_eq!(
        style & (WS_EX_TRANSPARENT | WS_EX_NOACTIVATE),
        WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
        "it never takes a click or the focus"
    );
    assert_eq!(unsafe { GetFocus() }, panel);
    let (at, want) = expected(shown, client_lparam(x + 30, y));
    assert_eq!(at, want, "next to the pointer");
    drag_over(panel, work());
    let (at, want) = expected(shown, work());
    assert_eq!(at, want, "it follows the pointer");
    drop_at(panel, work());
    assert!(label().is_none());
    assert!(unsafe { IsWindow(shown) } == 0, "the drop destroys it");
    assert!(scratch.folder().join(r"work\a.md").exists());

    let gone = |shown: HWND| label().is_none() && unsafe { IsWindow(shown) } == 0;
    start_drag(window.hwnd, panel, &RowKind::Folder("work".into()));
    let shown = label().unwrap();
    assert_eq!(text(shown), "work");
    unsafe { SendMessageW(panel, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert!(gone(shown), "Esc");

    start_drag(window.hwnd, panel, &RowKind::Folder("work".into()));
    let shown = label().unwrap();
    mouse(panel, WM_RBUTTONDOWN, 2, work());
    assert!(gone(shown), "a right press");
    mouse(panel, WM_RBUTTONUP, 0, work());

    start_drag(window.hwnd, panel, &RowKind::Folder("work".into()));
    let shown = label().unwrap();
    unsafe { SetCapture(window.hwnd) };
    assert!(gone(shown), "a lost capture");
    unsafe { ReleaseCapture() };

    start_drag(window.hwnd, panel, &RowKind::Folder("work".into()));
    let shown = label().unwrap();
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    assert!(gone(shown), "another view");
}

#[test]
fn tree_drag_a_note_dropped_on_a_groups_middle_opens_there_and_on_an_edge_splits() {
    // Break caught: tree rows only dropping on folders, so dragging a note onto the editor
    // does nothing; or an edge drop opening in the existing group instead of a new one.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("tree-drag-group");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert!(super::super::activate_group(window.hwnd, first));
    let panel = sidebar_windows(window.hwnd).1;
    let holds = |group, path: &std::path::Path| {
        app_mut(window.hwnd)
            .tabs
            .group_documents(group)
            .iter()
            .any(|document| document.path.as_deref() == Some(path))
    };

    start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
    let target = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let content = super::super::with_group_id(window.hwnd, second, |state| state.content).unwrap();
    let middle = lparam_in(
        panel,
        target,
        (content.left + content.right) / 2,
        (content.top + content.bottom) / 2,
    );
    drag_over(panel, middle);
    assert!(app_mut(window.hwnd).drop_overlay.is_some());
    // Break caught: the overlay destroyed and remade on every move, so it never gets to paint.
    let shown = app_mut(window.hwnd).drop_overlay.unwrap().hwnd();
    drag_over(panel, middle);
    assert_eq!(app_mut(window.hwnd).drop_overlay.unwrap().hwnd(), shown);
    drop_at(panel, middle);
    assert!(holds(second, &a), "opened in the group it was dropped on");
    assert!(
        app_mut(window.hwnd)
            .tabs
            .group_documents(second)
            .iter()
            .any(|document| document.path.as_deref() == Some(a.as_path()) && !document.preview),
        "a drop opens a normal tab, not a preview"
    );
    assert!(!holds(first, &a));
    assert!(app_mut(window.hwnd).drop_overlay.is_none());

    let groups = app_mut(window.hwnd).tabs.group_ids().len();
    start_drag(window.hwnd, panel, &RowKind::Note("b.md".into()));
    let edge = lparam_in(
        panel,
        target,
        content.right - 2,
        (content.top + content.bottom) / 2,
    );
    drag_over(panel, edge);
    drop_at(panel, edge);
    assert_eq!(app_mut(window.hwnd).tabs.group_ids().len(), groups + 1);
    let new = app_mut(window.hwnd).tabs.active_group();
    assert!(holds(new, &b), "opened in the group split off the edge");
}
