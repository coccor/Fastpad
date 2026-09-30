//! The Notebook tree: rows, loading, favorites, moves and note commands.

use super::*;

#[test]
fn a_rescan_keeps_selection_and_expansion_by_path() {
    // Break caught: a rescan that rebuilds the rows and keeps the selected index, so the
    // highlight jumps to another note; one that collapses the folder the user had open; or a
    // vanished selection left pointing past the end of the list.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rescan-selection");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "b");
    scratch.note("c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), true);
    crate::window::notebook_view::rebuild(window.hwnd);
    let b = RowKind::Note(r"sub\b.md".into());
    select_row(window.hwnd, &b);

    scratch.note(r"sub\a.md", "a");
    rescan_and_wait(window.hwnd);
    assert_eq!(
        selected_kind(window.hwnd),
        Some(b.clone()),
        "followed by path"
    );
    let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
    assert!(notebook_view(window.hwnd).rows[sub].expanded);
    assert!(row_of(window.hwnd, &RowKind::Note(r"sub\a.md".into())) < row_of(window.hwnd, &b));

    let before = notebook_view(window.hwnd).list.selected.unwrap();
    std::fs::remove_file(scratch.folder().join(r"sub\b.md")).unwrap();
    rescan_and_wait(window.hwnd);
    let view = notebook_view(window.hwnd);
    let after = view
        .list
        .selected
        .expect("the selection moves, it does not vanish");
    assert!(after < view.rows.len());
    assert_eq!(after, before.min(view.rows.len() - 1));
    assert!(view.rows[sub].expanded);
}

#[test]
fn enter_on_a_note_row_opens_a_normal_tab_and_promotes_the_preview() {
    // Break caught: Enter opening the italic preview tab, so a keyboard user has no way to
    // keep a note open short of Ctrl+Enter, and a second Enter on the preview does nothing.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-enter");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, editor) = notebook_window(&scratch);
    let (_, panel) = sidebar_windows(window.hwnd);

    select_row(window.hwnd, &RowKind::Note("a.md".into()));
    unsafe { SetFocus(panel) };
    crate::window::notebook_view::key_down(window.hwnd, VK_RETURN);
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(a.as_path()));
    assert!(!active.preview, "Enter opens a normal tab");
    assert_eq!(
        unsafe { GetFocus() },
        editor.hwnd(),
        "and moves to the editor"
    );

    // A note already in the preview tab becomes a normal tab on Enter.
    let row = row_of(window.hwnd, &RowKind::Note("b.md".into()));
    crate::window::notebook_view::activate(window.hwnd, row, Activation::Click);
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
    select_row(window.hwnd, &RowKind::Note("b.md".into()));
    unsafe { SetFocus(panel) };
    crate::window::notebook_view::key_down(window.hwnd, VK_RETURN);
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(b.as_path()));
    assert!(!active.preview);
    assert_eq!(super::super::tab_count(window.hwnd), 2);
}

#[test]
fn clicking_a_note_row_opens_the_preview_and_a_double_click_keeps_it() {
    // Break caught: a click opening a normal tab every time (tabs pile up), or a double-click
    // opening a second tab instead of keeping the preview, or a click moving the keyboard to
    // the editor so F2 and Del no longer reach the row just clicked.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-click");
    let a = scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    let row = row_of(window.hwnd, &RowKind::Note("a.md".into()));
    let (_, panel) = sidebar_windows(window.hwnd);
    unsafe { SetFocus(panel) };

    crate::window::notebook_view::activate(window.hwnd, row, Activation::Click);
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(a.as_path()));
    assert!(active.preview);
    assert_eq!(
        unsafe { GetFocus() },
        panel,
        "a click keeps focus in the tree"
    );

    let row = row_of(window.hwnd, &RowKind::Note("a.md".into()));
    crate::window::notebook_view::activate(window.hwnd, row, Activation::Permanent);
    assert_eq!(super::super::tab_count(window.hwnd), 1);
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
}

#[test]
fn switching_to_a_note_in_a_subfolder_selects_its_row_and_expands_its_folders() {
    // Break caught: the tree not following the active tab, or following it into a collapsed
    // folder so the selected row is hidden, or forgetting that expansion at the next start.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-reveal");
    std::fs::create_dir_all(scratch.folder().join(r"sub\deep")).unwrap();
    let b = scratch.note(r"sub\deep\b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);

    super::super::open_path(window.hwnd, &b).unwrap();
    crate::window::side_panel::active_tab_changed(window.hwnd);

    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"sub\deep\b.md".into()))
    );
    let expanded = crate::window::library_host::expanded(window.hwnd);
    assert!(expanded.contains(&std::path::PathBuf::from("sub")));
    assert!(expanded.contains(&std::path::PathBuf::from(r"sub\deep")));
    let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
    let written = crate::library::local::read(&local, &scratch.folder());
    assert!(
        written
            .expanded
            .contains(&std::path::PathBuf::from(r"sub\deep"))
    );
}

#[test]
fn right_expands_a_folder_then_enters_it_and_left_climbs_back_out() {
    // Break caught: arrow keys that only move up and down, so a folder cannot be opened from
    // the keyboard, or Left on a child that does nothing.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LEFT, VK_RIGHT};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-keys");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    scratch.note("z.md", "z");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let sub = RowKind::Folder("sub".into());
    select_row(window.hwnd, &sub);
    let key = |key| crate::window::notebook_view::key_down(window.hwnd, key);

    assert!(key(VK_RIGHT));
    assert!(notebook_view(window.hwnd).rows[row_of(window.hwnd, &sub)].expanded);
    assert!(key(VK_RIGHT));
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"sub\a.md".into()))
    );
    assert!(key(VK_LEFT));
    assert_eq!(selected_kind(window.hwnd), Some(sub.clone()));
    assert!(key(VK_LEFT));
    assert!(!notebook_view(window.hwnd).rows[row_of(window.hwnd, &sub)].expanded);
}

#[test]
fn the_view_says_loading_then_shows_the_tree_and_recent_notebooks_once_closed() {
    // Break caught: an empty panel while the worker loads, a tree left on screen after Close
    // notebook, or a no-notebook state without the RECENT list.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-states");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &crate::library::local::RecentFolders {
            folders: vec![scratch.folder()],
            ..Default::default()
        },
    )
    .unwrap();

    app_mut(window.hwnd).library.folder = Some(scratch.folder());
    crate::window::notebook_view::rebuild(window.hwnd);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Loading);

    scratch.install(window.hwnd);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);

    execute_command(window.hwnd, CommandId::CloseNotebook);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::NoNotebook);
    assert_eq!(notebook_view(window.hwnd).recent, vec![scratch.folder()]);
}

#[test]
fn a_failed_load_offers_retry_and_open_instead_of_loading_forever() {
    // Break caught: a notebook whose load failed showing "Loading…" in both views for good,
    // with no way to try again but reopening it.
    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-failed");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    app_mut(window.hwnd).library.folder = Some(scratch.folder());
    let generation = app_mut(window.hwnd).library.generation;
    crate::window::library_host::library_ready(
        window.hwnd,
        crate::window::library_host::test_ready_payload(generation, Err("boom".to_owned())),
    );
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Failed);
    let panel = notebook_view(window.hwnd).panel;
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let buttons = notebook_view(window.hwnd).buttons(client, dpi);
    let names: Vec<&str> = buttons.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        names.ends_with(&["Retry", "Open notebook…"]),
        "exposed to screen readers like the empty-state buttons: {names:?}"
    );
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    assert_eq!(
        crate::window::search_view::status(window.hwnd),
        Some(crate::window::notebook_view::LOAD_FAILED)
    );

    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    let retry = buttons[buttons.len() - 2].1;
    let lparam = ((((retry.top + retry.bottom) / 2) as u32) << 16
        | ((retry.left + retry.right) / 2) as u32) as LPARAM;
    unsafe {
        SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
        SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
    }
    assert_eq!(
        notebook_view(window.hwnd).mode,
        Mode::Loading,
        "Retry clears the failure and loads the same notebook again"
    );
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);
}

#[test]
fn at_startup_both_views_say_loading_for_the_notebook_being_opened() {
    // Break caught: the Search view saying "Open a notebook to search it." while the
    // remembered notebook loads.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("startup-loading");
    scratch.note("a.md", "a");
    write_notebooks(&scratch.data(), vec![scratch.folder()], vec![]);
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    assert_eq!(
        crate::window::search_view::status(window.hwnd),
        Some(crate::window::search_view::NO_NOTEBOOK)
    );

    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(
        crate::window::search_view::status(window.hwnd),
        Some(crate::window::search_view::LOADING)
    );
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Loading);
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    assert_eq!(crate::window::search_view::status(window.hwnd), None);
}

#[test]
fn an_empty_notebook_stays_empty_with_untitled_tabs_open() {
    // Break caught: untitled tabs still listed in the tree as well as in Open Editors, or an
    // empty notebook's state hidden by them (open editors spec §3.4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("empty-untitled");
    let (window, _editor) = notebook_window(&scratch);
    execute_command(window.hwnd, CommandId::New);
    crate::window::notebook_view::rebuild(window.hwnd);
    let view = notebook_view(window.hwnd);
    assert_eq!(view.mode, crate::window::notebook_view::Mode::Empty);
    assert!(view.rows.is_empty());
}

#[test]
fn the_header_star_favorites_the_notebook_and_every_state_paints() {
    // Break caught: a star that does nothing, or a paint path that panics on an empty tree,
    // the loading state or the no-notebook state.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-star");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);

    crate::window::notebook_view::header_clicked(
        window.hwnd,
        crate::window::notebook_view::HeaderButton::Favorite,
    );
    assert!(crate::window::library_host::is_favorite(window.hwnd));

    let panel = notebook_view(window.hwnd).panel;
    let area = RECT {
        left: 0,
        top: 0,
        right: 260,
        bottom: 400,
    };
    let dc = unsafe { windows_sys::Win32::Graphics::Gdi::GetDC(panel) };
    let paint = |hwnd: HWND| {
        let view_paint = crate::window::side_panel::view_paint(hwnd, panel, dc, area);
        crate::window::notebook_view::paint(hwnd, &view_paint);
    };
    paint(window.hwnd);
    execute_command(window.hwnd, CommandId::CloseNotebook);
    paint(window.hwnd);
    app_mut(window.hwnd).library.folder = Some(scratch.folder());
    crate::window::notebook_view::rebuild(window.hwnd);
    paint(window.hwnd);
    unsafe { windows_sys::Win32::Graphics::Gdi::ReleaseDC(panel, dc) };
}

#[test]
fn switching_tabs_in_an_unchanged_notebook_does_not_reflatten() {
    // Break caught: every tab switch flattening the whole tree again (tens of milliseconds
    // in a big, expanded notebook).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-no-reflatten");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();
    let id_of = |path: &std::path::Path| app_mut(window.hwnd).tabs.find_stored_path(path).unwrap();
    let (a_id, b_id) = (id_of(&a), id_of(&b));
    // The deferred startup steps (theme, chrome) refresh the sidebar once; let them run.
    pump_posted_messages(window.hwnd);
    let before = notebook_view(window.hwnd).rebuilds;

    assert!(super::super::activate_document_by_id(window.hwnd, a_id));
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("a.md".into()))
    );
    assert!(super::super::activate_document_by_id(window.hwnd, b_id));
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("b.md".into()))
    );
    assert_eq!(notebook_view(window.hwnd).rebuilds, before, "no re-flatten");
}

#[test]
fn switching_sidebar_views_saves_the_view_without_restyling_the_editor_or_reflattening() {
    // Break caught: every view switch (and every panel resize) re-applying the editor
    // settings, which lays the Markdown preview out again, or flattening an unchanged tree
    // each time the Notebook view comes back.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-switch-cheap");
    scratch.note("a.md", "a");
    let settings = scratch.root.join("fastpad.ini");
    super::super::save_settings_to(Some(settings.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    pump_posted_messages(window.hwnd);
    let applied = super::super::editor_settings_applied();
    let rebuilds = notebook_view(window.hwnd).rebuilds;
    use crate::config::SidebarView;

    crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
    super::super::save_settings_to(None);

    assert_eq!(super::super::editor_settings_applied(), applied);
    assert_eq!(notebook_view(window.hwnd).rebuilds, rebuilds);
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        SidebarView::Notebook
    );
    let saved = std::fs::read_to_string(&settings).unwrap();
    assert!(saved.contains("sidebar_view=notebook"), "{saved}");
}

#[test]
fn a_notebooks_first_load_selects_the_restored_active_note_and_expands_its_folders() {
    // Break caught: a restored session whose active note sits in a collapsed folder, with
    // nothing selected, until the user switches tabs.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-restore-reveal");
    std::fs::create_dir_all(scratch.folder().join(r"sub\deep")).unwrap();
    let b = scratch.note(r"sub\deep\b.md", "b");
    scratch.note("c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    super::super::open_path(window.hwnd, &b).unwrap();

    scratch.install(window.hwnd);

    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"sub\deep\b.md".into()))
    );
    let expanded = crate::window::library_host::expanded(window.hwnd);
    assert!(expanded.contains(&std::path::PathBuf::from("sub")));
    assert!(expanded.contains(&std::path::PathBuf::from(r"sub\deep")));
}

fn write_notebooks(
    data: &std::path::Path,
    folders: Vec<std::path::PathBuf>,
    favorites: Vec<std::path::PathBuf>,
) {
    crate::library::local::write_folders(
        &crate::library::local::folders_file(data),
        &crate::library::local::RecentFolders {
            folders,
            favorites,
            closed: false,
        },
    )
    .unwrap();
}

#[test]
fn moving_a_note_to_another_notebook_moves_the_file_drops_its_pin_and_its_tab_follows() {
    // Break caught: a move that copies without deleting, a pin record left pointing at a file
    // that left the notebook, or a tab still on the old path, where autosave would recreate it.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-a");
    let second = LibraryScratch::new("move-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    crate::window::library_host::toggle_pin(window.hwnd, &a);
    write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);

    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    let moved = second.folder().join("a.md");
    assert!(!a.exists());
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "a");
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(moved.as_path())
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(!state.is_pinned(&a));
        assert!(state.record_for(&a).is_none());
        assert!(
            !state
                .notes
                .iter()
                .any(|note| note.path == std::path::Path::new("a.md"))
        );
    });
    editor.set_text("b").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible,
        "a plain file outside the notebook now"
    );
}

#[test]
fn a_move_onto_an_existing_name_changes_nothing_and_says_why() {
    // Break caught: MoveFileExW's replace flag, or a fallback copy, overwriting the other
    // notebook's note of the same name.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-clash-a");
    let second = LibraryScratch::new("move-clash-b");
    let theirs = second.note("a.md", "theirs");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "mine");
    write_notebooks(&first.data(), vec![second.folder()], vec![]);

    crate::window::library_host::move_to_notebook(window.hwnd, &a);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    assert_eq!(std::fs::read_to_string(&a).unwrap(), "mine");
    assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "theirs");
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("already exists"))
    );
}

#[test]
fn move_offers_favorites_by_name_then_recent_never_the_open_one_then_browse() {
    // Break caught: the open notebook offered as a destination, a notebook listed twice, or
    // Browse… not reachable.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-list");
    let third = LibraryScratch::new("move-browse");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    let (zeta, alpha, beta) = (
        first.root.join("Zeta"),
        first.root.join("alpha"),
        first.root.join("beta"),
    );
    write_notebooks(
        &first.data(),
        vec![first.folder(), beta.clone(), alpha.clone()],
        vec![zeta.clone(), alpha.clone(), first.folder()],
    );

    crate::window::library_host::move_to_notebook(window.hwnd, &a);
    let (note, destinations) = app_mut(window.hwnd).library.shown_move.clone().unwrap();
    assert_eq!(note, a);
    assert_eq!(destinations, vec![alpha, zeta, beta]);

    crate::window::answer_next_folder_dialog({
        let folder = third.folder();
        move |_| Some(folder)
    });
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(3),
    );
    assert!(third.folder().join("a.md").exists());
}

#[test]
fn a_new_note_saves_into_the_folder_selected_when_it_was_created_or_the_root_if_that_is_gone() {
    // Break caught: Ctrl+N with a subfolder selected saving into the notebook root anyway, or
    // a first save failing because the remembered folder was deleted meanwhile.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-note-folder");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), true);
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Note(r"sub\b.md".into()));

    execute_command(window.hwnd, CommandId::New);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().save_folder,
        Some(scratch.folder().join("sub"))
    );
    editor.set_text("Idea").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(scratch.folder().join(r"sub\Idea.md").exists());

    let gone = scratch.folder().join("gone");
    untitled_tab_saving_in(window.hwnd, gone);
    editor.set_text("Other").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(scratch.folder().join("Other.md").exists());
}

#[test]
fn the_context_menu_acts_on_its_row_not_the_active_tab() {
    // Break caught: Pin from a row's menu pinning the active tab's note instead, or "New
    // note here" ignoring the folder.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("context-menu");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note("b.md", "b");
    scratch.note(r"sub\c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    let a = open_note(&window, &scratch, "a.md", "a");
    let menu = |kind: &RowKind, answer: CommandId| {
        crate::window::menus::answer_next_popup_menu(move |_| Some(answer));
        let index = row_of(window.hwnd, kind);
        crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
    };

    menu(&RowKind::Note("b.md".into()), CommandId::NoteTogglePin);
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_pinned(&scratch.folder().join("b.md")));
        assert!(!state.is_pinned(&a));
    });

    menu(&RowKind::Folder("sub".into()), CommandId::NoteNew);
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::NewNote("sub".into()))
    );
    crate::window::inline_name::cancel(window.hwnd);

    menu(
        &RowKind::Folder("sub".into()),
        CommandId::NoteRevealInExplorer,
    );
    execute_command(window.hwnd, CommandId::NoteRevealInExplorer);
    assert_eq!(
        crate::platform::shell::take_revealed(),
        vec![scratch.folder().join("sub"), a.clone()]
    );
}

#[test]
fn f2_and_rename_on_a_note_row_rename_it_in_the_tree_without_opening_a_tab() {
    // Break caught: F2 opening the note as a tab first, renaming the active tab's note, a
    // prefill that selects the extension, or the renamed row losing the selection and the
    // focus (inline naming spec §3.3, §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_F2, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-note");
    let a = scratch.note("a.md", "a");
    scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    let tabs = super::super::tab_count(window.hwnd);
    select_row(window.hwnd, &RowKind::Note("b.md".into()));

    assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::RenameNote(
            "b.md".into()
        ))
    );
    assert_eq!(field_text(window.hwnd), "b.md");
    assert_eq!(field_selection(window.hwnd), (0, 1), "the stem is selected");
    assert_eq!(super::super::tab_count(window.hwnd), tabs, "no tab opened");
    type_into_field(window.hwnd, "c");
    field_key(window.hwnd, VK_RETURN);

    assert!(!scratch.folder().join("b.md").exists());
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("c.md")).unwrap(),
        "b"
    );
    assert_eq!(super::super::tab_count(window.hwnd), tabs);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note("c.md".into()))
    );
    assert_eq!(unsafe { GetFocus() }, sidebar_windows(window.hwnd).1);

    let index = row_of(window.hwnd, &RowKind::Note("c.md".into()));
    crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteRename));
    crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::RenameNote(
            "c.md".into()
        ))
    );
    assert_eq!(super::super::tab_count(window.hwnd), tabs);
}

#[test]
fn renaming_open_dirty_and_preview_notes_in_the_tree_rebinds_their_tabs() {
    // Break caught: a rename that saves a dirty tab, turns the preview into a normal tab,
    // or leaves either on the old path (spec §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F2, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-tabs");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, editor) = notebook_window(&scratch);
    // Autosave would save `a` the moment `b` opens; the rename must leave it dirty.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &a).unwrap();
    editor.set_text("a, edited").unwrap();
    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();
    let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
    let b_id = app_mut(window.hwnd).tabs.find_stored_path(&b).unwrap();
    let rename = |from: &str, to: &str| {
        select_row(window.hwnd, &RowKind::Note(from.into()));
        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
        type_into_field(window.hwnd, to);
        field_key(window.hwnd, VK_RETURN);
    };

    rename("a.md", "a2");
    rename("b.md", "b2");

    let tabs = &app_mut(window.hwnd).tabs;
    assert_eq!(
        tabs.document(a_id).unwrap().path,
        Some(scratch.folder().join("a2.md"))
    );
    assert!(tabs.document(a_id).unwrap().dirty);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("a2.md")).unwrap(),
        "a",
        "nothing was saved"
    );
    assert_eq!(
        tabs.document(b_id).unwrap().path,
        Some(scratch.folder().join("b2.md"))
    );
    assert_eq!(
        tabs.preview_id(),
        Some(b_id),
        "the preview stays the preview"
    );
    assert_eq!(super::super::tab_count(window.hwnd), 2);
}

#[test]
fn a_case_only_rename_in_the_tree_renames_the_note_and_the_folder() {
    // Break caught: "plan.md" → "Plan.md" or "sub" → "Sub" refused as a clash with itself,
    // or a no-op on NTFS (spec §4.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-case");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note("plan.md", "p");
    let (window, _editor) = notebook_window(&scratch);
    let names = || {
        let mut names: Vec<String> = std::fs::read_dir(scratch.folder())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
            .collect();
        names.sort();
        names
    };

    crate::window::inline_name::rename(window.hwnd, &RowKind::Note("plan.md".into()));
    type_into_field(window.hwnd, "Plan.md");
    assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
    field_key(window.hwnd, VK_RETURN);
    crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
    type_into_field(window.hwnd, "Sub");
    field_key(window.hwnd, VK_RETURN);

    assert_eq!(names(), ["Plan.md", "Sub"]);
    assert!(!inline_open(window.hwnd));
}

#[test]
fn note_rename_from_the_palette_with_the_sidebar_hidden_reveals_the_row_and_edits_it() {
    // Break caught: Note: Rename… on the active tab opening the name bar while the note has
    // a row, or editing a row nobody can see in a hidden sidebar or a collapsed folder
    // (spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-reveal");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    open_note(&window, &scratch, r"sub\a.md", "a");
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), false);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Hidden, false);

    execute_command(window.hwnd, CommandId::NoteRename);

    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        crate::config::SidebarView::Notebook
    );
    assert!(
        crate::window::library_host::expanded(window.hwnd)
            .contains(&std::path::PathBuf::from("sub"))
    );
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"sub\a.md".into()))
    );
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::RenameNote(
            r"sub\a.md".into()
        ))
    );
    assert_eq!(field_text(window.hwnd), "a.md");
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert!(
        !app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.is_visible())
    );
}

#[test]
fn note_rename_on_a_file_outside_the_notebook_uses_the_name_bar() {
    // Break caught: a file with no row revealing nothing and doing nothing, or the tree
    // edited for some other row (spec §3.3).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-outside");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    let outside = scratch.root.join("outside.md");
    std::fs::write(&outside, "o").unwrap();
    super::super::open_path(window.hwnd, &outside).unwrap();
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;

    execute_command(window.hwnd, CommandId::NoteRename);

    assert!(!inline_open(window.hwnd));
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(
        name_box.purpose(),
        Some(&crate::window::name_box::NamePurpose::RenameNote(id))
    );
}

#[test]
fn note_rename_on_a_recorded_row_that_left_the_library_renames_nothing() {
    // Break caught: Note: Rename… on a focused note row that vanished before Enter opening
    // the name bar on the active tab's file, one the user did not choose (spec §3.3).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-gone");
    let active = scratch.note("active.md", "active");
    let row = scratch.note("row.md", "row");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &active).unwrap();
    select_row(window.hwnd, &RowKind::Note("row.md".into()));
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
    crate::window::library_host::with_state(window.hwnd, |state| state.remove_note(&row));
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };

    assert!(!inline_open(window.hwnd));
    assert!(
        !app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.is_visible())
    );
    assert!(active.exists());
    assert!(row.exists());
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(active.as_path())
    );
}

#[test]
fn a_note_rename_whose_tab_cannot_follow_and_cannot_be_undone_stands() {
    // Break caught: a failed undo swallowed, leaving the file renamed on disk while the
    // library still lists the old name and the field claims nothing happened (spec §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-rename-undo-fails");
    let a = scratch.note("a.md", "a");
    let top = scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &top).unwrap();
    // Another tab already names the path `a` would move to, so `a`'s tab cannot follow.
    let top_id = app_mut(window.hwnd).tabs.find_stored_path(&top).unwrap();
    app_mut(window.hwnd).tabs.document_mut(top_id).unwrap().path =
        Some(scratch.folder().join("c.md"));
    crate::window::library_host::fail_next_note_rename_back();

    crate::window::inline_name::rename(window.hwnd, &RowKind::Note("a.md".into()));
    type_into_field(window.hwnd, "c");
    field_key(window.hwnd, VK_RETURN);

    assert!(!inline_open(window.hwnd));
    assert!(!a.exists());
    assert!(scratch.folder().join("c.md").exists());
    assert!(
        app_mut(window.hwnd).tabs.find_stored_path(&a).is_some(),
        "a's tab is left on its old path"
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        let notes: Vec<_> = state.notes.iter().map(|note| note.path.clone()).collect();
        assert!(
            notes.contains(&std::path::PathBuf::from("c.md")),
            "{notes:?}"
        );
        assert!(
            !notes.contains(&std::path::PathBuf::from("a.md")),
            "{notes:?}"
        );
    });
    let expected = "FastPad could not undo renaming \u{201c}a.md\u{201d} to \u{201c}c.md\u{201d}. \u{201c}a.md\u{201d} is still open at its old path.";
    assert!(
        notices(window.hwnd).iter().any(|notice| notice == expected),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn palette_commands_act_on_the_row_focused_when_the_palette_opened() {
    // Break caught: opening the palette moves focus to its query field, so by the time the
    // chosen command runs, a live focus check sees nothing on the panel and falls back to
    // the active tab instead of the row the user actually picked -- wrong for spec §6.3, and
    // dangerous for Delete.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("palette-note-target");
    let active_path = scratch.note("active.md", "active");
    scratch.note("row.md", "row");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &active_path).unwrap();
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Note("row.md".into()));
    let (_, panel) = sidebar_windows(window.hwnd);
    unsafe { SetFocus(panel) };
    assert_eq!(
        unsafe { GetFocus() },
        panel,
        "the panel must hold focus to record the row"
    );

    execute_command(window.hwnd, CommandId::CommandPalette);
    // Opening the palette moved focus to its own query field.
    assert_ne!(unsafe { GetFocus() }, panel);
    let query = app_mut(window.hwnd)
        .command_palette
        .as_ref()
        .unwrap()
        .query_hwnd();
    let typed = crate::platform::wide_null("Toggle pin");
    unsafe { SetWindowTextW(query, typed.as_ptr()) };
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };

    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_pinned(&scratch.folder().join("row.md")));
        assert!(!state.is_pinned(&active_path));
    });
    // The panel had focus when the palette opened, so it gets it back.
    assert_eq!(unsafe { GetFocus() }, panel);
}

#[test]
fn moving_a_note_onto_a_path_already_open_in_another_tab_is_refused() {
    // Break caught: the target file having been deleted on disk lets the clash check through,
    // then MoveFileExW succeeds and rebind_open_tab silently fails because another tab already
    // has that path, leaving that tab pointing at a file that no longer exists anywhere.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-target-open-a");
    let second = LibraryScratch::new("move-target-open-b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    let target = second.note("a.md", "theirs");
    super::super::open_path(window.hwnd, &target).unwrap();
    std::fs::remove_file(&target).unwrap();
    write_notebooks(&first.data(), vec![second.folder()], vec![]);

    crate::window::library_host::move_to_notebook(window.hwnd, &a);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    assert!(a.exists());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
    assert!(
        !target.exists(),
        "nothing was moved, so the deleted file stays deleted"
    );
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("already has"))
    );
}

#[test]
fn note_commands_reach_a_focused_tree_row_even_with_no_tab_open() {
    // Break caught: the needs_document gate returning early for Ctrl+Shift+M and Reveal
    // whenever no tab happens to be open, even though the tree still has a focused row.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("gate-no-tabs");
    let path = scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Note("a.md".into()));
    let (_, panel) = sidebar_windows(window.hwnd);
    unsafe { SetFocus(panel) };
    while super::super::tab_count(window.hwnd) > 0 {
        super::super::close_active_document(window.hwnd);
    }
    assert_eq!(super::super::tab_count(window.hwnd), 0);

    execute_command(window.hwnd, CommandId::NoteRevealInExplorer);

    assert_eq!(crate::platform::shell::take_revealed(), vec![path]);
}

#[test]
fn moving_a_note_with_unsaved_edits_keeps_them_and_writes_only_at_the_new_path() {
    // Break caught: a move losing the tab's unsaved edits, or an autosave after the move
    // recreating the file at the old location instead of writing it to the new one.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-dirty-a");
    let second = LibraryScratch::new("move-dirty-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    editor.set_text("unsaved edit").unwrap();
    write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);

    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    let moved = second.folder().join("a.md");
    assert!(!a.exists());
    assert_eq!(
        std::fs::read_to_string(&moved).unwrap(),
        "unsaved edit",
        "the edits were saved first, so they moved with the file"
    );
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(moved.as_path()));
    assert!(!active.dirty);
    assert_eq!(editor.text().unwrap(), "unsaved edit");

    // A later edit and save must land only at the new path, never re-create the old one.
    editor.set_text("later edit").unwrap();
    crate::window::library_host::save_command(window.hwnd);
    assert!(
        !a.exists(),
        "a save must not recreate the file at the old location"
    );
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "later edit");
}

#[test]
fn a_note_changed_outside_fastpad_is_not_moved_over_its_change() {
    // Break caught: a move saving the tab's edits over a change made outside FastPad (a sync,
    // another editor), which autosave refuses to do.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-changed-a");
    let second = LibraryScratch::new("move-changed-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    editor.set_text("unsaved edit").unwrap();
    write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);
    std::fs::write(&a, "changed elsewhere").unwrap();
    let before = notices(window.hwnd).len();

    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    assert_eq!(std::fs::read_to_string(&a).unwrap(), "changed elsewhere");
    assert!(!second.folder().join("a.md").exists(), "nothing moved");
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(a.as_path()));
    assert!(active.dirty && active.autosave_paused);
    let added = &notices(window.hwnd)[before..];
    assert_eq!(added.len(), 1, "{added:?}");
    assert!(added[0].contains("Nothing was moved"), "{added:?}");

    // Already paused: refused again, without writing.
    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "changed elsewhere");
    assert!(!second.folder().join("a.md").exists());
}

#[test]
fn a_note_whose_edits_cannot_be_saved_is_not_moved() {
    // Break caught: a move going ahead after the save of the tab's edits failed, so the
    // moved file lacks them and the tab's text no longer matches either location.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-save-fails-a");
    let second = LibraryScratch::new("move-save-fails-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    editor.set_text("unsaved edit").unwrap();
    write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);
    // A file held open without sharing makes the save (and the move) fail.
    use std::os::windows::fs::OpenOptionsExt;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&a)
        .unwrap();
    let before = notices(window.hwnd).len();

    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );
    drop(lock);

    assert!(a.exists());
    assert!(!second.folder().join("a.md").exists(), "nothing moved");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(a.as_path()));
    assert!(active.dirty);
    let added = &notices(window.hwnd)[before..];
    assert_eq!(added.len(), 1, "{added:?}");
    assert!(added[0].contains("Nothing was moved"), "{added:?}");
}

#[test]
fn a_palette_rename_and_a_move_turn_the_preview_into_a_normal_tab() {
    // Break caught: a preview tab renamed from the palette, or moved to another notebook,
    // staying the preview, so the next click in the tree replaces it.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("promote-a");
    let second = LibraryScratch::new("promote-b");
    let a = first.note("a.md", "a");
    let b = first.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    first.install(window.hwnd);
    write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);

    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();
    assert!(app_mut(window.hwnd).tabs.preview_id().is_some());
    // The name bar itself: Note: Rename… would edit the note's row in the tree.
    crate::window::library_host::rename_note(window.hwnd);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None, "rename");
    crate::window::library_host::close_name_box(window.hwnd);

    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();
    assert!(app_mut(window.hwnd).tabs.preview_id().is_some());
    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );
    assert!(second.folder().join("b.md").exists());
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None, "move");
}
