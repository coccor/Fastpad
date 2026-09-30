use super::*;

fn all(_: &Path) -> bool {
    true
}

fn build_with(notes: &[&str], folders: &[&str], pinned: &[&str]) -> NoteTree {
    let notes: Vec<PathBuf> = notes.iter().map(PathBuf::from).collect();
    let folders: Vec<PathBuf> = folders.iter().map(PathBuf::from).collect();
    let pinned: Vec<PathBuf> = pinned.iter().map(PathBuf::from).collect();
    NoteTree::build(&notes, &folders, &pinned)
}

fn build(notes: &[&str], pinned: &[&str]) -> NoteTree {
    build_with(notes, &[], pinned)
}

/// Each row as `<indent><name><marker>`: `/` a folder, `*` a pinned note.
fn outline(rows: &[TreeRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let marker = match (&row.kind, row.pinned) {
                (RowKind::Folder(_), _) => "/",
                (RowKind::Draft, _) => "+",
                (RowKind::Note(_), true) => "*",
                (RowKind::Note(_), false) => "",
            };
            format!(
                "{}{}{marker}",
                "  ".repeat(usize::from(row.depth)),
                row.name
            )
        })
        .collect()
}

#[test]
fn natural_order_compares_digit_runs_as_numbers_and_ignores_case() {
    // Break caught: "Note 10" sorting before "Note 2", or "apple" after "Banana".
    assert_eq!(natural_cmp("Note 2", "Note 10"), Ordering::Less);
    assert_eq!(natural_cmp("note 2", "Note 2"), Ordering::Equal);
    assert_eq!(natural_cmp("apple", "Banana"), Ordering::Less);
    assert_eq!(natural_cmp("file10b", "file10a"), Ordering::Greater);
    assert_eq!(natural_cmp("x9", "x09"), Ordering::Equal);
    assert_eq!(natural_cmp("", "a"), Ordering::Less);
    assert_eq!(
        natural_cmp("n123456789012345678901234567890", "n2"),
        Ordering::Greater,
        "digit runs longer than any integer type"
    );
    assert_eq!(natural_cmp("Äpfel", "äpfel"), Ordering::Equal);
}

#[test]
fn each_folder_lists_pinned_notes_then_subfolders_then_other_notes() {
    // Break caught: pins mixed into the name order, folders after the notes, or "Gamma 10"
    // before "Gamma 9".
    let tree = build(
        &[
            "b.md",
            "Note 10.md",
            "Note 2.md",
            r"beta\x.md",
            r"Alpha\y.md",
            r"Gamma 10\q.md",
            r"Gamma 9\q.md",
            "z.md",
            r"Alpha\pinned.md",
        ],
        &["z.md", r"Alpha\pinned.md"],
    );
    assert_eq!(
        outline(&tree.rows(&all)),
        [
            "z.md*",
            "Alpha/",
            "  pinned.md*",
            "  y.md",
            "beta/",
            "  x.md",
            "Gamma 9/",
            "  q.md",
            "Gamma 10/",
            "  q.md",
            "b.md",
            "Note 2.md",
            "Note 10.md",
        ]
    );
    assert_eq!(tree.note_count(), 9);
}

#[test]
fn ties_are_broken_by_extension_then_by_exact_name() {
    let tree = build(&["a.txt", "A2.md", "a1.md", "a.md", "a01.md"], &[]);
    let paths: Vec<PathBuf> = tree
        .rows(&all)
        .into_iter()
        .map(|row| match row.kind {
            RowKind::Note(path) => path,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        paths,
        ["a.md", "a.txt", "a01.md", "a1.md", "A2.md"].map(PathBuf::from)
    );
}

#[test]
fn rows_do_not_depend_on_the_order_the_scan_listed_notes_in() {
    // Break caught: a rescan after an edit (which changes a file's position in the scan)
    // moving its row, so opening or editing a note reorders the tree.
    let notes = [r"s\b.md", "c.md", r"s\a.md", "a.md", "b.md"];
    let mut reversed = notes;
    reversed.reverse();
    assert_eq!(
        build(&notes, &["b.md"]).rows(&all),
        build(&reversed, &["b.md"]).rows(&all)
    );
}

#[test]
fn collapsed_folders_hide_their_rows_and_the_root_is_always_open() {
    let tree = build(&["top.md", r"a\one.md", r"a\b\two.md"], &[]);
    let only_a = |path: &Path| path == Path::new("a");
    let rows = tree.rows(&only_a);
    assert_eq!(outline(&rows), ["a/", "  b/", "  one.md", "top.md"]);
    assert!(rows[0].expanded && !rows[1].expanded);
    assert_eq!(rows[1].kind, RowKind::Folder(PathBuf::from(r"a\b")));
    assert_eq!(rows[2].kind, RowKind::Note(PathBuf::from(r"a\one.md")));
    assert_eq!(outline(&tree.rows(&|_| false)), ["a/", "top.md"]);
}

#[test]
fn folders_and_notes_match_ignoring_case() {
    // Break caught: `Sub\a.md` and `sub\b.md` showing as two folders, or a case-only rename
    // leaving the old row behind.
    let mut tree = build(&[r"Sub\a.md", r"sub\b.md"], &[]);
    assert_eq!(outline(&tree.rows(&all)), ["Sub/", "  a.md", "  b.md"]);
    tree.insert_note(Path::new(r"SUB\A.MD"), true);
    assert_eq!(outline(&tree.rows(&all)), ["Sub/", "  A.MD*", "  b.md"]);
    assert_eq!(tree.note_count(), 2);
    tree.remove_note(Path::new(r"sub\b.MD"));
    tree.set_pinned(Path::new(r"sub\a.md"), false);
    assert_eq!(outline(&tree.rows(&all)), ["Sub/", "  A.MD"]);
    assert_eq!(tree.note_count(), 1);
}

#[test]
fn incremental_updates_match_a_full_rebuild() {
    // Break caught: an insert, removal, rename or pin change leaving the tree in an order, or
    // without a folder a note once made, that a fresh build of the same notes and folders
    // would not have.
    let names: Vec<PathBuf> = [
        "a.md",
        "b.md",
        r"x\c.md",
        r"x\d.md",
        r"x\y\e.md",
        r"z\f.md",
        "Note 2.md",
        "Note 10.md",
    ]
    .map(PathBuf::from)
    .to_vec();
    let mut model: Vec<(PathBuf, bool)> = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    let mut tree = NoteTree::default();
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |bound: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % bound
    };
    for step in 0..400 {
        let path = names[next(names.len())].clone();
        match next(4) {
            0 => {
                let pinned = next(2) == 0;
                tree.insert_note(&path, pinned);
                folders.extend(ancestors(&path));
                model.retain(|(existing, _)| existing != &path);
                model.push((path, pinned));
            }
            1 => {
                tree.remove_note(&path);
                model.retain(|(existing, _)| existing != &path);
            }
            2 => {
                let pinned = next(2) == 0;
                tree.set_pinned(&path, pinned);
                if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                    entry.1 = pinned;
                }
            }
            _ => {
                let target = names[next(names.len())].clone();
                // A rename onto an existing note never happens: the file system refuses it.
                if model.iter().any(|(existing, _)| existing == &target) {
                    continue;
                }
                tree.rename_note(&path, &target);
                if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                    folders.extend(ancestors(&target));
                    entry.0 = target;
                }
            }
        }
        let notes: Vec<PathBuf> = model.iter().map(|(path, _)| path.clone()).collect();
        let pinned: Vec<PathBuf> = model
            .iter()
            .filter(|(_, pinned)| *pinned)
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(
            tree.rows(&all),
            NoteTree::build(&notes, &folders, &pinned).rows(&all),
            "step {step}"
        );
        assert_eq!(tree.note_count(), model.len(), "step {step}");
    }
}

#[test]
fn removing_the_last_note_in_a_folder_chain_keeps_the_chain() {
    // Break caught: a folder row vanishing when its last note is deleted or moved while the
    // folder is still on disk (spec §3.2).
    let mut tree = build(&[r"a\b\c\n.md", r"a\keep.md", "top.md"], &[]);
    tree.remove_note(Path::new(r"a\b\c\n.md"));
    assert_eq!(
        outline(&tree.rows(&all)),
        ["a/", "  b/", "    c/", "  keep.md", "top.md"]
    );
    tree.remove_note(Path::new(r"a\keep.md"));
    assert_eq!(
        outline(&tree.rows(&all)),
        ["a/", "  b/", "    c/", "top.md"]
    );
    tree.remove_note(Path::new("missing.md"));
    assert_eq!(tree.note_count(), 1);
}

#[test]
fn listed_folders_get_rows_even_empty_and_still_sort_first() {
    // Break caught: an empty folder with no row, a listed folder sorting among the notes, or
    // a note's own spelling of its folder replacing the one on disk.
    let tree = build_with(
        &["b.md", r"notes\a.md", r"SUB\x.md"],
        &["empty", "Zeta", r"notes\inner", "Sub"],
        &[],
    );
    assert_eq!(
        outline(&tree.rows(&all)),
        [
            "empty/", "notes/", "  inner/", "  a.md", "Sub/", "  x.md", "Zeta/", "b.md"
        ]
    );
    assert_eq!(tree.note_count(), 3);
    assert!(tree.contains_folder(Path::new(r"NOTES\Inner")));
    assert!(!tree.contains_folder(Path::new("b.md")));
    assert!(!tree.contains_folder(Path::new("")));
}

#[test]
fn a_folder_the_list_left_out_still_gets_a_row_from_its_notes() {
    // Break caught: a folder past the scan's folder cap hiding the notes inside it.
    let tree = build_with(&[r"c\n.md"], &["a", "b"], &[]);
    assert_eq!(outline(&tree.rows(&all)), ["a/", "b/", "c/", "  n.md"]);
}

#[test]
fn insert_folder_adds_its_missing_ancestors_and_keeps_an_existing_spelling() {
    // Break caught: a new folder inside a chain with no parent row, a second row for a folder
    // typed in another case, or an absolute path making a row.
    let mut tree = build(&["top.md"], &[]);
    tree.insert_folder(Path::new(r"x\y\z"));
    assert_eq!(
        outline(&tree.rows(&all)),
        ["x/", "  y/", "    z/", "top.md"]
    );
    tree.insert_folder(Path::new(r"X\Y"));
    for bad in [r"C:\abs", "", r"..\up", r"\rooted"] {
        tree.insert_folder(Path::new(bad));
    }
    assert_eq!(
        outline(&tree.rows(&all)),
        ["x/", "  y/", "    z/", "top.md"]
    );
    assert_eq!(tree.note_count(), 1);
}

#[test]
fn the_row_in_a_removed_folders_place_is_the_next_one_after_its_subtree_else_the_one_before() {
    // Break caught: a folder delete selecting a row inside the folder it removed, or nothing
    // when the folder was the last row.
    let tree = build_with(&[r"a\x\n.md", r"a\m.md", "top.md"], &["a", r"a\x"], &[]);
    let expanded = [PathBuf::from("a"), PathBuf::from(r"a\x")];
    let rows = tree.rows(&|path| expanded.iter().any(|entry| entry == path));
    let kind = |index| row_in_place_of(&rows, index).map(|row| row.kind.clone());
    let a = row_index(&rows, &RowKind::Folder("a".into())).unwrap();
    let x = row_index(&rows, &RowKind::Folder(r"a\x".into())).unwrap();
    let top = row_index(&rows, &RowKind::Note("top.md".into())).unwrap();
    assert_eq!(kind(a), Some(RowKind::Note("top.md".into())));
    assert_eq!(kind(x), Some(RowKind::Note(r"a\m.md".into())));
    assert_eq!(kind(top), Some(rows[top - 1].kind.clone()));
    assert_eq!(row_in_place_of(&rows[..1], 0), None);
    assert_eq!(row_in_place_of(&rows, rows.len()), None);
}

#[test]
fn only_a_plain_relative_path_is_a_folder_the_folder_commands_act_on() {
    // Break caught: an empty, `.`, `..`, absolute or rooted path reaching a folder delete or
    // rename, where joined onto the notebook root it names the root itself or a folder
    // outside the notebook.
    for bad in ["", ".", "..", r"C:\Notes", r"\x", r"a\..\b", r"C:x"] {
        assert!(!is_plain_relative_folder(Path::new(bad)), "{bad:?}");
    }
    for good in ["a", r"a\b", "a/b", "sub way"] {
        assert!(is_plain_relative_folder(Path::new(good)), "{good:?}");
    }
}

#[test]
fn remove_folder_takes_its_whole_subtree_and_its_notes_count() {
    // Break caught: a deleted folder leaving its subfolders or notes behind, or a note count
    // that still includes them.
    let mut tree = build_with(
        &[r"a\one.md", r"a\b\two.md", r"a\b\c\three.md", "top.md"],
        &[r"a\b\empty"],
        &[r"a\b\two.md"],
    );
    tree.remove_folder(Path::new(r"A\B"));
    assert_eq!(outline(&tree.rows(&all)), ["a/", "  one.md", "top.md"]);
    assert_eq!(tree.note_count(), 2);
    tree.remove_folder(Path::new("missing"));
    tree.remove_folder(Path::new("top.md"));
    assert_eq!(tree.note_count(), 2);
    tree.remove_folder(Path::new("a"));
    assert_eq!(outline(&tree.rows(&all)), ["top.md"]);
    assert_eq!(tree.note_count(), 1);
}

#[test]
fn rename_folder_moves_the_subtree_with_its_notes_and_pins() {
    // Break caught: a renamed folder losing its notes, their pins or its empty subfolders, a
    // case-only rename leaving the old spelling, or a replayed rename losing the folder.
    let mut tree = build_with(
        &[r"work\plan.md", r"work\sub\deep.md", "top.md"],
        &[r"work\empty\deeper"],
        &[r"work\plan.md"],
    );
    tree.rename_folder(Path::new("work"), Path::new("Archive"));
    assert_eq!(
        outline(&tree.rows(&all)),
        [
            "Archive/",
            "  plan.md*",
            "  empty/",
            "    deeper/",
            "  sub/",
            "    deep.md",
            "top.md"
        ]
    );
    assert_eq!(tree.note_count(), 3);
    let rows = tree.rows(&all);
    assert!(row_index(&rows, &RowKind::Note(PathBuf::from(r"Archive\sub\deep.md"))).is_some());
    assert!(!tree.contains_folder(Path::new("work")));

    tree.rename_folder(Path::new("archive"), Path::new("ARCHIVE"));
    assert_eq!(tree.rows(&all)[0].name, "ARCHIVE");
    assert_eq!(tree.note_count(), 3);

    tree.rename_folder(Path::new("gone"), Path::new("Made"));
    assert!(
        tree.contains_folder(Path::new("Made")),
        "a rescan that already saw the rename"
    );

    let mut merged = build_with(&[r"a\x.md", r"b\y.md"], &[], &[r"a\x.md"]);
    merged.rename_folder(Path::new("a"), Path::new("b"));
    assert_eq!(outline(&merged.rows(&all)), ["b/", "  x.md*", "  y.md"]);
    assert_eq!(merged.note_count(), 2);
}

#[test]
fn pins_and_renames_move_rows_and_renames_keep_the_pin() {
    let mut tree = build(&["a.md", "b.md", "c.md"], &[]);
    tree.set_pinned(Path::new("c.md"), true);
    assert_eq!(outline(&tree.rows(&all)), ["c.md*", "a.md", "b.md"]);
    tree.rename_note(Path::new("c.md"), Path::new(r"sub\c2.md"));
    assert_eq!(
        outline(&tree.rows(&all)),
        ["sub/", "  c2.md*", "a.md", "b.md"]
    );
    tree.rename_note(Path::new("gone.md"), Path::new("new.md"));
    assert_eq!(
        tree.note_count(),
        3,
        "renaming a note the tree lacks does nothing"
    );
}

#[test]
fn rows_are_found_by_path_parent_and_typed_prefix() {
    // Break caught: Left on a nested note jumping to the wrong folder, or type-ahead stuck
    // on the current row instead of moving on.
    let tree = build(&[r"a\one.md", r"a\b\two.md", "top.md"], &[]);
    let rows = tree.rows(&all);
    assert_eq!(
        outline(&rows),
        ["a/", "  b/", "    two.md", "  one.md", "top.md"]
    );
    assert_eq!(
        row_index(&rows, &RowKind::Note(PathBuf::from(r"A\B\TWO.md"))),
        Some(2)
    );
    assert_eq!(
        row_index(&rows, &RowKind::Folder(PathBuf::from(r"a\b"))),
        Some(1)
    );
    assert_eq!(parent_index(&rows, 2), Some(1));
    assert_eq!(parent_index(&rows, 3), Some(0));
    assert_eq!(parent_index(&rows, 1), Some(0));
    assert_eq!(parent_index(&rows, 0), None);
    assert_eq!(parent_index(&rows, 4), None);
    assert_eq!(parent_index(&rows, 9), None);
    assert_eq!(type_ahead(&rows, 0, "t"), Some(2));
    assert_eq!(type_ahead(&rows, 3, "T"), Some(4));
    assert_eq!(type_ahead(&rows, 5, "on"), Some(3), "wraps around");
    assert_eq!(type_ahead(&rows, 0, ""), None);
    assert_eq!(type_ahead(&rows, 0, "zz"), None);
    assert_eq!(
        ancestors(Path::new(r"a\b\two.md")),
        [PathBuf::from("a"), PathBuf::from(r"a\b")]
    );
    assert!(ancestors(Path::new("top.md")).is_empty());
}

#[test]
fn a_draft_row_is_found_by_its_kind() {
    // Break caught: the view losing its draft row after a rebuild because `row_index`
    // never matches it.
    let rows = vec![TreeRow {
        kind: RowKind::Draft,
        depth: 0,
        name: String::new(),
        pinned: false,
        expanded: false,
    }];
    assert_eq!(row_index(&rows, &RowKind::Draft), Some(0));
    assert_eq!(row_index(&rows, &RowKind::Note(PathBuf::new())), None);
}

#[test]
fn ten_thousand_notes_in_one_folder_flatten_quickly() {
    // Break caught: a quadratic build or flatten, so opening a big notebook or expanding its
    // one huge folder stalls.
    let notes: Vec<PathBuf> = (0..10_000)
        .map(|index| PathBuf::from(format!(r"big\Note {index}.md")))
        .collect();
    let pinned = vec![PathBuf::from(r"big\Note 9999.md")];
    let started = std::time::Instant::now();
    let tree = NoteTree::build(&notes, &[], &pinned);
    let rows = tree.rows(&all);
    let elapsed = started.elapsed();
    assert_eq!(tree.note_count(), 10_000);
    assert_eq!(rows.len(), 10_001);
    assert_eq!(
        outline(&rows[..4]),
        ["big/", "  Note 9999.md*", "  Note 0.md", "  Note 1.md"]
    );
    assert_eq!(rows[10_000].name, "Note 9998.md");
    assert_eq!(tree.rows(&|_| false).len(), 1);
    if !cfg!(debug_assertions) {
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "{elapsed:?}"
        );
    }
}

#[test]
fn a_very_deep_folder_chain_builds_flattens_and_empties_without_recursion() {
    // Break caught: a recursive walk (or drop) overflowing the UI thread's stack on a
    // notebook nested thousands of folders deep.
    let depth = 2_000;
    let folder: PathBuf = (0..depth).map(|index| format!("d{index}")).collect();
    let note = folder.join("deep.md");
    let mut tree = NoteTree::build(std::slice::from_ref(&note), &[], &[]);
    let rows = tree.rows(&all);
    assert_eq!(rows.len(), depth + 1);
    assert_eq!(usize::from(rows[depth].depth), depth);
    assert_eq!(rows[depth].kind, RowKind::Note(note.clone()));
    tree.remove_note(&note);
    assert_eq!(tree.rows(&all).len(), depth, "the emptied folders stay");
    tree.remove_folder(Path::new("d0"));
    assert!(tree.rows(&all).is_empty());
    let deep = NoteTree::build(std::slice::from_ref(&note), &[], &[]);
    drop(deep);
}

#[test]
fn paths_that_are_not_plain_relative_names_are_ignored() {
    let mut tree = build(
        &[r"C:\abs\x.md", r"..\up.md", "", r".\dot.md", "ok.md"],
        &[],
    );
    tree.insert_note(Path::new(r"\rooted.md"), false);
    assert_eq!(outline(&tree.rows(&all)), ["ok.md"]);
    assert_eq!(tree.note_count(), 1);
}

#[test]
fn a_second_spelling_given_to_build_keeps_the_one_given_first() {
    // Break caught: the compact name store sorting away the input order, so the spelling
    // kept (or the count) depends on which one sorts first.
    let tree = build(
        &["x.md", "X.md", r"Sub\B.md", r"sub\b.md", "x.md", "*.md"],
        &["X.MD"],
    );
    assert_eq!(
        outline(&tree.rows(&all)),
        ["x.md*", "Sub/", "  B.md", "*.md"]
    );
    assert_eq!(tree.note_count(), 3);
}

#[test]
fn names_stay_right_when_removals_compact_their_store() {
    // Break caught: a note pointing into the old name buffer after a compaction, showing
    // another note's name or a torn one.
    let all_notes: Vec<PathBuf> = (0..100)
        .map(|index| PathBuf::from(format!(r"f\Note {index}.md")))
        .collect();
    let mut tree = NoteTree::build(&all_notes, &[], &[]);
    for path in all_notes.iter().filter(|path| {
        let name = path.to_string_lossy();
        !name.ends_with("7.md")
    }) {
        tree.remove_note(path);
    }
    tree.insert_note(Path::new(r"f\Later.md"), true);
    tree.set_pinned(Path::new(r"f\Note 17.md"), true);
    let mut kept: Vec<PathBuf> = all_notes
        .iter()
        .filter(|path| path.to_string_lossy().ends_with("7.md"))
        .cloned()
        .collect();
    kept.push(PathBuf::from(r"f\Later.md"));
    let fresh = NoteTree::build(
        &kept,
        &[],
        &[PathBuf::from(r"f\Later.md"), PathBuf::from(r"f\Note 17.md")],
    );
    assert_eq!(tree.rows(&all), fresh.rows(&all));
    assert_eq!(tree.note_count(), 11);
}

#[test]
fn a_name_no_file_can_have_is_refused_without_leaving_its_folder() {
    let long = PathBuf::from("f").join("n".repeat(70_000));
    let mut tree = NoteTree::build(std::slice::from_ref(&long), &[], &[]);
    tree.insert_note(&long, false);
    assert!(tree.rows(&all).is_empty());
    assert_eq!(tree.note_count(), 0);
}
