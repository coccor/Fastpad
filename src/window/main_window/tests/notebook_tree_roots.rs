//! The Notebook tree with several notebooks open: one root row per notebook, with its rows,
//! placeholder, marker, buttons and menu (multi-root notebooks spec §4).

use super::*;
use crate::library::tree::Placeholder;
use crate::window::notebook_view::HeaderButton;

/// The notebooks A and B with the Notebook view shown and its rows built.
fn roots_ab(label: &str) -> NotebooksAB {
    let ab = notebooks_a_and_b(label, "a", "b");
    crate::window::side_panel::show_view(
        ab.window.hwnd,
        crate::config::SidebarView::Notebook,
        false,
    );
    crate::window::notebook_view::rebuild(ab.window.hwnd);
    ab
}

/// The text of row `index`.
fn row_text(hwnd: HWND, index: usize) -> String {
    notebook_view(hwnd).rows[index].name.clone()
}

/// Presses and releases the left button in the middle of `rect`.
fn click_rect(panel: HWND, rect: RECT) {
    click(
        panel,
        (rect.left + rect.right) / 2,
        (rect.top + rect.bottom) / 2,
    );
}

#[test]
fn rows_are_built_root_by_root_with_one_root_row_each() {
    // Break caught: only the default notebook listed, a notebook's rows without a root row
    // above them, or a row tagged with the wrong notebook.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-rows");
    let hwnd = ab.window.hwnd;
    assert_eq!(
        view_rows(hwnd),
        vec![
            (ab.a_id, RowKind::Root),
            (ab.a_id, RowKind::Note("a.md".into())),
            (ab.b_id, RowKind::Root),
            (ab.b_id, RowKind::Note("b.md".into())),
        ]
    );
    let rows = &notebook_view(hwnd).rows;
    let depths: Vec<u16> = rows.iter().map(|row| row.depth).collect();
    assert_eq!(
        depths,
        [0, 1, 0, 1],
        "a root's contents are one level under it"
    );
    assert_eq!((rows[0].name.as_str(), rows[2].name.as_str()), ("A", "B"));
}

#[test]
fn a_collapsed_root_hides_its_rows_and_remembers_it() {
    // Break caught: collapsing one root hiding every notebook, or the collapse forgotten when
    // the notebook is read again from disk.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-collapse");
    let hwnd = ab.window.hwnd;
    let panel = notebook_view(hwnd).panel;
    let root_b = row_in(hwnd, ab.b_id, &RowKind::Root);
    let rect = notebook_view(hwnd).row_rect_at(root_b).unwrap();
    // The chevron, at the row's left.
    click(panel, rect.left + 10, (rect.top + rect.bottom) / 2);

    assert_eq!(
        view_rows(hwnd),
        vec![
            (ab.a_id, RowKind::Root),
            (ab.a_id, RowKind::Note("a.md".into())),
            (ab.b_id, RowKind::Root),
        ]
    );
    assert!(!crate::window::library_host::root_expanded_of(
        hwnd, ab.b_id
    ));
    assert!(crate::window::library_host::root_expanded_of(hwnd, ab.a_id));

    // B closed and read again from disk keeps its root collapsed.
    assert!(crate::window::library_host::close_notebook_of(
        hwnd, ab.b_id
    ));
    let b = open_and_load_folder(&ab.window, &ab.b);
    crate::window::notebook_view::rebuild(hwnd);
    assert!(!crate::window::library_host::root_expanded_of(hwnd, b));
    assert_eq!(view_rows(hwnd).last(), Some(&(b, RowKind::Root)));
}

#[test]
fn an_unloaded_root_shows_one_placeholder_row() {
    // Break caught: a notebook still loading, failed or empty putting the whole view into a
    // state, or Retry reloading every notebook.
    use crate::window::library_host::{generations_of, library_ready, test_ready_payload};
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-placeholder");
    let hwnd = ab.window.hwnd;
    let c = ab.scratch.root.join("C");
    std::fs::create_dir_all(&c).unwrap();
    std::fs::write(c.join("c.md"), "c").unwrap();
    // C is opened but its worker's answer is not pumped: it is still loading.
    crate::window::library_host::open_folder(hwnd, &c);
    let c_id = crate::window::library_host::notebooks(hwnd)
        .into_iter()
        .find(|(id, _)| *id != ab.a_id && *id != ab.b_id)
        .map(|(id, _)| id)
        .unwrap();
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(
        view_rows(hwnd)[4..],
        [
            (c_id, RowKind::Root),
            (c_id, RowKind::Placeholder(Placeholder::Loading))
        ]
    );
    assert_eq!(row_text(hwnd, 5), "Loading…");

    let (generation, _, _) = generations_of(hwnd, c_id);
    library_ready(hwnd, test_ready_payload(generation, Err("boom".to_owned())));
    assert_eq!(
        view_rows(hwnd)[4..],
        [
            (c_id, RowKind::Root),
            (c_id, RowKind::Placeholder(Placeholder::Failed))
        ]
    );
    assert_eq!(row_text(hwnd, 5), "Couldn't load. Retry");

    // Clicking it reloads C, and only C.
    let (a_before, b_before) = (
        generations_of(hwnd, ab.a_id).0,
        generations_of(hwnd, ab.b_id).0,
    );
    let rect = notebook_view(hwnd).row_rect_at(5).unwrap();
    click_rect(notebook_view(hwnd).panel, rect);
    assert!(!crate::window::library_host::load_failed_of(hwnd, c_id));
    assert_ne!(generations_of(hwnd, c_id).0, generation, "C loads again");
    assert_eq!(generations_of(hwnd, ab.a_id).0, a_before);
    assert_eq!(generations_of(hwnd, ab.b_id).0, b_before);
    pump_until(hwnd, || {
        crate::window::library_host::with_notebook(hwnd, c_id, |_| ()).is_some()
    });

    // An empty notebook says so under its root.
    let d = ab.scratch.root.join("D");
    std::fs::create_dir_all(&d).unwrap();
    let d_id = open_and_load_folder(&ab.window, &d);
    crate::window::notebook_view::rebuild(hwnd);
    let rows = view_rows(hwnd);
    assert_eq!(
        rows[rows.len() - 2..],
        [
            (d_id, RowKind::Root),
            (d_id, RowKind::Placeholder(Placeholder::Empty))
        ]
    );
    assert_eq!(row_text(hwnd, rows.len() - 1), "No notes yet");
}

#[test]
fn the_default_root_shows_its_marker() {
    // Break caught: no way to tell which notebook capture and the single-notebook commands
    // go to, or the marker on every root.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-marker");
    let hwnd = ab.window.hwnd;
    let label = |id| crate::window::notebook_view::root_label(hwnd, id).unwrap();
    assert_eq!(label(ab.a_id).marker, Some("Default notebook"));
    assert_eq!(label(ab.b_id).marker, None);
    assert_eq!(label(ab.b_id).name, "B");

    crate::window::library_host::set_default_notebook(hwnd, ab.b_id);
    assert_eq!(
        label(ab.a_id).marker,
        None,
        "the marker follows the default"
    );
    assert_eq!(label(ab.b_id).marker, Some("Default notebook"));
}

#[test]
fn the_root_menu_offers_close_default_and_favorite() {
    // Break caught: a root's menu acting on the default notebook, offering to make the default
    // the default, or missing the notebook's own commands.
    use crate::window::notebook_view::{ROOT_MENU_SET_DEFAULT, header_clicked};
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-menu");
    let hwnd = ab.window.hwnd;
    let labels = || -> Vec<String> {
        crate::window::menus::last_popup_entries()
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    };
    let has = |labels: &[String], wanted: &str| labels.iter().any(|label| label == wanted);

    answer_next_popup_menu(|_| None);
    header_clicked(hwnd, ab.b_id, HeaderButton::More);
    let on_b = labels();
    for wanted in [
        "Close notebook",
        "Set as default notebook",
        "Add to Favorites",
        "New note from template\u{2026}",
    ] {
        assert!(has(&on_b, wanted), "{wanted:?} in {on_b:?}");
    }

    answer_next_popup_menu(|_| None);
    header_clicked(hwnd, ab.a_id, HeaderButton::More);
    let on_a = labels();
    assert!(has(&on_a, "Close notebook"), "{on_a:?}");
    assert!(!has(&on_a, "Set as default notebook"), "{on_a:?}");

    answer_next_popup_menu(|_| Some(ROOT_MENU_SET_DEFAULT));
    header_clicked(hwnd, ab.b_id, HeaderButton::More);
    assert_eq!(
        crate::window::library_host::default_notebook(hwnd),
        Some(ab.b_id)
    );
    let ini = crate::library::local::folders_file(&ab.scratch.data());
    let text = std::fs::read_to_string(ini).unwrap();
    assert!(
        text.contains(&format!("default={}\r\n", ab.b.display())),
        "{text}"
    );
}

#[test]
fn two_roots_with_the_same_name_show_their_parent_hint() {
    // Break caught: two notebooks both called "Notes" telling nothing apart in the tree.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("roots-hint");
    let (x, y) = (scratch.root.join(r"X\Notes"), scratch.root.join(r"Y\Notes"));
    std::fs::create_dir_all(&x).unwrap();
    std::fs::create_dir_all(&y).unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    ensure_sidebar(window.hwnd);
    let x_id = open_and_load_folder(&window, &x);
    let y_id = open_and_load_folder(&window, &y);
    crate::window::notebook_view::rebuild(window.hwnd);
    let label = |id| crate::window::notebook_view::root_label(window.hwnd, id).unwrap();
    assert_eq!(
        (label(x_id).name, label(x_id).hint),
        ("Notes".to_owned(), Some("X".to_owned()))
    );
    assert_eq!(label(y_id).hint, Some("Y".to_owned()));
}

#[test]
fn root_buttons_act_on_their_own_root() {
    // Break caught: B's New note naming a note in A, the default notebook.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-buttons");
    let hwnd = ab.window.hwnd;
    let panel = notebook_view(hwnd).panel;
    // The buttons show on the hovered row.
    let root_b = row_in(hwnd, ab.b_id, &RowKind::Root);
    let row = notebook_view(hwnd).row_rect_at(root_b).unwrap();
    mouse(panel, WM_MOUSEMOVE, 0, centre(row));
    let button = notebook_view(hwnd)
        .root_button_rect(ab.b_id, HeaderButton::NewNote)
        .expect("B's New note shows on its hovered row");
    mouse(panel, WM_MOUSEMOVE, 0, centre(button));
    click_rect(panel, button);

    assert!(inline_open(hwnd));
    assert_eq!(
        crate::window::inline_name::purpose(hwnd),
        Some(crate::window::inline_name::Purpose::NewNote(
            std::path::PathBuf::new()
        ))
    );
    assert_eq!(
        crate::window::inline_name::edit_root(hwnd).as_deref(),
        Some(ab.b.as_path())
    );
    let root_b = row_in(hwnd, ab.b_id, &RowKind::Root);
    assert_eq!(
        row_in(hwnd, ab.b_id, &RowKind::Draft),
        root_b + 1,
        "first under B's root"
    );
}

#[test]
fn with_no_notebook_open_the_view_shows_the_open_a_notebook_panel() {
    // Break caught: root rows taking the place of the no-notebook panel and its buttons.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("roots-none");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    ensure_sidebar(window.hwnd);
    crate::window::notebook_view::rebuild(window.hwnd);
    let view = notebook_view(window.hwnd);
    assert_eq!(view.mode, Mode::NoNotebook);
    assert!(view.rows.is_empty());
    let panel = view.panel;
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let names: Vec<String> = view
        .buttons(client, dpi)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["Open notebook…", "New notebook…"]);
}

#[test]
fn with_the_notebook_view_focused_and_a_b_row_selected_the_target_is_b() {
    // Break caught: palette commands acting on the default notebook while a row of another
    // notebook is selected in the tree (multi-root notebooks spec §5.1).
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-target");
    let hwnd = ab.window.hwnd;
    let panel = notebook_view(hwnd).panel;
    let b_row = row_in(hwnd, ab.b_id, &RowKind::Note("b.md".into()));
    notebook_view(hwnd).list.selected = Some(b_row);
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(panel) };
    assert_eq!(
        crate::window::library_host::target_notebook(hwnd),
        Some(ab.b_id)
    );
    let a_root = row_in(hwnd, ab.a_id, &RowKind::Root);
    notebook_view(hwnd).list.selected = Some(a_root);
    assert_eq!(
        crate::window::library_host::target_notebook(hwnd),
        Some(ab.a_id),
        "a root row is its notebook's"
    );
}

#[test]
fn a_refresh_while_search_is_hidden_keeps_its_query() {
    // Break caught: a hidden Search view retargeted by a stale tree row on a refresh, its
    // query cleared although the notebook it searches did not change.
    let _scintilla = load_native_scintilla();
    let ab = notebooks_a_and_b("roots-search-hidden", "alpha needle", "beta needle");
    let hwnd = ab.window.hwnd;
    super::super::open_path(hwnd, &ab.b.join("b.md")).unwrap();
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true);
    search_for(hwnd, "needle");
    assert_eq!(
        crate::window::search_view::shown_header(hwnd),
        "Searching B"
    );

    // A stale A row selected while the Notebook view shows, then a refresh.
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, false);
    let a_row = row_in(hwnd, ab.a_id, &RowKind::Note("a.md".into()));
    notebook_view(hwnd).list.selected = Some(a_row);
    crate::window::side_panel::refresh(hwnd);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, false);

    assert_eq!(
        crate::window::search_view::current_query(hwnd).map(|(text, _)| text),
        Some("needle".to_owned())
    );
    assert_eq!(
        crate::window::search_view::shown_header(hwnd),
        "Searching B"
    );
}

/// The notebooks A and B, each with one note tagged `#x`, indexed, with the Tags view shown.
fn tagged_ab(label: &str) -> NotebooksAB {
    let ab = notebooks_a_and_b(label, "a #x\n", "b #x\n");
    let hwnd = ab.window.hwnd;
    let tagged = |id, path: &str| {
        crate::window::library_host::note_index_of(hwnd, id)
            .facts(std::path::Path::new(path))
            .is_some_and(|facts| !facts.tags.is_empty())
    };
    pump_until(hwnd, || tagged(ab.a_id, "a.md") && tagged(ab.b_id, "b.md"));
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Tags, false);
    crate::window::side_panel::refresh(hwnd);
    ab
}

#[test]
fn tags_merge_every_notebook() {
    // Break caught: Tags listing only the target notebook (multi-root search spec §6), a shared
    // tag listed once per notebook, note rows that don't say their notebook, or a note opened
    // under the wrong folder.
    use crate::window::tags_view::{self, TagAction, TagRowKind};
    let _scintilla = load_native_scintilla();
    let ab = tagged_ab("roots-tags-merged");
    let hwnd = ab.window.hwnd;
    assert_eq!(tags_view::shown_header(hwnd), "Tags in all notebooks");
    let rows = tags_view::shown_rows(hwnd);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0].name.as_str(), rows[0].count), ("#x", 2));

    tags_view::test_run(hwnd, TagAction::Toggle("x".to_owned()));
    let rows = tags_view::shown_rows(hwnd);
    let notes: Vec<_> = rows
        .iter()
        .filter(|row| matches!(row.kind, TagRowKind::Note(_)))
        .map(|row| (row.name.as_str(), row.detail.as_str()))
        .collect();
    assert_eq!(notes, [("a.md", "A"), ("b.md", "B")]);
    let TagRowKind::Note(b_note) = rows[2].kind.clone() else {
        panic!("{rows:?}");
    };
    tags_view::test_run(
        hwnd,
        TagAction::Open {
            note: b_note,
            keep: true,
        },
    );
    assert_eq!(active_path(hwnd), Some(ab.b.join("b.md")));
}

#[test]
fn tags_refresh_when_a_notebook_closes() {
    // Break caught: a closed notebook's notes still counted under its tags.
    let _scintilla = load_native_scintilla();
    let ab = tagged_ab("roots-tags-close");
    let hwnd = ab.window.hwnd;
    assert!(crate::window::library_host::close_notebook_of(
        hwnd, ab.b_id
    ));
    let rows = crate::window::tags_view::shown_rows(hwnd);
    assert_eq!((rows[0].name.as_str(), rows[0].count), ("#x", 1));
    assert_eq!(crate::window::tags_view::shown_header(hwnd), "Tags in A");
}

#[test]
fn a_new_note_from_a_template_is_named_under_the_target_root() {
    // Break caught: B's templates listed while the note is named and made in A, the default.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-template");
    let hwnd = ab.window.hwnd;
    std::fs::create_dir_all(ab.b.join(r".fastpad\templates")).unwrap();
    std::fs::write(ab.b.join(r".fastpad\templates\Idea.md"), "# {{title}}\n").unwrap();
    super::super::open_path(hwnd, &ab.b.join("b.md")).unwrap();

    execute_command(hwnd, CommandId::NotesNewFromTemplate);
    crate::window::library_host::picked(
        hwnd,
        crate::window::command_palette::PickerKind::Template,
        crate::window::command_palette::PickerChoice::Item(0),
    );
    assert_eq!(
        crate::window::inline_name::edit_root(hwnd).as_deref(),
        Some(ab.b.as_path())
    );
    let root_b = row_in(hwnd, ab.b_id, &RowKind::Root);
    assert_eq!(
        row_in(hwnd, ab.b_id, &RowKind::Draft),
        root_b + 1,
        "named inline under B's root"
    );
    type_into_field(hwnd, "Spark");
    field_key(hwnd, VK_RETURN);

    assert_eq!(
        std::fs::read_to_string(ab.b.join("Spark.md")).unwrap(),
        "# Spark\n"
    );
    assert!(!ab.a.join("Spark.md").exists());
}

/// Selects row `index` of the view's list.
fn select_index(hwnd: HWND, index: usize) {
    notebook_view(hwnd).list.selected = Some(index);
}

fn key(hwnd: HWND, key: u16) {
    crate::window::notebook_view::key_down(hwnd, key);
}

#[test]
fn arrow_keys_collapse_expand_and_enter_a_root() {
    // Break caught: Left moving off a root, Right not moving into the first child, or Enter
    // not toggling (multi-root notebooks spec §4.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LEFT, VK_RETURN, VK_RIGHT};
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-keys");
    let hwnd = ab.window.hwnd;
    let root_b = (ab.b_id, RowKind::Root);
    let note_b = (ab.b_id, RowKind::Note("b.md".into()));
    select_index(hwnd, row_in(hwnd, ab.b_id, &RowKind::Root));

    key(hwnd, VK_LEFT);
    assert!(!view_rows(hwnd).contains(&note_b), "Left collapses B");
    assert_eq!(selected_row_of(hwnd), Some(root_b.clone()));
    key(hwnd, VK_LEFT);
    assert_eq!(
        selected_row_of(hwnd),
        Some(root_b.clone()),
        "stays when collapsed"
    );
    assert!(!view_rows(hwnd).contains(&note_b));

    key(hwnd, VK_RIGHT);
    assert!(view_rows(hwnd).contains(&note_b), "Right expands B");
    assert_eq!(
        selected_row_of(hwnd),
        Some(note_b.clone()),
        "and moves to its first child"
    );

    select_index(hwnd, row_in(hwnd, ab.b_id, &RowKind::Root));
    key(hwnd, VK_RIGHT);
    assert_eq!(
        selected_row_of(hwnd),
        Some(note_b.clone()),
        "an expanded root moves in"
    );

    select_index(hwnd, row_in(hwnd, ab.b_id, &RowKind::Root));
    key(hwnd, VK_RETURN);
    assert!(
        !view_rows(hwnd).contains(&note_b),
        "Enter collapses an expanded root"
    );
    assert_eq!(selected_row_of(hwnd), Some(root_b.clone()));
    key(hwnd, VK_RETURN);
    assert!(
        view_rows(hwnd).contains(&note_b),
        "Enter expands a collapsed root"
    );
    assert_eq!(selected_row_of(hwnd), Some(root_b));
}

#[test]
fn left_from_a_top_level_note_moves_to_its_root() {
    // Break caught: Left on a depth-1 row doing nothing or landing on the previous notebook.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_LEFT;
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-left");
    let hwnd = ab.window.hwnd;
    select_index(hwnd, row_in(hwnd, ab.b_id, &RowKind::Note("b.md".into())));
    key(hwnd, VK_LEFT);
    assert_eq!(selected_row_of(hwnd), Some((ab.b_id, RowKind::Root)));
}

#[test]
fn f2_and_delete_on_a_root_do_nothing() {
    // Break caught: a root renamed or deleted from the tree. No answer is queued, so a
    // Recycle Bin question would panic.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_F2};
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-f2-del");
    let hwnd = ab.window.hwnd;
    let before = view_rows(hwnd);
    select_index(hwnd, row_in(hwnd, ab.b_id, &RowKind::Root));
    key(hwnd, VK_F2);
    key(hwnd, VK_DELETE);
    assert!(!inline_open(hwnd), "no name field");
    assert!(!name_box_visible(hwnd), "no name box");
    assert_eq!(view_rows(hwnd), before);
    assert!(ab.b.join("b.md").exists() && ab.b.exists());
}

fn type_char(hwnd: HWND, ch: char) {
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;
    let panel = notebook_view(hwnd).panel;
    unsafe { SendMessageW(panel, WM_CHAR, ch as usize, 0) };
}

#[test]
fn type_ahead_matches_a_root_name() {
    // Break caught: type-ahead skipping root rows and landing on the note b.md below them.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-typeahead");
    let hwnd = ab.window.hwnd;
    select_index(hwnd, 0);
    type_char(hwnd, 'b');
    assert_eq!(selected_row_of(hwnd), Some((ab.b_id, RowKind::Root)));
}

#[test]
fn type_ahead_skips_placeholder_rows() {
    // Break caught: typing "n" selecting the "No notes yet" row of an empty notebook.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-typeahead-placeholder");
    let hwnd = ab.window.hwnd;
    let d = ab.scratch.root.join("D");
    std::fs::create_dir_all(&d).unwrap();
    open_and_load_folder(&ab.window, &d);
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(row_text(hwnd, view_rows(hwnd).len() - 1), "No notes yet");
    select_index(hwnd, 0);
    type_char(hwnd, 'n');
    assert_eq!(selected_row_of(hwnd), Some((ab.a_id, RowKind::Root)));
}

#[test]
fn closing_the_selected_rows_notebook_selects_a_neighbouring_root() {
    // Break caught: the selection clamping onto whatever row took its index, which can sit
    // in another notebook and silently change the target notebook.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-close-selection");
    let hwnd = ab.window.hwnd;
    let c = ab.scratch.root.join("C");
    std::fs::create_dir_all(&c).unwrap();
    std::fs::write(c.join("c.md"), "c").unwrap();
    let c_id = open_and_load_folder(&ab.window, &c);
    crate::window::notebook_view::rebuild(hwnd);

    // The next notebook's root row when one follows.
    select_index(hwnd, row_in(hwnd, ab.b_id, &RowKind::Note("b.md".into())));
    assert!(crate::window::library_host::close_notebook_of(
        hwnd, ab.b_id
    ));
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(selected_row_of(hwnd), Some((c_id, RowKind::Root)));

    // The previous one when it was last.
    select_index(hwnd, row_in(hwnd, c_id, &RowKind::Note("c.md".into())));
    assert!(crate::window::library_host::close_notebook_of(hwnd, c_id));
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(selected_row_of(hwnd), Some((ab.a_id, RowKind::Root)));

    // None when no notebook remains.
    assert!(crate::window::library_host::close_notebook_of(
        hwnd, ab.a_id
    ));
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(notebook_view(hwnd).list.selected, None);
}

#[test]
fn a_rebuild_under_a_still_pointer_does_not_leave_invisible_buttons_clickable() {
    // Break caught: a stale hover after rows shift, so a click at a button position on a row
    // that draws no buttons presses that invisible button.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_LEFT;
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-stale-hover");
    let hwnd = ab.window.hwnd;
    let panel = notebook_view(hwnd).panel;
    let row = notebook_view(hwnd)
        .row_rect_at(row_in(hwnd, ab.b_id, &RowKind::Root))
        .unwrap();
    mouse(panel, WM_MOUSEMOVE, 0, centre(row));
    assert!(notebook_view(hwnd).list.hover.is_some());

    // Collapsing A moves B's root up a row, under the unmoved pointer.
    select_index(hwnd, 0);
    key(hwnd, VK_LEFT);
    assert_eq!(
        notebook_view(hwnd).list.hover,
        None,
        "rows moved: hover reset"
    );

    let button = notebook_view(hwnd)
        .root_parts_of(ab.b_id)
        .unwrap()
        .shown()
        .find(|(button, _)| *button == HeaderButton::NewNote)
        .map(|(_, rect)| rect)
        .unwrap();
    click_rect(panel, button);
    assert!(
        !inline_open(hwnd),
        "B's buttons are not drawn, so none is pressed"
    );
}

#[test]
fn each_root_is_a_top_level_automation_item() {
    // Break caught: the default root not named "(default)", B's rows not under B, or a
    // placeholder exposed as an outline item.
    use windows_sys::Win32::UI::Accessibility::{ROLE_SYSTEM_OUTLINEITEM, ROLE_SYSTEM_STATICTEXT};
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-uia");
    let hwnd = ab.window.hwnd;
    let d = ab.scratch.root.join("D");
    std::fs::create_dir_all(&d).unwrap();
    open_and_load_folder(&ab.window, &d);
    crate::window::notebook_view::rebuild(hwnd);
    let panel = notebook_view(hwnd).panel;
    let count = crate::window::side_panel::accessible_item_count(panel);
    let items: Vec<_> = (0..count)
        .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
        .collect();
    let first = items
        .iter()
        .position(|item| item.name.starts_with("A"))
        .expect("A's root item");
    let tree: Vec<_> = items[first..]
        .iter()
        .filter(|item| {
            item.role == ROLE_SYSTEM_OUTLINEITEM
                || (item.role == ROLE_SYSTEM_STATICTEXT && item.name == "No notes yet")
        })
        .map(|item| (item.name.as_str(), item.value.as_str(), item.role))
        .collect();
    assert_eq!(
        tree,
        [
            ("A (default)", "1", ROLE_SYSTEM_OUTLINEITEM),
            ("a.md, Markdown", "2", ROLE_SYSTEM_OUTLINEITEM),
            ("B", "1", ROLE_SYSTEM_OUTLINEITEM),
            ("b.md, Markdown", "2", ROLE_SYSTEM_OUTLINEITEM),
            ("D", "1", ROLE_SYSTEM_OUTLINEITEM),
            ("No notes yet", "", ROLE_SYSTEM_STATICTEXT),
        ],
        "{items:?}"
    );
}

#[test]
fn deleting_a_notebooks_last_row_keeps_the_selection_in_that_notebook() {
    // Break caught: the row that takes a deleted last row's place being the next notebook's
    // root, so Ctrl+N, New folder or Today's note silently go to the other notebook.
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-delete-last");
    let hwnd = ab.window.hwnd;
    std::fs::create_dir_all(ab.a.join("zfolder")).unwrap();
    std::fs::write(ab.a.join("zfolder").join("z.md"), "z").unwrap();
    rescan_and_wait(hwnd);
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(
        view_rows(hwnd)[..3],
        [
            (ab.a_id, RowKind::Root),
            (ab.a_id, RowKind::Folder("zfolder".into())),
            (ab.a_id, RowKind::Note("a.md".into())),
        ]
    );

    // A note that was the notebook's last row.
    select_index(hwnd, row_in(hwnd, ab.a_id, &RowKind::Note("a.md".into())));
    crate::window::answer_next_confirm(|_| true);
    crate::window::library_host::delete_file(hwnd, &ab.a.join("a.md"));
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(selected_row_of(hwnd).map(|(id, _)| id), Some(ab.a_id));
    assert_eq!(
        crate::window::library_host::target_notebook(hwnd),
        Some(ab.a_id)
    );

    // A folder that was the notebook's last row.
    select_index(
        hwnd,
        row_in(hwnd, ab.a_id, &RowKind::Folder("zfolder".into())),
    );
    crate::window::answer_next_confirm(|_| true);
    crate::window::library_host::delete_folder_of(hwnd, ab.a_id, std::path::Path::new("zfolder"));
    crate::window::notebook_view::rebuild(hwnd);
    assert_eq!(selected_row_of(hwnd), Some((ab.a_id, RowKind::Root)));
    assert_eq!(
        crate::window::library_host::target_notebook(hwnd),
        Some(ab.a_id)
    );
}

#[test]
fn the_default_marker_shows_its_own_tooltip() {
    // Break caught: the notebook's folder tip, added first over the whole name, covering the
    // marker so "Default notebook" never shows.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let ab = roots_ab("roots-marker-tip");
    let hwnd = ab.window.hwnd;
    let panel = notebook_view(hwnd).panel;
    let root_a = row_in(hwnd, ab.a_id, &RowKind::Root);
    let row = notebook_view(hwnd).row_rect_at(root_a).unwrap();
    mouse(panel, WM_MOUSEMOVE, 0, centre(row));
    let marker = crate::window::notebook_view::marker_tool_rect(hwnd)
        .expect("the default's hovered root row has a marker tool");
    let (x, y) = (
        (marker.left + marker.right) / 2,
        (marker.top + marker.bottom) / 2,
    );
    assert_eq!(
        crate::window::notebook_view::tooltip_at(hwnd, x, y).as_deref(),
        Some("Default notebook")
    );
    assert_eq!(
        crate::window::notebook_view::tooltip_at(hwnd, marker.left - 2, y).as_deref(),
        Some(ab.a.display().to_string().as_str()),
        "the name still shows the folder"
    );
}
