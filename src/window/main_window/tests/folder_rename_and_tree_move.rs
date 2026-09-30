//! Renaming folders and moving notes and folders in the tree.

use super::*;

#[test]
fn renaming_a_folder_with_an_open_note_rebinds_the_tab_and_keeps_the_notes_pin() {
    // Break caught: a folder rename leaving its open tab on the old path (the next save
    // re-creating the old folder), dropping the note's pin, or losing the row's selection.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    let old = open_note(&window, &scratch, r"sub\a.md", "a");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Folder("sub".into()));

    assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::RenameFolder(
            "sub".into()
        ))
    );
    assert_eq!(field_text(window.hwnd), "sub");
    type_into_field(window.hwnd, "Projects");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );

    let new = scratch.folder().join(r"Projects\a.md");
    assert!(!inline_open(window.hwnd));
    assert!(new.exists());
    assert!(!old.exists());
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(new.as_path())
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder("Projects".into()))
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_pinned(&new));
        assert!(!state.is_folder(std::path::Path::new("sub")));
    });
    crate::window::library_host::flush_now(window.hwnd);
    let reloaded = crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
    assert!(
        reloaded.is_pinned(&new),
        "the pin was written under the new path"
    );
}

#[test]
fn renaming_a_folder_moves_its_preview_and_dirty_tabs_without_saving_them() {
    // Break caught: a rename that saves a dirty tab (touching the file's contents), turns the
    // preview into a normal tab, or leaves either on the old path (spec §4.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-tabs");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let a = scratch.note(r"sub\a.md", "a");
    let b = scratch.note(r"sub\b.md", "b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    // Autosave would save `a` the moment `b` opens; the rename must leave it dirty.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &a).unwrap();
    editor.set_text("a, edited").unwrap();
    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();
    let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
    let b_id = app_mut(window.hwnd).tabs.find_stored_path(&b).unwrap();
    crate::window::notebook_view::rebuild(window.hwnd);

    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Moved");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );

    let moved = scratch.folder().join("Moved");
    let tabs = &app_mut(window.hwnd).tabs;
    assert_eq!(tabs.document(a_id).unwrap().path, Some(moved.join("a.md")));
    assert!(tabs.document(a_id).unwrap().dirty);
    assert_eq!(tabs.document(b_id).unwrap().path, Some(moved.join("b.md")));
    assert_eq!(tabs.preview_id(), Some(b_id));
    assert_eq!(
        std::fs::read_to_string(moved.join("a.md")).unwrap(),
        "a",
        "nothing was saved"
    );
}

#[test]
fn a_case_only_folder_rename_renames_it_on_disk_and_in_tabs_and_expansion() {
    // Break caught: "sub" → "Sub" refused as a clash with itself, a no-op on NTFS, or the tab
    // and the expanded entry left in the old case.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-case");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    open_note(&window, &scratch, r"sub\a.md", "a");
    crate::window::notebook_view::rebuild(window.hwnd);

    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Sub");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );

    let names: Vec<String> = std::fs::read_dir(scratch.folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"Sub".to_owned()), "{names:?}");
    assert!(!inline_open(window.hwnd));
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path,
        Some(scratch.folder().join(r"Sub\a.md"))
    );
    assert!(
        crate::window::library_host::expanded(window.hwnd)
            .contains(&std::path::PathBuf::from("Sub"))
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder("Sub".into()))
    );
}

#[test]
fn a_folder_rename_an_open_tab_cannot_follow_is_undone() {
    // Break caught: the folder renamed on disk while a tab stays on the old path (its next
    // save re-creating the old folder), or a half-done rename left behind (spec §4.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-undo");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let a = scratch.note(r"sub\a.md", "a");
    let top = scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &top).unwrap();
    // Another tab already names the path `a` would move to, so `a`'s tab cannot follow.
    let top_id = app_mut(window.hwnd).tabs.find_stored_path(&top).unwrap();
    app_mut(window.hwnd).tabs.document_mut(top_id).unwrap().path =
        Some(scratch.folder().join(r"Moved\a.md"));
    crate::window::notebook_view::rebuild(window.hwnd);

    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Moved");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );

    assert!(inline_open(window.hwnd));
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("Another tab already has that file open.")
    );
    assert!(a.exists());
    assert!(!scratch.folder().join("Moved").exists());
    let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a);
    assert!(a_id.is_some(), "a's tab is back on its old path");
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_folder(std::path::Path::new("sub")));
        assert!(!state.is_folder(std::path::Path::new("Moved")));
    });
}

#[test]
fn a_folder_rename_onto_a_sibling_is_refused_and_the_same_name_changes_nothing() {
    // Break caught: a rename onto an existing sibling (in another case) merging or failing
    // oddly, Note: Rename on a focused folder row renaming the active tab instead, or an
    // unchanged name showing a message (spec §4.3, §4.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-clash");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("Other")).unwrap();
    scratch.note(r"sub\a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    select_row(window.hwnd, &RowKind::Folder("sub".into()));
    let (_, panel) = sidebar_windows(window.hwnd);
    unsafe { SetFocus(panel) };
    assert_eq!(unsafe { GetFocus() }, panel);

    execute_command(window.hwnd, CommandId::NoteRename);
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::RenameFolder(
            "sub".into()
        ))
    );
    type_into_field(window.hwnd, "other");
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("other already exists here.")
    );
    field_key(window.hwnd, VK_RETURN);
    assert!(inline_open(window.hwnd));
    assert!(scratch.folder().join(r"sub\a.md").exists());

    type_into_field(window.hwnd, "sub");
    assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
    field_key(window.hwnd, VK_RETURN);
    assert!(!inline_open(window.hwnd));
    assert!(scratch.folder().join(r"sub\a.md").exists());
}

#[test]
fn tree_move_a_note_moves_into_a_folder_with_its_dirty_tab_and_pin() {
    // Break caught: a drop that saves the dirty tab, leaves it on the old path, drops the pin,
    // or leaves the target folder collapsed and the row unselected (tree drag spec §4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-move-note");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let a = scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &a).unwrap();
    editor.set_text("a, edited").unwrap();
    crate::window::library_host::toggle_pin(window.hwnd, &a);
    crate::window::notebook_view::rebuild(window.hwnd);
    // Autosave-off and Pinned notices from setup are not what this test is about.
    app_mut(window.hwnd).notifications.dismiss_all();

    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Note("a.md".into()),
        std::path::Path::new("work"),
    );

    let moved = scratch.folder().join(r"work\a.md");
    assert!(moved.exists() && !a.exists());
    assert_eq!(
        std::fs::read_to_string(&moved).unwrap(),
        "a",
        "nothing was saved"
    );
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(moved.as_path()));
    assert!(active.dirty);
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_pinned(&moved));
    });
    assert!(
        crate::window::library_host::expanded(window.hwnd)
            .contains(&std::path::PathBuf::from("work"))
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"work\a.md".into()))
    );
    assert!(
        notices(window.hwnd).is_empty(),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn tree_move_a_folder_moves_to_the_root_with_its_tabs_and_expanded_folders() {
    // Break caught: tabs under the moved folder left on old paths, or its expanded state and
    // its own expanded subfolder lost (tree drag spec §4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-move-folder");
    std::fs::create_dir_all(scratch.folder().join(r"work\inner\deep")).unwrap();
    let (window, _editor) = notebook_window(&scratch);
    let c = open_note(&window, &scratch, r"work\inner\c.md", "c");
    for folder in ["work", r"work\inner", r"work\inner\deep"] {
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new(folder), true);
    }
    crate::window::notebook_view::rebuild(window.hwnd);

    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Folder(r"work\inner".into()),
        std::path::Path::new(""),
    );

    let moved = scratch.folder().join(r"inner\c.md");
    assert!(moved.exists() && !c.exists());
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(moved.as_path())
    );
    let expanded = crate::window::library_host::expanded(window.hwnd);
    assert!(
        expanded.contains(&std::path::PathBuf::from("inner")),
        "{expanded:?}"
    );
    assert!(
        expanded.contains(&std::path::PathBuf::from(r"inner\deep")),
        "{expanded:?}"
    );
    assert!(
        !expanded.contains(&std::path::PathBuf::from(r"work\inner")),
        "{expanded:?}"
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder("inner".into()))
    );
}

#[test]
fn tree_move_a_taken_name_is_refused_in_memory_and_on_disk() {
    // Break caught: a drop onto a listed name in another letter case (a clash on NTFS), or
    // onto a file the tree has not seen yet, overwriting or half-moving (tree drag spec §5).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-move-taken");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\A.md", "work A");
    let a = scratch.note("a.md", "root a");
    let z = scratch.note("z.md", "root z");
    let (window, _editor) = notebook_window(&scratch);
    // Made after the scan: only the disk knows it.
    std::fs::write(scratch.folder().join(r"work\z.md"), "unseen").unwrap();

    let drop = |name: &str, folder: &str| {
        crate::window::tree_move::drop_into(
            window.hwnd,
            &RowKind::Note(name.into()),
            std::path::Path::new(folder),
        )
    };
    drop("a.md", "work");
    drop("z.md", "work");
    // work\A.md onto the root, where a.md is: the notice names the notebook.
    drop(r"work\A.md", "");
    let root_name = crate::window::library_host::notebook_name(&scratch.folder());

    assert_eq!(std::fs::read_to_string(&a).unwrap(), "root a");
    assert_eq!(std::fs::read_to_string(&z).unwrap(), "root z");
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join(r"work\z.md")).unwrap(),
        "unseen"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join(r"work\A.md")).unwrap(),
        "work A"
    );
    assert_eq!(
        notices(window.hwnd),
        vec![
            "a.md already exists in work. Nothing was moved.".to_owned(),
            "z.md already exists in work. Nothing was moved.".to_owned(),
            format!("A.md already exists in {root_name}. Nothing was moved."),
        ]
    );
}

#[test]
fn tree_move_a_vanished_or_locked_source_says_so_and_moves_nothing() {
    // Break caught: a missing file reported as a generic failure (or as a clash), or a
    // sharing violation swallowed (tree drag spec §5).
    use std::os::windows::fs::OpenOptionsExt;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-move-gone");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let gone = scratch.note("gone.md", "g");
    let locked = scratch.note("locked.md", "l");
    let (window, _editor) = notebook_window(&scratch);
    std::fs::remove_file(&gone).unwrap();
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked)
        .unwrap();

    for name in ["gone.md", "locked.md"] {
        crate::window::tree_move::drop_into(
            window.hwnd,
            &RowKind::Note(name.into()),
            std::path::Path::new("work"),
        );
    }

    assert!(!scratch.folder().join(r"work\gone.md").exists());
    assert!(locked.exists() && !scratch.folder().join(r"work\locked.md").exists());
    let notices = notices(window.hwnd);
    assert_eq!(notices[0], "gone.md no longer exists.");
    assert!(
        notices[1].starts_with("Couldn't move locked.md: "),
        "{notices:?}"
    );
    assert_eq!(notices.len(), 2, "{notices:?}");
}

#[test]
fn tree_move_a_drop_into_a_folder_deleted_outside_fastpad_says_so_not_that_the_note_is_gone() {
    // Break caught: ERROR_PATH_NOT_FOUND for the destination's vanished parent folder read as
    // the source itself being missing, which said "a.md no longer exists" instead of naming
    // the real problem and asking for a rescan (final review Important 2; tree drag spec §5).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-move-target-gone");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    // request_rescan only starts a load with a data dir to write the local state to.
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    std::fs::remove_dir(scratch.folder().join("work")).unwrap();

    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Note("a.md".into()),
        std::path::Path::new("work"),
    );

    assert!(a.exists(), "the source never moved");
    let notices = notices(window.hwnd);
    assert!(
        notices
            .last()
            .is_some_and(|notice| notice.starts_with("Couldn't move a.md: ")),
        "{notices:?}"
    );
    assert!(
        app_mut(window.hwnd).library.scanning,
        "a rescan catches the vanished folder up"
    );
}

#[test]
fn tree_move_a_tab_that_cannot_follow_undoes_the_move_or_names_the_stuck_tab() {
    // Break caught: a move that leaves a tab on a path that no longer exists without saying
    // so, or keeps the move when it could be undone (tree drag spec §5).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("drag-move-tab");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let a = scratch.note("a.md", "a");
    let top = scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &top).unwrap();
    // Another tab already names the path `a` would move to, so `a`'s tab cannot follow.
    let top_id = app_mut(window.hwnd).tabs.find_stored_path(&top).unwrap();
    app_mut(window.hwnd).tabs.document_mut(top_id).unwrap().path =
        Some(scratch.folder().join(r"work\a.md"));
    let drop = || {
        crate::window::tree_move::drop_into(
            window.hwnd,
            &RowKind::Note("a.md".into()),
            std::path::Path::new("work"),
        )
    };

    drop();
    assert!(a.exists(), "moved back");
    assert_eq!(
        notices(window.hwnd),
        vec!["Couldn't move a.md: another tab already has that file open.".to_owned()]
    );

    crate::window::library_host::fail_next_note_rename_back();
    drop();
    assert!(!a.exists() && scratch.folder().join(r"work\a.md").exists());
    let expected = crate::window::library_host::rename_undo_failed_notice(
        "a.md",
        r"work\a.md",
        std::slice::from_ref(&a),
    );
    assert_eq!(notices(window.hwnd).last(), Some(&expected));
}
