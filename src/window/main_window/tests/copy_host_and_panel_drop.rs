//! Copying files into a notebook, panel drops and the drop overlay.

use super::*;

#[test]
fn copy_host_copies_files_and_folders_indexes_notes_and_says_what_is_hidden() {
    // Break caught: a copied note missing from the tree until a rescan, a copied folder not
    // listed, a file that is neither a note nor an image copied silently, or the single
    // copied row not selected (open editors spec §4.5, §4.6; image preview spec §9).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-into");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let outside = scratch.root.join("outside");
    std::fs::create_dir_all(outside.join(r"pics\deep")).unwrap();
    std::fs::write(outside.join("draft.md"), "d").unwrap();
    std::fs::write(outside.join(r"pics\deep\x.png"), [1u8]).unwrap();
    std::fs::write(outside.join("archive.zip"), [1u8]).unwrap();
    let (window, _editor) = notebook_window(&scratch);

    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![outside.join("draft.md")],
        std::path::Path::new("work"),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert!(scratch.folder().join(r"work\draft.md").exists());
    assert!(outside.join("draft.md").exists(), "a copy, not a move");
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"work\draft.md".into()))
    );

    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![outside.join("pics"), outside.join("archive.zip")],
        std::path::Path::new(""),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert!(scratch.folder().join(r"pics\deep\x.png").exists());
    assert!(scratch.folder().join("archive.zip").exists());
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice.contains("isn't shown") || notice.contains("aren't shown"))
    );
}

#[test]
fn copy_host_a_clash_asks_ok_replaces_and_cancel_skips() {
    // Break caught: a clash replaced without asking, Cancel stopping the whole drop, or a
    // replaced clean tab left showing the old text (open editors spec §4.4, §4.7).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-clash");
    let a = scratch.note("a.md", "old a");
    scratch.note("b.md", "old b");
    let outside = scratch.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("a.md"), "new a").unwrap();
    std::fs::write(outside.join("b.md"), "new b").unwrap();
    std::fs::write(outside.join("c.md"), "new c").unwrap();
    let (window, editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();

    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::modal::answer_next_confirm(|_| false);
    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![
            outside.join("a.md"),
            outside.join("b.md"),
            outside.join("c.md"),
        ],
        std::path::Path::new(""),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    let folder = crate::window::library_host::notebook_name(&scratch.folder());
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some(format!("b.md already exists in {folder}. Replace it?").as_str())
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "new a");
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("b.md")).unwrap(),
        "old b"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("c.md")).unwrap(),
        "new c"
    );
    assert_eq!(editor.text().unwrap(), "new a", "the clean tab reloaded");
}

#[test]
fn copy_host_a_failed_recycle_skips_that_item_and_says_so() {
    // Break caught (Review Focus 3): an item copied (or half-copied) after its Recycle Bin
    // step failed, or the rest of the drop abandoned.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-recycle-fails");
    scratch.note("a.md", "old a");
    let outside = scratch.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("a.md"), "new a").unwrap();
    std::fs::write(outside.join("b.md"), "new b").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::copy_host::fail_next_recycle();
    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![outside.join("a.md"), outside.join("b.md")],
        std::path::Path::new(""),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("a.md")).unwrap(),
        "old a"
    );
    assert!(scratch.folder().join("b.md").exists());
    assert!(
        notices(window.hwnd)
            .contains(&"a.md was not copied: it could not be moved to the Recycle Bin.".to_owned())
    );
}

#[test]
fn copy_host_an_alias_of_the_source_is_refused_and_nothing_is_recycled() {
    // Break caught: a `\\?\` or 8.3 spelling of the item itself, or of the folder holding it,
    // passing the lexical plan as a clash, so answering OK recycled the very item being copied.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-alias");
    let note = scratch.note("a-long-note-name.md", "keep");
    std::fs::create_dir_all(scratch.folder().join(r"work\work")).unwrap();
    std::fs::write(scratch.folder().join(r"work\work\in.md"), "in").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    let verbatim = |path: &std::path::Path| PathBuf::from(format!(r"\\?\{}", path.display()));

    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![
            verbatim(&note),
            verbatim(&scratch.folder().join(r"work\work")),
        ],
        std::path::Path::new(""),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "keep");
    assert!(scratch.folder().join(r"work\work\in.md").exists());
    let said = notices(window.hwnd);
    assert!(said.contains(&"a-long-note-name.md was not copied: it is already there.".to_owned()));
    assert!(
        said.contains(&"work was not copied: it would replace the folder it is in.".to_owned())
    );

    let short = crate::platform::files::short_path_for_test(&note);
    if short.file_name() == note.file_name() {
        // The scratch volume makes no 8.3 names: the `\\?\` spelling above stands in.
        return;
    }
    let short_name = crate::window::tree_copy::item_name(&short);
    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::copy_host::copy_into(window.hwnd, vec![short], std::path::Path::new(""), None);
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert_eq!(std::fs::read_to_string(&note).unwrap(), "keep");
    assert!(notices(window.hwnd).contains(&format!(
        "{short_name} was not copied: it is already there."
    )));
}

#[test]
fn copy_host_a_junction_in_the_source_path_does_not_hide_the_folder_holding_it() {
    // Break caught: the identity check walking only the folders of the source as spelled, so
    // with a junction on the way the real folder holding it was not seen, and answering OK
    // recycled that folder with the source inside it.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-junction");
    let inner = scratch.folder().join("work").join("x").join("work");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("in.md"), "in").unwrap();
    let link = scratch.root.join("j");
    crate::platform::files::junction_for_test(&link, &scratch.folder().join("work").join("x"));
    let (window, _editor) = notebook_window(&scratch);

    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![link.join("work")],
        std::path::Path::new(""),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    let folder = crate::window::library_host::notebook_name(&scratch.folder());
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some(format!("work already exists in {folder}. Replace it?").as_str()),
        "the plan saw a clash, not the refusal"
    );
    assert_eq!(std::fs::read_to_string(inner.join("in.md")).unwrap(), "in");
    assert!(
        notices(window.hwnd)
            .contains(&"work was not copied: it would replace the folder it is in.".to_owned())
    );
}

/// A drag from Explorer onto panel point `x`, `y`: DragEnter, DragOver, then Drop or
/// DragLeave, as OLE runs them. The effects each answered.
fn explorer_drop(panel: HWND, x: i32, y: i32, paths: &[&std::path::Path]) -> [u32; 3] {
    let mut point = windows_sys::Win32::Foundation::POINT { x, y };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(panel, &mut point) };
    crate::editor::file_drop::test_support::drag_and_drop_at(
        panel,
        paths,
        windows_sys::Win32::Foundation::POINTL {
            x: point.x,
            y: point.y,
        },
    )
}

#[test]
fn panel_drop_onto_the_root_row_copies_and_onto_open_editors_opens() {
    // Break caught: Explorer drops refused on the panel, dropped on the wrong folder, or
    // Open Editors copying instead of opening (open editors spec §4.1, §4.3).
    use windows_sys::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_NONE};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("panel-drop");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let outside = scratch.root.join("x.md");
    std::fs::write(&outside, "x").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::side_panel::accept_file_drops(window.hwnd);
    let panel = sidebar_windows(window.hwnd).1;
    let root = notebook_view(window.hwnd).root_rect();
    let effects = explorer_drop(
        panel,
        root.left + 40,
        (root.top + root.bottom) / 2,
        &[&outside],
    );
    assert_eq!(effects, [DROPEFFECT_COPY; 3]);
    pump_until(window.hwnd, || scratch.folder().join("x.md").exists());
    crate::window::copy_host::wait_for_copies(window.hwnd);

    let header = notebook_view(window.hwnd).editors_header_rect();
    explorer_drop(panel, header.left + 40, header.top + 5, &[&outside]);
    pump_until(window.hwnd, || {
        tab_paths(window.hwnd).contains(&Some(outside.clone()))
    });

    let title = 10;
    assert_eq!(
        explorer_drop(panel, 40, title, &[&outside])[1],
        DROPEFFECT_NONE,
        "the title band takes nothing"
    );
}

#[test]
fn panel_drop_returns_before_asking_and_the_posted_drop_asks() {
    // Break caught (Review Focus 4): the clash prompt shown inside Drop, which keeps
    // Explorer's drag waiting on FastPad.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("panel-drop-post");
    scratch.note("x.md", "old");
    let outside = scratch.root.join("x.md");
    std::fs::write(&outside, "new").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::side_panel::accept_file_drops(window.hwnd);
    let panel = sidebar_windows(window.hwnd).1;
    let root = notebook_view(window.hwnd).root_rect();
    let _ = crate::window::modal::take_last_confirm();
    explorer_drop(
        panel,
        root.left + 40,
        (root.top + root.bottom) / 2,
        &[&outside],
    );
    assert!(
        crate::window::modal::take_last_confirm().is_none(),
        "nothing asked during Drop"
    );
    crate::window::modal::answer_next_confirm(|_| true);
    pump_until(window.hwnd, || {
        crate::window::modal::take_last_confirm().is_some()
    });
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("x.md")).unwrap(),
        "new"
    );
}

#[test]
fn open_editors_a_tab_and_an_explorer_file_copy_into_an_empty_notebook() {
    // Break caught: a notebook with no notes refusing every copy, because the drag's hover
    // only looked for a tree (open editors spec §4.1: the root row or empty space copies into
    // the root).
    use windows_sys::Win32::System::Ole::DROPEFFECT_COPY;
    let _scintilla = load_native_scintilla();
    // The root row of an empty notebook takes a tab.
    {
        let scratch = LibraryScratch::new("empty-copy");
        let outside = scratch.root.join("tab.md");
        std::fs::write(&outside, "t").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        super::super::open_path(window.hwnd, &outside).unwrap();
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);
        let panel = sidebar_windows(window.hwnd).1;

        let tab = notebook_view(window.hwnd)
            .editors
            .rows
            .iter()
            .position(|entry| {
                entry
                    .row()
                    .is_some_and(|row| row.path.as_deref() == Some(outside.as_path()))
            })
            .unwrap();
        start_tab_drag(window.hwnd, panel, tab);
        let root = notebook_view(window.hwnd).root_rect();
        let on_root = client_lparam(root.left + 40, (root.top + root.bottom) / 2);
        drag_over(panel, on_root);
        assert_eq!(
            notebook_view(window.hwnd).drag.as_ref().unwrap().target,
            Some(PathBuf::new()),
            "the root row takes the tab"
        );
        drop_at(panel, on_root);
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert!(scratch.folder().join("tab.md").exists());
    }

    // Another empty notebook, and the space under its root row.
    let scratch = LibraryScratch::new("empty-copy-body");
    let dropped = scratch.root.join("dropped.md");
    std::fs::write(&dropped, "d").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);
    let panel = sidebar_windows(window.hwnd).1;
    crate::window::side_panel::accept_file_drops(window.hwnd);
    let root = notebook_view(window.hwnd).root_rect();
    let effects = explorer_drop(panel, root.left + 40, root.bottom + 40, &[&dropped]);
    assert_eq!(effects, [DROPEFFECT_COPY; 3]);
    pump_until(window.hwnd, || scratch.folder().join("dropped.md").exists());
    crate::window::copy_host::wait_for_copies(window.hwnd);
}

#[test]
fn inline_name_a_new_note_or_rename_with_the_root_collapsed_expands_it_and_shows_the_field() {
    // Break caught: New note, New folder or Rename opening their name field in a collapsed
    // root, hidden, with the keyboard focus in it (open editors spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-root-collapsed");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let collapse = || {
        crate::window::library_host::set_root_expanded(window.hwnd, false);
        crate::window::notebook_view::rebuild(window.hwnd);
        assert!(!notebook_view(window.hwnd).tree_shown());
    };
    collapse();
    let panel = sidebar_windows(window.hwnd).1;
    let root = notebook_view(window.hwnd).root_rect();
    let (_, new_note) = crate::window::notebook_layout::root_parts(root, 96)
        .buttons
        .into_iter()
        .find(|(button, _)| *button == crate::window::notebook_view::HeaderButton::NewNote)
        .unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(new_note));
    mouse(panel, WM_LBUTTONUP, 0, centre(new_note));
    assert!(inline_open(window.hwnd));
    assert!(crate::window::library_host::root_expanded(window.hwnd));
    assert!(notebook_view(window.hwnd).tree_shown());
    assert!(is_shown(inline_field(window.hwnd)), "the name field shows");
    field_key(window.hwnd, VK_ESCAPE);

    collapse();
    crate::window::inline_name::rename(window.hwnd, &RowKind::Note("a.md".into()));
    assert!(inline_open(window.hwnd));
    assert!(notebook_view(window.hwnd).tree_shown());
    assert!(
        is_shown(inline_field(window.hwnd)),
        "the rename field shows"
    );
    field_key(window.hwnd, VK_ESCAPE);

    collapse();
    crate::window::inline_name::rename_note_at(window.hwnd, &scratch.folder().join("a.md"));
    assert!(inline_open(window.hwnd));
    assert!(notebook_view(window.hwnd).tree_shown());
    assert!(
        is_shown(inline_field(window.hwnd)),
        "the revealed row's field shows"
    );
}

#[test]
fn open_editors_a_collapsed_root_takes_the_keyboard_off_the_hidden_tree() {
    // Break caught: with the root collapsed, the keyboard selection left on a tree row that
    // isn't shown, type-ahead selecting hidden rows, and Del deleting one (open editors spec
    // §3.3, §3.5).
    use crate::window::panel_cursor::Cursor;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DELETE;
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-root-keys");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    select_row(window.hwnd, &RowKind::Note("a.md".into()));
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Tree);

    crate::window::library_host::set_root_expanded(window.hwnd, false);
    crate::window::notebook_view::rebuild(window.hwnd);
    assert_eq!(notebook_view(window.hwnd).cursor, Cursor::Root);

    // Even a selection left in the tree some other way acts on nothing hidden.
    notebook_view(window.hwnd).cursor = Cursor::Tree;
    let panel = sidebar_windows(window.hwnd).1;
    unsafe { SendMessageW(panel, WM_CHAR, 'b' as usize, 0) };
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("a.md".into())),
        "type-ahead selects no hidden row"
    );
    let _ = crate::window::modal::take_last_confirm();
    crate::window::modal::answer_next_confirm(|_| true);
    crate::window::notebook_view::key_down(window.hwnd, VK_DELETE);
    assert!(
        crate::window::modal::take_last_confirm().is_none(),
        "no prompt"
    );
    assert!(a.exists() && b.exists());
}

#[test]
fn copy_host_a_failure_part_way_through_a_folder_says_how_many_files_were_copied() {
    // Break caught: a folder's failure without its count, or "1 files" (open editors spec
    // §4.6, §8).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-part-way");
    let pack = scratch.root.join("pack");
    std::fs::create_dir_all(&pack).unwrap();
    for name in ["a.md", "b.md", "c.md"] {
        std::fs::write(pack.join(name), name).unwrap();
    }
    let (window, _editor) = notebook_window(&scratch);
    let lock = locked(&pack.join("b.md"));
    crate::window::copy_host::copy_into(
        window.hwnd,
        vec![pack.clone()],
        std::path::Path::new(""),
        None,
    );
    crate::window::copy_host::wait_for_copies(window.hwnd);
    drop(lock);
    assert!(scratch.folder().join(r"pack\a.md").exists());
    assert!(
        !scratch.folder().join(r"pack\c.md").exists(),
        "the rest stops"
    );
    let said = notices(window.hwnd);
    assert!(
        said.iter().any(|notice| {
            notice.starts_with("pack could not be copied: ")
                && notice.ends_with(". 1 file was copied before the failure.")
        }),
        "{said:?}"
    );
}

#[test]
fn copy_host_a_dirty_tab_whose_copy_fails_gets_only_the_failure_notice() {
    // Break caught: "Copied the saved version of…" shown for a copy that failed, next to its
    // failure notice (open editors spec §4.6).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("copy-dirty-fails");
    let outside = scratch.root.join("draft.md");
    std::fs::write(&outside, "saved").unwrap();
    let (window, editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    editor.set_text("changed").unwrap();
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    let lock = locked(&outside);
    crate::window::copy_host::copy_tab_into(window.hwnd, id, &outside, std::path::Path::new(""));
    crate::window::copy_host::wait_for_copies(window.hwnd);
    drop(lock);
    let said = notices(window.hwnd);
    assert!(
        said.iter()
            .any(|notice| notice.starts_with("draft.md could not be copied: ")),
        "{said:?}"
    );
    assert!(
        !said
            .iter()
            .any(|notice| notice.starts_with("Copied the saved")),
        "{said:?}"
    );

    crate::window::copy_host::copy_tab_into(window.hwnd, id, &outside, std::path::Path::new(""));
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert!(
        notices(window.hwnd).contains(&crate::window::tree_copy::dirty_notice("draft.md")),
        "a copy that worked still says so"
    );
}

#[test]
fn panel_drop_while_a_modal_runs_is_refused_rather_than_lost() {
    // Break caught: an Explorer drop answered COPY during a modal dialog, then dropped
    // silently when its posted message arrived (open editors spec §4.3).
    use windows_sys::Win32::System::Ole::DROPEFFECT_NONE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("panel-drop-modal");
    scratch.note("a.md", "a");
    let outside = scratch.root.join("x.md");
    std::fs::write(&outside, "x").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::side_panel::accept_file_drops(window.hwnd);
    let panel = sidebar_windows(window.hwnd).1;
    let root = notebook_view(window.hwnd).root_rect();
    let header = notebook_view(window.hwnd).editors_header_rect();
    let modal = crate::window::modal::ModalScope::enter(window.hwnd);
    let effects = explorer_drop(
        panel,
        root.left + 40,
        (root.top + root.bottom) / 2,
        &[&outside],
    );
    assert_eq!(effects, [DROPEFFECT_NONE; 3]);
    assert_eq!(
        explorer_drop(panel, header.left + 40, header.top + 5, &[&outside]),
        [DROPEFFECT_NONE; 3],
        "nor does Open Editors open it"
    );
    assert!(notebook_view(window.hwnd).drag.is_none(), "no band left");
    drop(modal);
    pump_posted_messages(window.hwnd);
    assert!(!scratch.folder().join("x.md").exists());
    assert!(!tab_paths(window.hwnd).contains(&Some(outside.clone())));
}

#[test]
fn the_drop_overlay_covers_its_rectangle_and_lets_the_pointer_through() {
    // Break caught: an overlay that steals the drag's clicks, or lands off by the frame.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowRect, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let rect = RECT {
        left: 100,
        top: 120,
        right: 300,
        bottom: 220,
    };
    let overlay = crate::window::drop_overlay::DropOverlay::show(
        window.hwnd,
        rect,
        0x00ff_0000,
        crate::window::drop_overlay::TINT_ALPHA,
    )
    .expect("overlay");
    let mut shown = RECT::default();
    unsafe { GetWindowRect(overlay.hwnd(), &mut shown) };
    assert_eq!(
        (shown.left, shown.top, shown.right, shown.bottom),
        (100, 120, 300, 220)
    );
    let style = unsafe { GetWindowLongPtrW(overlay.hwnd(), GWL_EXSTYLE) } as u32;
    assert_ne!(style & WS_EX_LAYERED, 0);
    assert_ne!(style & WS_EX_TRANSPARENT, 0);
    let moved = RECT {
        left: 10,
        top: 20,
        right: 30,
        bottom: 40,
    };
    overlay.place(moved, 0x0000_ff00, crate::window::drop_overlay::BAR_ALPHA);
    assert_eq!(overlay.rect().right, 30);
    let hwnd = overlay.hwnd();
    overlay.destroy();
    assert_eq!(unsafe { super::super::IsWindow(hwnd) }, 0);
}

#[test]
fn a_tab_label_is_painted_without_the_sidebar() {
    // Break caught: the tab drag reaching into the notebook view, which is gone when the
    // sidebar is hidden.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let image = crate::window::notebook_view::tab_label_image(
        window.hwnd,
        group,
        crate::window::icon_sets::TreeItem::Note(crate::window::file_icons::NoteKind::Text),
        "notes.txt",
    )
    .expect("label image");
    assert!(image.size.cx > image.size.cy);
}
