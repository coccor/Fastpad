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

#[test]
fn a_folder_that_appears_at_the_destination_before_the_worker_starts_is_never_removed() {
    // Break caught (final review 1): a same-named folder created at the destination while the
    // question was open, deleted for good when the copy then failed on it.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xdrive-appeared",
        &[(r"work\x.md", "x"), ("stay.md", r"[[work\x]]")],
        &[],
    );
    let hwnd = t.window.hwnd;
    let keep = t.b.join(r"work\keep.md");
    force_other_volume_for_test(true);
    crate::window::modal::answer_next_choice({
        let keep = keep.clone();
        move |_| {
            std::fs::create_dir_all(keep.parent().unwrap()).unwrap();
            std::fs::write(&keep, "theirs").unwrap();
            0
        }
    });
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "".into(),
        },
    );
    finish(&t);

    assert_eq!(std::fs::read_to_string(&keep).unwrap(), "theirs");
    assert!(t.a.join(r"work\x.md").exists());
    assert!(has_notice(
        hwnd,
        "work already exists in B. Nothing was moved."
    ));
}

#[test]
fn the_worker_never_removes_a_destination_it_did_not_create() {
    // Break caught (final review 1): the worker's own clean-up deleting a folder that was
    // there before it started.
    let scratch = LibraryScratch::new("xdrive-worker-existing");
    let source = scratch.root.join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("a.md"), "a").unwrap();
    let destination = scratch.root.join("dest");
    std::fs::create_dir_all(&destination).unwrap();
    std::fs::write(destination.join("theirs.md"), "theirs").unwrap();
    let outcome = move_worker::run_for_test(&source, &destination);
    assert!(
        matches!(outcome, move_worker::Outcome::Failed(_)),
        "{outcome:?}"
    );
    assert!(destination.join("theirs.md").exists());
}

#[test]
fn a_file_added_to_the_moving_folder_fails_the_move_instead_of_being_recycled() {
    // Break caught (final review 2): a note saved into the folder during the copy, never
    // copied, then sent to the Recycle Bin with the original.
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-added", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    std::fs::write(t.a.join(r"work\deep\new.md"), "new").unwrap();
    finish(&t);

    assert_eq!(
        std::fs::read_to_string(t.a.join(r"work\deep\new.md")).unwrap(),
        "new"
    );
    assert!(!t.b.join("work").exists());
    assert!(has_notice(
        hwnd,
        "work changed while it was being moved. Nothing was moved."
    ));
}

#[test]
fn nothing_new_goes_into_a_moving_folder_or_its_landing_place() {
    // Break caught (final review 2, 3): a Ctrl copy, a cross move or a Save As into the
    // folder being copied or into its partial copy, lost when the move ends.
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-guarded", true);
    let hwnd = t.window.hwnd;
    copied(&t);
    let wait = "Wait for the move of work to finish.";
    let outside = t._scratch.root.join("outside.md");
    std::fs::write(&outside, "outside").unwrap();
    crate::window::copy_host::copy_into_of(hwnd, t.a_id, vec![outside], Path::new("work"), None);
    assert!(has_notice(hwnd, wait), "a copy into the source");
    app_mut(hwnd).notifications.dismiss_all();
    cross_move_host::move_items(
        hwnd,
        t.b_id,
        vec![RowKind::Note("b-note.md".into())],
        Destination::Open {
            id: t.b_id,
            folder: "work".into(),
        },
    );
    assert!(has_notice(hwnd, wait), "a move into the landing place");
    finish(&t);
    assert!(!t.a.join(r"work\outside.md").exists());
}

#[test]
fn an_ancestor_of_the_landing_place_cannot_be_deleted_during_the_move() {
    // Break caught (final review 3): the parent of the partial copy moved or deleted while the
    // copy is written into it.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xdrive-ancestor",
        &[(r"work\x.md", "x")],
        &[(r"projects\keep.md", "")],
    );
    let hwnd = t.window.hwnd;
    force_other_volume_for_test(true);
    move_worker::pause_before_check(true);
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "projects".into(),
        },
    );
    pump_until(hwnd, || {
        cross_move_host::moving_progress(hwnd) == Some((1, 1))
    });
    library_host::delete_folder_of(hwnd, t.b_id, Path::new("projects"));
    assert!(has_notice(hwnd, "Wait for the move of work to finish."));
    assert!(t.b.join(r"projects\keep.md").exists());
    finish(&t);
    assert!(t.b.join(r"projects\work\x.md").exists());
}

#[test]
fn a_window_closing_after_the_check_removes_the_copy_it_never_landed() {
    // Break caught (final review 4): a full copy left in the destination when the window went
    // away between the worker's "copied" and the UI handling it.
    let _scintilla = load_native_scintilla();
    let t = start("xdrive-stop", false);
    let hwnd = t.window.hwnd;
    // The worker finishes and posts; the window never reads the post.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !cross_move_host::worker_finished(hwnd) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(t.b.join(r"work\x.md").exists(), "the copy is there");
    cross_move_host::stop(hwnd);
    force_other_volume_for_test(false);

    assert!(!t.b.join("work").exists(), "the copy went");
    assert!(t.a.join(r"work\x.md").exists());
    // The late post finds no move and is dropped.
    pump_posted_messages(hwnd);
}

#[test]
fn an_original_only_partly_recycled_is_rescanned_and_its_tabs_follow_the_copy() {
    // Break caught (final review 5): a source notebook still listing, and tabs still showing,
    // notes the failed Recycle Bin step had already removed.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xdrive-partial",
        &[(r"work\x.md", "x"), (r"work\y.md", "y")],
        &[],
    );
    let hwnd = t.window.hwnd;
    super::super::open_path(hwnd, &t.a.join(r"work\x.md")).unwrap();
    crate::window::copy_host::fail_next_recycle_after_removing(t.a.join(r"work\x.md"));
    force_other_volume_for_test(true);
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "".into(),
        },
    );
    finish(&t);

    assert_eq!(active_path(hwnd), Some(t.b.join(r"work\x.md")));
    pump_until(hwnd, || !listed(hwnd, t.a_id, r"work\x.md"));
    assert!(listed(hwnd, t.a_id, r"work\y.md"));
}

#[test]
fn a_note_edited_during_a_move_with_images_keeps_its_edit_and_gets_the_new_link() {
    // Break caught (final review 6): a held tab skipped by the link rewrite, then saved with
    // the old link to an image that moved away.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xdrive-images",
        &[
            (r"work\plan.md", "![x](../attachments/x.png)\n"),
            (r"attachments\x.png", "x-image"),
        ],
        &[(r"projects\keep.md", "")],
    );
    let hwnd = t.window.hwnd;
    super::super::open_path(hwnd, &t.a.join(r"work\plan.md")).unwrap();
    force_other_volume_for_test(true);
    move_worker::pause_before_check(true);
    crate::window::modal::answer_next_choice(|_| 0);
    cross_move_host::move_items(
        hwnd,
        t.a_id,
        vec![RowKind::Folder("work".into())],
        Destination::Open {
            id: t.b_id,
            folder: "projects".into(),
        },
    );
    pump_until(hwnd, || {
        cross_move_host::moving_progress(hwnd) == Some((1, 1))
    });
    t.editor
        .set_text("![x](../attachments/x.png)\nedited\n")
        .unwrap();
    finish(&t);

    assert_eq!(
        std::fs::read_to_string(t.b.join(r"projects\work\plan.md")).unwrap(),
        "![x](../../attachments/x.png)\nedited\n"
    );
    assert!(t.b.join(r"attachments\x.png").exists());
}
