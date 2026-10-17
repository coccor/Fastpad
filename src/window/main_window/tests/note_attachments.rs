//! Images and files inside Markdown notes (second brain phase 1 spec §10.5).

use super::*;
use crate::library::capture_file::Source;
use crate::library::dates::{Date, Time};

fn ten_thirty() {
    crate::window::library_host::set_now(
        Date::new(2026, 10, 19).unwrap(),
        Time {
            hour: 10,
            minute: 30,
        },
    );
}

fn notebook(label: &str) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
    let scratch = LibraryScratch::new(label);
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    ten_thirty();
    (scratch, window, editor)
}

fn active_id(hwnd: HWND) -> crate::document::DocumentId {
    app_mut(hwnd).tabs.active().unwrap().id
}

#[test]
fn a_pasted_image_is_saved_beside_the_notebook_and_linked_at_the_caret_in_one_undo_step() {
    // Break caught: a screenshot pasted into a note going nowhere, or two undo steps to remove.
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = notebook("attach-image");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    open_note(&window, &scratch, r"work\Atlas plan.md", "Before after");
    editor.set_selection(7..7).unwrap();

    let inserted = crate::window::attachments::attach(
        window.hwnd,
        &editor,
        active_id(window.hwnd),
        vec![Source::Png(b"png".to_vec())],
        None,
    );

    assert!(inserted);
    let saved = scratch
        .folder()
        .join(r"attachments\Atlas plan 2026-10-19 103000.png");
    assert_eq!(std::fs::read(&saved).unwrap(), b"png");
    assert_eq!(
        editor.text().unwrap(),
        "Before ![](../attachments/Atlas%20plan%202026-10-19%20103000.png)after"
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_listed(std::path::Path::new(
            r"attachments\Atlas plan 2026-10-19 103000.png"
        )));
    });
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), "Before after", "one undo step");
    assert!(saved.exists(), "undoing the link never deletes the file");
}

#[test]
fn dropped_files_are_copied_and_linked_where_they_land() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = notebook("attach-drop");
    let outside = std::env::temp_dir().join(format!("fastpad-attach-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    let log = outside.join("ledger batch.log");
    std::fs::write(&log, "log").unwrap();
    std::fs::create_dir_all(scratch.folder().join("attachments")).unwrap();
    scratch.note(r"attachments\ledger batch.log", "older");
    open_note(&window, &scratch, "a.md", "");

    let taken = crate::window::attachments::dropped(
        window.hwnd,
        &editor,
        active_id(window.hwnd),
        vec![log.clone(), outside.clone()],
        None,
    );

    assert!(taken);
    assert_eq!(
        editor.text().unwrap(),
        "[ledger batch 2.log](attachments/ledger%20batch%202.log)",
        "a taken name gets a number, and the folder is skipped"
    );
    assert!(log.exists(), "the original is copied, not moved");
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn a_markdown_file_outside_the_notebook_keeps_attachments_beside_it() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = notebook("attach-outside");
    open_note(&window, &scratch, "a.md", "");
    let outside = std::env::temp_dir().join(format!("fastpad-attach-out-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    let readme = outside.join("README.md");
    std::fs::write(&readme, "").unwrap();
    super::super::open_path(window.hwnd, &readme).unwrap();

    crate::window::attachments::attach(
        window.hwnd,
        &editor,
        active_id(window.hwnd),
        vec![Source::Png(b"png".to_vec())],
        None,
    );

    assert!(
        outside
            .join(r"attachments\README 2026-10-19 103000.png")
            .exists()
    );
    assert_eq!(
        editor.text().unwrap(),
        "![](attachments/README%202026-10-19%20103000.png)"
    );
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn an_untitled_or_plain_text_tab_takes_no_attachment() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = notebook("attach-untitled");
    open_note(&window, &scratch, "notes.txt", "");
    assert!(
        !crate::window::attachments::dropped(
            window.hwnd,
            &editor,
            active_id(window.hwnd),
            vec![scratch.folder().join("notes.txt")],
            None,
        ),
        "a .txt tab opens dropped files as before"
    );

    execute_command(window.hwnd, CommandId::New);
    let id = active_id(window.hwnd);
    app_mut(window.hwnd).tabs.document_mut(id).unwrap().language =
        crate::document::Language::Markdown;
    app_mut(window.hwnd).notifications.dismiss_all();
    crate::window::attachments::attach(
        window.hwnd,
        &editor,
        active_id(window.hwnd),
        vec![Source::Png(b"png".to_vec())],
        None,
    );
    assert_eq!(
        notices(window.hwnd),
        [crate::window::attachments::SAVE_FIRST]
    );
    assert!(!scratch.folder().join("attachments").exists());
}

#[test]
fn moving_a_note_keeps_its_links_to_pdfs_and_moving_an_image_updates_links_to_it() {
    // Break caught: attachments the notebook doesn't list left unreachable after a move.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("attach-moves");
    std::fs::create_dir_all(scratch.folder().join("attachments")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"attachments\spec.pdf", "pdf");
    scratch.note(r"attachments\shot.png", "png");
    let note = open_note(
        &window,
        &scratch,
        "a.md",
        "[spec](attachments/spec.pdf) ![](attachments/shot.png)\n",
    );

    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Note("a.md".into()),
        std::path::Path::new("work"),
    );
    let moved = scratch.folder().join(r"work\a.md");
    assert!(!note.exists());
    assert_eq!(
        std::fs::read_to_string(&moved).unwrap(),
        "[spec](../attachments/spec.pdf) ![](../attachments/shot.png)\n"
    );

    pump_until(window.hwnd, || !app_mut(window.hwnd).library.index_running);
    crate::window::library_host::request_index(window.hwnd);
    pump_until(window.hwnd, || !app_mut(window.hwnd).library.index_running);
    crate::window::modal::answer_next_choice(|_| 0);
    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Note(r"attachments\shot.png".into()),
        std::path::Path::new("work"),
    );
    assert_eq!(
        std::fs::read_to_string(&moved).unwrap(),
        "[spec](../attachments/spec.pdf) ![](shot.png)\n"
    );
}
