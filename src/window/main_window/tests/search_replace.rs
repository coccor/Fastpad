//! The Search view's replace: fields, replace all, row replace and its accessibility.

use super::*;

#[test]
fn the_search_view_finds_note_text_shows_folders_and_enter_opens_a_normal_tab() {
    // Break caught: a search over names instead of text, results without their folder, or
    // Enter opening the preview tab, which a keyboard user cannot then keep.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-view");
    scratch.note("Alpha.md", "the alpha plan");
    scratch.note("beta.md", "nothing here");
    scratch.note("gamma.md", "Alphabet soup");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\notes.md", "  alpha, indented");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    assert_eq!(crate::window::search_view::status(window.hwnd), None);

    search_for(window.hwnd, "alpha");
    assert_eq!(
        search_rows(window.hwnd),
        vec![
            search_row("Alpha", "the alpha plan"),
            search_row("gamma", "Alphabet soup"),
            search_row("notes", "alpha, indented"),
        ]
    );
    let results = &app_mut(window.hwnd)
        .sidebar
        .as_ref()
        .unwrap()
        .search
        .results;
    assert_eq!(results[0].folder, "");
    assert_eq!(results[2].folder, "sub");
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some(("3 notes".to_owned(), false))
    );
    let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    unsafe {
        SendMessageW(
            edit,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            usize::from(windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN),
            0,
        )
    };
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(
        active.path.as_deref(),
        Some(scratch.folder().join("Alpha.md").as_path())
    );
    assert!(!active.preview, "Enter opens a normal tab");
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

    search_for(window.hwnd, "zzz");
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some((crate::window::search_view::NO_MATCH.to_owned(), false))
    );
}

#[test]
fn the_search_query_survives_a_view_switch_but_not_a_notebook_switch() {
    // Break caught: the query lost whenever another view is shown, or kept (with results
    // from the old notebook, or its search still reading) after a different notebook opens.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("search-keep-a");
    first.note("plan.md", "the plan");
    let second = LibraryScratch::new("search-keep-b");
    second.note("other.md", "the plan too");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    first.install(window.hwnd);
    use crate::config::SidebarView;
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
    search_for(window.hwnd, "plan");
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
    assert_eq!(search_rows(window.hwnd).len(), 1);

    crate::window::text_search_host::run_now(window.hwnd);
    let flag = crate::window::text_search_host::cancel_flag(window.hwnd).unwrap();
    second.install(window.hwnd);
    assert!(
        flag.load(Ordering::Relaxed),
        "the notebook change cancelled it"
    );
    assert!(search_rows(window.hwnd).is_empty());
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );
    let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    assert_eq!(
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(edit) },
        0
    );
    // A late batch of the cancelled search shows nothing.
    pump_posted_messages(window.hwnd);
    assert!(search_rows(window.hwnd).is_empty());
}

#[test]
fn a_hidden_search_view_searches_again_only_once_it_shows() {
    // Break caught: every library refresh re-running a query nobody sees, or the results
    // staying stale when the Search view comes back.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-stale");
    scratch.note("plan.md", "plan");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    use crate::config::SidebarView;
    // The box is made when the Search view first shows, not with the sidebar.
    assert!(crate::window::search_view::edit_hwnd(window.hwnd).is_none());
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
    assert!(crate::window::search_view::edit_hwnd(window.hwnd).is_some());
    search_for(window.hwnd, "pl");
    assert_eq!(search_rows(window.hwnd).len(), 1);

    crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
    let added = scratch.note("planning.md", "planning");
    crate::window::library_host::with_state(window.hwnd, |state| state.add_note(&added));
    let before = search_generation(window.hwnd);
    crate::window::side_panel::refresh(window.hwnd);
    // Past the debounce, so a re-run wrongly scheduled would have started.
    pump_past_debounce(window.hwnd);
    assert_eq!(
        search_generation(window.hwnd),
        before,
        "a hidden Search view is not searched again"
    );
    assert_eq!(search_rows(window.hwnd).len(), 1);
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
    wait_for_search(window.hwnd, before);
    assert_eq!(
        search_rows(window.hwnd),
        vec![
            search_row("plan", "plan"),
            search_row("planning", "planning")
        ]
    );
}

#[test]
fn ctrl_shift_f_takes_a_single_line_selection_and_ignores_a_multi_line_one() {
    // Break caught: Ctrl+Shift+F ignoring the selection, pasting a multi-line one into the
    // box, or clearing the box when nothing is selected.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-prefill-selection");
    scratch.note("a.md", "alpha beta");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let query = || crate::window::search_view::current_query(window.hwnd).map(|(query, _)| query);
    editor.populate_clean("alpha beta\r\ngamma").unwrap();

    editor.set_selection(6..10).unwrap();
    execute_command(window.hwnd, CommandId::ShowSearchView);
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        crate::config::SidebarView::Search
    );
    assert_eq!(query().as_deref(), Some("beta"));
    // The prefill searches at once, with no keystroke to start the debounce.
    pump_until(window.hwnd, || {
        crate::window::search_view::shown_results(window.hwnd).len() == 1
    });

    editor.set_selection(6..14).unwrap();
    execute_command(window.hwnd, CommandId::ShowSearchView);
    assert_eq!(query().as_deref(), Some("beta"), "a multi-line selection");

    editor.set_selection(3..3).unwrap();
    execute_command(window.hwnd, CommandId::ShowSearchView);
    assert_eq!(query().as_deref(), Some("beta"), "no selection");
}

#[test]
fn ctrl_shift_h_shows_search_with_the_replace_field_and_takes_a_single_line_selection() {
    // Break caught: Ctrl+Shift+H dead in the running app, Search shown without the replace
    // field, the selection Ctrl+Shift+F takes ignored, or Ctrl+Shift+F closing the field
    // again (spec §11: it leaves the field as it is).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-shortcut");
    scratch.note("a.md", "alpha beta");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    editor.populate_clean("alpha beta\r\ngamma").unwrap();
    editor.set_selection(6..10).unwrap();
    assert!(!crate::window::search_view::replace_open(window.hwnd));

    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = 0x80;
    keys[VK_SHIFT as usize] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let message = MSG {
        hwnd: editor.hwnd(),
        message: WM_KEYDOWN,
        wParam: usize::from(b'H'),
        ..Default::default()
    };
    let translated =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };
    unsafe { SetKeyboardState(original.as_ptr()) };

    assert!(translated, "Ctrl+Shift+H was not translated");
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        crate::config::SidebarView::Search
    );
    assert!(crate::window::search_view::replace_open(window.hwnd));
    assert_eq!(
        crate::window::search_view::current_query(window.hwnd)
            .map(|(query, _)| query)
            .as_deref(),
        Some("beta")
    );
    // The prefill searches at once, as Ctrl+Shift+F's does.
    pump_until(window.hwnd, || {
        crate::window::search_view::shown_results(window.hwnd).len() == 1
    });

    execute_command(window.hwnd, CommandId::ShowSearchView);
    assert!(
        crate::window::search_view::replace_open(window.hwnd),
        "Ctrl+Shift+F leaves the field open"
    );
}

#[test]
fn ctrl_shift_h_focuses_the_replace_field_and_typing_there_runs_no_search() {
    // Break caught: the caret left in the search box, the replace text read by WM_GETTEXT
    // under the App borrow instead of kept, a keystroke in the replace field restarting the
    // search, or Esc and Up in it doing what they do in the box.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_UP};
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-field");
    scratch.note("a.md", "alpha needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);

    execute_command(window.hwnd, CommandId::ReplaceInNotes);
    let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
    let search_box = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    assert!(is_shown(replace));
    assert_eq!(unsafe { GetFocus() }, replace);

    search_for(window.hwnd, "needle");
    let generation = search_generation(window.hwnd);
    type_into_replace(window.hwnd, "pin");
    assert_eq!(crate::window::search_view::replace_text(window.hwnd), "pin");
    pump_past_debounce(window.hwnd);
    assert_eq!(
        search_generation(window.hwnd),
        generation,
        "typing a replacement runs no search"
    );
    assert_eq!(search_rows(window.hwnd).len(), 1);

    unsafe { SendMessageW(replace, WM_KEYDOWN, VK_UP as usize, 0) };
    assert_eq!(unsafe { GetFocus() }, search_box, "Up goes to the box");
    unsafe { SendMessageW(replace, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert_eq!(
        crate::window::search_view::replace_text(window.hwnd),
        "",
        "Esc clears the field"
    );
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(replace) };
    unsafe { SendMessageW(replace, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert_eq!(
        unsafe { GetFocus() },
        editor.hwnd(),
        "Esc in the empty field returns to the editor"
    );
}

#[test]
fn the_chevron_opens_and_closes_the_replace_field() {
    // Break caught: a chevron that does nothing, a replace field made before the user asks
    // for it, one left showing (or holding the caret) once closed, or the results kept under
    // the replace row.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-chevron");
    scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "needle");
    assert!(
        crate::window::search_view::replace_edit_hwnd(window.hwnd).is_none(),
        "made the first time it opens"
    );
    let panel = sidebar_panel(window.hwnd);
    let (width, height) = client_size(panel);
    let client = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
    let chevron = crate::window::search_view::SearchView::chevron_rect(client, dpi);
    let (x, y) = (
        (chevron.left + chevron.right) / 2,
        (chevron.top + chevron.bottom) / 2,
    );
    let list_top = || {
        app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .unwrap()
            .search
            .list_area(client, dpi)
            .top
    };
    let closed_top = list_top();

    click(panel, x, y);
    assert!(crate::window::search_view::replace_open(window.hwnd));
    let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
    assert!(is_shown(replace));
    assert_eq!(unsafe { GetFocus() }, replace);
    assert!(
        list_top() > closed_top,
        "the results move under the replace row"
    );

    click(panel, x, y);
    assert!(!crate::window::search_view::replace_open(window.hwnd));
    assert!(!is_shown(replace));
    assert_eq!(
        unsafe { GetFocus() },
        crate::window::search_view::edit_hwnd(window.hwnd).unwrap(),
        "the caret goes back to the search box"
    );
    assert_eq!(list_top(), closed_top);
}

#[test]
fn a_background_dirty_tab_is_replaced_in_the_editor_not_on_disk() {
    // Break caught (Review Focus 5): a background tab's unsaved edits replaced from the
    // note's disk text or written over on disk, the same note also written as a closed note,
    // the active tab changed in the background tab's place, the background tab left clean,
    // or its replacement taking more than one undo.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-background-dirty");
    let a = scratch.note("a.md", "old needle\r\n");
    let b = scratch.note("b.md", "b needle\n");
    let c = scratch.note("c.md", "c needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    // Leaving a tab autosaves it (`switching_tabs_autosaves_the_tab_being_left`); a's edits
    // must stay unsaved.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    crate::window::modal::take_last_confirm();
    super::super::open_path(window.hwnd, &a).unwrap();
    pump_posted_messages(window.hwnd);
    editor.set_text("typed needle here\r\n").unwrap();
    let a_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    let b_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert_ne!(a_id, b_id);

    search_to_replace(window.hwnd, "needle", "pin");
    assert_eq!(search_rows(window.hwnd).len(), 3);
    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_all(window.hwnd);
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 3 matches in 3 notes."
    );
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        Some(format!(
            "Replace 3 matches in 3 notes with \"pin\"?{SAVED_LINE}"
        )),
        "c is closed: the warning shows"
    );

    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "old needle\r\n",
        "a's file is never written"
    );
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        "b needle\n",
        "b is open too: changed in the editor only"
    );
    assert_eq!(
        std::fs::read_to_string(&c).unwrap(),
        "c pin",
        "c is closed: written"
    );
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().id,
        b_id,
        "b stays in front"
    );
    assert_eq!(editor.text().unwrap(), "b pin\n");
    assert!(app_mut(window.hwnd).tabs.document(a_id).unwrap().dirty);

    assert!(super::super::activate_document_by_id(window.hwnd, a_id));
    assert_eq!(
        editor.text().unwrap(),
        "typed pin here\r\n",
        "replaced in the tab's live text"
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.text().unwrap(),
        "typed needle here\r\n",
        "one undo action"
    );
}

#[test]
fn replace_all_writes_the_closed_notes_updates_the_library_and_searches_again() {
    // Break caught: a closed note left unwritten, a note written that the search never
    // listed, the library keeping the old size (the next rescan would read FastPad's own
    // write as an outside change), or the results still listing notes with nothing left to
    // match.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-closed");
    let a = scratch.note("a.md", "one needle, two needle\r\n");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    let b = scratch.note(r"sub\b.md", "needle\n");
    let c = scratch.note("c.md", "nothing");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    let before = search_generation(window.hwnd);

    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_all(window.hwnd);
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 3 matches in 2 notes."
    );
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        Some(format!(
            "Replace 3 matches in 2 notes with \"pin\"?{SAVED_LINE}"
        ))
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "one pin, two pin\r\n");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "pin\n");
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "nothing");
    let size = |relative: &str| {
        crate::window::library_host::with_state(window.hwnd, |state| {
            state
                .notes
                .iter()
                .find(|note| {
                    crate::library::model::same_path(&note.path, std::path::Path::new(relative))
                })
                .map(|note| note.size)
        })
        .flatten()
    };
    assert_eq!(size("a.md"), Some("one pin, two pin\r\n".len() as u64));
    assert_eq!(size(r"sub\b.md"), Some("pin\n".len() as u64));
    assert!(!crate::window::text_search_host::replacing(window.hwnd));

    wait_for_search(window.hwnd, before);
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some((crate::window::search_view::NO_MATCH.to_owned(), false))
    );
}

#[test]
fn declining_the_question_writes_nothing_and_a_later_replace_still_runs() {
    // Break caught: a No that still writes the closed notes or changes the open tab, or one
    // that leaves the replace marked as running, so Replace all never works again.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-declined");
    let a = scratch.note("a.md", "needle");
    let b = scratch.note("b.md", "b needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");

    let asked = decline_next_confirm();
    crate::window::text_search_host::replace_all(window.hwnd);
    pump_until(window.hwnd, || asked.get());
    pump_past_debounce(window.hwnd);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
    assert_eq!(editor.text().unwrap(), "b needle");
    assert!(
        !notices(window.hwnd)
            .iter()
            .any(|notice| notice.starts_with("Replaced "))
    );
    assert!(!crate::window::text_search_host::replacing(window.hwnd));

    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_all(window.hwnd);
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 2 matches in 2 notes."
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
    assert_eq!(editor.text().unwrap(), "b pin");
}

#[test]
fn results_open_nothing_and_the_summary_says_replacing_while_a_replace_runs() {
    // Break caught: a result opened (and so a tab made, whose text the split then changes in
    // the editor instead of the file the question warned about) while the count or the
    // write runs, or the summary still claiming the old results.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-busy");
    scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    let tabs = || tab_paths(window.hwnd).len();
    let before = tabs();

    let asked = decline_next_confirm();
    crate::window::text_search_host::replace_all(window.hwnd);
    assert!(crate::window::text_search_host::replacing(window.hwnd));
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some((crate::window::search_view::REPLACING.to_owned(), false))
    );
    crate::window::search_view::open_selected(window.hwnd, super::super::OpenMode::Preview, false);
    crate::window::search_view::open_selected(window.hwnd, super::super::OpenMode::Permanent, true);
    assert_eq!(tabs(), before, "nothing opened");
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

    pump_until(window.hwnd, || asked.get());
    assert!(!crate::window::text_search_host::replacing(window.hwnd));
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some(("1 note".to_owned(), false))
    );
    crate::window::search_view::open_selected(window.hwnd, super::super::OpenMode::Preview, false);
    assert!(
        app_mut(window.hwnd).tabs.preview_id().is_some(),
        "opens again"
    );
}

#[test]
fn a_note_changed_on_disk_since_the_search_is_skipped_and_named_in_the_report() {
    // Break caught (Review Focus 1, in the window): a sync client's newer text overwritten
    // with a replacement of the text the search read, or the skip left out of the report.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-changed");
    let a = scratch.note("a.md", "needle");
    let b = scratch.note("b.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    std::fs::write(&b, "needle, edited elsewhere").unwrap();

    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_all(window.hwnd);
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 1 match in 1 note. 1 note was skipped because it changed since the search. (b)"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
    assert_eq!(
        std::fs::read_to_string(&b).unwrap(),
        "needle, edited elsewhere"
    );
}

#[test]
fn the_row_replace_changes_one_note_and_asks_only_when_it_is_closed() {
    // Break caught: a row's button replacing in every result, asking about a note whose
    // change one Ctrl+Z undoes, or saving a closed note without asking.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-row");
    let a = scratch.note("a.md", "needle");
    let b = scratch.note("b.md", "b needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    crate::window::modal::take_last_confirm();
    let before = search_generation(window.hwnd);

    crate::window::text_search_host::replace_in(window.hwnd, std::path::Path::new("b.md"));
    assert_eq!(wait_for_report(window.hwnd), "Replaced 1 match in 1 note.");
    assert_eq!(crate::window::modal::take_last_confirm(), None, "b is open");
    assert_eq!(editor.text().unwrap(), "b pin");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
    wait_for_search(window.hwnd, before);
    assert_eq!(
        search_rows(window.hwnd),
        vec![search_row("a", "needle")],
        "b's row is gone"
    );

    app_mut(window.hwnd).notifications.dismiss_all();
    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_in(window.hwnd, std::path::Path::new("a.md"));
    assert_eq!(wait_for_report(window.hwnd), "Replaced 1 match in 1 note.");
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Replace 1 match in \"a\" with \"pin\"? The note is saved and this can't be undone.")
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
}

#[test]
fn ctrl_alt_enter_replaces_an_open_tab_with_its_groups_as_one_undo_action() {
    // Break caught: Ctrl+Alt+Enter opening a result instead, `$1` inserted literally in regex
    // mode (spec §12a), the tab saved, the warning shown with every note open, or the
    // replacement taking one undo per match.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-ctrl-alt-enter");
    let a = scratch.note("a.md", "x needle y needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &a).unwrap();
    pump_posted_messages(window.hwnd);
    crate::window::search_view::show_replace(window.hwnd);
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
    search_for(window.hwnd, "n(ee)dle");
    type_into_replace(window.hwnd, "[$1]");
    let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();

    crate::window::answer_next_confirm(|_| true);
    press_with(replace, VK_RETURN, true, false, true);

    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 2 matches in 1 note."
    );
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Replace 2 matches in 1 note with \"[$1]\"?"),
        "every note is open: no warning line"
    );
    assert_eq!(editor.text().unwrap(), "x [ee] y [ee]");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "x needle y needle");
    editor.undo().unwrap();
    assert_eq!(
        editor.text().unwrap(),
        "x needle y needle",
        "one undo action"
    );
}

#[test]
fn the_replace_controls_are_exposed_with_their_names_and_states() {
    // Break caught (spec §11 names): the chevron missing or read without its expanded state
    // (or silent when it changes), the replace field or Replace all invisible to a screen
    // reader, Replace all read as pressable while a search runs, or the row's button unnamed.
    use crate::window::sidebar_accessibility::{
        STATE_COLLAPSED, STATE_EXPANDED, STATE_UNAVAILABLE, take_raised,
    };
    use windows_sys::Win32::UI::Accessibility::{ROLE_SYSTEM_PUSHBUTTON, ROLE_SYSTEM_TEXT};
    use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-msaa");
    scratch.note("a.md", "one beta");
    scratch.note("b.md", "beta two");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "beta");
    let panel = sidebar_panel(window.hwnd);
    let items = || {
        (0..crate::window::side_panel::accessible_item_count(panel))
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .collect::<Vec<_>>()
    };
    let index_of = |name: &str| items().iter().position(|item| item.name == name);

    let shown = items();
    assert_eq!(shown[0].role, ROLE_SYSTEM_TEXT, "the box keeps child ID 1");
    assert_eq!(
        index_of("Toggle replace"),
        Some(4),
        "after the three toggles"
    );
    assert_eq!(shown[4].role, ROLE_SYSTEM_PUSHBUTTON);
    assert_ne!(shown[4].state & STATE_COLLAPSED, 0);
    assert_eq!(index_of("Replace"), None);
    assert_eq!(index_of("Replace all"), None);
    assert_eq!(index_of("Replace in a"), None);

    take_raised();
    crate::window::search_view::toggle_replace(window.hwnd);
    assert!(
        take_raised().contains(&(panel as usize, EVENT_OBJECT_STATECHANGE, 5)),
        "the chevron (ID 5) raises a state change"
    );
    let shown = items();
    assert_ne!(shown[4].state & STATE_EXPANDED, 0);
    let field = &shown[index_of("Replace").expect("the replace field is a child")];
    assert_eq!(field.role, ROLE_SYSTEM_TEXT);
    assert_eq!(
        field.window,
        crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap()
    );
    let all = &shown[index_of("Replace all").expect("Replace all is a child")];
    assert_eq!(all.role, ROLE_SYSTEM_PUSHBUTTON);
    assert_eq!(all.state & STATE_UNAVAILABLE, 0);
    let row = &shown[index_of("Replace in a").expect("the selected row's button")];
    assert_eq!(row.role, ROLE_SYSTEM_PUSHBUTTON);
    type_into_replace(window.hwnd, "x");
    assert_eq!(items()[index_of("Replace").unwrap()].value, "x");

    take_raised();
    // The same query again: its results stay, and Replace all waits for the search.
    crate::window::text_search_host::run_now(window.hwnd);
    let all_index = index_of("Replace all").unwrap();
    assert_ne!(items()[all_index].state & STATE_UNAVAILABLE, 0);
    assert!(
        take_raised().contains(&(
            panel as usize,
            EVENT_OBJECT_STATECHANGE,
            all_index as i32 + 1
        )),
        "Replace all says it became unavailable"
    );
}

#[test]
fn replace_all_and_the_row_button_are_unavailable_while_a_replace_runs() {
    // Break caught (final review FR3): Replace all and the row's button drawn and announced
    // as pressable during the count, the question or the write, when a press does nothing,
    // or left unavailable once the replace ends.
    use crate::window::sidebar_accessibility::{STATE_UNAVAILABLE, take_raised};
    use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-unavailable");
    scratch.note("a.md", "one beta");
    scratch.note("b.md", "beta two");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "beta");
    crate::window::search_view::toggle_replace(window.hwnd);
    let panel = sidebar_panel(window.hwnd);
    let items = || {
        (0..crate::window::side_panel::accessible_item_count(panel))
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .collect::<Vec<_>>()
    };
    let index_of = |name: &str| items().iter().position(|item| item.name == name).unwrap();
    let unavailable = |name: &str| items()[index_of(name)].state & STATE_UNAVAILABLE != 0;
    let raised_for = |raised: &[(usize, u32, i32)], name: &str| {
        raised.contains(&(
            panel as usize,
            EVENT_OBJECT_STATECHANGE,
            index_of(name) as i32 + 1,
        ))
    };
    assert!(!unavailable("Replace all"));
    assert!(!unavailable("Replace in a"));
    assert!(crate::window::search_view::replace_all_enabled(window.hwnd));

    take_raised();
    let asked = decline_next_confirm();
    crate::window::text_search_host::replace_all(window.hwnd);
    assert!(crate::window::text_search_host::replacing(window.hwnd));
    // What paints the buttons dim (`button_color` and `row_button_color` take it).
    assert!(!crate::window::search_view::replace_all_enabled(
        window.hwnd
    ));
    assert!(unavailable("Replace all"));
    assert!(unavailable("Replace in a"));
    let raised = take_raised();
    assert!(raised_for(&raised, "Replace all"), "{raised:?}");
    assert!(raised_for(&raised, "Replace in a"), "{raised:?}");

    pump_until(window.hwnd, || asked.get());
    assert!(!crate::window::text_search_host::replacing(window.hwnd));
    assert!(crate::window::search_view::replace_all_enabled(window.hwnd));
    assert!(!unavailable("Replace all"));
    assert!(!unavailable("Replace in a"));
    let raised = take_raised();
    assert!(raised_for(&raised, "Replace all"), "{raised:?}");
    assert!(raised_for(&raised, "Replace in a"), "{raised:?}");
}

#[test]
fn a_closed_note_is_never_written_when_the_question_had_no_saved_line() {
    // Break caught (final review FR1): a closed note whose read failed during the count (so
    // it counted 0 and the question never said notes are saved) written at the apply,
    // where its read succeeds, its stamp matches and it has a match.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-no-saved-line");
    let a = scratch.note("a.md", "a needle");
    let b = scratch.note("b.md", "b needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    assert_eq!(search_rows(window.hwnd).len(), 2);
    crate::window::modal::take_last_confirm();

    // The count as a failed read of a.md leaves it: b's match only, from its tab.
    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_counted(
        window.hwnd,
        crate::window::text_search_host::test_counted(
            window.hwnd,
            crate::window::text_search_host::replace_generation(window.hwnd),
            crate::library::text_replace::ReplaceCount {
                matches: 1,
                notes: 1,
                closed_notes: 0,
            },
        ),
    );
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 1 match in 1 note. 1 note was skipped because it changed since the search. (a)"
    );
    assert_eq!(
        crate::window::modal::take_last_confirm().as_deref(),
        Some("Replace 1 match in 1 note with \"pin\"?"),
        "no saved line"
    );
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "a needle",
        "not written"
    );
    assert_eq!(
        editor.text().unwrap(),
        "b pin",
        "the open tab still changes"
    );
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b needle");
}

#[test]
fn a_plan_made_stale_before_its_write_starts_writes_nothing() {
    // Break caught (Task 6 re-review New #1): `apply_plan`, finding no cancel flag (as a
    // `cancel_replace` leaves it), making a fresh one and writing the notes of a replace
    // that was already cancelled.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-stale-apply");
    let a = scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    let generation = crate::window::text_search_host::replace_generation(window.hwnd);
    crate::window::text_search_host::cancel_replace(window.hwnd);
    let ended = crate::window::text_search_host::writer_hooks::ended();

    crate::window::text_search_host::test_apply(window.hwnd, generation);
    pump_past_debounce(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "needle",
        "nothing written"
    );
    assert_eq!(
        crate::window::text_search_host::writer_hooks::ended(),
        ended,
        "no writer started"
    );
    assert!(
        !notices(window.hwnd)
            .iter()
            .any(|notice| notice.starts_with("Replaced "))
    );

    // The same plan for the current generation, with no flag either, writes.
    crate::window::text_search_host::test_apply(
        window.hwnd,
        crate::window::text_search_host::replace_generation(window.hwnd),
    );
    assert_eq!(wait_for_report(window.hwnd), "Replaced 1 match in 1 note.");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
}
