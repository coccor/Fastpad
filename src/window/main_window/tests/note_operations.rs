//! Pinning, renaming, deleting and relabelling notes, and library file handling.

use super::*;

#[test]
fn toggling_a_pin_applies_to_the_active_note_and_persists() {
    // Break caught: the pin command changing only memory, recording the wrong file, or an
    // unpin leaving a record behind.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("pin");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a");
    let hwnd = window.hwnd;

    execute_command(hwnd, CommandId::NoteTogglePin);
    assert!(
        app_mut(hwnd)
            .library
            .state
            .as_ref()
            .unwrap()
            .is_pinned(&path)
    );
    assert!(notices(hwnd).iter().any(|n| n == "Pinned."));
    crate::window::library_host::flush_now(hwnd);
    let ini =
        std::fs::read_to_string(crate::library::store::library_file(&scratch.folder())).unwrap();
    assert!(ini.starts_with("version=2\r\n"), "{ini:?}");
    assert!(ini.contains("|p|") && ini.ends_with("|a.md\r\n"), "{ini:?}");
    let reloaded = crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
    assert!(reloaded.is_pinned(&path));

    execute_command(hwnd, CommandId::NoteTogglePin);
    crate::window::library_host::flush_now(hwnd);
    assert!(notices(hwnd).iter().any(|n| n == "Unpinned."));
    assert!(
        library(hwnd).notes.is_empty(),
        "an unpinned note keeps no record"
    );
}

#[test]
fn pinning_a_file_outside_the_open_notebook_is_refused() {
    // Break caught: a pin on a file outside the notebook writing an absolute-path record
    // that version 2 of library.ini cannot hold.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("pin-outside");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let outside = scratch.root.join("outside.md");
    std::fs::write(&outside, "x").unwrap();
    super::super::open_path(window.hwnd, &outside).unwrap();
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n == "Only notes in the open notebook can be pinned.")
    );
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    assert!(state.pending.is_empty());
    assert!(state.library.notes.is_empty());
}

#[test]
fn an_untitled_tab_must_be_saved_before_it_can_be_organized() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("untitled-organize");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::create_new_document(window.hwnd).unwrap();
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n == "Save this note first to organize it.")
    );
}

#[test]
fn an_unreadable_library_disables_pinning_with_an_explanation() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("readonly");
    let ini = crate::library::store::library_file(&scratch.folder());
    std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
    std::fs::write(&ini, "version=99\r\n").unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    open_note(&window, &scratch, "a.md", "a");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::library_host::flush_now(window.hwnd);
    assert_eq!(std::fs::read_to_string(&ini).unwrap(), "version=99\r\n");
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("can't be read, so pins are off until it is fixed or removed"))
    );
}

#[test]
fn renaming_a_note_renames_its_file_and_keeps_its_metadata() {
    // Break caught: a rename losing the note's pin, or the tab still pointing at the old path.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rename");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let old = open_note(&window, &scratch, "a.md", "text");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    // The name bar itself: Note: Rename… would edit the note's row in the tree.
    crate::window::library_host::rename_note(window.hwnd);
    assert_eq!(
        app_mut(window.hwnd).name_box.as_ref().unwrap().text(),
        "a.md"
    );
    type_into_name_box(window.hwnd, "Plan");
    crate::window::library_host::name_box_submit(window.hwnd);
    let new = scratch.folder().join("Plan.md");
    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "text");
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(new.as_path())
    );
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    assert!(state.is_pinned(&new));
}

#[test]
fn renaming_onto_an_existing_file_is_refused() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rename-clash");
    scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let a = open_note(&window, &scratch, "a.md", "a");
    // The name bar itself: Note: Rename… would edit the note's row in the tree.
    crate::window::library_host::rename_note(window.hwnd);
    type_into_name_box(window.hwnd, "B.md");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(a.exists());
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("b.md")).unwrap(),
        "b"
    );
    assert!(
        app_mut(window.hwnd)
            .name_box
            .as_ref()
            .unwrap()
            .error()
            .is_some()
    );
}

#[test]
fn renaming_a_note_changing_only_letter_case_works() {
    // Break caught: the clash check or the tab collision check treating the note's own file
    // as a different one that already has the new name.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rename-case");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    open_note(&window, &scratch, "plan.md", "text");
    // The name bar itself: Note: Rename… would edit the note's row in the tree.
    crate::window::library_host::rename_note(window.hwnd);
    type_into_name_box(window.hwnd, "Plan.md");
    crate::window::library_host::name_box_submit(window.hwnd);
    let names: Vec<String> = std::fs::read_dir(scratch.folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".md"))
        .collect();
    assert_eq!(names, ["Plan.md"]);
    let new = scratch.folder().join("Plan.md");
    assert_eq!(
        app_mut(window.hwnd)
            .tabs
            .active()
            .unwrap()
            .path
            .as_deref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        Some("Plan.md".to_owned())
    );
    assert!(!name_box_visible(window.hwnd));
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "text");
}

#[test]
fn deleting_a_note_asks_then_recycles_it_and_keeps_its_record_hidden() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("delete-note");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::answer_next_confirm(|_| false);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert!(path.exists());
    crate::window::answer_next_confirm(|_| true);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert!(!path.exists());
    assert_eq!(super::super::tab_count(window.hwnd), 0);
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    let record = state.record_for(&path).unwrap();
    assert!(record.deleted);
    assert!(state.local.missing_since(record.id).is_some());
    assert!(state.notes.is_empty());
}

#[test]
fn a_note_renamed_outside_fastpad_moves_its_open_tab_and_autosave_resumes() {
    // Break caught: the rescan looking the tab up through the old, now missing path, so the
    // tab kept it: autosave stayed paused, and Keep mine or Ctrl+S re-created the old file
    // while the metadata followed the new one.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("outside-rename");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let old = open_note(&window, &scratch, "a.md", "text");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::library_host::flush_now(window.hwnd);
    let new = scratch.folder().join("b.md");
    std::fs::rename(&old, &new).unwrap();
    editor.set_text("edited").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Paused,
        "the old file is gone, so autosave pauses until the rescan"
    );

    scratch.install(window.hwnd);

    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(new.as_path()));
    assert!(!active.autosave_paused, "the moved file is unchanged");
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Saved
    );
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "edited");
    assert!(!old.exists(), "the old name is not re-created");
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    assert!(state.record_for(&new).unwrap().pinned);
}

#[test]
fn a_note_moved_and_changed_outside_fastpad_follows_but_does_not_autosave_over_the_change() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("outside-rename-changed");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let old = open_note(&window, &scratch, "a.md", "text");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::library_host::flush_now(window.hwnd);
    let new = scratch.folder().join("b.md");
    std::fs::rename(&old, &new).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::OpenOptions::new()
        .append(true)
        .open(&new)
        .and_then(|mut file| std::io::Write::write_all(&mut file, b" and more"))
        .unwrap();

    scratch.install(window.hwnd);
    editor.set_text("mine").unwrap();

    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(new.as_path())
    );
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Paused
    );
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "text and more");
}

#[test]
fn renaming_to_the_prefilled_name_keeps_a_non_note_or_extensionless_file_as_it_is() {
    // Break caught: "script.lua" prefilled and submitted as-is becoming script.lua.lua, and an
    // extensionless README gaining ".md".
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rename-kinds");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    for name in ["script.lua", "README"] {
        let path = open_note(&window, &scratch, name, "x");
        execute_command(window.hwnd, CommandId::NoteRename);
        assert_eq!(app_mut(window.hwnd).name_box.as_ref().unwrap().text(), name);
        crate::window::library_host::name_box_submit(window.hwnd);
        assert!(!name_box_visible(window.hwnd));
        assert!(path.exists(), "{name} is left alone");
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(path.as_path())
        );
    }
    let mut names: Vec<String> = std::fs::read_dir(scratch.folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    assert_eq!(names, ["README", "script.lua"]);

    execute_command(window.hwnd, CommandId::NoteRename);
    type_into_name_box(window.hwnd, "tool");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(
        scratch.folder().join("tool").exists(),
        "no extension is added"
    );
}

#[test]
fn turning_notes_mode_on_labels_every_untitled_tab_without_switching_to_it() {
    // Break caught: only the active tab getting its label, every other untitled tab reading
    // "Untitled" until the user visited it.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).settings.notes_mode = false;
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("First idea\nbody").unwrap();
    let first = app_mut(window.hwnd).tabs.active().unwrap().id;
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Second idea").unwrap();
    pump_posted_messages(window.hwnd);
    let title = |id| app_mut(window.hwnd).tabs.document(id).unwrap().title();
    assert_eq!(title(first), "Untitled *");

    let settings = RecoveryScratch::new("labels-toggle");
    super::super::save_settings_to(Some(settings.path().join("fastpad.ini")));
    execute_command(window.hwnd, CommandId::ToggleNotesMode);
    super::super::save_settings_to(None);
    assert!(app_mut(window.hwnd).settings.notes_mode);

    assert_eq!(title(first), "First idea *");
    let active = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert_ne!(active, first, "no tab switch");
    assert_eq!(title(active), "Second idea *");
}

#[test]
fn restored_and_recovered_untitled_tabs_are_labelled_from_their_text() {
    // Break caught: a background untitled tab restored from the session reading "Untitled"
    // because only the active tab's label is computed.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-labels");
    let recovery = scratch.path().join("Recovery");
    let first_id = RecoveryId::from_u128(0x1ab1);
    let second_id = RecoveryId::from_u128(0x1ab2);
    for (id, text) in [(first_id, "# Shopping\nmilk"), (second_id, "Plans")] {
        write_snapshot(&recovery, &Snapshot::new(id, None, Encoding::Utf8, text)).unwrap();
    }
    write_session(
        &scratch,
        vec![
            SessionEntry::new(SessionSource::Snapshot(first_id)),
            SessionEntry::new(SessionSource::Snapshot(second_id)),
        ],
        1,
    );
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);

    run_session_restore(window.hwnd);

    {
        let app = app_mut(window.hwnd);
        let documents = app.tabs.documents().collect::<Vec<_>>();
        assert_eq!(documents[0].title(), "Shopping *");
        assert_eq!(documents[0].label_watch, 0);
        assert_eq!(documents[1].title(), "Plans *");
    }

    // A crash-recovered tab keeps its "Recovered:" title but knows its label for a save.
    let root = RecoveryScratch::new("recovered-label");
    write_snapshot(
        root.path(),
        &Snapshot::new(
            RecoveryId::from_u128(0x1ab3),
            None,
            Encoding::Utf8,
            "Lost thought",
        ),
    )
    .unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    super::super::recover_snapshots(window.hwnd);
    let recovered = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(recovered.untitled_label.as_deref(), Some("Lost thought"));
}

#[test]
fn a_metadata_flush_leaves_the_local_file_alone_and_an_expansion_change_writes_it() {
    // Break caught: every 500 ms metadata flush, every rescan or every opened note
    // re-encoding and rewriting the whole per-PC local file (with its scan cache) on the UI
    // thread.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("local-untouched");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    open_note(&window, &scratch, "a.md", "a");
    crate::window::library_host::flush_now(window.hwnd);
    let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
    assert!(local.exists(), "the load wrote it");
    std::fs::remove_file(&local).unwrap();

    let b = scratch.note("b.md", "b");
    super::super::open_path(window.hwnd, &b).unwrap();
    crate::window::library_host::flush_now(window.hwnd);
    assert!(!local.exists(), "opening a note changes nothing local");

    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::library_host::flush_now(window.hwnd);
    assert!(crate::library::store::library_file(&scratch.folder()).exists());
    assert!(!local.exists(), "only library.ini changed");

    crate::window::library_host::with_state(window.hwnd, |state| {
        state.local.set_expanded(std::path::Path::new("sub"), true);
    });
    crate::window::library_host::flush_now(window.hwnd);
    assert!(
        std::fs::read_to_string(&local)
            .unwrap()
            .contains("expanded=sub\r\n"),
        "an expansion change is written"
    );
}

#[test]
fn the_library_step_checks_no_folder_on_the_ui_thread_and_the_worker_falls_back() {
    // Break caught: the startup existence check on a remembered folder on an offline mapped
    // drive stalling the UI thread for an SMB timeout.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("startup-gone");
    let gone = scratch.root.join("gone");
    let mut folders = crate::library::local::RecentFolders::default();
    folders.push(gone.clone());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &folders,
    )
    .unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());

    let before = crate::library::folder_checks();
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(crate::library::folder_checks(), before, "no stat here");
    assert_eq!(crate::window::library_host::folder(window.hwnd), Some(gone));

    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    let fallback =
        crate::library::normalize_folder(&crate::platform::paths::default_notes_folder().unwrap());
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(fallback)
    );
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("could not find the notebook")),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn a_session_that_ended_with_no_notebook_open_opens_none_at_startup() {
    // Break caught: open=none ignored, so a closed notebook came back at the next start, or
    // a worker started (and stat-ed a folder) to find out there was nothing to open.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("startup-closed");
    let mut folders = crate::library::local::RecentFolders::default();
    folders.push(scratch.folder());
    folders.set_closed(true);
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &folders,
    )
    .unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());

    let before = crate::library::folder_checks();
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(crate::library::folder_checks(), before);
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    assert!(!app_mut(window.hwnd).library.scanning, "no worker starts");
}

#[test]
fn turning_notes_mode_off_says_so_when_metadata_cannot_be_written_and_closes_the_name_box() {
    // Break caught: the toggle dropping unsaved pins silently when the flush
    // failed, or leaving a name box open that did nothing on Enter.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = open_first_save_box("mode-off-flush");
    let a = scratch.note("a.md", "a");
    crate::window::library_host::with_state(window.hwnd, |state| {
        let mut ids = crate::library::ids::IdSource::new(1, 1);
        let target = state.note_ref(&mut ids, &a);
        state
            .apply(crate::library::ops::PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
    });
    std::fs::write(scratch.folder().join(".fastpad"), "not a directory").unwrap();

    app_mut(window.hwnd).settings.notes_mode = false;
    crate::window::library_host::notes_mode_changed(window.hwnd, false);

    assert!(!name_box_visible(window.hwnd));
    assert!(
        app_mut(window.hwnd).library.state.is_none(),
        "the setting applies"
    );
    let expected = format!(
        "Metadata changes could not be written to {}",
        scratch
            .folder()
            .join(".fastpad")
            .join("library.ini")
            .display()
    );
    assert!(
        notices(window.hwnd).contains(&expected),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn deleting_a_note_with_unsaved_edits_says_they_are_discarded() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("delete-dirty");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a");
    crate::window::answer_next_confirm(|_| false);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Move \u{201c}a.md\u{201d} to the Recycle Bin?")
    );
    editor.set_text("unsaved").unwrap();
    crate::window::answer_next_confirm(|_| false);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Move \u{201c}a.md\u{201d} to the Recycle Bin and discard unsaved changes?")
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a",
        "nothing is autosaved first"
    );
}

#[test]
fn opening_another_folder_autosaves_the_old_folders_notes_and_normalizes_the_new_path() {
    // Break caught: a dirty note in the old folder left unsaved (and no longer autosaved)
    // after a switch, or a folder spelled with a trailing separator becoming a second
    // recent folder.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("switch-autosave-a");
    let second = LibraryScratch::new("switch-autosave-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "one");
    // Off while editing, so nothing but the switch saves it.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    editor.set_text("two").unwrap();
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);

    let spelled = std::path::PathBuf::from(format!("{}\\", second.folder().display()));
    crate::window::library_host::open_folder(window.hwnd, &spelled);

    assert_eq!(std::fs::read_to_string(&a).unwrap(), "two");
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(second.folder())
    );
    let recent =
        crate::library::local::read_folders(&crate::library::local::folders_file(&first.data()));
    assert_eq!(recent.folders.first(), Some(&second.folder()));
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
}

#[test]
fn a_library_file_held_open_by_a_sync_keeps_the_operations_and_retries_without_a_notice() {
    // Break caught: a sharing violation on library.ini turning organizing off or dropping
    // the pending pins with an error notice.
    use std::os::windows::fs::OpenOptionsExt;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("busy-flush-window");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let a = open_note(&window, &scratch, "a.md", "a");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    // Another PC's sync writes the file, so the flush must re-read it, and holds it open.
    let ini = crate::library::store::library_file(&scratch.folder());
    crate::library::store::write(&ini, &crate::library::model::Library::default()).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&ini)
        .unwrap();
    let before = notices(window.hwnd).len();
    crate::window::library_host::flush_now(window.hwnd);
    drop(lock);
    assert_eq!(
        notices(window.hwnd).len(),
        before,
        "no notice for a brief sync"
    );
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    assert_eq!(state.metadata, crate::library::Metadata::Ready);
    assert_eq!(state.pending.len(), 1);

    crate::window::library_host::flush_now(window.hwnd);
    let reloaded = crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
    assert!(reloaded.is_pinned(&a));
}
