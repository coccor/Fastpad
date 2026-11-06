//! A folder moving to a notebook on another drive (multi-root move spec §2.3, §3.3): copied on a
//! worker, checked, then the original recycled, with progress and Cancel on its row.

use super::cross_move::two;
use super::*;
use crate::library::tree::Placeholder;
use crate::platform::files::force_other_volume_for_test;
use crate::window::cross_move_host::{self, Destination};
use crate::window::library_host;
use crate::window::move_worker;
use std::path::Path;

/// Moves A's `work` (x.md, deep\y.md) to B's root as if B were on another drive. With `pause`,
/// the worker stops after copying, before its check, until `move_worker::pause_before_check`
/// is turned off.
fn start(label: &str, pause: bool) -> super::cross_move::Two {
    let t = two(label, &[(r"work\x.md", "x"), (r"work\deep\y.md", "y")], &[]);
    force_other_volume_for_test(true);
    move_worker::pause_before_check(pause);
    cross_move_host::move_items(
        t.window.hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "".into(),
        },
    );
    t
}

/// Waits for the worker to reach its pause, all files copied.
fn copied(t: &super::cross_move::Two) {
    pump_until(t.window.hwnd, || {
        cross_move_host::moving_progress(t.window.hwnd) == Some((2, 2))
    });
}

fn finish(t: &super::cross_move::Two) {
    move_worker::pause_before_check(false);
    cross_move_host::wait_for_moves(t.window.hwnd);
    force_other_volume_for_test(false);
}

fn listed(hwnd: HWND, id: library_host::NotebookId, relative: &str) -> bool {
    library_host::with_notebook(hwnd, id, |state| state.is_listed(Path::new(relative)))
        .unwrap_or(false)
}

fn has_notice(hwnd: HWND, text: &str) -> bool {
    notices(hwnd).iter().any(|notice| notice == text)
}

#[test]
fn a_folder_to_another_drive_copies_checks_recycles_and_hands_off() {
    // Break caught: a cross-drive folder refused, or left in both notebooks.
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-ok", false);
    let hwnd = t.window.hwnd;
    finish(&t);

    assert!(
        !t.a.join("work").exists(),
        "the original went to the Recycle Bin"
    );
    assert_eq!(
        std::fs::read_to_string(t.b.join(r"work\deep\y.md")).unwrap(),
        "y"
    );
    assert!(!listed(hwnd, t.a_id, r"work\x.md"));
    assert!(listed(hwnd, t.b_id, r"work\deep\y.md"));
    assert!(
        has_notice(hwnd, "Moved work (2 notes) to B."),
        "{:?}",
        notices(hwnd)
    );
}

#[test]
fn the_row_shows_progress_and_refuses_another_move_and_a_drag() {
    // Break caught: a long move with no sign it is running, or a second move of the same
    // folder racing the first.
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-progress", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    crate::window::notebook_view::rebuild(hwnd);
    let row = notebook_view(hwnd)
        .rows
        .iter()
        .find(|row| row.kind == RowKind::Placeholder(Placeholder::Moving))
        .map(|row| (row.notebook, row.name.clone()));
    assert_eq!(
        row,
        Some((t.a_id, "Moving to B… 2 of 2 — Cancel".to_owned()))
    );
    assert_eq!(cross_move_host::busy(hwnd, t.a_id).as_deref(), Some("work"));
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Note(r"work\x.md".into())],
        Destination::Open {
            id: t.b_id,
            folder: "".into(),
        },
    );
    assert!(has_notice(hwnd, "Wait for the move of work to finish."));
    finish(&t);
    assert!(t.b.join(r"work\x.md").exists());
}

#[test]
fn cancel_removes_the_partial_copy_and_keeps_the_original() {
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-cancel", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    cross_move_host::cancel_move(hwnd, t.a_id);
    finish(&t);

    assert!(t.a.join(r"work\deep\y.md").exists());
    assert!(!t.b.join("work").exists(), "the partial copy went");
    assert!(listed(hwnd, t.a_id, r"work\x.md"));
    assert!(!listed(hwnd, t.b_id, r"work\x.md"));
    assert!(has_notice(
        hwnd,
        "Move of work cancelled. Nothing was moved."
    ));
    assert_eq!(cross_move_host::busy(hwnd, t.a_id), None);
}

#[test]
fn activating_the_progress_row_cancels() {
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-row-cancel", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    crate::window::notebook_view::rebuild(hwnd);
    let index = notebook_view(hwnd)
        .rows
        .iter()
        .position(|row| row.kind == RowKind::Placeholder(Placeholder::Moving))
        .unwrap();
    crate::window::notebook_view::activate(
        hwnd,
        index,
        crate::window::notebook_view::Activation::Click,
    );
    finish(&t);
    assert!(t.a.join(r"work\x.md").exists());
    assert!(!t.b.join("work").exists());
}

#[test]
fn a_failed_copy_removes_the_partial_copy() {
    let _scintilla = load_native_scintilla();
    move_worker::fail_next_copy();
    let t = start("xdrive-fail", false);
    let hwnd = t.window.hwnd;
    finish(&t);

    assert!(t.a.join(r"work\x.md").exists());
    assert!(!t.b.join("work").exists());
    assert!(
        notices(hwnd)
            .iter()
            .any(|notice| notice.starts_with("Couldn't move work: ")
                && notice.ends_with(". Nothing was moved.")),
        "{:?}",
        notices(hwnd)
    );
}

#[test]
fn a_file_changed_during_the_move_fails_the_check() {
    // Break caught: the original recycled while it held a change the copy never saw.
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-changed", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    std::fs::write(t.a.join(r"work\x.md"), "changed during the move").unwrap();
    finish(&t);

    assert_eq!(
        std::fs::read_to_string(t.a.join(r"work\x.md")).unwrap(),
        "changed during the move"
    );
    assert!(!t.b.join("work").exists());
    assert!(has_notice(
        hwnd,
        "work changed while it was being moved. Nothing was moved."
    ));
}

#[test]
fn a_failed_recycle_leaves_both_copies_listed() {
    let _scintilla = load_native_scintilla();
    crate::window::copy_host::fail_next_recycle();
    let t = start("xdrive-recycle", false);
    let hwnd = t.window.hwnd;
    finish(&t);

    assert!(t.a.join(r"work\x.md").exists());
    assert!(t.b.join(r"work\x.md").exists());
    assert!(listed(hwnd, t.a_id, r"work\x.md"));
    assert!(listed(hwnd, t.b_id, r"work\x.md"));
    assert!(has_notice(
        hwnd,
        "work was copied to B, but the original in A couldn't be removed."
    ));
}

#[test]
fn closing_the_destination_cancels() {
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-close-dest", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    assert!(library_host::close_notebook_of(hwnd, t.b_id));
    finish(&t);

    assert!(t.a.join(r"work\x.md").exists());
    assert!(!t.b.join("work").exists());
}

#[test]
fn closing_the_source_asks_first_and_keep_running_keeps_it_open() {
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-close-source", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    crate::window::modal::answer_next_choice(|_| 1);
    assert!(!library_host::close_notebook_of(hwnd, t.a_id), "kept open");
    let (text, buttons) = crate::window::modal::take_last_choice().unwrap();
    assert_eq!(text, "A move to B is still running. Cancel it and close?");
    assert_eq!(buttons, ["Cancel move and close", "Keep running"]);
    finish(&t);
    assert!(t.b.join(r"work\x.md").exists(), "the move went on");
}

#[test]
fn closing_the_source_with_cancel_drops_the_late_result() {
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-close-source-cancel", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    crate::window::modal::answer_next_choice(|_| 0);
    assert!(library_host::close_notebook_of(hwnd, t.a_id));
    finish(&t);

    assert!(t.a.join(r"work\x.md").exists(), "nothing moved");
    assert!(!t.b.join("work").exists());
    assert!(!listed(hwnd, t.b_id, r"work\x.md"));
}

#[test]
fn a_tab_edited_during_the_move_saves_to_the_new_path() {
    // Break caught: a save during the move written into the folder being copied (lost, or
    // failing the check), or edits left unsaved after it.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xdrive-tab",
        &[(r"work\x.md", "x"), (r"work\deep\y.md", "y")],
        &[],
    );
    let hwnd = t.window.hwnd;
    super::super::open_path(hwnd, &t.a.join(r"work\x.md")).unwrap();
    force_other_volume_for_test(true);
    move_worker::pause_before_check(true);
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "".into(),
        },
    );
    copied(&t);
    t.editor.set_text("edited during").unwrap();
    super::super::save_active_document(hwnd);
    assert_eq!(
        std::fs::read_to_string(t.a.join(r"work\x.md")).unwrap(),
        "x",
        "the save waits"
    );
    finish(&t);

    assert_eq!(
        std::fs::read_to_string(t.b.join(r"work\x.md")).unwrap(),
        "edited during"
    );
    assert_eq!(active_path(hwnd), Some(t.b.join(r"work\x.md")));
}

#[test]
fn a_tab_edited_during_a_cancelled_move_saves_to_the_old_path() {
    let _scintilla = load_native_scintilla();
    let t = two("xdrive-tab-cancel", &[(r"work\x.md", "x")], &[]);
    let hwnd = t.window.hwnd;
    super::super::open_path(hwnd, &t.a.join(r"work\x.md")).unwrap();
    force_other_volume_for_test(true);
    move_worker::pause_before_check(true);
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "".into(),
        },
    );
    pump_until(hwnd, || {
        cross_move_host::moving_progress(hwnd) == Some((1, 1))
    });
    t.editor.set_text("edited during").unwrap();
    cross_move_host::cancel_move(hwnd, t.a_id);
    finish(&t);

    assert_eq!(
        std::fs::read_to_string(t.a.join(r"work\x.md")).unwrap(),
        "edited during"
    );
    assert!(!t.b.join("work").exists());
}
