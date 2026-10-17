//! Files changed outside FastPad while a tab has them open (second brain phase 1 spec §5).

use super::*;
use crate::library::external::ConflictKind;

/// Lets the next write get a different write time than the last.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

fn active(hwnd: HWND) -> &'static crate::document::Document {
    app_mut(hwnd).tabs.active().unwrap()
}

#[test]
fn a_clean_tab_reloads_when_its_file_changes_and_undo_brings_the_old_text_back() {
    // Break caught: an agent's edit staying invisible until the file is reopened, or a reload
    // that can't be undone.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-reload");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one\ntwo\nthree\n");
    editor.set_selection(4..4).unwrap();
    tick();
    std::fs::write(&path, "one\nTWO, edited elsewhere\nthree\n").unwrap();

    crate::window::library_host::check_all_tabs(window.hwnd);
    pump_posted_messages(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "one\nTWO, edited elsewhere\nthree\n"
    );
    assert!(!active(window.hwnd).dirty);
    assert_eq!(
        active(window.hwnd).disk_stamp,
        crate::library::disk_stamp(&path)
    );
    // Only the changed line was replaced: a caret before it stays put.
    assert_eq!(editor.selection().unwrap(), 4..4);

    editor.undo().unwrap();
    pump_posted_messages(window.hwnd);
    assert_eq!(editor.text().unwrap(), "one\ntwo\nthree\n");
    assert!(active(window.hwnd).dirty);
}

#[test]
fn text_appended_on_disk_joins_a_dirty_tab_without_a_conflict() {
    // Break caught: an agent appending to log.md while the user edits it, turning every
    // append into a conflict, or losing the user's edit.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-append");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "log.md", "- 09:00 first\n");
    editor.replace_target(0..0, "# Log\n").unwrap();
    pump_posted_messages(window.hwnd);
    assert!(active(window.hwnd).dirty);
    tick();
    std::fs::write(&path, "- 09:00 first\n- 10:00 second\n").unwrap();

    crate::window::library_host::check_all_tabs(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "# Log\n- 09:00 first\n- 10:00 second\n"
    );
    assert!(active(window.hwnd).dirty);
    assert_eq!(active(window.hwnd).conflict, None);
    assert!(!active(window.hwnd).autosave_paused);

    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Saved
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Log\n- 09:00 first\n- 10:00 second\n"
    );
}

#[test]
fn an_append_that_arrives_just_before_autosave_is_saved_with_the_tab() {
    // Break caught: autosave pausing on an append it could have taken in.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-append-autosave");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "inbox.md", "- a\n");
    editor.replace_target(0..0, "# Inbox\n").unwrap();
    tick();
    std::fs::write(&path, "- a\n- b\n").unwrap();

    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Saved
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# Inbox\n- a\n- b\n"
    );
}

#[test]
fn any_other_change_to_a_dirty_tab_is_a_conflict_that_autosave_never_overwrites() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-conflict");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one\n");
    editor.replace_target(0..0, "mine ").unwrap();
    tick();
    std::fs::write(&path, "theirs\n").unwrap();

    crate::window::library_host::check_all_tabs(window.hwnd);
    assert_eq!(active(window.hwnd).conflict, Some(ConflictKind::Changed));
    assert!(active(window.hwnd).autosave_paused);
    assert_eq!(editor.text().unwrap(), "mine one\n");
    let conflict_notices = |hwnd| {
        notices(hwnd)
            .iter()
            .filter(|n| n.contains("changed on disk"))
            .count()
    };
    assert_eq!(conflict_notices(window.hwnd), 1);
    // Screen readers hear what the warning glyph in the tab shows.
    assert!(
        app_mut(window.hwnd)
            .tabs
            .view()
            .snapshot()
            .tabs
            .iter()
            .any(|tab| tab.title == "a.md *, changed on disk")
    );
    // A second check of the same conflict doesn't repeat the notice.
    crate::window::library_host::check_all_tabs(window.hwnd);
    assert_eq!(conflict_notices(window.hwnd), 1);
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");

    // Reload from disk resolves it as an undoable edit.
    execute_command(window.hwnd, CommandId::NoteReloadFromDisk);
    assert_eq!(editor.text().unwrap(), "theirs\n");
    assert_eq!(active(window.hwnd).conflict, None);
    assert!(!active(window.hwnd).dirty);
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), "mine one\n");
}

#[test]
fn compare_with_disk_opens_the_file_read_only_in_a_split_beside_the_tab() {
    // Break caught: no way to see what changed on disk before choosing a side, or a compare
    // view that can be typed into, saved, or kept by the session.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-compare");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one\n");
    let mine = active(window.hwnd).id;
    editor.replace_target(0..0, "mine ").unwrap();
    tick();
    std::fs::write(&path, "theirs\n").unwrap();
    crate::window::library_host::check_all_tabs(window.hwnd);

    execute_command(window.hwnd, CommandId::NoteCompareWithDisk);
    pump_posted_messages(window.hwnd);
    let app = app_mut(window.hwnd);
    assert_eq!(app.groups.len(), 2, "the disk version opens in a split");
    let compare = app.tabs.active().unwrap();
    assert!(compare.compare.is_some());
    assert_eq!(compare.path, None);
    assert_eq!(compare.title(), "a.md (on disk)");
    let compare_editor = app.editor().cloned().unwrap();
    assert_eq!(compare_editor.text().unwrap(), "theirs\n");

    // Typing does nothing: the view is read-only.
    let _ = compare_editor.replace_target(0..0, "typed ");
    assert_eq!(compare_editor.text().unwrap(), "theirs\n");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
    // The conflicting tab keeps its edits.
    assert_eq!(
        super::super::document_text(window.hwnd, mine).as_deref(),
        Some("mine one\n")
    );
}

#[test]
fn keep_my_version_resolves_a_conflict_by_saving_over_the_file() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-keep");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one\n");
    editor.replace_target(0..0, "mine ").unwrap();
    tick();
    std::fs::write(&path, "theirs\n").unwrap();
    crate::window::library_host::check_all_tabs(window.hwnd);

    execute_command(window.hwnd, CommandId::NoteKeepMine);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine one\n");
    assert_eq!(active(window.hwnd).conflict, None);
    assert!(!active(window.hwnd).autosave_paused);
}

#[test]
fn a_deleted_file_marks_its_clean_tab_and_saving_creates_it_again() {
    // Break caught: a tab silently closing, or autosave re-creating a file deleted on purpose.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-deleted");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "gone.md", "keep me\n");
    std::fs::remove_file(&path).unwrap();

    crate::window::library_host::check_all_tabs(window.hwnd);
    assert!(active(window.hwnd).deleted_on_disk);
    assert_eq!(active(window.hwnd).title(), "gone.md (deleted)");
    assert!(!path.exists());

    execute_command(window.hwnd, CommandId::Save);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me\n");
    assert!(!active(window.hwnd).deleted_on_disk);
}

#[test]
fn a_hidden_tab_takes_its_file_changes_and_stays_clean() {
    // Break caught: only the visible tab noticing an agent's edit, or a background reload
    // leaving the tab dirty.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-hidden");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let first = open_note(&window, &scratch, "a.md", "first\n");
    let first_id = active(window.hwnd).id;
    let second = scratch.note("b.md", "second\n");
    super::super::open_path(window.hwnd, &second).unwrap();
    pump_posted_messages(window.hwnd);
    assert_eq!(editor.text().unwrap(), "second\n");
    tick();
    std::fs::write(&first, "first, edited elsewhere\n").unwrap();

    crate::window::library_host::check_all_tabs(window.hwnd);
    assert_eq!(
        super::super::document_text(window.hwnd, first_id).as_deref(),
        Some("first, edited elsewhere\n")
    );
    let hidden = app_mut(window.hwnd).tabs.document(first_id).unwrap();
    assert!(!hidden.dirty);
    assert_eq!(hidden.disk_stamp, crate::library::disk_stamp(&first));
    assert_eq!(editor.text().unwrap(), "second\n");
}

#[test]
fn with_reload_set_to_ask_a_clean_tab_waits_for_the_user() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-ask");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).settings.reload_changed_files = false;
    let path = open_note(&window, &scratch, "a.md", "one\n");
    tick();
    std::fs::write(&path, "two\n").unwrap();

    crate::window::library_host::check_all_tabs(window.hwnd);
    assert_eq!(editor.text().unwrap(), "one\n");
    assert_eq!(active(window.hwnd).conflict, Some(ConflictKind::Changed));
}

#[test]
fn a_tab_shown_again_picks_up_changes_made_while_it_was_hidden() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-shown");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let first = open_note(&window, &scratch, "a.md", "first\n");
    let first_id = active(window.hwnd).id;
    let second = scratch.note("b.md", "second\n");
    super::super::open_path(window.hwnd, &second).unwrap();
    pump_posted_messages(window.hwnd);
    tick();
    std::fs::write(&first, "first again\n").unwrap();

    assert!(super::super::activate_document_by_id(window.hwnd, first_id));
    assert_eq!(editor.text().unwrap(), "first again\n");
    assert!(!active(window.hwnd).dirty);
}

/// Pumps messages for `duration`, so the watcher's batches (posted 250 ms after a change) land.
fn pump_for(hwnd: HWND, duration: std::time::Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        pump_posted_messages(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn the_watcher_reloads_an_open_note_without_the_window_being_reactivated() {
    // Break caught: an agent's edit showing only after the user clicks away and back.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("watch-reload");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "before\n");
    assert!(crate::window::library_host::watching(window.hwnd));
    tick();
    std::fs::write(&path, "after, from an agent\n").unwrap();

    pump_until(window.hwnd, || {
        editor
            .text()
            .is_ok_and(|text| text == "after, from an agent\n")
    });
    assert!(!active(window.hwnd).dirty);
}

#[test]
fn a_new_note_written_by_another_program_appears_in_the_notebook() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("watch-new");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    // A rescan only starts with a data folder to write the per-PC state to.
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "a\n");
    std::fs::write(scratch.folder().join("from-agent.md"), "hello\n").unwrap();

    pump_until(window.hwnd, || {
        app_mut(window.hwnd)
            .library
            .state
            .as_ref()
            .is_some_and(|state| {
                state
                    .notes
                    .iter()
                    .any(|note| note.path == std::path::Path::new("from-agent.md"))
            })
    });
}

#[test]
fn changes_inside_git_and_other_skipped_folders_cause_no_rescan() {
    // Break caught: every `git commit` in the notebook rescanning it.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("watch-git");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "a\n");
    std::fs::create_dir_all(scratch.folder().join(".git").join("objects")).unwrap();
    pump_for(window.hwnd, std::time::Duration::from_millis(600));
    let scans = app_mut(window.hwnd).library.generation;

    for index in 0..5 {
        std::fs::write(
            scratch
                .folder()
                .join(".git")
                .join("objects")
                .join(format!("o{index}")),
            "x",
        )
        .unwrap();
    }
    std::fs::write(scratch.folder().join(".git").join("index"), "x").unwrap();
    pump_for(window.hwnd, std::time::Duration::from_millis(600));
    assert_eq!(app_mut(window.hwnd).library.generation, scans);
}

#[test]
fn fastpads_own_background_writes_are_not_outside_changes() {
    // Break caught: replace across notes or a copy turning its own writes into reloads or
    // conflicts in the tabs that show them.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("watch-own");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one\n");
    editor.replace_target(0..0, "mine ").unwrap();
    crate::window::library_host::begin_own_writes(window.hwnd, [path.as_path()]);
    tick();
    std::fs::write(&path, "written by FastPad's worker\n").unwrap();
    pump_for(window.hwnd, std::time::Duration::from_millis(600));

    assert_eq!(active(window.hwnd).conflict, None);
    assert!(
        !notices(window.hwnd)
            .iter()
            .any(|notice| notice.contains("changed on disk"))
    );
}

#[test]
fn the_watcher_stops_when_the_notebook_closes_or_notes_mode_goes_off() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("watch-stop");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    open_note(&window, &scratch, "a.md", "a\n");
    assert!(crate::window::library_host::watching(window.hwnd));

    crate::window::library_host::close_notebook(window.hwnd);
    assert!(!crate::window::library_host::watching(window.hwnd));

    scratch.install(window.hwnd);
    assert!(crate::window::library_host::watching(window.hwnd));
    crate::window::library_host::notes_mode_changed(window.hwnd, false);
    assert!(!crate::window::library_host::watching(window.hwnd));
}

fn marked(hwnd: HWND, relative: &str) -> bool {
    app_mut(hwnd)
        .library
        .state
        .as_ref()
        .is_some_and(|state| state.local.is_changed(std::path::Path::new(relative)))
}

#[test]
fn a_note_changed_outside_fastpad_is_marked_until_it_is_opened() {
    // Break caught: no way to see which notes an agent touched overnight, or a mark that never
    // goes away.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("marks-open");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let other = scratch.note("b.md", "b\n");
    open_note(&window, &scratch, "a.md", "a\n");
    tick();
    std::fs::write(&other, "b, edited by an agent\n").unwrap();

    pump_until(window.hwnd, || marked(window.hwnd, "b.md"));
    let local_path = app_mut(window.hwnd)
        .library
        .state
        .as_ref()
        .unwrap()
        .local_path
        .clone();
    assert!(
        std::fs::read_to_string(&local_path)
            .unwrap()
            .contains("changed=b.md"),
        "the mark is kept for the next start"
    );

    super::super::open_path(window.hwnd, &other).unwrap();
    pump_posted_messages(window.hwnd);
    assert!(!marked(window.hwnd, "b.md"));
}

#[test]
fn a_note_a_tab_shows_is_never_marked() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("marks-visible");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a\n");
    tick();
    std::fs::write(&path, "a, edited\n").unwrap();
    pump_until(window.hwnd, || {
        editor.text().is_ok_and(|text| text == "a, edited\n")
    });
    pump_for(window.hwnd, std::time::Duration::from_millis(300));
    assert!(!marked(window.hwnd, "a.md"));
}

#[test]
fn clear_changed_marks_clears_them_all() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("marks-clear");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let other = scratch.note("b.md", "b\n");
    open_note(&window, &scratch, "a.md", "a\n");
    tick();
    std::fs::write(&other, "b, edited\n").unwrap();
    pump_until(window.hwnd, || marked(window.hwnd, "b.md"));

    execute_command(window.hwnd, CommandId::NotesClearChanged);
    assert!(!marked(window.hwnd, "b.md"));
}

#[test]
fn enter_follows_the_files_line_endings_after_opening_and_after_a_reload() {
    // Break caught: Enter inserting CRLF into an LF file, which Notepad then read as two line
    // breaks and saved back doubled.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("ext-eol");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let enter = || unsafe {
        SendMessageW(editor.hwnd(), WM_KEYDOWN, 0x0D, 0);
        SendMessageW(editor.hwnd(), WM_CHAR, 0x0D, 0);
    };
    let path = open_note(&window, &scratch, "a.md", "one\ntwo\n");
    let end = editor.text().unwrap().len();
    editor.set_selection(end..end).unwrap();
    enter();
    assert_eq!(editor.text().unwrap(), "one\ntwo\n\n");

    super::super::save_active_document(window.hwnd);
    tick();
    std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
    crate::window::library_host::check_all_tabs(window.hwnd);
    pump_posted_messages(window.hwnd);
    let end = editor.text().unwrap().len();
    editor.set_selection(end..end).unwrap();
    enter();
    assert_eq!(editor.text().unwrap(), "one\r\ntwo\r\n\r\n");
}
