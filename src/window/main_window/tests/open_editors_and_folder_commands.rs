//! The Open Editors section, folder commands, and inline edit focus handling.

use super::*;

#[test]
fn open_editors_lists_the_tabs_and_follows_opening_closing_and_saving() {
    // Break caught: a tab opened or closed without its row following, a dirty tab without
    // its dot, or the active tab's row not the selected one (open editors spec §3.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-follow");
    let a = scratch.note("a.md", "a");
    let (window, editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    let outside = scratch.root.join("outside.txt");
    std::fs::write(&outside, "x").unwrap();
    super::super::open_path(window.hwnd, &outside).unwrap();
    let names = |hwnd| {
        notebook_view(hwnd)
            .editors
            .rows
            .iter()
            .filter_map(crate::window::open_editors::EditorEntry::row)
            .map(|row| (row.name.clone(), row.dirty, row.active))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(window.hwnd),
        [
            ("a.md".into(), false, false),
            ("outside.txt".into(), false, true)
        ]
    );
    editor.set_text("changed").unwrap();
    assert!(names(window.hwnd)[1].1, "the dirty dot follows the edit");
    crate::window::modal::answer_next_close_prompt(|_| CloseDecision::Discard);
    execute_command(window.hwnd, CommandId::CloseTab);
    assert_eq!(names(window.hwnd).len(), 1);
}

#[test]
fn open_editors_click_switches_close_box_and_middle_click_close() {
    // Break caught: a click that opens nothing, the close box closing the wrong tab, or a
    // middle-click ignored in the panel (open editors spec §3.2).
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-click");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let c = scratch.note("c.md", "c");
    let (window, _editor) = notebook_window(&scratch);
    for path in [&a, &b, &c] {
        super::super::open_path(window.hwnd, path).unwrap();
    }
    let panel = sidebar_windows(window.hwnd).1;
    let row0 = notebook_view(window.hwnd).editor_rect_at(0).unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(row0));
    mouse(panel, WM_LBUTTONUP, 0, centre(row0));
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    assert_eq!(
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() },
        panel,
        "the focus stays in the panel, as a click on a tree row leaves it"
    );

    let row1 = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
    let close = crate::window::open_editors::close_rect(row1, 96);
    mouse(panel, WM_LBUTTONDOWN, 1, centre(close));
    mouse(panel, WM_LBUTTONUP, 0, centre(close));
    assert_eq!(tab_paths(window.hwnd), [Some(a.clone()), Some(c.clone())]);

    let row1 = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
    mouse(panel, WM_MBUTTONDOWN, 4, centre(row1));
    mouse(panel, WM_MBUTTONUP, 0, centre(row1));
    assert_eq!(tab_paths(window.hwnd), [Some(a)]);
}

#[test]
fn open_editors_expanded_after_a_switch_while_collapsed_shows_every_row() {
    // Break caught: a tab switch while the section was collapsed (a list 0 px high) scrolling
    // the rows to the active one, so expanding showed one row and blank space below, with
    // clicks landing on the wrong row.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-collapsed-switch");
    let paths: Vec<_> = (0..6)
        .map(|index| scratch.note(&format!("n{index}.md"), "x"))
        .collect();
    let (window, _editor) = notebook_window(&scratch);
    for path in &paths {
        super::super::open_path(window.hwnd, path).unwrap();
    }
    assert_eq!(notebook_view(window.hwnd).editors.rows.len(), 6);
    let panel = sidebar_windows(window.hwnd).1;
    let toggle = || {
        let header = notebook_view(window.hwnd).editors_header_rect();
        mouse(panel, WM_LBUTTONDOWN, 1, centre(header));
        mouse(panel, WM_LBUTTONUP, 0, centre(header));
    };
    toggle();
    assert!(!super::super::open_editors_expanded(window.hwnd));
    let fifth = notebook_view(window.hwnd).editors.row(4).unwrap().id;
    super::super::activate_document_by_id(window.hwnd, fifth);
    assert_eq!(notebook_view(window.hwnd).editors.active_index(), Some(4));
    toggle();
    assert!(super::super::open_editors_expanded(window.hwnd));
    assert_eq!(notebook_view(window.hwnd).editors.list.top, 0);
    for index in 0..6 {
        assert!(
            notebook_view(window.hwnd).editor_rect_at(index).is_some(),
            "row {index} is in view"
        );
    }
    let row0 = notebook_view(window.hwnd).editor_rect_at(0).unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(row0));
    mouse(panel, WM_LBUTTONUP, 0, centre(row0));
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(paths[0].as_path()),
        "a click lands on the row drawn there"
    );
}

#[test]
fn open_editors_and_the_root_collapse_and_stay_so() {
    // Break caught: the chevrons doing nothing, the tree still hit-tested while the root is
    // collapsed, or either state lost (open editors spec §3.3).
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-collapse");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let header = notebook_view(window.hwnd).editors_header_rect();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(header));
    mouse(panel, WM_LBUTTONUP, 0, centre(header));
    assert!(!super::super::open_editors_expanded(window.hwnd));
    let root = notebook_view(window.hwnd).root_rect();
    let chevron = crate::window::notebook_layout::root_parts(root, 96).chevron;
    mouse(panel, WM_LBUTTONDOWN, 1, centre(chevron));
    mouse(panel, WM_LBUTTONUP, 0, centre(chevron));
    assert!(!crate::window::library_host::root_expanded(window.hwnd));
    assert!(!notebook_view(window.hwnd).tree_shown());
    let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
    assert!(
        std::fs::read_to_string(local)
            .unwrap()
            .contains("root=collapsed")
    );
}

#[test]
fn the_arrow_keys_cross_from_open_editors_into_the_tree_and_del_on_a_tab_row_deletes_nothing() {
    // Break caught: the keyboard stuck in the tree, Enter on a tab row doing nothing, or Del
    // on a tab row deleting the tree's selected note (open editors spec §3.5).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_DOWN, VK_HOME, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-keys");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();
    let key = |vk: u16| crate::window::notebook_view::key_down(window.hwnd, vk);
    key(VK_HOME);
    assert_eq!(
        notebook_view(window.hwnd).cursor,
        crate::window::panel_cursor::Cursor::EditorsHeader
    );
    key(VK_DOWN);
    key(VK_DELETE);
    assert!(a.exists() && b.exists(), "Del on a tab row deletes nothing");
    key(VK_RETURN);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    for _ in 0..3 {
        key(VK_DOWN);
    }
    assert_eq!(
        notebook_view(window.hwnd).cursor,
        crate::window::panel_cursor::Cursor::Tree
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("a.md".into()))
    );
}

#[test]
fn a_note_command_with_a_tab_row_selected_leaves_the_trees_selected_note_alone() {
    // Break caught: Delete run while the keyboard selection is on an Open Editors row
    // deleting the tree's selected note instead of the active tab's (open editors spec
    // §3.5).
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-delete");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();
    let panel = sidebar_windows(window.hwnd).1;
    let row1 = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(row1));
    mouse(panel, WM_LBUTTONUP, 0, centre(row1));
    assert_eq!(
        notebook_view(window.hwnd).cursor,
        crate::window::panel_cursor::Cursor::Editor(1)
    );
    assert_eq!(
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() },
        panel
    );
    select_row(window.hwnd, &RowKind::Note("a.md".into()));
    crate::window::modal::answer_next_confirm(|_| true);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert!(a.exists(), "the tree's selected note stays");
    assert!(!b.exists(), "the active tab's note goes");
}

#[test]
fn the_root_rows_new_note_button_still_makes_the_note_in_the_selected_folder() {
    // Break caught: a click on the root row's New note moving the keyboard selection off the
    // tree, so the note went to the notebook's root (open editors spec §3.3).
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-root-new-note");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    select_row(window.hwnd, &RowKind::Folder("sub".into()));
    let panel = sidebar_windows(window.hwnd).1;
    let root = notebook_view(window.hwnd).root_rect();
    let (_, new_note) = crate::window::notebook_layout::root_parts(root, 96)
        .buttons
        .into_iter()
        .find(|(button, _)| *button == crate::window::notebook_view::HeaderButton::NewNote)
        .unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(new_note));
    mouse(panel, WM_LBUTTONUP, 0, centre(new_note));
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::NewNote("sub".into()))
    );
}

#[test]
fn without_a_notebook_the_arrows_cross_between_open_editors_and_recent() {
    // Break caught: the keyboard stuck in RECENT or in Open Editors while no notebook is
    // open (open editors spec §3.5).
    use crate::window::panel_cursor::Cursor;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_HOME, VK_UP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-recent-a");
    let other = LibraryScratch::new("editors-recent-b");
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &crate::library::local::RecentFolders {
            folders: vec![scratch.folder(), other.folder()],
            ..Default::default()
        },
    )
    .unwrap();
    super::super::open_path(window.hwnd, &a).unwrap();
    execute_command(window.hwnd, CommandId::CloseNotebook);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::NoNotebook);
    assert!(!notebook_view(window.hwnd).recent.is_empty());
    let tabs = notebook_view(window.hwnd).editors.rows.len();
    assert!(tabs >= 1);
    let key = |vk: u16| crate::window::notebook_view::key_down(window.hwnd, vk);
    key(VK_HOME);
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::EditorsHeader);
    for _ in 0..tabs {
        key(VK_DOWN);
    }
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Editor(tabs - 1));
    key(VK_DOWN);
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Tree);
    assert_eq!(
        notebook_view(window.hwnd).list.selected,
        Some(0),
        "RECENT's first row"
    );
    key(VK_UP);
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Editor(tabs - 1));
}

#[test]
fn page_keys_move_within_the_open_editors_rows() {
    // Break caught: Page Up and Page Down dropped on an Open Editors row, or moving the
    // active tab's row instead of the keyboard selection (open editors spec §3.5).
    use crate::window::panel_cursor::Cursor;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_HOME, VK_NEXT, VK_PRIOR};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-page");
    let paths: Vec<_> = (0..12)
        .map(|index| scratch.note(&format!("n{index:02}.md"), "x"))
        .collect();
    let (window, _editor) = notebook_window(&scratch);
    for path in &paths {
        super::super::open_path(window.hwnd, path).unwrap();
    }
    let key = |vk: u16| crate::window::notebook_view::key_down(window.hwnd, vk);
    key(VK_HOME);
    key(VK_DOWN);
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Editor(0));
    key(VK_NEXT);
    let Cursor::Editor(paged) = notebook_view(window.hwnd).cursor else {
        panic!("{:?}", notebook_view(window.hwnd).cursor);
    };
    assert!(paged > 1 && paged < 12, "{paged}");
    assert!(
        notebook_view(window.hwnd).editor_rect_at(paged).is_some(),
        "the paged-to row is in view"
    );
    assert_eq!(notebook_view(window.hwnd).editors.active_index(), Some(11));
    assert_eq!(notebook_view(window.hwnd).editors.list.selected, Some(11));
    key(VK_PRIOR);
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Editor(0));
}

#[test]
fn screen_readers_see_the_sections_and_the_tab_rows() {
    // Break caught: Open Editors rows invisible to screen readers, or headers without their
    // expanded state (open editors spec §7).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-msaa");
    let a = scratch.note("a.md", "a");
    let (window, editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    editor.set_text("changed").unwrap();
    let panel = sidebar_windows(window.hwnd).1;
    let count = crate::window::side_panel::accessible_item_count(panel);
    let items: Vec<_> = (0..count)
        .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
        .collect();
    // Break caught (spec §7): tab rows exposed unlike tree rows, or a flat outline where the
    // tree's top rows sit at the level of the rows that hold them.
    let outline = |name: &str| {
        let item = items
            .iter()
            .find(|item| item.name == name)
            .unwrap_or_else(|| panic!("{name} in {items:?}"));
        assert_eq!(
            item.role,
            windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINEITEM,
            "{name}"
        );
        item.value.clone()
    };
    assert_eq!(outline("Open editors, 1"), "0");
    assert_eq!(outline("a.md, open editor, modified"), "1");
    let notebook = crate::window::library_host::notebook_name(&scratch.folder());
    assert_eq!(outline(&notebook), "0");
    assert_eq!(outline("a.md, Markdown"), "1", "a top-level tree row");
}

#[test]
fn a_folder_rename_selects_the_whole_name_even_with_a_dot() {
    // Break caught: "v1.2" opening with only "v1" selected, as a file name's stem would be,
    // so typing keeps ".2" (spec §3.3).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-dot");
    std::fs::create_dir_all(scratch.folder().join("v1.2")).unwrap();
    let (window, _editor) = notebook_window(&scratch);

    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("v1.2".into()));

    assert_eq!(field_text(window.hwnd), "v1.2");
    assert_eq!(field_selection(window.hwnd), (0, 4));
}

#[test]
fn a_folder_renamed_while_a_rescan_runs_keeps_its_new_name_once_the_rescan_lands() {
    // Break caught: a rescan that listed the folder before the rename bringing the old row
    // back, with its note at a path that is gone, and hiding the new one (spec §3.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-rescan");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::library_host::request_rescan(window.hwnd);
    assert!(app_mut(window.hwnd).library.scanning);

    crate::window::notebook_view::rebuild(window.hwnd);
    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Moved");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );
    pump_until(window.hwnd, || !app_mut(window.hwnd).library.scanning);

    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_folder(std::path::Path::new("Moved")));
        assert!(!state.is_folder(std::path::Path::new("sub")));
        let notes: Vec<_> = state.notes.iter().map(|note| note.path.clone()).collect();
        assert_eq!(notes, [std::path::PathBuf::from(r"Moved\a.md")]);
    });
    row_of(window.hwnd, &RowKind::Folder("Moved".into()));
    assert!(
        crate::library::tree::row_index(
            &notebook_view(window.hwnd).rows,
            &RowKind::Folder("sub".into())
        )
        .is_none()
    );
}

#[test]
fn deleting_a_folder_recycles_it_closes_its_tabs_and_removes_its_rows() {
    // Break caught: a folder delete that leaves its rows, its open tab on a file that is gone
    // or its pinned note's record looking alive; one that deletes on Cancel; or a selection
    // that jumps away from where the folder was (spec §4.3).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-delete");
    std::fs::create_dir_all(scratch.folder().join(r"sub\inner")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("empty")).unwrap();
    scratch.note(r"sub\inner\b.md", "b");
    scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    let a = open_note(&window, &scratch, r"sub\a.md", "a");
    execute_command(window.hwnd, CommandId::NoteTogglePin);
    crate::window::notebook_view::rebuild(window.hwnd);
    let menu = |kind: &RowKind, answer: CommandId| {
        crate::window::menus::answer_next_popup_menu(move |_| Some(answer));
        let index = row_of(window.hwnd, kind);
        crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
    };
    crate::window::modal::take_last_confirm();

    crate::window::answer_next_confirm(|_| false);
    menu(&RowKind::Folder("empty".into()), CommandId::NoteDelete);
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Move the folder \u{201c}empty\u{201d} to the Recycle Bin?")
    );
    assert!(
        scratch.folder().join("empty").exists(),
        "Cancel deletes nothing"
    );

    crate::window::answer_next_confirm(|_| true);
    menu(&RowKind::Folder("sub".into()), CommandId::NoteDelete);
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Move \u{201c}sub\u{201d} and its 2 notes to the Recycle Bin?")
    );
    assert!(!scratch.folder().join("sub").exists());
    assert!(
        tab_paths(window.hwnd)
            .iter()
            .all(|path| path.as_deref() != Some(a.as_path()))
    );
    let rows = &notebook_view(window.hwnd).rows;
    for gone in ["sub", r"sub\inner"] {
        assert!(
            crate::library::tree::row_index(rows, &RowKind::Folder(gone.into())).is_none(),
            "{gone}"
        );
    }
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("top.md".into()))
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        let record = state.record_for(&a).unwrap();
        assert!(record.deleted);
        assert!(state.local.missing_since(record.id).is_some());
        let notes: Vec<_> = state.notes.iter().map(|note| note.path.clone()).collect();
        assert_eq!(notes, [std::path::PathBuf::from("top.md")]);
    });
}

#[test]
fn deleting_a_folder_that_holds_the_only_tab_and_the_draft_target_warns_and_cancels_the_draft() {
    // Break caught: unsaved edits discarded without a word, the last tab left on a deleted
    // file, or a draft still offering to create inside a folder that is gone (spec §5.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DELETE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-delete-only-tab");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    let a = open_note(&window, &scratch, r"sub\a.md", "a");
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    editor.set_text("unsaved").unwrap();
    crate::window::notebook_view::rebuild(window.hwnd);
    crate::window::inline_name::new_folder(window.hwnd, Some("sub".into()));
    assert!(inline_open(window.hwnd));
    select_row(window.hwnd, &RowKind::Folder("sub".into()));
    crate::window::modal::take_last_confirm();
    crate::window::answer_next_confirm(|_| true);

    assert!(crate::window::notebook_view::key_down(
        window.hwnd,
        VK_DELETE
    ));

    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some(
            "Move \u{201c}sub\u{201d} and its 1 note to the Recycle Bin?\n1 open note has unsaved changes, which will be lost."
        )
    );
    assert!(!a.exists());
    assert_eq!(super::super::tab_count(window.hwnd), 0);
    assert!(!inline_open(window.hwnd));
    assert_eq!(draft_row(window.hwnd), None);
}

#[test]
fn deleting_a_folder_with_autosave_on_closes_its_dirty_tabs_without_a_changed_on_disk_notice() {
    // Break caught: closing a deleted folder's tabs one by one autosaving the dirty one being
    // left, whose file is gone, so a false "changed on disk. Autosave is paused" notice shows.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-delete-autosave");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let a = scratch.note(r"sub\a.md", "a");
    let b = scratch.note(r"sub\b.md", "b");
    scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();
    editor.set_text("b, unsaved").unwrap();
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert!(active.dirty && active.path.as_deref() == Some(b.as_path()));
    crate::window::answer_next_confirm(|_| true);

    crate::window::library_host::delete_folder(window.hwnd, std::path::Path::new("sub"));

    assert!(!scratch.folder().join("sub").exists());
    assert_eq!(super::super::tab_count(window.hwnd), 0);
    assert!(
        !notices(window.hwnd)
            .iter()
            .any(|notice| notice.contains("changed on disk")),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn folder_commands_on_an_empty_or_escaping_path_touch_no_disk() {
    // Break caught: a folder delete or rename, or a new note or folder, handed the empty
    // path (the notebook root) or a `..` path recycling, renaming or creating outside the
    // folder the user picked, even when a commit is reached with such a path directly.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-bad-path");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    crate::window::modal::take_last_confirm();
    let notes =
        || crate::window::library_host::with_state(window.hwnd, |state| state.notes.len()).unwrap();

    crate::window::answer_next_confirm(|_| true);
    crate::window::library_host::delete_folder(window.hwnd, std::path::Path::new(""));
    crate::window::library_host::delete_folder(window.hwnd, std::path::Path::new(".."));
    for bad in ["", ".."] {
        crate::window::inline_name::rename(window.hwnd, &RowKind::Folder(bad.into()));
        assert!(!inline_open(window.hwnd), "{bad:?} has no row");
    }
    crate::window::inline_name::new_folder(window.hwnd, Some("..".into()));
    assert!(!inline_open(window.hwnd), "no draft outside the notebook");
    // The commits' own guards, reached with purposes no row gives.
    use crate::window::inline_name::Purpose;
    let listing = |folder: &std::path::Path| {
        let mut names = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    let (around, inside) = (listing(&scratch.root), listing(&scratch.folder()));
    for (purpose, text) in [
        (Purpose::NewFolder("..".into()), "Outside"),
        (Purpose::NewNote("..".into()), "Outside"),
        (Purpose::RenameFolder("".into()), "Renamed"),
        (Purpose::RenameFolder("..".into()), "Renamed"),
        (Purpose::RenameFolder("sub".into()), ".."),
    ] {
        let shown = format!("{purpose:?} {text:?}");
        crate::window::inline_name::commit_unchecked(window.hwnd, purpose, text);
        assert!(!inline_open(window.hwnd), "{shown}");
    }

    assert_eq!(crate::window::modal::take_last_confirm(), None);
    assert!(scratch.folder().join(r"sub\a.md").exists());
    assert_eq!(
        listing(&scratch.root),
        around,
        "nothing made beside the notebook"
    );
    assert_eq!(
        listing(&scratch.folder()),
        inside,
        "nothing made or renamed in it"
    );
    assert_eq!(notes(), 1);
}

#[test]
fn a_new_note_made_in_a_folder_saves_into_it_after_the_folder_is_renamed() {
    // Break caught: an untitled tab made (Ctrl+N) with a folder of the notebook as its save
    // folder keeping the folder's old path, so after a rename its first save silently lands
    // in the notebook root instead.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-save-folder");
    std::fs::create_dir_all(scratch.folder().join(r"sub\inner")).unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    untitled_tab_saving_in(window.hwnd, scratch.folder().join("sub"));
    let in_sub = app_mut(window.hwnd).tabs.active().unwrap().id;
    untitled_tab_saving_in(window.hwnd, scratch.folder().join(r"SUB\inner"));
    let in_inner = app_mut(window.hwnd).tabs.active().unwrap().id;
    untitled_tab_saving_in(window.hwnd, scratch.folder());
    let at_root = app_mut(window.hwnd).tabs.active().unwrap().id;

    crate::window::notebook_view::rebuild(window.hwnd);
    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Moved");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );

    let moved = scratch.folder().join("Moved");
    let save_folder = |id| {
        app_mut(window.hwnd)
            .tabs
            .document(id)
            .unwrap()
            .save_folder
            .clone()
    };
    assert_eq!(save_folder(in_sub), Some(moved.clone()));
    assert_eq!(save_folder(in_inner), Some(moved.join("inner")));
    assert_eq!(save_folder(at_root), Some(scratch.folder()));
    assert!(super::super::activate_document_by_id(window.hwnd, in_sub));
    editor.set_text("Idea").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(moved.join("Idea.md").exists());
}

#[test]
fn deleting_a_folder_selects_the_row_after_it_even_when_closing_its_tab_expands_another() {
    // Break caught: the selection after a delete chosen by index, so when closing the
    // folder's tab switches to a note in a collapsed folder above (expanding it), a row
    // inside that folder is selected instead of the one that took the deleted folder's place.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-delete-selection");
    std::fs::create_dir_all(scratch.folder().join("above")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("doomed")).unwrap();
    let x = scratch.note(r"above\x.md", "x");
    let y = scratch.note(r"doomed\y.md", "y");
    scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &x).unwrap();
    super::super::open_path(window.hwnd, &y).unwrap();
    let path = std::path::Path::new;
    crate::window::library_host::set_expanded(window.hwnd, path("above"), false);
    crate::window::library_host::set_expanded(window.hwnd, path("doomed"), false);
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Folder("doomed".into()));
    assert!(
        crate::library::tree::row_index(
            &notebook_view(window.hwnd).rows,
            &RowKind::Note(r"above\x.md".into())
        )
        .is_none(),
        "`above` starts collapsed"
    );
    crate::window::answer_next_confirm(|_| true);

    crate::window::library_host::delete_folder(window.hwnd, path("doomed"));

    assert!(!scratch.folder().join("doomed").exists());
    assert!(
        crate::library::tree::row_index(
            &notebook_view(window.hwnd).rows,
            &RowKind::Note(r"above\x.md".into())
        )
        .is_some(),
        "switching to x's tab expanded `above`"
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("top.md".into()))
    );
}

#[test]
fn a_folder_rename_that_cannot_be_undone_stands_and_names_the_tab_left_behind() {
    // Break caught: a failed undo swallowed, leaving the folder renamed on disk while the
    // library still lists the old one and every tab points at a path that is gone.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("folder-rename-undo-fails");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let a = scratch.note(r"sub\a.md", "a");
    let b = scratch.note(r"sub\b.md", "b");
    let top = scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();
    super::super::open_path(window.hwnd, &top).unwrap();
    let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
    let b_id = app_mut(window.hwnd).tabs.find_stored_path(&b).unwrap();
    // Another tab already names the path `a` would move to, so `a`'s tab cannot follow.
    let top_id = app_mut(window.hwnd).tabs.find_stored_path(&top).unwrap();
    app_mut(window.hwnd).tabs.document_mut(top_id).unwrap().path =
        Some(scratch.folder().join(r"Moved\a.md"));
    untitled_tab_saving_in(window.hwnd, scratch.folder().join("sub"));
    let untitled = app_mut(window.hwnd).tabs.active().unwrap().id;
    crate::window::library_host::fail_next_folder_rename_back();

    crate::window::notebook_view::rebuild(window.hwnd);
    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Moved");
    field_key(
        window.hwnd,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN,
    );

    let moved = scratch.folder().join("Moved");
    assert!(moved.join("b.md").exists());
    assert_eq!(
        app_mut(window.hwnd)
            .tabs
            .document(untitled)
            .unwrap()
            .save_folder,
        Some(moved.clone()),
        "the rename stands, so the untitled tab's first save follows it"
    );
    assert!(!scratch.folder().join("sub").exists());
    assert!(!inline_open(window.hwnd));
    let tabs = &app_mut(window.hwnd).tabs;
    assert_eq!(tabs.document(b_id).unwrap().path, Some(moved.join("b.md")));
    assert_eq!(tabs.document(a_id).unwrap().path, Some(a.clone()));
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_folder(std::path::Path::new("Moved")));
        assert!(!state.is_folder(std::path::Path::new("sub")));
    });
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder("Moved".into()))
    );
    let expected = "FastPad could not undo renaming \u{201c}sub\u{201d} to \u{201c}Moved\u{201d}. \u{201c}a.md\u{201d} is still open at its old path.";
    assert!(
        notices(window.hwnd).iter().any(|notice| notice == expected),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn palette_rename_with_a_folder_row_focused_edits_the_folder_row() {
    // Break caught: the palette's Note: Rename renaming the active tab's note while the user
    // had a folder row focused, or renaming anything before Enter (spec §9).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("palette-folder-rename");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let active = scratch.note("active.md", "active");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &active).unwrap();
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Folder("sub".into()));
    let (_, panel) = sidebar_windows(window.hwnd);
    unsafe { SetFocus(panel) };
    assert_eq!(unsafe { GetFocus() }, panel);

    execute_command(window.hwnd, CommandId::CommandPalette);
    let query = app_mut(window.hwnd)
        .command_palette
        .as_ref()
        .unwrap()
        .query_hwnd();
    let typed = crate::platform::wide_null("Note: Rename");
    unsafe { SetWindowTextW(query, typed.as_ptr()) };
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };

    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::RenameFolder(
            "sub".into()
        ))
    );
    assert_eq!(field_text(window.hwnd), "sub");
    assert!(
        scratch.folder().join("sub").is_dir(),
        "nothing renamed before Enter"
    );
    assert!(active.exists());
}

#[test]
fn focus_moving_to_the_editor_commits_and_a_taken_name_closes_with_a_notice() {
    // Break caught: a name lost when the user clicks into the editor, a click away that
    // leaves the field hanging, or a taken name closed without saying why (spec §5.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-focus-editor");
    let a = scratch.note("a.md", "a");
    let (window, editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();

    crate::window::inline_name::new_folder(window.hwnd, None);
    type_into_field(window.hwnd, "Plans");
    unsafe { SetFocus(editor.hwnd()) };
    pump_posted_messages(window.hwnd);
    assert!(!inline_open(window.hwnd));
    assert!(scratch.folder().join("Plans").is_dir());

    // At the root: with the new folder's row selected, None would draft inside it.
    crate::window::inline_name::new_folder(window.hwnd, Some(std::path::PathBuf::new()));
    type_into_field(window.hwnd, "plans");
    assert!(crate::window::inline_name::problem(window.hwnd).is_some());
    unsafe { SetFocus(editor.hwnd()) };
    pump_posted_messages(window.hwnd);
    assert!(!inline_open(window.hwnd));
    assert_eq!(draft_row(window.hwnd), None);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice == "plans already exists here."),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn losing_focus_to_another_app_keeps_the_field_and_reactivation_refocuses_it() {
    // Break caught: Alt+Tab creating a half-typed note, or coming back to FastPad with the
    // field open but the caret in the editor (spec §5.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_SETFOCUS;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-focus-app");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    crate::window::inline_name::new_note(window.hwnd, None);
    type_into_field(window.hwnd, "half");

    // What deactivation does to the focused field: focus goes to no window of this thread.
    unsafe { SetFocus(std::ptr::null_mut()) };
    pump_posted_messages(window.hwnd);
    assert!(inline_open(window.hwnd));
    assert!(!scratch.folder().join("half.md").exists());

    unsafe { SendMessageW(window.hwnd, WM_SETFOCUS, 0, 0) };
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert_eq!(field_text(window.hwnd), "half");
}

#[test]
fn a_commit_on_focus_loss_waits_for_a_modal_prompt_to_end() {
    // Break caught: a note created or renamed while a modal prompt that took the focus is
    // still asking something (spec §5.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-focus-modal");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    crate::window::inline_name::new_folder(window.hwnd, None);
    type_into_field(window.hwnd, "Later");

    let modal = crate::window::modal::ModalScope::enter(window.hwnd);
    // The prompt takes the focus: here the frame does, a window of this thread.
    unsafe { SetFocus(window.hwnd) };
    pump_posted_messages(window.hwnd);
    assert!(inline_open(window.hwnd));
    assert!(!scratch.folder().join("Later").exists(), "held while modal");

    // Leaving the outermost modal scope re-posts what it held.
    drop(modal);
    pump_posted_messages(window.hwnd);
    assert!(!inline_open(window.hwnd));
    assert!(scratch.folder().join("Later").is_dir());
}

#[test]
fn a_click_on_a_row_below_a_draft_commits_first_and_acts_on_that_row() {
    // Break caught: the click selecting whatever row slid under the pointer once the empty
    // draft went, or the rename not committed before the click (spec §5.3).
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-click");
    scratch.note("a.md", "a");
    scratch.note("b.md", "b");
    scratch.note("c.md", "c");
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let click = |lparam| unsafe {
        SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
        SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
    };

    crate::window::inline_name::new_note(window.hwnd, None);
    assert_eq!(draft_row(window.hwnd), Some((0, 0)), "first at the root");
    click(row_lparam(window.hwnd, &RowKind::Note("b.md".into())));
    assert!(!inline_open(window.hwnd));
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("b.md".into()))
    );
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path,
        Some(scratch.folder().join("b.md"))
    );

    crate::window::inline_name::rename(window.hwnd, &RowKind::Note("c.md".into()));
    type_into_field(window.hwnd, "d");
    click(row_lparam(window.hwnd, &RowKind::Note("a.md".into())));
    assert!(scratch.folder().join("d.md").exists(), "committed first");
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("a.md".into()))
    );
}

#[test]
fn starting_an_edit_while_one_is_open_commits_the_open_one_first() {
    // Break caught: a second edit dropping the first one's typing, or two fields at once
    // (spec §3.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-one-edit");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    select_row(window.hwnd, &RowKind::Note("a.md".into()));
    assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
    type_into_field(window.hwnd, "a2");

    crate::window::notebook_view::header_clicked(
        window.hwnd,
        crate::window::notebook_view::HeaderButton::NewFolder,
    );

    assert!(scratch.folder().join("a2.md").exists());
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::NewFolder(
            std::path::PathBuf::new()
        ))
    );
    assert_eq!(field_text(window.hwnd), "");
}

#[test]
fn switching_the_sidebar_view_or_hiding_it_cancels_the_edit() {
    // Break caught: a field left typing into a view nobody can see, or Ctrl+B committing a
    // half-typed rename (spec §5.4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-view-switch");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    for view in [
        crate::config::SidebarView::Search,
        crate::config::SidebarView::Hidden,
    ] {
        crate::window::inline_name::rename(window.hwnd, &RowKind::Note("a.md".into()));
        type_into_field(window.hwnd, "zzz");
        crate::window::side_panel::show_view(window.hwnd, view, false);
        pump_posted_messages(window.hwnd);
        assert!(!inline_open(window.hwnd), "{view:?}");
        assert!(scratch.folder().join("a.md").exists(), "{view:?}");
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
    }
}

#[test]
fn ctrl_z_and_ctrl_y_in_the_field_stay_with_the_field() {
    // Break caught: Ctrl+Z in the name field undoing the note in the editor, or Ctrl+Y
    // redoing it, because the accelerator table takes the key first (spec §5.1, §11).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_CHAR, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-ctrl-z");
    let a = scratch.note("a.md", "a");
    let (window, editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    editor.set_text("typed in the editor").unwrap();
    editor.undo().unwrap();
    assert!(editor.can_redo().unwrap(), "the editor has a step to redo");
    let before = editor.text().unwrap();
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    crate::window::inline_name::new_note(window.hwnd, None);
    let field = inline_field(window.hwnd);
    let typed = crate::platform::wide_null("abc");
    unsafe {
        SendMessageW(
            field,
            windows_sys::Win32::UI::Controls::EM_REPLACESEL,
            1,
            typed.as_ptr() as isize,
        )
    };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let ctrl = |key: u8| MSG {
        hwnd: field,
        message: WM_KEYDOWN,
        wParam: usize::from(key),
        ..Default::default()
    };

    let redo_taken =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &ctrl(b'Y')) };
    unsafe {
        SendMessageW(field, WM_KEYDOWN, usize::from(b'Y'), 0);
        SendMessageW(field, WM_CHAR, 0x19, 0);
    }
    let after_redo = (field_text(window.hwnd), editor.text().unwrap());
    let undo_taken =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &ctrl(b'Z')) };
    unsafe {
        SendMessageW(field, WM_KEYDOWN, usize::from(b'Z'), 0);
        SendMessageW(field, WM_CHAR, 0x1a, 0);
    }
    unsafe { SetKeyboardState(original.as_ptr()) };

    assert!(!redo_taken, "the field keeps Ctrl+Y");
    assert_eq!(after_redo, ("abc".to_owned(), before.clone()));
    assert!(!undo_taken, "the field keeps Ctrl+Z");
    assert_eq!(field_text(window.hwnd), "");
    assert_eq!(editor.text().unwrap(), before);
    assert!(
        editor.can_redo().unwrap(),
        "nothing redid the editor's step"
    );
}

#[test]
fn pressing_the_scroll_thumb_keeps_the_edit_open_and_the_field_focused() {
    // Break caught: grabbing the tree's scroll thumb mid-rename renaming the note to the
    // half-typed name, or taking the focus from the field; scrolling keeps the edit (inline
    // naming spec §5.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-thumb");
    for index in 0..80 {
        scratch.note(&format!("n{index:02}.md"), "n");
    }
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    crate::window::inline_name::rename(window.hwnd, &RowKind::Note("n00.md".into()));
    type_into_field(window.hwnd, "half");
    let (x, y) = notebook_view(window.hwnd)
        .thumb_point()
        .expect("80 notes overflow the list");

    unsafe {
        SendMessageW(panel, WM_LBUTTONDOWN, 1, client_lparam(x, y));
        SendMessageW(panel, WM_LBUTTONUP, 0, client_lparam(x, y));
    }
    pump_posted_messages(window.hwnd);

    assert!(inline_open(window.hwnd));
    assert_eq!(field_text(window.hwnd), "half");
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert!(scratch.folder().join("n00.md").exists());
    assert!(!scratch.folder().join("half.md").exists());
}
