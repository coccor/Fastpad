//! Saving the session on close and restoring it at startup.

use super::*;

#[test]
fn session_close_records_every_tab_without_prompting() {
    // Break caught: a session close that still asks about unsaved text, drops an unsaved or
    // clean tab from the manifest, loses the active tab, or deletes the snapshot the next
    // launch needs.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-close");
    let file = scratch.path().join("notes.txt");
    std::fs::write(&file, "saved text").unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    App::open_path(window.hwnd, &file).unwrap();
    execute_command(window.hwnd, CommandId::New);
    editor.set_text("unsaved words").unwrap();
    execute_command(window.hwnd, CommandId::New);
    let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = std::rc::Rc::clone(&prompted);
    answer_next_close_prompt(move |_| {
        seen.set(true);
        CloseDecision::Cancel
    });

    unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

    assert!(!prompted.get(), "session restore must not prompt");
    assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
    let session = crate::session::read(&scratch.path().join("session.ini")).unwrap();
    assert_eq!(
        session.groups[0].entries.len(),
        2,
        "the empty untitled tab is skipped"
    );
    assert_eq!(
        session.groups[0].entries[0].source,
        SessionSource::File(file)
    );
    let SessionSource::Snapshot(id) = session.groups[0].entries[1].source else {
        panic!("the unsaved tab must be recorded as a snapshot");
    };
    assert_eq!(
        session.groups[0].active, 1,
        "the skipped active tab falls back to the one before"
    );
    let snapshot = crate::recovery::snapshot::snapshot_path(&scratch.path().join("Recovery"), id);
    let snapshot = Snapshot::decode(&std::fs::read(snapshot).unwrap()).unwrap();
    assert_eq!(snapshot.text, "unsaved words");
}

#[test]
fn session_close_still_prompts_when_restore_is_off() {
    // Break caught: the setting being ignored, so unsaved text is kept silently even though
    // the user asked to be prompted, or a manifest from an earlier close outliving it.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-off");
    write_session(
        &scratch,
        vec![SessionEntry::new(SessionSource::Snapshot(
            RecoveryId::from_u128(0x5e58),
        ))],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    app_mut(window.hwnd).settings.restore_session = false;
    editor.set_text("dirty").unwrap();
    answer_next_close_prompt(|_| CloseDecision::Cancel);

    unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

    assert_ne!(
        unsafe { IsWindow(window.hwnd) },
        0,
        "Cancel keeps the window"
    );
    assert!(!scratch.path().join("session.ini").exists());
}

#[test]
fn a_three_group_session_comes_back_with_its_layout_views_and_positions() {
    // Break caught: the layout flattened into one group, a view's caret lost, or a document
    // shown in two groups restored twice (or failing on its second snapshot entry).
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-groups");
    let file = scratch.path().join("plan.txt");
    std::fs::write(&file, "one\ntwo\nthree\nfour").unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    super::super::open_path(window.hwnd, &file).unwrap();
    editor
        .apply_view_state(crate::editor::ViewState {
            caret: 5,
            anchor: 5,
            first_line: 1,
            x_offset: 0,
        })
        .unwrap();
    execute_command(window.hwnd, CommandId::SplitRight);
    execute_command(window.hwnd, CommandId::New);
    super::super::group_editor(window.hwnd, super::super::group_order(window.hwnd)[1])
        .unwrap()
        .set_text("unsaved and shared")
        .unwrap();
    execute_command(window.hwnd, CommandId::SplitDown);
    assert_eq!(super::super::group_order(window.hwnd).len(), 3);
    assert!(super::super::save_session_for_close(window.hwnd));
    let saved = std::fs::read_to_string(scratch.path().join("session.ini")).unwrap();
    assert!(
        saved.contains("layout=row(1:0.5,column(2:0.5,3:0.5):0.5)"),
        "{saved}"
    );
    drop(window);

    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    run_session_restore(window.hwnd);
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 3);
    let tabs = &app_mut(window.hwnd).tabs;
    let shared = tabs
        .documents()
        .find(|document| document.path.is_none())
        .map(|document| document.id)
        .unwrap();
    assert_eq!(tabs.views_of(shared), vec![order[1], order[2]]);
    assert_eq!(tabs.documents().count(), 2);
    let first_editor = super::super::group_editor(window.hwnd, order[0]).unwrap();
    assert_eq!(first_editor.view_state().unwrap().caret, 5);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), order[2]);
    assert!(
        notices(window.hwnd).is_empty(),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn a_group_whose_files_are_gone_or_whose_window_fails_folds_into_the_others() {
    // Break caught: an empty group left in the layout after its files vanished, or a group
    // whose window can't be made losing its tabs.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-groups-gone");
    let kept = scratch.path().join("kept.txt");
    std::fs::write(&kept, "kept").unwrap();
    let other = scratch.path().join("other.txt");
    std::fs::write(&other, "other").unwrap();
    let gone = scratch.path().join("gone.txt");
    let group = |number, path: &std::path::Path| crate::session::SessionGroup {
        number,
        active: 0,
        entries: vec![SessionEntry::new(SessionSource::File(path.to_path_buf()))],
    };
    let session = crate::session::Session {
        layout: crate::session::SessionLayout::parse("row(1:0.3,2:0.3,3:0.4)").unwrap(),
        active_group: 0,
        groups: vec![group(1, &kept), group(2, &gone), group(3, &other)],
    };
    crate::session::write(&scratch.path().join("session.ini"), &session).unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    // Group 2's window is made; group 3's fails.
    super::super::fail_group_creation_after(1);
    run_session_restore(window.hwnd);
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 1, "group 2 emptied, group 3's window failed");
    let tabs = &app_mut(window.hwnd).tabs;
    assert!(tabs.find_path(&kept).is_some());
    assert!(
        tabs.find_path(&other).is_some(),
        "group 3's views went to the first group"
    );
}

#[test]
fn session_restore_reopens_files_and_unsaved_text_in_order() {
    // Break caught: restored tabs out of order, an unsaved file reopening untitled (so Ctrl+S
    // asks for a path), a stray empty startup tab, a lost caret, a manifest that restores
    // twice, crash recovery opening a restored snapshot again, or a restored tab still
    // pointing at the exited process's snapshot, which other windows would recover.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-restore");
    let recovery = scratch.path().join("Recovery");
    let notes = scratch.path().join("notes.txt");
    std::fs::write(&notes, "saved text").unwrap();
    let draft = scratch.path().join("draft.txt");
    std::fs::write(&draft, "on disk").unwrap();
    let draft_id = RecoveryId::from_u128(0x5e55);
    write_snapshot(
        &recovery,
        &Snapshot::new(
            draft_id,
            Some(draft.clone()),
            Encoding::Utf8,
            "unsaved draft",
        ),
    )
    .unwrap();
    let scratch_id = RecoveryId::from_u128(0x5e56);
    write_snapshot(
        &recovery,
        &Snapshot::new(scratch_id, None, Encoding::Utf8, "scratch words"),
    )
    .unwrap();
    write_session(
        &scratch,
        vec![
            SessionEntry {
                source: SessionSource::Snapshot(draft_id),
                caret: 3,
                anchor: 1,
                first_line: 0,
            },
            SessionEntry::new(SessionSource::File(notes.clone())),
            SessionEntry::new(SessionSource::Snapshot(scratch_id)),
        ],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);

    run_session_restore(window.hwnd);

    {
        let app = app_mut(window.hwnd);
        let documents = app.tabs.documents().collect::<Vec<_>>();
        assert_eq!(documents.len(), 3, "the empty startup tab is closed");
        assert_eq!(documents[0].path.as_deref(), Some(draft.as_path()));
        assert!(documents[0].dirty);
        assert_eq!(documents[0].title(), "draft.txt *");
        assert_eq!(documents[1].path.as_deref(), Some(notes.as_path()));
        assert!(!documents[1].dirty);
        assert_eq!(documents[2].path, None);
        // Notes mode is on by default: this restored tab was briefly active while its
        // document loaded, and its untitled label was picked up from its first line.
        assert_eq!(documents[2].title(), "scratch words *");
        assert_eq!(app.tabs.active_index(), 0);
    }
    assert_eq!(editor.text().unwrap(), "unsaved draft");
    assert_eq!(editor.selection().unwrap(), 1..3);
    assert!(
        !scratch.path().join("session.ini").exists(),
        "the manifest is consumed"
    );
    let own = |index: usize| {
        let id = app_mut(window.hwnd)
            .tabs
            .documents()
            .nth(index)
            .unwrap()
            .recovery_id;
        crate::recovery::snapshot::snapshot_path(&recovery, id)
    };
    for (index, source, text) in [
        (0, draft_id, "unsaved draft"),
        (2, scratch_id, "scratch words"),
    ] {
        assert!(
            !crate::recovery::snapshot::snapshot_path(&recovery, source).exists(),
            "the dead process's snapshot would look like a crash leftover to other windows"
        );
        let adopted = Snapshot::decode(&std::fs::read(own(index)).unwrap()).unwrap();
        assert_eq!(adopted.text, text);
    }
    let draft_snapshot = own(0);

    unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0) };
    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        3,
        "recovery must not duplicate a tab"
    );

    execute_command(window.hwnd, CommandId::Save);
    assert_eq!(std::fs::read_to_string(&draft).unwrap(), "unsaved draft");
    assert!(
        !draft_snapshot.exists(),
        "saving a restored tab removes its snapshot"
    );
}

#[test]
fn restored_tabs_enter_the_activation_order_in_strip_order_with_the_active_tab_first() {
    // Break caught: the restore wiring missing, so Ctrl+P after a restart lists the tabs in
    // the reverse order they reopened in, not the saved active tab first.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-activation-order");
    let files = ["one.txt", "two.txt", "three.txt"].map(|name| {
        let path = scratch.path().join(name);
        std::fs::write(&path, name).unwrap();
        path
    });
    write_session(
        &scratch,
        files
            .iter()
            .map(|path| SessionEntry::new(SessionSource::File(path.clone())))
            .collect(),
        1,
    );
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);

    run_session_restore(window.hwnd);

    let app = app_mut(window.hwnd);
    let paths = app
        .tabs
        .activation_order()
        .iter()
        .map(|&(_, id)| app.tabs.document(id).unwrap().path.clone().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        [files[1].clone(), files[0].clone(), files[2].clone()]
    );
}

#[test]
fn session_restore_skips_unreopenable_entries_with_one_notice() {
    // Break caught: one missing file aborting the rest of the restore, a notice per file, or
    // no tab activated when the saved active entry is the one that failed.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-missing");
    let kept = scratch.path().join("kept.txt");
    std::fs::write(&kept, "still here").unwrap();
    write_session(
        &scratch,
        vec![
            SessionEntry::new(SessionSource::File(scratch.path().join("gone.txt"))),
            SessionEntry::new(SessionSource::File(kept.clone())),
            SessionEntry::new(SessionSource::Snapshot(RecoveryId::from_u128(0xdead))),
        ],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);

    run_session_restore(window.hwnd);

    let app = app_mut(window.hwnd);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(
        app.tabs.active().unwrap().path.as_deref(),
        Some(kept.as_path())
    );
    assert_eq!(editor.text().unwrap(), "still here");
    let notices = app
        .notifications
        .pending()
        .iter()
        .filter(|notice| notice.message.contains("last session"))
        .map(|notice| notice.message.clone())
        .collect::<Vec<_>>();
    assert_eq!(notices, vec![crate::session::restore_failure_notice(2)]);
}

#[test]
fn session_restore_ignores_a_window_outside_the_session() {
    // Break caught: a --new-window instance consuming the primary window's session.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-outside");
    let notes = scratch.path().join("notes.txt");
    std::fs::write(&notes, "saved text").unwrap();
    write_session(
        &scratch,
        vec![SessionEntry::new(SessionSource::File(notes))],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);

    app_mut(window.hwnd).instance_mutex = None;
    run_session_restore(window.hwnd);

    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path, None);
    assert!(scratch.path().join("session.ini").exists());
}

#[test]
fn session_restore_with_the_setting_off_deletes_a_stale_manifest() {
    // Break caught: a primary with the setting off leaving an old manifest behind, so turning
    // the setting back on later reopens tabs crash recovery already brought back.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-stale");
    let notes = scratch.path().join("notes.txt");
    std::fs::write(&notes, "saved text").unwrap();
    write_session(
        &scratch,
        vec![SessionEntry::new(SessionSource::File(notes))],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    app_mut(window.hwnd).settings.restore_session = false;

    run_session_restore(window.hwnd);

    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path, None);
    assert!(app_mut(window.hwnd).session_restore.is_none());
    assert!(!scratch.path().join("session.ini").exists());
}

#[test]
fn recovery_leaves_snapshots_a_saved_session_names_while_restore_is_on() {
    // Break caught: a --new-window instance recovering the primary's saved unsaved tabs as
    // "Recovered: ..." (and deleting them on Discard) before the next launch restores them.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-recover-skip");
    let recovery = scratch.path().join("Recovery");
    let saved_id = RecoveryId::from_u128(0x5e57);
    write_snapshot(
        &recovery,
        &Snapshot::new(saved_id, None, Encoding::Utf8, "kept for the session"),
    )
    .unwrap();
    write_session(
        &scratch,
        vec![SessionEntry::new(SessionSource::Snapshot(saved_id))],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    app_mut(window.hwnd).instance_mutex = None;

    super::super::recover_snapshots(window.hwnd);

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        1,
        "the saved snapshot waits"
    );
    assert!(crate::recovery::snapshot::snapshot_path(&recovery, saved_id).exists());

    app_mut(window.hwnd).settings.restore_session = false;
    super::super::recover_snapshots(window.hwnd);

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        2,
        "with the setting off, crash recovery brings it back"
    );
}

#[test]
fn session_restore_holds_forwarded_launches_until_it_finishes() {
    // Break caught: a forwarded file opening mid-restore and being buried under the rest of
    // the session, or never opening because the held request is not replayed.
    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-forwarded");
    let first = scratch.path().join("first.txt");
    std::fs::write(&first, "one").unwrap();
    let second = scratch.path().join("second.txt");
    std::fs::write(&second, "two").unwrap();
    let forwarded = scratch.path().join("forwarded.txt");
    std::fs::write(&forwarded, "asked for mid-restore").unwrap();
    write_session(
        &scratch,
        vec![
            SessionEntry::new(SessionSource::File(first)),
            SessionEntry::new(SessionSource::File(second)),
        ],
        0,
    );
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);
    let restore = crate::window::WM_FASTPAD_RESTORE_SESSION;
    let request = crate::window::WM_FASTPAD_IPC_REQUEST;

    unsafe { PostMessageW(window.hwnd, restore, 0, 0) };
    let mut message = MSG::default();
    assert_ne!(
        unsafe { PeekMessageW(&mut message, window.hwnd, restore, restore, PM_REMOVE) },
        0
    );
    unsafe { DispatchMessageW(&message) };
    assert!(app_mut(window.hwnd).session_restore.is_some());
    app_mut(window.hwnd)
        .ipc_requests
        .push(crate::ipc::IpcRequest::Open(forwarded.clone()));
    unsafe { SendMessageW(window.hwnd, request, 0, 0) };

    assert!(
        app_mut(window.hwnd).tabs.find_path(&forwarded).is_none(),
        "a forwarded file must wait for the restore"
    );
    assert_eq!(app_mut(window.hwnd).ipc_requests.len(), 1);

    run_session_restore(window.hwnd);
    assert!(app_mut(window.hwnd).session_restore.is_none());
    assert_ne!(
        unsafe { PeekMessageW(&mut message, window.hwnd, request, request, PM_REMOVE) },
        0,
        "finishing the restore replays the held requests"
    );
    unsafe { DispatchMessageW(&message) };
    discard_posted(window.hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE);
    discard_posted(window.hwnd, crate::window::WM_FASTPAD_RECOVERY);
    discard_posted(window.hwnd, crate::window::WM_FASTPAD_OPEN_LIBRARY);

    let app = app_mut(window.hwnd);
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(
        app.tabs.active().unwrap().path.as_deref(),
        Some(forwarded.as_path())
    );
    assert_eq!(editor.text().unwrap(), "asked for mid-restore");
}

#[test]
fn session_restore_opens_the_launch_file_last() {
    // Break caught: the command-line file opening before the restored tabs, so a restored tab
    // ends up active instead of the file the user just asked for.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-launch");
    let restored = scratch.path().join("restored.txt");
    std::fs::write(&restored, "from last time").unwrap();
    let launched = scratch.path().join("launched.txt");
    std::fs::write(&launched, "asked for now").unwrap();
    write_session(
        &scratch,
        vec![SessionEntry::new(SessionSource::File(restored))],
        0,
    );
    let mut app = make_app();
    app.launch.request = crate::launch::LaunchRequest::Open(launched.into_os_string());
    let window = ProductionWindow::new(app);
    let editor = install_test_editor(&window);
    enable_session(window.hwnd, &scratch);

    run_session_restore(window.hwnd);
    unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_OPEN_REQUEST, 0, 0) };

    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
    assert_eq!(editor.text().unwrap(), "asked for now");
}
