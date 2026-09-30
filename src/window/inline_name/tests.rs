use super::*;

fn row(kind: RowKind, name: &str, depth: u16, expanded: bool) -> TreeRow {
    TreeRow {
        kind,
        depth,
        name: name.to_owned(),
        pinned: false,
        expanded,
    }
}

fn names(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn purpose_accessors_read_the_parent_draft_parent_and_own_row() {
    // Break caught: a new item's folder, draft-row parent or own row read wrong, which
    // would misplace the draft row or check a rename against its own name.
    let new_note = Purpose::NewNote("sub".into());
    assert_eq!(new_note.parent(), Path::new("sub"));
    assert_eq!(new_note.draft_parent(), Some(Path::new("sub")));
    assert_eq!(new_note.own_row(), None);

    let new_folder = Purpose::NewFolder(PathBuf::new());
    assert_eq!(new_folder.parent(), Path::new(""));
    assert_eq!(new_folder.draft_parent(), Some(Path::new("")));
    assert_eq!(new_folder.own_row(), None);

    let rename_note = Purpose::RenameNote(r"sub\plan.md".into());
    assert_eq!(rename_note.parent(), Path::new("sub"));
    assert_eq!(rename_note.draft_parent(), None);
    assert_eq!(
        rename_note.own_row(),
        Some(RowKind::Note(r"sub\plan.md".into()))
    );

    let rename_folder = Purpose::RenameFolder("archive".into());
    assert_eq!(rename_folder.parent(), Path::new(""));
    assert_eq!(rename_folder.draft_parent(), None);
    assert_eq!(
        rename_folder.own_row(),
        Some(RowKind::Folder("archive".into()))
    );
}

#[test]
fn typed_names_cancel_when_empty_or_unchanged_and_a_case_change_is_a_rename() {
    // Break caught: an empty draft creating "Untitled.md", Enter on an unchanged rename
    // renaming onto itself, or "plan.md" to "Plan.md" treated as no change (spec §4).
    let note = Purpose::NewNote(PathBuf::new());
    assert_eq!(typed_name(&note, "todo").as_deref(), Some("todo.md"));
    assert_eq!(typed_name(&note, "  "), None);
    let folder = Purpose::NewFolder("sub".into());
    assert_eq!(typed_name(&folder, " a/b: c?. ").as_deref(), Some("ab c"));
    assert_eq!(typed_name(&folder, "..."), None);
    let rename = Purpose::RenameNote(r"sub\plan.md".into());
    assert_eq!(typed_name(&rename, "plan.md"), None);
    assert_eq!(typed_name(&rename, "Plan.md").as_deref(), Some("Plan.md"));
    assert_eq!(typed_name(&rename, "draft").as_deref(), Some("draft.md"));
    let rename_folder = Purpose::RenameFolder("v1.2".into());
    assert_eq!(typed_name(&rename_folder, "v1.2"), None);
    assert_eq!(typed_name(&rename_folder, "V1.2").as_deref(), Some("V1.2"));
}

#[test]
fn the_live_check_finds_a_taken_name_ignoring_case_and_refuses_hidden_folder_names() {
    // Break caught: "TODO" slipping past a listed todo.md, a note's own name reported as
    // taken, or a ".git" folder created that the next rescan hides (spec §4.4).
    let siblings = names(&["todo.md", "archive"]);
    let note = Purpose::NewNote(PathBuf::new());
    assert_eq!(
        check(&note, "TODO", &siblings).as_deref(),
        Some("TODO.md already exists here.")
    );
    assert_eq!(check(&note, "other", &siblings), None);
    assert_eq!(check(&note, "", &siblings), None, "an empty name cancels");
    let folder = Purpose::NewFolder(PathBuf::new());
    assert_eq!(
        check(&folder, "Archive", &siblings).as_deref(),
        Some("Archive already exists here.")
    );
    assert_eq!(
        check(&folder, ".git", &siblings).as_deref(),
        Some("FastPad hides folders named \u{201c}.git\u{201d}. Choose another name.")
    );
    // A rename's own row is left out of `siblings` (`sibling_names`' `own`).
    let rename = Purpose::RenameNote("plan.md".into());
    assert_eq!(check(&rename, "PLAN.md", &names(&["b.md"])), None);
    assert_eq!(
        check(&rename, "b", &names(&["b.md"])).as_deref(),
        Some("b.md already exists here.")
    );
}

#[test]
fn a_rename_selects_the_stem_of_a_note_and_all_of_a_folder() {
    // Break caught: typing over "a.md" also replacing ".md", ".gitignore" opening with
    // nothing selected, or a "v1.2" folder keeping ".2" (spec §3.3).
    assert_eq!(rename_selection("a.md", false), (0, 1));
    assert_eq!(rename_selection(".gitignore", false), (0, 10));
    assert_eq!(rename_selection("archive.tar.gz", false), (0, 11));
    assert_eq!(rename_selection("README", false), (0, 6));
    assert_eq!(rename_selection("v1.2", true), (0, 4));
    assert_eq!(rename_selection("é.md", false), (0, 1), "UTF-16 units");
}

#[test]
fn the_draft_icon_follows_the_typed_note_extension() {
    // Break caught: a new JSON note drawn as Markdown, or a folder draft drawn as a note.
    let note = Purpose::NewNote(PathBuf::new());
    let markdown = TreeItem::Note(NoteKind::Markdown);
    assert_eq!(draft_icon(&note, ""), markdown);
    assert_eq!(
        draft_icon(&note, "data.json"),
        TreeItem::Note(note_kind(std::path::Path::new("a.json")))
    );
    assert_eq!(draft_icon(&note, "v1.2"), markdown);
    assert_eq!(
        draft_icon(&Purpose::NewFolder(PathBuf::new()), "x.json"),
        TreeItem::Folder { expanded: false }
    );
}

#[test]
fn the_field_is_named_for_what_it_names_and_where() {
    // Break caught: a screen reader hearing a bare "edit", or the root named "" (spec §6).
    assert_eq!(
        accessible_name(&Purpose::NewNote(PathBuf::new()), "Notes"),
        "New note name, in Notes"
    );
    assert_eq!(
        accessible_name(&Purpose::NewFolder(r"a\sub".into()), "Notes"),
        "New folder name, in sub"
    );
    assert_eq!(
        accessible_name(&Purpose::RenameNote(r"a\b.md".into()), "Notes"),
        "Rename b.md"
    );
}

#[test]
fn siblings_are_the_rows_directly_in_the_folder_without_the_own_row() {
    // Break caught: a name in a subfolder or the renamed row itself counted as taken, or a
    // sibling below a nested folder missed.
    let rows = vec![
        row(RowKind::Folder("sub".into()), "sub", 0, true),
        row(RowKind::Note(r"sub\A.md".into()), "A.md", 1, false),
        row(RowKind::Folder(r"sub\deep".into()), "deep", 1, true),
        row(RowKind::Note(r"sub\deep\x.md".into()), "x.md", 2, false),
        row(RowKind::Note(r"sub\b.md".into()), "b.md", 1, false),
        row(RowKind::Note("top.md".into()), "top.md", 0, false),
    ];
    assert_eq!(parent_row(&rows, Path::new("sub")), Some(Some(0)));
    assert_eq!(parent_row(&rows, Path::new("")), Some(None));
    assert_eq!(parent_row(&rows, Path::new("gone")), None);
    assert_eq!(
        sibling_names(&rows, Some(0), None),
        names(&["a.md", "deep", "b.md"])
    );
    assert_eq!(
        sibling_names(&rows, Some(0), Some(1)),
        names(&["deep", "b.md"])
    );
    assert_eq!(sibling_names(&rows, None, None), names(&["sub", "top.md"]));
}

#[test]
fn the_draft_row_is_the_first_child_of_an_expanded_folder_or_first_at_the_root() {
    // Break caught: a draft row at the end of its folder, at the wrong depth, or inside a
    // collapsed folder (spec §3.1).
    let mut rows = vec![
        row(RowKind::Folder("sub".into()), "sub", 0, true),
        row(RowKind::Note(r"sub\a.md".into()), "a.md", 1, false),
        row(RowKind::Folder("shut".into()), "shut", 0, false),
    ];
    assert_eq!(insert_draft(&mut rows, Some(0)), Some(1));
    assert_eq!((rows[1].kind.clone(), rows[1].depth), (RowKind::Draft, 1));
    rows.remove(1);
    assert_eq!(insert_draft(&mut rows, None), Some(0));
    assert_eq!((rows[0].kind.clone(), rows[0].depth), (RowKind::Draft, 0));
    rows.remove(0);
    assert_eq!(insert_draft(&mut rows, Some(2)), None, "collapsed");
    assert_eq!(rows.len(), 3);
}

#[test]
fn ctrl_backspace_deletes_spaces_then_one_run_of_word_or_punctuation() {
    // Break caught: Ctrl+Backspace typing a box character or deleting the whole name.
    let wide = |text: &str| text.encode_utf16().collect::<Vec<_>>();
    assert_eq!(word_start(&wide("my note.md"), 10), 8);
    assert_eq!(word_start(&wide("my note.md"), 8), 7);
    assert_eq!(word_start(&wide("my note  "), 9), 3);
    assert_eq!(word_start(&wide("my"), 0), 0);
    assert_eq!(word_start(&wide("my"), 99), 0, "a caret past the end");
}

#[test]
fn the_field_covers_the_name_up_to_the_pin_and_stays_inside_the_list() {
    // Break caught: the field drawn over the chevron, icon or pin, over the header when its
    // row is scrolled up, or below the list's bottom edge (spec §3.3, §5.4).
    let list = RECT {
        left: 0,
        top: 38,
        right: 240,
        bottom: 400,
    };
    let row_at = |top: i32| RECT {
        left: 0,
        top,
        right: 240,
        bottom: top + 26,
    };
    let parts = super::super::notebook_view::row_parts(row_at(60), 1, 96);
    let layout = field_layout(row_at(60), list, 1, 96, 16).unwrap();
    assert!(layout.frame.left > parts.icon.right - 1);
    assert_eq!(layout.frame.right, parts.pin.left);
    assert!(layout.edit.left > layout.frame.left && layout.edit.right < layout.frame.right);
    assert!(layout.edit.top > layout.frame.top && layout.edit.bottom < layout.frame.bottom);
    assert!(
        field_layout(row_at(12), list, 1, 96, 16).is_none(),
        "under the header"
    );
    assert!(
        field_layout(row_at(400), list, 1, 96, 16).is_none(),
        "below the list"
    );
    let cut = field_layout(row_at(390), list, 1, 96, 16).unwrap();
    assert!(cut.frame.bottom <= list.bottom && cut.edit.bottom <= list.bottom);
}

#[test]
fn the_problem_goes_under_the_field_or_above_it_on_the_last_row() {
    // Break caught: a message drawn past the list's bottom, hidden under the next paint, or
    // over the header (spec §4.4).
    let list = RECT {
        left: 0,
        top: 38,
        right: 240,
        bottom: 400,
    };
    let frame = RECT {
        left: 55,
        top: 62,
        right: 216,
        bottom: 84,
    };
    let below = message_rect(frame, list, 30);
    assert_eq!((below.top, below.bottom), (84, 114));
    let last = RECT {
        top: 380,
        bottom: 398,
        ..frame
    };
    let above = message_rect(last, list, 30);
    assert_eq!((above.top, above.bottom), (350, 380));
    let tiny = RECT {
        top: 38,
        bottom: 70,
        ..list
    };
    let first = RECT {
        top: 40,
        bottom: 60,
        ..frame
    };
    assert_eq!(message_rect(first, tiny, 40).top, 38);
}

#[test]
fn only_ctrl_z_and_ctrl_y_among_the_field_keys_are_accelerators() {
    // Break caught: an accelerator on Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V, Del, Home, End,
    // Ctrl+Left, Ctrl+Right or Ctrl+Backspace taking the key from the field, which keeps
    // only Ctrl+Z and Ctrl+Y from the table (inline naming spec §5.1, §11).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_BACK, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RIGHT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{FCONTROL, FSHIFT};
    let keymap = crate::window::keymap::Keymap::defaults();
    let bound = |modifiers: u8, key: u16| {
        keymap
            .bindings()
            .iter()
            .any(|binding| binding.stroke.accel_flags() == modifiers && binding.stroke.vk == key)
    };
    let letter = |key: u8| u16::from(key);
    for (modifiers, key) in [
        (FCONTROL, letter(b'A')),
        (FCONTROL, letter(b'C')),
        (FCONTROL, letter(b'X')),
        (FCONTROL, letter(b'V')),
        (0, VK_DELETE),
        (0, VK_HOME),
        (0, VK_END),
        (FSHIFT, VK_HOME),
        (FSHIFT, VK_END),
        (FCONTROL, VK_LEFT),
        (FCONTROL, VK_RIGHT),
        (FCONTROL | FSHIFT, VK_LEFT),
        (FCONTROL | FSHIFT, VK_RIGHT),
        (FCONTROL, VK_BACK),
    ] {
        assert!(!bound(modifiers, key), "{modifiers:#x} {key:#x}");
    }
    assert!(
        bound(FCONTROL, letter(b'Z')),
        "Ctrl+Z is Undo: the field keeps it"
    );
    assert!(
        bound(FCONTROL, letter(b'Y')),
        "Ctrl+Y is Redo: the field keeps it"
    );
}
