//! The Notebook tree's multi-selection on the view: what it paints, reports and keeps.

use super::*;
use crate::window::sidebar_accessibility::{MK_CONTROL, MK_LBUTTON, MK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};

/// Ticks `kinds` (two or more) of the first notebook, the cursor on the first.
fn tick(hwnd: HWND, kinds: &[RowKind]) {
    select_row(hwnd, &kinds[0]);
    let id = first_id(hwnd);
    let rows = notebook_view(hwnd).row_keys();
    let view = notebook_view(hwnd);
    for kind in kinds {
        view.multi.toggle(&rows, id, kind, None);
    }
    assert!(view.multi.is_multi());
}

#[test]
fn selected_rows_report_selected_and_only_the_cursor_row_focused() {
    // Break caught: a ticked row an MSAA client sees as unselected, or every ticked row claiming
    // the keyboard focus so a screen reader announces the wrong "current" row.
    use crate::window::sidebar_accessibility::{STATE_FOCUSED, STATE_SELECTED};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("multi-msaa");
    scratch.note("a.md", "a");
    scratch.note("b.md", "b");
    scratch.note("c.md", "c");
    let (window, _editor) = notebook_window(&scratch);
    let (a, b, c) = (
        RowKind::Note("a.md".into()),
        RowKind::Note("b.md".into()),
        RowKind::Note("c.md".into()),
    );
    tick(window.hwnd, &[a.clone(), b.clone()]);
    let panel = sidebar_panel(window.hwnd);
    let source = &crate::window::side_panel::PANEL_ACCESSIBLE;
    let buttons = (source.count)(panel) - notebook_view(window.hwnd).rows.len();
    let item = |kind: &RowKind| {
        let index = buttons + row_of(window.hwnd, kind);
        (source.item)(panel, index).expect("the row is an accessible item")
    };
    // The panel has the keyboard, so the cursor row (a) is the focused one.
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(panel) };
    let (item_a, item_b, item_c) = (item(&a), item(&b), item(&c));
    assert_ne!(item_a.state & STATE_SELECTED, 0, "{item_a:?}");
    assert_ne!(item_b.state & STATE_SELECTED, 0, "{item_b:?}");
    assert_eq!(item_c.state & STATE_SELECTED, 0, "{item_c:?}");
    assert_ne!(item_a.state & STATE_FOCUSED, 0, "{item_a:?}");
    assert_eq!(item_b.state & STATE_FOCUSED, 0, "{item_b:?}");
    assert!(notebook_view(window.hwnd).multi_contains_row(row_of(window.hwnd, &b)));
    assert!(!notebook_view(window.hwnd).multi_contains_row(row_of(window.hwnd, &c)));
}

#[test]
fn a_rebuild_drops_a_selected_note_deleted_outside_fastpad() {
    // Break caught: a selection that outlives its note, so a later command works on a path that
    // is gone; or one that drops the rows a collapsed folder merely hides.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("multi-rebuild");
    scratch.note("a.md", "a");
    scratch.note("b.md", "b");
    scratch.note("c.md", "c");
    let (window, _editor) = notebook_window(&scratch);
    let (a, b, c) = (
        RowKind::Note("a.md".into()),
        RowKind::Note("b.md".into()),
        RowKind::Note("c.md".into()),
    );
    tick(window.hwnd, &[a.clone(), b.clone(), c.clone()]);
    assert_eq!(
        crate::window::notebook_view::selected_items(window.hwnd),
        Some((first_id(window.hwnd), vec![a.clone(), b, c.clone()]))
    );
    std::fs::remove_file(scratch.folder().join("b.md")).unwrap();
    // A rescan needs the data folder the load reads its index from.
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::library_host::request_rescan_of(window.hwnd, first_id(window.hwnd));
    // The rescan lands and rebuilds the rows without the deleted note.
    let gone = RowKind::Note("b.md".into());
    pump_until(window.hwnd, || {
        !view_rows(window.hwnd).iter().any(|(_, kind)| *kind == gone)
    });
    assert_eq!(
        crate::window::notebook_view::selected_items(window.hwnd),
        Some((first_id(window.hwnd), vec![a, c]))
    );
}

#[test]
fn a_rebuild_keeps_selected_notes_a_collapsed_folder_hides() {
    // Break caught: a rebuild that treats rows missing from the view as gone, so collapsing a
    // folder silently drops the notes ticked inside it.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("multi-collapse");
    std::fs::create_dir_all(scratch.folder().join("x")).unwrap();
    scratch.note("x/a.md", "a");
    scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("x"), true);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    crate::window::notebook_view::rebuild(window.hwnd);
    let (a, b) = (
        RowKind::Note(std::path::PathBuf::from("x").join("a.md")),
        RowKind::Note("b.md".into()),
    );
    tick(window.hwnd, &[a.clone(), b.clone()]);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("x"), false);
    crate::window::notebook_view::rebuild(window.hwnd);
    assert!(
        !view_rows(window.hwnd).iter().any(|(_, kind)| *kind == a),
        "the folder hides the note"
    );
    assert_eq!(
        crate::window::notebook_view::selected_items(window.hwnd),
        Some((first_id(window.hwnd), vec![a, b]))
    );
}

/// Presses and releases the left button on the row showing `kind` of notebook `id`, with
/// `buttons` (MK_CONTROL, MK_SHIFT) held.
fn click_row_in(
    hwnd: HWND,
    id: crate::window::library_host::NotebookId,
    kind: &RowKind,
    buttons: usize,
) {
    let at = row_point(hwnd, id, kind);
    let panel = sidebar_windows(hwnd).1;
    mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON | buttons, at);
    mouse(panel, WM_LBUTTONUP, buttons, at);
}

/// `click_row_in` for the first open notebook.
fn click_row(hwnd: HWND, kind: &RowKind, buttons: usize) {
    click_row_in(hwnd, first_id(hwnd), kind, buttons);
}

/// The centre of the row showing `kind` of notebook `id`, as a mouse message's `lParam`.
fn row_point(hwnd: HWND, id: crate::window::library_host::NotebookId, kind: &RowKind) -> LPARAM {
    centre(
        notebook_view(hwnd)
            .row_rect_at(row_in(hwnd, id, kind))
            .expect("the row is in view"),
    )
}

/// Whether a tab shows `path`.
fn opened(hwnd: HWND, path: &std::path::Path) -> bool {
    tab_paths(hwnd)
        .iter()
        .flatten()
        .any(|open| crate::library::model::same_path(open, path))
}

/// Notes `names` in a fresh notebook shown in the Notebook view.
fn notes_window(
    label: &str,
    names: &[&str],
) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
    let scratch = LibraryScratch::new(label);
    for name in names {
        scratch.note(name, name);
    }
    let (window, editor) = notebook_window(&scratch);
    (scratch, window, editor)
}

fn note(name: &str) -> RowKind {
    RowKind::Note(name.into())
}

#[test]
fn ctrl_click_builds_a_selection_and_opens_nothing() {
    // Break caught: a Ctrl+click that opens a preview tab as a plain click does, or that drops
    // the row clicked before it; or a Ctrl+press on a ticked row taking it out at the press, so
    // a Ctrl+drag of the group could not start from it.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notes_window("multi-ctrl", &["a.md", "b.md", "c.md"]);
    let hwnd = window.hwnd;
    let (a, c) = (note("a.md"), note("c.md"));
    click_row(hwnd, &a, 0);
    assert!(
        opened(hwnd, &scratch.folder().join("a.md")),
        "a plain click opens"
    );
    click_row(hwnd, &c, MK_CONTROL);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((first_id(hwnd), vec![a.clone(), c.clone()]))
    );
    assert!(
        !opened(hwnd, &scratch.folder().join("c.md")),
        "no tab for c"
    );
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), c.clone())));

    // A Ctrl+press on a ticked row keeps it until the release.
    let panel = sidebar_windows(hwnd).1;
    let at = row_point(hwnd, first_id(hwnd), &a);
    mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON | MK_CONTROL, at);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((first_id(hwnd), vec![a.clone(), c.clone()]))
    );
    mouse(panel, WM_LBUTTONUP, MK_CONTROL, at);
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
}

#[test]
fn shift_click_selects_the_range_from_the_cursor() {
    // Break caught: a range that starts anywhere but the row clicked before it, opens the row
    // Shift+clicked, or moves its start with each Shift+click.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notes_window("multi-shift", &["a.md", "b.md", "c.md", "d.md"]);
    let hwnd = window.hwnd;
    let (a, b, c, d) = (note("a.md"), note("b.md"), note("c.md"), note("d.md"));
    click_row(hwnd, &b, 0);
    click_row(hwnd, &d, MK_SHIFT);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((first_id(hwnd), vec![b.clone(), c, d.clone()]))
    );
    assert!(
        !opened(hwnd, &scratch.folder().join("d.md")),
        "no tab for d"
    );
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), d)));
    // The range still starts at b.
    click_row(hwnd, &a, MK_SHIFT);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((first_id(hwnd), vec![a, b]))
    );
}

#[test]
fn a_plain_click_inside_the_selection_collapses_it_on_release_and_opens_the_note() {
    // Break caught: a press inside the selection that drops it at once (so no group drag can
    // start), opens the note on the press, or a release that leaves the group ticked.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notes_window("multi-inside", &["a.md", "b.md", "c.md"]);
    let hwnd = window.hwnd;
    let (a, c) = (note("a.md"), note("c.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &c, MK_CONTROL);
    let panel = sidebar_windows(hwnd).1;
    let at = row_point(hwnd, first_id(hwnd), &c);
    mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON, at);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((first_id(hwnd), vec![a, c.clone()])),
        "the press keeps the group"
    );
    assert!(
        !opened(hwnd, &scratch.folder().join("c.md")),
        "nothing opens on the press"
    );
    assert!(notebook_view(hwnd).drag.is_some(), "the press arms a drag");
    mouse(panel, WM_LBUTTONUP, 0, at);
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert!(
        opened(hwnd, &scratch.folder().join("c.md")),
        "the release opens c"
    );
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), c)));
}

#[test]
fn a_plain_click_outside_clears_the_selection() {
    // Break caught: a plain click that leaves the old group ticked beside the new row.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notes_window("multi-outside", &["a.md", "b.md", "c.md"]);
    let hwnd = window.hwnd;
    let (a, b, c) = (note("a.md"), note("b.md"), note("c.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &b, MK_CONTROL);
    click_row(hwnd, &c, 0);
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert!(!notebook_view(hwnd).multi_contains_row(row_of(hwnd, &a)));
    assert!(opened(hwnd, &scratch.folder().join("c.md")));
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), c)));
}

#[test]
fn ctrl_click_in_another_notebook_restarts_the_selection_there() {
    // Break caught: a selection spanning two notebooks, or one whose old rows in A stay
    // ticked, so Del would recycle notes of A along with the row clicked in B.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DELETE;
    let _scintilla = load_native_scintilla();
    let ab = notebooks_a_and_b("multi-other-notebook", "a", "b");
    let hwnd = ab.window.hwnd;
    std::fs::write(ab.a.join("a2.md"), "a2").unwrap();
    rescan_and_wait(hwnd);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, false);
    crate::window::notebook_view::rebuild(hwnd);
    let (a, a2, b) = (note("a.md"), note("a2.md"), note("b.md"));
    click_row_in(hwnd, ab.a_id, &a, 0);
    click_row_in(hwnd, ab.a_id, &a2, MK_CONTROL);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((ab.a_id, vec![a.clone(), a2.clone()]))
    );
    click_row_in(hwnd, ab.b_id, &b, MK_CONTROL);
    assert_eq!(notebook_view(hwnd).multi.notebook(), Some(ab.b_id));
    assert!(!notebook_view(hwnd).multi.contains(ab.a_id, &a));
    assert!(!notebook_view(hwnd).multi.contains(ab.a_id, &a2));
    assert_eq!(selected_row_of(hwnd), Some((ab.b_id, b)));

    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::notebook_view::key_down(hwnd, VK_DELETE);
    assert!(!ab.b.join("b.md").exists(), "b went to the Recycle Bin");
    assert!(ab.a.join("a.md").exists());
    assert!(ab.a.join("a2.md").exists());
}

#[test]
fn escape_clears_the_selection_first_and_keeps_focus_then_returns_to_the_editor() {
    // Break caught: Esc leaving the tree with the group still ticked, or needing a mouse click
    // to drop a selection.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOW, ShowWindow, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = notes_window("multi-escape", &["a.md", "b.md"]);
    let hwnd = window.hwnd;
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    super::super::return_focus_to_editor(hwnd);
    let editor = focused();
    let panel = sidebar_windows(hwnd).1;
    click_row(hwnd, &note("a.md"), 0);
    click_row(hwnd, &note("b.md"), MK_CONTROL);
    assert!(crate::window::notebook_view::selected_items(hwnd).is_some());
    assert_eq!(focused(), panel);
    let escape = || unsafe { SendMessageW(panel, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    escape();
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(focused(), panel, "the first Esc keeps the tree focused");
    escape();
    assert_eq!(focused(), editor, "the second Esc returns to the editor");
}

#[test]
fn a_right_click_inside_the_selection_keeps_it() {
    // Break caught: a right press inside the group dropping it, so its menu could only ever
    // act on one row; or one outside it leaving the group ticked.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_RBUTTONDOWN;
    const MK_RBUTTON: usize = 0x0002;
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = notes_window("multi-right", &["a.md", "b.md", "c.md"]);
    let hwnd = window.hwnd;
    let (a, b, c) = (note("a.md"), note("b.md"), note("c.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &c, MK_CONTROL);
    let panel = sidebar_windows(hwnd).1;
    mouse(
        panel,
        WM_RBUTTONDOWN,
        MK_RBUTTON,
        row_point(hwnd, first_id(hwnd), &a),
    );
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((first_id(hwnd), vec![a.clone(), c]))
    );
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), a)));
    mouse(
        panel,
        WM_RBUTTONDOWN,
        MK_RBUTTON,
        row_point(hwnd, first_id(hwnd), &b),
    );
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), b)));
}

#[test]
fn escape_in_another_sidebar_view_returns_to_the_editor_despite_a_selection() {
    // Break caught: Esc in Favorites or Search silently dropping a tree selection nobody can
    // see there, leaving the keyboard stuck in the panel.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOW, ShowWindow, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = notes_window("multi-escape-other", &["a.md", "b.md"]);
    let hwnd = window.hwnd;
    unsafe { ShowWindow(hwnd, SW_SHOW) };
    super::super::return_focus_to_editor(hwnd);
    let editor = focused();
    click_row(hwnd, &note("a.md"), 0);
    click_row(hwnd, &note("b.md"), MK_CONTROL);
    assert!(crate::window::notebook_view::selected_items(hwnd).is_some());
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Favorites, true);
    let panel = sidebar_windows(hwnd).1;
    assert_eq!(focused(), panel);
    unsafe { SendMessageW(panel, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    assert_eq!(focused(), editor, "one Esc returns to the editor");
}

#[test]
fn a_release_after_a_rebuild_clicks_the_row_pressed_not_the_one_now_at_its_place() {
    // Break caught: a deferred click kept as a row index, so a rebuild between press and
    // release (the file watcher adding a note above) opens whichever note slid into that place,
    // or one that was deleted meanwhile.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notes_window("multi-release-moved", &["b.md", "c.md", "d.md"]);
    let hwnd = window.hwnd;
    app_mut(hwnd).library.data_dir = Some(scratch.data());
    let (b, c, d) = (note("b.md"), note("c.md"), note("d.md"));
    let panel = sidebar_windows(hwnd).1;

    // A note added above the pressed one.
    click_row(hwnd, &b, 0);
    click_row(hwnd, &d, MK_CONTROL);
    let at = row_point(hwnd, first_id(hwnd), &d);
    let old_index = row_of(hwnd, &d);
    mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON, at);
    scratch.note("a.md", "a");
    rescan_and_wait(hwnd);
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(view_rows(hwnd)[old_index].1, c, "c now sits where d was");
    mouse(panel, WM_LBUTTONUP, 0, at);
    assert!(
        opened(hwnd, &scratch.folder().join("d.md")),
        "the pressed note opens"
    );
    assert!(!opened(hwnd, &scratch.folder().join("c.md")));
    assert_eq!(selected_row_of(hwnd), Some((first_id(hwnd), d.clone())));
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);

    // The pressed note deleted before the release: nothing opens.
    click_row(hwnd, &b, 0);
    click_row(hwnd, &c, MK_CONTROL);
    click_row(hwnd, &d, MK_CONTROL);
    let at = row_point(hwnd, first_id(hwnd), &d);
    mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON, at);
    std::fs::remove_file(scratch.folder().join("d.md")).unwrap();
    rescan_and_wait(hwnd);
    crate::window::notebook_view::rebuild(hwnd);
    let tabs = tab_paths(hwnd);
    mouse(panel, WM_LBUTTONUP, 0, at);
    assert_eq!(tab_paths(hwnd), tabs, "nothing opens");
}

#[test]
fn a_cancelled_drag_from_the_selection_neither_collapses_nor_untoggles_it() {
    // Break caught: a press's deferred click outliving the drag it became, so the release
    // after an Esc-cancelled drag collapses the group and opens a note, or untoggles the row
    // a Ctrl+drag started from.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_MOUSEMOVE};
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notes_window("multi-drag-cancel", &["a.md", "b.md", "c.md"]);
    let hwnd = window.hwnd;
    let (a, b, c) = (note("a.md"), note("b.md"), note("c.md"));
    let panel = sidebar_windows(hwnd).1;
    let id = first_id(hwnd);
    let group = Some((id, vec![a.clone(), c.clone()]));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &c, MK_CONTROL);
    let tabs = tab_paths(hwnd);

    // Drags the press on `from` (with `buttons`) to b, cancels it with Esc, then releases.
    let drag_and_cancel = |from: &RowKind, buttons: usize| {
        let at = row_point(hwnd, id, from);
        mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON | buttons, at);
        mouse(
            panel,
            WM_MOUSEMOVE,
            MK_LBUTTON | buttons,
            row_point(hwnd, id, &b),
        );
        assert!(
            notebook_view(hwnd)
                .drag
                .as_ref()
                .is_some_and(|drag| drag.started),
            "the drag started"
        );
        unsafe { SendMessageW(panel, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
        assert!(notebook_view(hwnd).drag.is_none(), "Esc cancelled it");
        mouse(panel, WM_LBUTTONUP, buttons, at);
    };

    drag_and_cancel(&c, 0);
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), group);
    assert_eq!(tab_paths(hwnd), tabs, "no note opened");
    assert!(!opened(hwnd, &scratch.folder().join("c.md")));

    drag_and_cancel(&a, MK_CONTROL);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        group,
        "a stays ticked"
    );
}

use crate::window::notebook_view::Modifiers;

const SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: false,
};
const CTRL: Modifiers = Modifiers {
    shift: false,
    ctrl: true,
};

fn key_with(hwnd: HWND, key: u16, modifiers: Modifiers) -> bool {
    crate::window::notebook_view::key_down_with(hwnd, key, modifiers)
}

#[test]
fn shift_down_extends_and_a_plain_down_clears() {
    // Break caught: Shift+Arrow that only moves the cursor, a range that restarts at each step
    // instead of keeping its anchor, or a plain arrow that leaves the group ticked.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_UP};
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) =
        notes_window("multi-shift-down", &["a.md", "b.md", "c.md", "d.md"]);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    let (a, b, c) = (note("a.md"), note("b.md"), note("c.md"));
    click_row(hwnd, &a, 0);
    key_with(hwnd, VK_DOWN, SHIFT);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((id, vec![a.clone(), b.clone()]))
    );
    key_with(hwnd, VK_DOWN, SHIFT);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((id, vec![a.clone(), b.clone(), c.clone()]))
    );
    assert_eq!(selected_row_of(hwnd), Some((id, c.clone())));
    // Shift+Up shrinks the range back toward the anchor.
    key_with(hwnd, VK_UP, SHIFT);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((id, vec![a, b.clone()]))
    );
    // A plain arrow drops the group and moves the cursor as today.
    key_with(hwnd, VK_DOWN, Modifiers::default());
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(selected_row_of(hwnd), Some((id, c)));
}

#[test]
fn ctrl_space_toggles_and_ctrl_a_selects_the_notebooks_notes_and_folders() {
    // Break caught: Ctrl+Space that moves or opens instead of ticking the cursor row, or Ctrl+A
    // that takes the root row, other notebooks' rows or rows of the wrong kind.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_SPACE, VK_UP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("multi-ctrl-space");
    std::fs::create_dir_all(scratch.folder().join("x")).unwrap();
    scratch.note("x/c.md", "c");
    scratch.note("a.md", "a");
    scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    let (a, b) = (note("a.md"), note("b.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &b, MK_CONTROL);
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((id, vec![a.clone(), b.clone()]))
    );
    // Ctrl+Space on the cursor row (b) takes it out, leaving a lone note: no selection.
    key_with(hwnd, VK_SPACE, CTRL);
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(selected_row_of(hwnd), Some((id, b.clone())));
    // A Ctrl+arrow moves nothing into the selection.
    key_with(hwnd, VK_UP, CTRL);
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);

    key_with(hwnd, b'A' as u16, CTRL);
    let expected: Vec<RowKind> = view_rows(hwnd)
        .into_iter()
        .map(|(_, kind)| kind)
        .filter(|kind| *kind != RowKind::Root)
        .collect();
    assert!(expected.contains(&RowKind::Folder("x".into())));
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((id, expected))
    );
}

#[test]
fn f2_on_a_selection_renames_the_cursor_row_only() {
    // Break caught: F2 with a group ticked renaming the wrong row, or dropping the group before
    // the rename starts.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = notes_window("multi-f2", &["a.md", "b.md", "c.md"]);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    let (a, c) = (note("a.md"), note("c.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &c, MK_CONTROL);
    assert!(key_with(hwnd, VK_F2, Modifiers::default()));
    assert_eq!(selected_row_of(hwnd), Some((id, c.clone())));
    assert!(
        crate::window::inline_name::is_open(hwnd),
        "the rename edit opened"
    );
    assert_eq!(
        crate::window::notebook_view::selected_items(hwnd),
        Some((id, vec![a, c]))
    );
}

/// Notes `names` plus the folder `work` (holding `inside`) in a fresh notebook shown in the
/// Notebook view.
fn group_window(
    label: &str,
    names: &[&str],
    inside: &[&str],
) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
    let scratch = LibraryScratch::new(label);
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    for name in names {
        scratch.note(name, name);
    }
    for name in inside {
        scratch.note(&format!("work/{name}"), "taken");
    }
    let (window, editor) = notebook_window(&scratch);
    (scratch, window, editor)
}

#[test]
fn a_group_moves_into_a_folder_and_the_cursor_lands_on_the_last_one() {
    // Break caught: a group move that stops after the first row, leaves the ticks painted, or
    // parks the cursor on the first row instead of the last one moved.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = group_window("multi-move", &["a.md", "b.md", "c.md"], &[]);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    let (a, b) = (note("a.md"), note("b.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &b, MK_CONTROL);
    let (_, items) = crate::window::notebook_view::selected_items(hwnd).unwrap();
    crate::window::tree_move::drop_many_into_of(hwnd, id, &items, std::path::Path::new("work"));
    let folder = scratch.folder();
    assert!(folder.join("work/a.md").exists() && folder.join("work/b.md").exists());
    assert!(!folder.join("a.md").exists() && !folder.join("b.md").exists());
    assert!(folder.join("c.md").exists());
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(
        selected_row_of(hwnd),
        Some((
            id,
            RowKind::Note(std::path::PathBuf::from("work").join("b.md"))
        ))
    );
    assert!(notices(hwnd).is_empty(), "{:?}", notices(hwnd));
}

#[test]
fn a_taken_name_skips_that_item_moves_the_rest_and_says_so_once() {
    // Break caught: one clash aborting the whole group, or one notice per failure instead of a
    // single summary.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) =
        group_window("multi-taken", &["a.md", "b.md", "c.md"], &["b.md"]);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    app_mut(hwnd).notifications.dismiss_all();
    let items = [note("a.md"), note("b.md"), note("c.md")];
    crate::window::tree_move::drop_many_into_of(hwnd, id, &items, std::path::Path::new("work"));
    let folder = scratch.folder();
    assert!(folder.join("work/a.md").exists() && folder.join("work/c.md").exists());
    assert_eq!(
        std::fs::read_to_string(folder.join("work/b.md")).unwrap(),
        "taken"
    );
    assert!(folder.join("b.md").exists(), "the clashing note stayed");
    assert_eq!(
        notices(hwnd),
        vec![format!(
            "1 of 3 items were not moved:\n{}",
            crate::window::tree_move::taken_notice("b.md", "work")
        )]
    );
    assert_eq!(
        selected_row_of(hwnd),
        Some((
            id,
            RowKind::Note(std::path::PathBuf::from("work").join("c.md"))
        ))
    );
}

#[test]
fn a_folder_and_a_note_inside_it_move_once_with_the_folder() {
    // Break caught: the note inside a selected folder moving on its own (and failing, or
    // landing twice) because the group move saw both rows.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("multi-nested");
    std::fs::create_dir_all(scratch.folder().join("x")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note("x/n.md", "n");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    crate::window::library_host::set_expanded(hwnd, std::path::Path::new("x"), true);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, false);
    crate::window::notebook_view::rebuild(hwnd);
    let (folder_x, inner) = (
        RowKind::Folder("x".into()),
        RowKind::Note(std::path::PathBuf::from("x").join("n.md")),
    );
    click_row(hwnd, &folder_x, MK_CONTROL);
    click_row(hwnd, &inner, MK_CONTROL);
    app_mut(hwnd).notifications.dismiss_all();
    let (_, items) = crate::window::notebook_view::selected_items(hwnd).unwrap();
    assert_eq!(items, vec![folder_x]);
    crate::window::tree_move::drop_many_into_of(hwnd, id, &items, std::path::Path::new("work"));
    assert!(scratch.folder().join("work/x/n.md").exists());
    assert!(!scratch.folder().join("x").exists());
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(
        selected_row_of(hwnd),
        Some((
            id,
            RowKind::Folder(std::path::PathBuf::from("work").join("x"))
        ))
    );
    assert!(notices(hwnd).is_empty(), "{:?}", notices(hwnd));
}

/// The text of the drag's label, if it shows one.
fn drag_label_text(hwnd: HWND) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let label = notebook_view(hwnd).drag_label?.hwnd();
    let mut buffer = [0u16; 64];
    let length = unsafe { GetWindowTextW(label, buffer.as_mut_ptr(), 64) };
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

/// Presses on the row showing `kind` of notebook `id` with `buttons` held and moves past the
/// drag distance, the button still down.
fn start_group_drag(
    hwnd: HWND,
    id: crate::window::library_host::NotebookId,
    kind: &RowKind,
    buttons: usize,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let at = row_point(hwnd, id, kind);
    let panel = sidebar_windows(hwnd).1;
    mouse(panel, WM_LBUTTONDOWN, MK_LBUTTON | buttons, at);
    let (x, y) = ((at & 0xffff) as i32, (at >> 16) as i32);
    mouse(
        panel,
        WM_MOUSEMOVE,
        MK_LBUTTON | buttons,
        client_lparam(x + 30, y),
    );
}

#[test]
fn dragging_a_selection_onto_a_folder_moves_all_of_it() {
    // Break caught: a drag from a selection that carries only the pressed row, a label naming
    // one note, a group offered a folder one of its items refuses, or a drop moving only one.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = group_window("multi-drag", &["a.md", "b.md", "c.md"], &[]);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    let (a, b) = (note("a.md"), note("b.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &b, MK_CONTROL);
    start_group_drag(hwnd, id, &b, 0);
    assert_eq!(
        notebook_view(hwnd)
            .drag
            .as_ref()
            .map(|drag| drag.source.clone()),
        Some(DragSource::Rows(vec![a.clone(), b.clone()]))
    );
    assert_eq!(drag_label_text(hwnd).as_deref(), Some("2 items"));
    let panel = sidebar_windows(hwnd).1;
    // The root is where both already are.
    mouse(
        panel,
        WM_MOUSEMOVE,
        MK_LBUTTON,
        row_point(hwnd, id, &note("c.md")),
    );
    assert_eq!(notebook_view(hwnd).drag.as_ref().unwrap().target, None);
    let work = row_point(hwnd, id, &RowKind::Folder("work".into()));
    mouse(panel, WM_MOUSEMOVE, MK_LBUTTON, work);
    assert_eq!(
        notebook_view(hwnd).drag.as_ref().unwrap().target,
        Some(std::path::PathBuf::from("work"))
    );
    mouse(panel, WM_LBUTTONUP, 0, work);
    let folder = scratch.folder();
    assert!(folder.join("work/a.md").exists() && folder.join("work/b.md").exists());
    assert!(!folder.join("a.md").exists() && !folder.join("b.md").exists());
    assert!(folder.join("c.md").exists());
    assert_eq!(crate::window::notebook_view::selected_items(hwnd), None);
    assert_eq!(
        selected_row_of(hwnd),
        Some((
            id,
            RowKind::Note(std::path::PathBuf::from("work").join("b.md"))
        ))
    );
    assert!(drag_label_text(hwnd).is_none(), "the label went");
}

#[test]
fn ctrl_dragging_a_selection_copies_it() {
    // Break caught: a Ctrl+drag of a group that moves it, copies only the pressed row, or
    // untoggles the row the drag started from.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) =
        group_window("multi-drag-copy", &["a.md", "b.md", "c.md"], &[]);
    let hwnd = window.hwnd;
    let id = first_id(hwnd);
    let (a, b) = (note("a.md"), note("b.md"));
    click_row(hwnd, &a, 0);
    click_row(hwnd, &b, MK_CONTROL);
    start_group_drag(hwnd, id, &a, MK_CONTROL);
    assert_eq!(
        notebook_view(hwnd)
            .drag
            .as_ref()
            .map(|drag| drag.source.clone()),
        Some(DragSource::Rows(vec![a, b]))
    );
    let panel = sidebar_windows(hwnd).1;
    let work = row_point(hwnd, id, &RowKind::Folder("work".into()));
    mouse(panel, WM_MOUSEMOVE, MK_LBUTTON | MK_CONTROL, work);
    assert!(
        notebook_view(hwnd)
            .drag_hint()
            .is_some_and(|hint| hint.starts_with("Copy to ")),
        "{:?}",
        notebook_view(hwnd).drag_hint()
    );
    mouse(panel, WM_LBUTTONUP, MK_CONTROL, work);
    crate::window::copy_host::wait_for_copies(hwnd);
    let folder = scratch.folder();
    assert!(folder.join("a.md").exists() && folder.join("b.md").exists());
    assert!(folder.join("work/a.md").exists() && folder.join("work/b.md").exists());
    assert!(!folder.join("work/c.md").exists());
}

#[test]
fn dropping_a_selection_on_another_notebook_moves_it_all_or_nothing() {
    // Break caught: a group drop across notebooks that moves the items that fit and leaves the
    // clashing one behind, or one that moves only the pressed row.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let ab = notebooks_a_and_b("multi-drag-cross", "a", "b");
    let hwnd = ab.window.hwnd;
    std::fs::write(ab.a.join("a2.md"), "a2").unwrap();
    std::fs::write(ab.b.join("a2.md"), "taken").unwrap();
    rescan_and_wait(hwnd);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, false);
    crate::window::notebook_view::rebuild(hwnd);
    let (a, a2) = (note("a.md"), note("a2.md"));
    let panel = sidebar_windows(hwnd).1;
    let drag_to_b = || {
        click_row_in(hwnd, ab.a_id, &a, 0);
        click_row_in(hwnd, ab.a_id, &a2, MK_CONTROL);
        start_group_drag(hwnd, ab.a_id, &a, 0);
        let root_b = row_point(hwnd, ab.b_id, &RowKind::Root);
        mouse(panel, WM_MOUSEMOVE, MK_LBUTTON, root_b);
        assert_eq!(
            notebook_view(hwnd).drag_hint().as_deref(),
            Some("Move to B")
        );
        mouse(panel, WM_LBUTTONUP, 0, root_b);
    };

    // B already has a2.md: nothing moves.
    app_mut(hwnd).notifications.dismiss_all();
    drag_to_b();
    assert!(ab.a.join("a.md").exists() && ab.a.join("a2.md").exists());
    assert!(!ab.b.join("a.md").exists());
    assert_eq!(
        std::fs::read_to_string(ab.b.join("a2.md")).unwrap(),
        "taken"
    );
    assert_eq!(
        notices(hwnd),
        vec!["a2.md already exists in B. Nothing was moved.".to_owned()]
    );

    // Without the clash, both move.
    std::fs::remove_file(ab.b.join("a2.md")).unwrap();
    rescan_and_wait(hwnd);
    crate::window::notebook_view::rebuild(hwnd);
    drag_to_b();
    pump_until(hwnd, || {
        ab.b.join("a.md").exists() && ab.b.join("a2.md").exists()
    });
    assert!(!ab.a.join("a.md").exists() && !ab.a.join("a2.md").exists());
}
