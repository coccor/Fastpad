//! The Notebook tree's multi-selection on the view: what it paints, reports and keeps.

use super::*;

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
