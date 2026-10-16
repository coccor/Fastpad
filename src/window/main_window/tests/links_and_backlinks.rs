//! Links between notes (second brain phase 1 spec §7).

use super::*;

fn notebook(label: &str) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
    let scratch = LibraryScratch::new(label);
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    (scratch, window, editor)
}

fn active_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    app_mut(hwnd).tabs.active()?.path.clone()
}

#[test]
fn f12_on_a_wikilink_opens_the_note_it_names_at_its_heading() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = notebook("follow-existing");
    let target = scratch.note("Project Atlas.md", "# Atlas\n\nintro\n\n## Goals\n\nship\n");
    open_note(
        &window,
        &scratch,
        "a.md",
        "See [[project atlas#Goals|the goals]].\n",
    );
    editor.set_selection(8..8).unwrap();

    execute_command(window.hwnd, CommandId::NotesFollowLink);
    pump_posted_messages(window.hwnd);
    assert_eq!(active_path(window.hwnd), Some(target));
    let caret = editor.selection().unwrap().start;
    assert_eq!(
        editor.line_from_position(caret).unwrap(),
        4,
        "on the Goals heading"
    );
}

#[test]
fn following_a_link_to_a_missing_note_creates_it_in_the_notebook() {
    // Break caught: a link written before its note exists leading nowhere.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("follow-missing");
    let from = open_note(&window, &scratch, "a.md", "[[ideas/New Idea]]\n");
    crate::window::library_host::follow_wiki(window.hwnd, Some(&from), "ideas/New Idea", None);
    let created = scratch.folder().join("ideas").join("New Idea.md");
    assert!(created.is_file());
    assert_eq!(active_path(window.hwnd), Some(created));
}

#[test]
fn typing_two_brackets_offers_the_notebooks_notes() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = notebook("complete");
    scratch.note("Plan.md", "");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\Plan.md", "");
    scratch.note("Atlas.md", "");
    let from = open_note(&window, &scratch, "a.md", "[[");
    let items = crate::window::library_host::completion_items(window.hwnd, Some(&from));
    assert_eq!(items, ["Atlas", "Plan", "work/Plan"]);

    editor.set_selection(2..2).unwrap();
    crate::window::markdown_host::char_added(window.hwnd, &editor, i32::from(b'['));
    assert!(editor.completion_active());
}

#[test]
fn the_links_view_shows_backlinks_with_their_lines_and_what_the_note_links_to() {
    // Break caught: no way to see which notes point at the one in front of you.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("links-view");
    ensure_sidebar(window.hwnd);
    scratch.note("Meeting.md", "Agenda\n- budget for [[Atlas]] approved\n");
    scratch.note("Roadmap.md", "");
    open_note(
        &window,
        &scratch,
        "Atlas.md",
        "See [[Roadmap]] and [[Vendor list]].\n",
    );
    pump_until(window.hwnd, || {
        crate::window::library_host::index_loaded(window.hwnd)
    });
    execute_command(window.hwnd, CommandId::ShowLinksView);
    let rows = crate::window::links_view::shown_rows(window.hwnd);
    let shown: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(
        shown,
        [
            "Linked from (1)",
            "Meeting.md",
            "- budget for [[Atlas]] approved",
            "Links to (2)",
            "Roadmap.md",
            "Vendor list",
            "Missing notes (0)",
            "Not linked from anywhere (0)",
        ]
    );

    execute_command(window.hwnd, CommandId::NotesShowMissing);
    let rows = crate::window::links_view::shown_rows(window.hwnd);
    assert!(
        rows.iter()
            .any(|row| row.text == "Vendor list"
                && row.hint.as_deref() == Some("linked from 1 note"))
    );
}

fn index_settled(hwnd: HWND) {
    pump_until(hwnd, || {
        crate::window::library_host::index_loaded(hwnd) && !app_mut(hwnd).library.index_running
    });
}

#[test]
fn moving_a_linked_note_in_the_tree_offers_to_update_the_links_to_it() {
    // Break caught: a Markdown link left pointing at where a note used to be.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("move-updates-links");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note("Plan.md", "# Plan\n");
    let from = open_note(
        &window,
        &scratch,
        "a.md",
        "[[Plan]] and [the plan](Plan.md#plan)\n",
    );
    crate::window::modal::answer_next_choice(|_| 0);

    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Note("Plan.md".into()),
        std::path::Path::new("work"),
    );

    let (question, buttons) = crate::window::modal::take_last_choice().expect("asked");
    assert_eq!(
        question,
        "Update 1 link in 1 note to point to \u{201C}Plan\u{201D}?"
    );
    assert_eq!(
        buttons,
        ["Update links", "Leave them"],
        "a move keeps the name"
    );
    assert_eq!(
        std::fs::read_to_string(&from).unwrap(),
        "[[Plan]] and [the plan](work/Plan.md#plan)\n",
        "the wikilink still reaches it by name"
    );
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path, Some(from));
}

#[test]
fn renaming_a_linked_note_can_keep_its_old_name_as_an_alias() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("rename-keeps-alias");
    let plan = scratch.note("Plan.md", "# Plan\n");
    let from = open_note(&window, &scratch, "a.md", "See [[Plan|the plan]].\n");
    let root = scratch.folder();
    crate::window::tree_move::move_note(
        window.hwnd,
        &root,
        std::path::Path::new("Plan.md"),
        std::path::Path::new("Roadmap.md"),
    )
    .unwrap();
    crate::window::modal::answer_next_choice(|_| 0);

    crate::window::library_host::notes_moved(
        window.hwnd,
        &[("Plan.md".into(), "Roadmap.md".into())],
    );

    let (_, buttons) = crate::window::modal::take_last_choice().expect("asked");
    assert_eq!(
        buttons,
        ["Update links and keep alias", "Update links", "Leave them"]
    );
    assert_eq!(
        std::fs::read_to_string(&from).unwrap(),
        "See [[Roadmap|the plan]].\n"
    );
    assert!(!plan.exists());
    assert_eq!(
        std::fs::read_to_string(root.join("Roadmap.md")).unwrap(),
        "---\naliases: [Plan]\n---\n# Plan\n"
    );
}

#[test]
fn a_moved_notes_own_relative_links_are_fixed_without_asking() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("move-own-links");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note("Top.md", "");
    let moved = scratch.note(r"work\Plan.md", "Up: [top](../Top.md)\n");
    open_note(&window, &scratch, "a.md", "");

    crate::window::tree_move::drop_into(
        window.hwnd,
        &RowKind::Note(r"work\Plan.md".into()),
        std::path::Path::new(""),
    );

    assert!(!moved.exists());
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("Plan.md")).unwrap(),
        "Up: [top](Top.md)\n"
    );
    assert!(
        crate::window::modal::take_last_choice().is_none(),
        "nobody else linked to it"
    );
}

#[test]
fn a_note_renamed_outside_fastpad_is_named_and_its_links_can_be_updated() {
    // Break caught: links silently breaking when another tool renames a note.
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = notebook("outside-rename");
    let plan = scratch.note("Plan.md", "# Plan\n");
    let from = open_note(&window, &scratch, "a.md", "See [[Plan]].\n");
    index_settled(window.hwnd);
    app_mut(window.hwnd).notifications.dismiss_all();

    std::fs::rename(&plan, scratch.folder().join("Roadmap.md")).unwrap();
    rescan_and_wait(window.hwnd);
    index_settled(window.hwnd);

    assert!(notices(window.hwnd).iter().any(|notice| notice
        == "\u{201C}Plan\u{201D} was renamed to \u{201C}Roadmap\u{201D} outside FastPad. \
            1 link in 1 note still points to the old name. \
            Use Notes: Update links to renamed notes."));
    crate::window::modal::answer_next_choice(|_| 0);
    execute_command(window.hwnd, CommandId::NotesUpdateRenamedLinks);
    assert_eq!(
        std::fs::read_to_string(&from).unwrap(),
        "See [[Roadmap]].\n"
    );
    assert!(
        app_mut(window.hwnd).library.renamed_outside.is_empty(),
        "forgotten once handled"
    );
}
