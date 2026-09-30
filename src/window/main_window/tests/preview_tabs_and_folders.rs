//! Preview tabs, opening folders, the library step, drops and session-close fallbacks.

use super::*;

#[test]
fn the_first_edit_promotes_the_preview_so_a_later_click_opens_a_new_preview() {
    // Break caught: a click replacing a preview the user had started typing into, which drops
    // their text, or the edit not promoting so the tab keeps being replaced.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-edit");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    // Autosave (unrelated to preview promotion) would otherwise clean `a` the moment `b` is
    // opened, since opening a file autosaves the tab being left; see
    // `switching_tabs_autosaves_the_tab_being_left`.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);

    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();
    assert_eq!(
        tab_paths(window.hwnd),
        [Some(a.clone())],
        "the empty start tab is reused"
    );
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);

    editor.set_text("a, edited").unwrap();
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();
    assert_eq!(tab_paths(window.hwnd), [Some(a.clone()), Some(b.clone())]);
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
    let a_tab = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
    assert!(app_mut(window.hwnd).tabs.document(a_tab).unwrap().dirty);
}

#[test]
fn a_second_preview_replaces_the_first_in_place_keeping_its_tab_index() {
    // Break caught: the replacement landing at the end of the strip, or a normal tab being
    // replaced instead of the preview.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-replace");
    let x = scratch.note("x.md", "x");
    let a = scratch.note("a.md", "a");
    let y = scratch.note("y.md", "y");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &x).unwrap();
    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();
    super::super::open_path(window.hwnd, &y).unwrap();

    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();

    assert_eq!(tab_paths(window.hwnd), [Some(x), Some(b), Some(y)]);
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
}

#[test]
fn opening_an_already_open_note_switches_to_its_tab() {
    // Break caught: a click on an open note replacing the preview with a second tab for the
    // same file, or doing nothing.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-open");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();

    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();

    assert_eq!(super::super::tab_count(window.hwnd), 2);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);
}

#[test]
fn a_permanent_open_a_save_or_a_tab_double_click_keeps_the_preview() {
    // Break caught: Ctrl+Enter or a double-click opening a second tab for a note already in the
    // preview, or a saved preview still being replaced by the next click.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-keep");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let c = scratch.note("c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);

    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();
    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Permanent, false).unwrap();
    assert_eq!(super::super::tab_count(window.hwnd), 1);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();
    crate::window::library_host::document_saved(window.hwnd);
    assert_eq!(
        app_mut(window.hwnd).tabs.preview_id(),
        None,
        "a save promotes"
    );

    super::super::open_note(window.hwnd, &c, super::super::OpenMode::Preview, false).unwrap();
    let index = app_mut(window.hwnd).tabs.active_index();
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let center = super::super::strip_layout(window.hwnd)
        .unwrap()
        .tab(index)
        .unwrap()
        .center();
    let pack = |x: i32, y: i32| (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
    // Both clicks carry the same message time, well inside the double-click time.
    for _ in 0..2 {
        unsafe {
            SendMessageW(
                group,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN,
                1,
                pack(center.x, center.y),
            );
            SendMessageW(
                group,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
                0,
                pack(center.x, center.y),
            );
        }
    }
    assert_eq!(
        app_mut(window.hwnd).tabs.preview_id(),
        None,
        "a double-click promotes"
    );
    assert_eq!(super::super::tab_count(window.hwnd), 3);
}

#[test]
fn the_session_keeps_every_tabs_position_and_restores_it() {
    // Break caught: only the shown tab's caret saved, so every other tab reopens at the top;
    // or a restored background tab's position overwritten when the next entry opens.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("session-positions");
    let a = scratch.note("a.md", &"line\n".repeat(2000));
    let b = scratch.note("b.md", "b");
    let recovery = RecoveryScratch::new("session-positions");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Permanent, false).unwrap();
    let away = crate::editor::ViewState {
        caret: 1210 * 5,
        anchor: 1210 * 5,
        first_line: 1200,
        x_offset: 0,
    };
    editor.apply_view_state(away).unwrap();
    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Permanent, false).unwrap();

    let session = super::super::build_session(window.hwnd, recovery.path()).unwrap();
    let entries = &session.groups[0].entries;
    assert_eq!((entries[0].caret, entries[0].first_line), (1210 * 5, 1200));
    assert_eq!(session.groups[0].active, 1);

    for id in [b.clone(), a.clone()].map(|path| app_mut(window.hwnd).tabs.find_path(&path)) {
        super::super::close_document_without_prompt(window.hwnd, id.unwrap());
    }
    for entry in entries {
        let group = app_mut(window.hwnd).tabs.active_group();
        super::super::restore_session_entry(window.hwnd, group, entry).unwrap();
    }
    execute_command(window.hwnd, CommandId::SelectTab1);
    let restored = editor.view_state().unwrap();
    assert_eq!((restored.caret, restored.first_line), (1210 * 5, 1200));
}

#[test]
fn a_preview_tab_is_kept_by_the_session_and_comes_back_as_a_normal_tab() {
    // Break caught: the session skipping the preview tab, so it vanishes at restart, or the
    // restored tab still being a preview that the next click silently replaces.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-session");
    let a = scratch.note("a.md", "a");
    let recovery = RecoveryScratch::new("preview-session");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();

    let session = super::super::build_session(window.hwnd, recovery.path()).unwrap();
    assert_eq!(session.groups[0].entries.len(), 1);
    assert!(
        matches!(&session.groups[0].entries[0].source, SessionSource::File(path) if *path == a)
    );

    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    super::super::close_document_without_prompt(window.hwnd, id);
    let group = app_mut(window.hwnd).tabs.active_group();
    super::super::restore_session_entry(window.hwnd, group, &session.groups[0].entries[0]).unwrap();
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
}

#[test]
fn opening_another_folder_flushes_the_old_one_remembers_the_new_one_and_keeps_tabs() {
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("switch-a");
    let second = LibraryScratch::new("switch-b");
    let a = first.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    first.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
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

    crate::window::answer_next_folder_dialog({
        let folder = second.folder();
        move |_| Some(folder)
    });
    execute_command(window.hwnd, CommandId::OpenFolder);

    assert!(
        crate::library::store::library_file(&first.folder()).exists(),
        "old folder flushed"
    );
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(second.folder())
    );
    let recent =
        crate::library::local::read_folders(&crate::library::local::folders_file(&first.data()));
    assert_eq!(recent.folders.first(), Some(&second.folder()));
    assert_eq!(
        super::super::tab_count(window.hwnd),
        1,
        "open tabs stay open"
    );
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
}

#[test]
fn opening_a_path_that_is_not_a_folder_explains_why() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    crate::window::library_host::open_folder(
        window.hwnd,
        std::path::Path::new(r"Z:\no\such\folder"),
    );
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("is not a folder"))
    );
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
}

#[test]
fn a_failed_flush_keeps_the_current_folder_open() {
    // Break caught: switching folders after library.ini could not be written, which drops
    // the unsaved pins with the old state.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("flushfail-a");
    let second = LibraryScratch::new("flushfail-b");
    let a = first.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    first.install(window.hwnd);
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
    // A file where the .fastpad directory belongs makes the write fail.
    std::fs::write(first.folder().join(".fastpad"), "not a directory").unwrap();

    crate::window::library_host::open_folder(window.hwnd, &second.folder());

    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(first.folder())
    );
    assert!(app_mut(window.hwnd).library.state.is_some());
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("kept this notebook open")),
        "{:?}",
        notices(window.hwnd)
    );
    let recent =
        crate::library::local::read_folders(&crate::library::local::folders_file(&first.data()));
    assert!(!recent.folders.contains(&second.folder()));
}

#[test]
fn a_recent_folder_pick_opens_the_row_that_was_shown() {
    // Break caught: resolving the chosen row against folders.ini re-read after the picker
    // opened, which opens a different folder when another window changed the list.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("recent-a");
    let second = LibraryScratch::new("recent-b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let folders_file = crate::library::local::folders_file(&first.data());
    let write = |order: Vec<std::path::PathBuf>| {
        crate::library::local::write_folders(
            &folders_file,
            &crate::library::local::RecentFolders {
                folders: order,
                ..Default::default()
            },
        )
        .unwrap();
    };
    write(vec![first.folder(), second.folder()]);
    execute_command(window.hwnd, CommandId::OpenRecentFolder);
    write(vec![second.folder(), first.folder()]);

    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::RecentFolder,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    pump_until(window.hwnd, || {
        crate::window::library_host::folder(window.hwnd) == Some(first.folder())
    });
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
}

#[test]
fn a_launch_argument_naming_a_folder_is_not_reported_as_a_failed_open() {
    // Break caught: `fastpad D:\Notes` opening the folder as the library and then also
    // trying to open it as a file, which reports "could not open".
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("launch-dir");
    let mut app = make_app();
    app.launch.request = crate::launch::LaunchRequest::Open(scratch.folder().into_os_string());
    let window = ProductionWindow::new(app);
    let _editor = install_test_editor(&window);
    let tabs = super::super::tab_count(window.hwnd);

    unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_OPEN_REQUEST, 0, 0) };

    assert!(
        !notices(window.hwnd)
            .iter()
            .any(|n| n.contains("could not open")),
        "{:?}",
        notices(window.hwnd)
    );
    assert_eq!(super::super::tab_count(window.hwnd), tabs);
}

#[test]
fn files_dropped_on_the_editor_reach_the_drop_handler() {
    // Break caught: Scintilla's own OLE drop target refusing Explorer's files, so a drop on
    // the editor (most of the window) did nothing.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editor-drop");
    let note = scratch.note("dropped.md", "dropped");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    crate::window::library_host::accept_editor_file_drops(window.hwnd);

    let effects = crate::editor::file_drop::test_support::drag_and_drop(editor.hwnd(), &[&note]);

    assert_eq!(
        effects,
        [windows_sys::Win32::System::Ole::DROPEFFECT_COPY; 3]
    );
    pump_until(window.hwnd, || editor.text().unwrap() == "dropped");
}

#[test]
fn the_library_step_loads_the_remembered_folder_on_a_worker_thread() {
    // Break caught: the scan running on the UI thread, or the remembered folder ignored.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("startup");
    scratch.note("a.md", "a");
    let mut folders = crate::library::local::RecentFolders::default();
    folders.push(scratch.folder());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &folders,
    )
    .unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(scratch.folder())
    );
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    assert_eq!(
        app_mut(window.hwnd)
            .library
            .state
            .as_ref()
            .unwrap()
            .notes
            .len(),
        1
    );
}

#[test]
fn with_notes_mode_off_the_library_step_does_nothing() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("off");
    let window = ProductionWindow::new(make_app());
    app_mut(window.hwnd).settings.notes_mode = false;
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    assert!(!app_mut(window.hwnd).library.scanning);
}

#[test]
fn a_stale_ready_message_is_dropped_and_writes_are_flushed_on_demand() {
    // Break caught: a slow scan of the previous folder replacing the folder just opened.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("stale");
    let a = scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let stale = crate::window::library_host::test_ready_payload(
        app_mut(window.hwnd).library.generation.wrapping_sub(1),
        Err("old".into()),
    );
    crate::window::library_host::library_ready(window.hwnd, stale);
    assert!(app_mut(window.hwnd).library.state.is_some());
    assert!(notices(window.hwnd).iter().all(|n| !n.contains("old")));

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
    crate::window::library_host::flush_now(window.hwnd);
    assert!(crate::library::store::library_file(&scratch.folder()).exists());
}

#[test]
fn session_restore_holds_the_startup_chain_until_it_finishes() {
    // Break caught: a tab reopened mid-restore posting the language unit, which then runs
    // recovery, binds the IPC pipe and stamps FullyReady before the rest of the session is
    // back, so a forwarded launch can open mid-restore and lose the active tab.
    use windows_sys::Win32::UI::WindowsAndMessaging::{PM_NOREMOVE, PostMessageW};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-chain");
    let first = scratch.path().join("first.txt");
    std::fs::write(&first, "one").unwrap();
    let second = scratch.path().join("second.txt");
    std::fs::write(&second, "two").unwrap();
    write_session(
        &scratch,
        vec![
            SessionEntry::new(SessionSource::File(first)),
            SessionEntry::new(SessionSource::File(second)),
        ],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    let restore = crate::window::WM_FASTPAD_RESTORE_SESSION;
    let language = crate::window::WM_FASTPAD_APPLY_LANGUAGE;
    let recovery = crate::window::WM_FASTPAD_RECOVERY;

    unsafe { PostMessageW(window.hwnd, restore, 0, 0) };
    let mut message = MSG::default();
    assert_ne!(
        unsafe { PeekMessageW(&mut message, window.hwnd, restore, restore, PM_REMOVE) },
        0
    );
    unsafe { DispatchMessageW(&message) };
    assert!(
        app_mut(window.hwnd).session_restore.is_some(),
        "one entry is still to come"
    );
    let mut languages = 0;
    while unsafe { PeekMessageW(&mut message, window.hwnd, language, language, PM_REMOVE) } != 0 {
        languages += 1;
        unsafe { DispatchMessageW(&message) };
    }

    assert!(languages > 0, "the reopened tab posts the language unit");
    let recovery_posted =
        unsafe { PeekMessageW(&mut message, window.hwnd, recovery, recovery, PM_NOREMOVE) } != 0;
    run_session_restore(window.hwnd);
    discard_posted(window.hwnd, language);
    discard_posted(window.hwnd, recovery);
    assert!(
        !recovery_posted,
        "the chain must wait for the restore to finish"
    );
    assert!(app_mut(window.hwnd).session_restore.is_none());
    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
}

#[test]
fn session_close_falls_back_to_the_prompt_when_the_manifest_cannot_be_written() {
    // Break caught: a failed manifest write still closing silently, so the unsaved text is
    // named by no session and the user was never asked about it.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-unwritable");
    let blocker = scratch.path().join("blocker");
    std::fs::write(&blocker, "a file, not a folder").unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    app_mut(window.hwnd).session_path = Some(blocker.join("session.ini"));
    editor.set_text("unsaved words").unwrap();
    let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = std::rc::Rc::clone(&prompted);
    answer_next_close_prompt(move |_| {
        seen.set(true);
        CloseDecision::Cancel
    });

    unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

    assert!(
        prompted.get(),
        "an unwritable manifest must fall back to the prompt"
    );
    assert_ne!(
        unsafe { IsWindow(window.hwnd) },
        0,
        "Cancel keeps the window"
    );
}

#[test]
fn session_close_falls_back_to_the_prompt_when_a_snapshot_cannot_be_written() {
    // Break caught: a dirty tab whose text reached no snapshot file closing silently, so the
    // manifest names nothing for it and the user was never asked.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-no-snapshot");
    let blocker = scratch.path().join("blocker");
    std::fs::write(&blocker, "a file, not a folder").unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    app_mut(window.hwnd).recovery_root = Some(blocker);
    editor.set_text("unsaved words").unwrap();
    let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = std::rc::Rc::clone(&prompted);
    answer_next_close_prompt(move |_| {
        seen.set(true);
        CloseDecision::Cancel
    });

    unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

    assert!(
        prompted.get(),
        "a failed snapshot write must fall back to the prompt"
    );
    assert_ne!(
        unsafe { IsWindow(window.hwnd) },
        0,
        "Cancel keeps the window"
    );
    assert!(!scratch.path().join("session.ini").exists());
}

#[test]
fn session_close_during_a_restore_uses_the_prompt() {
    // Break caught: a close mid-restore writing a manifest of only the tabs reopened so far,
    // silently dropping the entries still to come.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-mid-restore");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    app_mut(window.hwnd).session_restore = Some(crate::session::SessionRestore::new(
        &Session::single(0, Vec::new()),
        None,
    ));
    editor.set_text("unsaved words").unwrap();
    let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = std::rc::Rc::clone(&prompted);
    answer_next_close_prompt(move |_| {
        seen.set(true);
        CloseDecision::Cancel
    });

    unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

    assert!(prompted.get(), "a close mid-restore must use the review");
    assert_ne!(
        unsafe { IsWindow(window.hwnd) },
        0,
        "Cancel keeps the window"
    );
    assert!(!scratch.path().join("session.ini").exists());
    app_mut(window.hwnd).session_restore = None;
}
