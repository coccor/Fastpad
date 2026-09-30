//! The first save of an untitled note, the name box, notebooks, and autosave.

use super::*;

#[test]
fn the_first_save_of_an_untitled_note_asks_for_a_name_in_the_folder_prefilled_from_its_label() {
    // Break caught: Ctrl+S on a new note opening the system dialog in some random folder, or
    // saving without letting the user confirm the name.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("first-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Meeting: notes?\nbody").unwrap();
    pump_posted_messages(window.hwnd);

    execute_command(window.hwnd, CommandId::Save);
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(name_box.text(), "Meeting notes.md");

    crate::window::library_host::name_box_submit(window.hwnd);
    let saved = scratch.folder().join("Meeting notes.md");
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap(),
        "Meeting: notes?\nbody"
    );
    assert!(!app_mut(window.hwnd).name_box.as_ref().unwrap().is_visible());
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(saved.as_path())
    );
    assert!(
        app_mut(window.hwnd)
            .library
            .state
            .as_ref()
            .unwrap()
            .notes
            .iter()
            .any(|n| n.path == std::path::Path::new("Meeting notes.md"))
    );
    assert!(
        app_mut(window.hwnd)
            .tabs
            .active()
            .unwrap()
            .disk_stamp
            .is_some()
    );
}

#[test]
fn a_name_that_already_exists_is_refused_with_a_suggestion() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("clash");
    scratch.note("Plan.md", "old");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Plan").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    type_into_name_box(window.hwnd, "plan.MD");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("Plan.md")).unwrap(),
        "old"
    );
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(
        name_box.error(),
        Some("plan.MD already exists. Try plan 2.MD.")
    );
}

#[test]
fn browse_and_the_close_prompt_use_the_save_dialog_starting_with_the_suggested_name() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("browse");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Ideas").unwrap();
    assert_eq!(
        crate::window::library_host::suggested_file_name(window.hwnd),
        "Ideas.md"
    );
    let elsewhere = scratch.root.join("elsewhere.md");
    crate::window::answer_next_save_dialog({
        let elsewhere = elsewhere.clone();
        move |_| Some(elsewhere)
    });
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_browse(window.hwnd);
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "Ideas");
}

#[test]
fn with_notes_mode_off_save_uses_the_dialog_as_before() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("mode-off-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).settings.notes_mode = false;
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("x").unwrap();
    let target = scratch.root.join("x.txt");
    crate::window::answer_next_save_dialog({
        let target = target.clone();
        move |_| Some(target)
    });
    execute_command(window.hwnd, CommandId::Save);
    assert!(target.exists());
    assert!(app_mut(window.hwnd).name_box.is_none());
    // Break caught: notes-mode naming leaking into plain-editor Save As.
    assert_eq!(
        crate::window::modal::take_last_save_request(),
        Some(("Untitled.txt".to_owned(), None))
    );
}

#[test]
fn save_as_on_a_tab_with_a_path_starts_where_the_dialog_always_did() {
    // Break caught: Save As of an ordinary file jumping to the notes folder.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("save-as-titled");
    let outside = scratch.root.join("outside.txt");
    std::fs::write(&outside, "o").unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &outside).unwrap();
    crate::window::answer_next_save_dialog(|_| None);
    execute_command(window.hwnd, CommandId::SaveAs);
    assert_eq!(
        crate::window::modal::take_last_save_request(),
        Some(("outside.txt".to_owned(), None))
    );
    assert!(app_mut(window.hwnd).name_box.is_none());
}

#[test]
fn a_first_save_never_replaces_a_file_that_took_the_name_after_the_check() {
    // Break caught: a note created in Explorer between the name check and the write being
    // overwritten by the first save.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("first-save-race");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("mine").unwrap();
    let late = scratch.note("Late.md", "theirs");
    let identity = unsafe { super::super::window_identity(window.hwnd) }.unwrap();
    assert_eq!(
        super::super::complete_first_save(window.hwnd, &identity, late.clone()),
        super::super::SaveOutcome::NameTaken
    );
    assert_eq!(std::fs::read_to_string(&late).unwrap(), "theirs");
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert!(active.path.is_none());
    assert!(active.dirty);
}

#[test]
fn escape_in_the_name_box_cancels_the_save() {
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = open_first_save_box("escape");
    let edit = app_mut(window.hwnd).name_box.as_ref().unwrap().edit_hwnd();
    unsafe {
        SendMessageW(
            edit,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE as usize,
            0,
        );
    }
    assert!(!name_box_visible(window.hwnd));
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert!(active.path.is_none());
    assert!(active.dirty);
}

#[test]
fn the_name_box_closes_with_its_tab_or_when_another_tab_is_activated() {
    // Break caught: a box left naming a tab that is gone or not the one on screen.
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = open_first_save_box("tab-change");
    super::super::create_new_document(window.hwnd).unwrap();
    assert!(!name_box_visible(window.hwnd));

    execute_command(window.hwnd, CommandId::Save);
    assert!(name_box_visible(window.hwnd));
    super::super::close_active_document(window.hwnd);
    assert!(!name_box_visible(window.hwnd));
}

#[test]
fn the_name_box_closes_once_its_tab_is_saved_another_way() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = open_first_save_box("saved-elsewhere");
    super::super::save_path_as(window.hwnd, &scratch.root.join("elsewhere.md"));
    assert!(!name_box_visible(window.hwnd));
}

#[test]
fn saving_again_while_the_box_is_open_keeps_what_was_typed() {
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = open_first_save_box("save-twice");
    type_into_name_box(window.hwnd, "Typed name.md");
    execute_command(window.hwnd, CommandId::Save);
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(name_box.text(), "Typed name.md");
}

#[test]
fn opening_find_closes_the_name_box() {
    let _scintilla = load_native_scintilla();
    let (_scratch, window, _editor) = open_first_save_box("find-closes");
    execute_command(window.hwnd, CommandId::Find);
    assert!(!name_box_visible(window.hwnd));
    assert!(app_mut(window.hwnd).find_bar().unwrap().is_visible());
}

#[test]
fn typed_names_with_invalid_characters_or_device_names_save_under_the_sanitized_name() {
    // Break caught: a typed "con" or "a/b" failing the save with a Windows path error.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = open_first_save_box("sanitize");
    type_into_name_box(window.hwnd, "a/b: c?.md");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("ab c.md")).unwrap(),
        "Draft"
    );

    super::super::create_new_document(window.hwnd).unwrap();
    app_mut(window.hwnd)
        .editor()
        .unwrap()
        .set_text("Device")
        .unwrap();
    execute_command(window.hwnd, CommandId::Save);
    type_into_name_box(window.hwnd, "con");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("con_.md")).unwrap(),
        "Device"
    );
    assert!(!name_box_visible(window.hwnd));
}

#[test]
fn closing_the_notebook_saves_its_notes_keeps_the_tabs_and_is_remembered_as_closed() {
    // Break caught: Close notebook dropping a dirty note's edits, closing its tab, or leaving
    // folders.ini without open=none so the next start reopens the notebook anyway.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("close-notebook");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &crate::library::local::RecentFolders {
            folders: vec![scratch.folder()],
            ..Default::default()
        },
    )
    .unwrap();
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.set_text("two").unwrap();

    execute_command(window.hwnd, CommandId::CloseNotebook);

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    assert_eq!(super::super::tab_count(window.hwnd), 1);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(path.as_path())
    );
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    assert!(app_mut(window.hwnd).library.state.is_none());
    let folders =
        crate::library::local::read_folders(&crate::library::local::folders_file(&scratch.data()));
    assert!(folders.closed);
    assert!(
        folders.folders.contains(&scratch.folder()),
        "it stays in the recent list"
    );
    // The tab is a plain file now: an edit is not autosaved.
    editor.set_text("three").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
}

#[test]
fn with_no_notebook_open_ctrl_s_uses_the_save_dialog_like_notes_mode_off() {
    // Break caught: after Close notebook, Ctrl+S on a new tab opening the name box for a notebook
    // that is gone, or the Save As dialog starting in the closed notebook under the tab's label.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("no-notebook-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::CloseNotebook);
    execute_command(window.hwnd, CommandId::New);
    editor.set_text("Plan\nbody").unwrap();
    let target = scratch.root.join("plan.txt");
    crate::window::answer_next_save_dialog({
        let target = target.clone();
        move |_| Some(target)
    });

    execute_command(window.hwnd, CommandId::Save);

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Plan\nbody");
    assert!(
        app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_none_or(|name_box| !name_box.is_visible())
    );
    assert_eq!(
        crate::window::modal::take_last_save_request(),
        Some(("Untitled.txt".to_owned(), None))
    );
}

#[test]
fn the_notebook_favorite_toggles_in_folders_ini_and_the_cached_lists() {
    // Break caught: a star that changes the sidebar but not folders.ini (lost at restart), or
    // favorite lists that read folders.ini on every sidebar paint.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("favorite-notebook");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let file = crate::library::local::folders_file(&scratch.data());

    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    assert!(crate::window::library_host::is_favorite(window.hwnd));
    assert_eq!(
        crate::window::library_host::favorites(window.hwnd),
        vec![scratch.folder()]
    );
    assert!(crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("to favorite notebooks"))
    );

    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    assert!(!crate::window::library_host::is_favorite(window.hwnd));
    assert!(!crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));

    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    crate::window::library_host::remove_favorite(window.hwnd, &scratch.folder());
    assert!(crate::window::library_host::favorites(window.hwnd).is_empty());
    assert!(!crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));

    // Listing answers from the cache: a file changed behind FastPad's back is not re-read.
    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    crate::library::local::write_folders(&file, &crate::library::local::RecentFolders::default())
        .unwrap();
    assert_eq!(
        crate::window::library_host::favorites(window.hwnd),
        vec![scratch.folder()]
    );
}

#[test]
fn opening_a_listed_notebook_that_is_missing_says_so_and_changes_nothing() {
    // Break caught: a click on an offline favorite unloading the open notebook first, checking
    // the drive on the UI thread, or falling back to Documents\FastPad the way startup does.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("listed-missing");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let missing = scratch.root.join("gone");
    let checks = crate::library::folder_checks();

    crate::window::library_host::open_listed_notebook(window.hwnd, &missing);
    assert_eq!(
        crate::library::folder_checks(),
        checks,
        "checked on the worker"
    );
    pump_until(window.hwnd, || {
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("is not available"))
    });

    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(scratch.folder())
    );
    assert!(app_mut(window.hwnd).library.state.is_some());
    let folders =
        crate::library::local::read_folders(&crate::library::local::folders_file(&scratch.data()));
    assert!(!folders.folders.contains(&missing));
}

#[test]
fn opening_a_listed_notebook_switches_once_the_worker_finds_it() {
    // Break caught: an explicit open that switches before the check lands (so a missing folder
    // would already have unloaded the notebook), or never switches at all.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("listed-a");
    let second = LibraryScratch::new("listed-b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    first.install(window.hwnd);

    crate::window::library_host::open_listed_notebook(window.hwnd, &second.folder());
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(first.folder()),
        "nothing changes before the check lands"
    );
    pump_until(window.hwnd, || {
        crate::window::library_host::folder(window.hwnd) == Some(second.folder())
    });
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    assert_eq!(
        crate::window::library_host::recent_notebooks(window.hwnd).first(),
        Some(&second.folder())
    );
}

#[test]
fn a_stale_listed_notebook_check_is_dropped_once_the_user_has_moved_on() {
    // Break caught: a slow existence check for a notebook landing after the user already
    // closed the open notebook (or switched to another one), reopening or replacing it anyway.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("stale-check");
    let stale = LibraryScratch::new("stale-check-target");
    let fresh = LibraryScratch::new("stale-check-fresh");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);

    crate::window::library_host::open_listed_notebook(window.hwnd, &stale.folder());
    // Close notebook runs on this thread before the check's worker answer is pumped: the
    // close must invalidate the still-in-flight check.
    execute_command(window.hwnd, CommandId::CloseNotebook);
    // Give the worker's existence check time to land and be pumped, then confirm it changed
    // nothing: there is no positive signal for "was dropped", so this waits out a generous
    // margin instead of polling for an effect that must not occur.
    for _ in 0..60 {
        pump_posted_messages(window.hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        None,
        "a check that started before Close notebook must not reopen the notebook"
    );

    // A later, non-stale listed open still works normally.
    crate::window::library_host::open_listed_notebook(window.hwnd, &fresh.folder());
    pump_until(window.hwnd, || {
        crate::window::library_host::folder(window.hwnd) == Some(fresh.folder())
    });
}

#[test]
fn a_note_inside_the_folder_autosaves_and_closing_it_never_prompts() {
    // Break caught: a notes-folder file still asking "Save changes?" or losing edits on close.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("autosave");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.set_text("two").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Saved
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);

    editor.set_text("three").unwrap();
    super::super::close_active_document(window.hwnd); // no answer_next_close_prompt: a prompt would fail the test
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "three");
}

#[test]
fn a_file_changed_on_disk_pauses_autosave_until_the_user_chooses() {
    // Break caught: autosave silently overwriting an edit OneDrive just synced from another PC.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("guard");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "from the other PC").unwrap();
    editor.set_text("mine").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Paused
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "from the other PC");
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("changed on disk"))
    );

    execute_command(window.hwnd, CommandId::NoteReloadFromDisk);
    assert_eq!(editor.text().unwrap(), "from the other PC");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().autosave_paused);

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "again").unwrap();
    editor.set_text("mine for real").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Paused
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "again");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    execute_command(window.hwnd, CommandId::NoteKeepMine);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine for real");
}

#[test]
fn files_outside_the_folder_and_folders_with_autosave_off_are_not_autosaved() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("not-eligible");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let outside = scratch.root.join("outside.md");
    std::fs::write(&outside, "x").unwrap();
    super::super::open_path(window.hwnd, &outside).unwrap();
    editor.set_text("y").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible
    );
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "x");

    let inside = open_note(&window, &scratch, "b.md", "b");
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    editor.set_text("c").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible
    );
    assert_eq!(std::fs::read_to_string(&inside).unwrap(), "b");
    assert!(
        !app_mut(window.hwnd)
            .library
            .state
            .as_ref()
            .unwrap()
            .local
            .autosave
    );
}

#[test]
fn switching_tabs_autosaves_the_tab_being_left() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("switch-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let a = open_note(&window, &scratch, "a.md", "a");
    let b = scratch.note("b.md", "b");
    super::super::open_path(window.hwnd, &b).unwrap();
    editor.set_text("b2").unwrap();
    execute_command(window.hwnd, CommandId::SelectTab1);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b2");
    editor.set_text("a2").unwrap();
    super::super::create_new_document(window.hwnd).unwrap();
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "a2",
        "a new tab also leaves the old one"
    );
}

#[test]
fn a_note_with_no_known_disk_stamp_pauses_instead_of_overwriting() {
    // Break caught: a tab restored from a snapshot autosaving over a file that changed while
    // FastPad was closed, because it has no stamp to compare against.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("no-stamp");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "on disk");
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    app_mut(window.hwnd)
        .tabs
        .document_mut(id)
        .unwrap()
        .disk_stamp = None;
    editor.set_text("restored").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Paused
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "on disk");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn a_single_edit_autosaves_once_the_idle_timer_fires() {
    // Break caught: the first keystroke after a save not arming the timer, because the edit
    // notification arrives before the tab is marked dirty.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("idle-timer");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.replace_target(0..0, "x").unwrap();
    pump_until(window.hwnd, || {
        std::fs::read_to_string(&path).is_ok_and(|text| text == "xone")
    });
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn a_note_whose_file_was_deleted_and_has_no_stamp_is_not_recreated() {
    // Break caught: a restored tab re-creating a note the user deleted on another PC.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("no-stamp-deleted");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "on disk");
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    app_mut(window.hwnd)
        .tabs
        .document_mut(id)
        .unwrap()
        .disk_stamp = None;
    std::fs::remove_file(&path).unwrap();
    editor.set_text("restored").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::Paused
    );
    assert!(!path.exists());
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn a_folder_whose_state_has_not_loaded_is_not_autosaved() {
    // Break caught: a folder with autosave turned off being autosaved while it (re)loads.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("state-loading");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    assert!(
        !app_mut(window.hwnd)
            .library
            .state
            .as_ref()
            .unwrap()
            .local
            .autosave
    );
    app_mut(window.hwnd).library.state = None;
    editor.set_text("two").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn an_edit_made_while_the_folder_loads_autosaves_once_it_has_loaded() {
    // Break caught: a keystroke that arrived before the folder's state never being autosaved,
    // because it found no state to arm the idle timer and nothing armed it after the load.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("edit-while-loading");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    app_mut(window.hwnd).library.state = None;
    editor.replace_target(0..0, "x").unwrap();
    let waited = std::time::Instant::now();
    while waited.elapsed() < std::time::Duration::from_millis(1_300) {
        pump_posted_messages(window.hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    scratch.install(window.hwnd);
    pump_until(window.hwnd, || {
        std::fs::read_to_string(&path).is_ok_and(|text| text == "xone")
    });
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn keep_my_version_on_an_untitled_tab_does_nothing() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("keep-untitled");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("draft").unwrap();
    let before = notices(window.hwnd).len();
    execute_command(window.hwnd, CommandId::NoteKeepMine);
    assert_eq!(notices(window.hwnd).len(), before);
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert!(active.path.is_none() && active.dirty);
}

#[test]
fn a_failed_autosave_names_the_note_once_and_keeps_the_tab_dirty() {
    // Break caught: a failed write marking the note clean, or two notices for one failure.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("autosave-fails");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.set_text("two").unwrap();
    // A file held open without sharing makes the atomic replace fail; its stamp is unchanged.
    use std::os::windows::fs::OpenOptionsExt;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let before = notices(window.hwnd).len();
    let outcome = crate::window::library_host::autosave_active(window.hwnd);
    drop(lock);
    assert_eq!(outcome, crate::window::library_host::Autosave::Failed);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    let added = &notices(window.hwnd)[before..];
    assert_eq!(added.len(), 1, "{added:?}");
    assert!(added[0].starts_with("Autosave failed for "), "{added:?}");
}

#[test]
fn closing_the_window_autosaves_every_eligible_note_and_keeps_the_active_tab() {
    // Break caught: a close leaving a second dirty note unsaved, saving a file outside the
    // folder or a paused note, or leaving the session on whichever tab saved last.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("autosave-all");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let a = open_note(&window, &scratch, "a.md", "a");
    // Off while the tabs are made dirty, so switching between them does not save them.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    editor.set_text("a2").unwrap();
    let b = scratch.note("b.md", "b");
    super::super::open_path(window.hwnd, &b).unwrap();
    editor.set_text("b2").unwrap();
    let paused = scratch.note("p.md", "p");
    super::super::open_path(window.hwnd, &paused).unwrap();
    editor.set_text("p2").unwrap();
    let paused_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    app_mut(window.hwnd)
        .tabs
        .document_mut(paused_id)
        .unwrap()
        .autosave_paused = true;
    let outside = scratch.root.join("outside.md");
    std::fs::write(&outside, "x").unwrap();
    super::super::open_path(window.hwnd, &outside).unwrap();
    editor.set_text("x2").unwrap();
    let outside_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);

    crate::window::library_host::autosave_all(window.hwnd);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a2");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b2");
    assert_eq!(std::fs::read_to_string(&paused).unwrap(), "p");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "x");
    let app = app_mut(window.hwnd);
    let dirty = |path: &std::path::Path| {
        app.tabs
            .documents()
            .find(|document| document.path.as_deref() == Some(path))
            .unwrap()
            .dirty
    };
    assert!(!dirty(&a) && !dirty(&b));
    assert!(dirty(&paused) && dirty(&outside));
    assert_eq!(app.tabs.active().unwrap().id, outside_id);
}

#[test]
fn switching_to_another_app_autosaves_the_active_note() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("deactivate");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.set_text("two").unwrap();
    crate::window::library_host::activation_changed(window.hwnd, false);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
}
