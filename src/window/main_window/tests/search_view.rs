//! The Search view: toggles, debounce, narrowing, regex errors and its box.

use super::*;

#[test]
fn the_search_toggle_commands_show_search_and_flip_its_options() {
    // Break caught: a palette toggle that flips an option nobody can see, or flips the
    // wrong one.
    use crate::search::MatchOptions;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-toggle-commands");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    let options = || crate::window::search_view::options(window.hwnd);

    execute_command(window.hwnd, CommandId::SearchToggleCase);
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        crate::config::SidebarView::Search
    );
    assert_eq!(
        options(),
        MatchOptions {
            case: true,
            ..MatchOptions::default()
        }
    );
    execute_command(window.hwnd, CommandId::SearchToggleWholeWord);
    execute_command(window.hwnd, CommandId::SearchToggleRegex);
    assert_eq!(
        options(),
        MatchOptions {
            case: true,
            whole_word: true,
            regex: true
        }
    );
    execute_command(window.hwnd, CommandId::SearchToggleCase);
    assert!(!options().case);
}

#[test]
fn with_notes_mode_off_ctrl_shift_f_and_the_search_toggles_do_nothing() {
    // Break caught: a sidebar command reaching code that assumes a sidebar, or reading and
    // changing editor state with notes mode off.
    let _scintilla = load_native_scintilla();
    let mut app = make_app();
    app.settings.notes_mode = false;
    let window = ProductionWindow::new(app);
    let editor = install_test_editor(&window);
    editor.populate_clean("alpha beta").unwrap();
    editor.set_selection(0..5).unwrap();
    let before = sidebar_command_runs();

    for command in [
        CommandId::ShowSearchView,
        CommandId::SearchToggleCase,
        CommandId::SearchToggleWholeWord,
        CommandId::SearchToggleRegex,
        CommandId::ReplaceInNotes,
    ] {
        execute_command(window.hwnd, command);
    }

    // The Search view's own state already reads as empty with no sidebar to hold it, so this
    // counts commands that ran past the notes-mode guard instead (see `sidebar_command_runs`).
    assert_eq!(
        sidebar_command_runs(),
        before,
        "the is_sidebar guard should have skipped every command"
    );
    assert!(app_mut(window.hwnd).sidebar.is_none());
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        crate::config::SidebarView::Hidden
    );
    assert_eq!(editor.selection().unwrap(), 0..5);
    assert!(notices(window.hwnd).is_empty());
}

#[test]
fn shift_alt_f_formats_json_and_ctrl_shift_f_no_longer_does() {
    // Break caught: Format JSON left on Ctrl+Shift+F, where it would rewrite a JSON file
    // the user only meant to search from, or not reachable from any shortcut.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN, WM_SYSKEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    // Presses F with the given modifiers held, through the accelerator table.
    let press_f = |ctrl: bool, shift: bool, alt: bool| {
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = if ctrl { 0x80 } else { 0 };
        keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
        keys[VK_MENU as usize] = if alt { 0x80 } else { 0 };
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let message = MSG {
            hwnd: editor.hwnd(),
            message: if alt { WM_SYSKEYDOWN } else { WM_KEYDOWN },
            wParam: usize::from(b'F'),
            // Bit 29, the context code, is set while Alt is down.
            lParam: if alt { 1 << 29 } else { 0 },
            ..Default::default()
        };
        let translated =
            unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };
        unsafe { SetKeyboardState(original.as_ptr()) };
        translated
    };
    editor.populate_clean("{\"a\":1}").unwrap();

    assert!(press_f(true, true, false));
    pump_posted_messages(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "{\"a\":1}",
        "Ctrl+Shift+F leaves JSON alone"
    );

    assert!(press_f(false, true, true));
    pump_posted_messages(window.hwnd);
    assert_eq!(editor.text().unwrap(), "{\n  \"a\": 1\n}");
}

#[test]
fn saving_a_listed_note_keeps_the_search_selection_and_does_not_rebuild_the_tree() {
    // Break caught: every save (autosave included) rebuilding the sidebar, re-running the
    // search, or snapping the Search selection back to the first result.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-save-keeps");
    scratch.note("plan.md", "plan a");
    let planning = scratch.note("planning.md", "plan b");
    scratch.note("plans.md", "plan c");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    use crate::config::SidebarView;
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
    search_for(window.hwnd, "plan");
    let results = search_rows(window.hwnd);
    assert_eq!(results.len(), 3);
    let index = results
        .iter()
        .position(|(name, _)| name == "planning")
        .unwrap();
    assert_ne!(index, 0);
    app_mut(window.hwnd)
        .sidebar
        .as_mut()
        .unwrap()
        .search
        .list
        .selected = Some(index);
    super::super::open_path(window.hwnd, &planning).unwrap();
    pump_posted_messages(window.hwnd);
    let rebuilds = notebook_view(window.hwnd).rebuilds;
    let searches = search_generation(window.hwnd);

    editor.set_text("edited plan").unwrap();
    assert!(super::super::save_active_document(window.hwnd));
    assert_eq!(std::fs::read_to_string(&planning).unwrap(), "edited plan");
    assert_eq!(
        search_selected(window.hwnd),
        Some(index),
        "kept by the save"
    );
    assert_eq!(
        notebook_view(window.hwnd).rebuilds,
        rebuilds,
        "a save of a listed note changes no row"
    );
    // A refresh with the same notes runs nothing (spec §7).
    crate::window::side_panel::refresh(window.hwnd);
    // Past the debounce, so a re-run wrongly scheduled by the save or the refresh would have
    // started.
    pump_past_debounce(window.hwnd);
    assert_eq!(search_generation(window.hwnd), searches, "no search re-ran");

    // The same query run again keeps the selection by path.
    let before = search_generation(window.hwnd);
    crate::window::text_search_host::run_now(window.hwnd);
    wait_for_search(window.hwnd, before);
    assert_eq!(
        search_selected(window.hwnd),
        Some(index),
        "kept by a re-run"
    );
    // A new query selects the same note again once it arrives.
    search_for(window.hwnd, "pla");
    assert_eq!(selected_name(window.hwnd).as_deref(), Some("planning"));
}

#[test]
fn typing_waits_for_the_debounce_and_gives_sorted_results_with_the_selection_kept_by_path() {
    // Break caught: a search per keystroke, results in the order the worker found them, or
    // results arriving above the selected row moving the selection to another note.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-debounce");
    scratch.note("c10.md", "needle");
    scratch.note("b.md", "a needle here");
    scratch.note("c9.md", "needle");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "needle too");
    scratch.note("d.md", "no match");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);

    type_into_search(window.hwnd, "needle");
    // The keystroke only restarted the timer: nothing has run yet.
    assert!(search_rows(window.hwnd).is_empty());
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );
    assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());
    wait_for_search(window.hwnd, search_generation(window.hwnd));
    assert_eq!(
        search_rows(window.hwnd),
        vec![
            search_row("b", "a needle here"),
            search_row("b", "needle too"),
            search_row("c9", "needle"),
            search_row("c10", "needle"),
        ]
    );
    let folders = app_mut(window.hwnd)
        .sidebar
        .as_ref()
        .unwrap()
        .search
        .results
        .iter()
        .map(|result| result.folder.clone())
        .collect::<Vec<_>>();
    assert_eq!(folders, ["", "sub", "", ""]);
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some(("4 notes".to_owned(), false))
    );

    // c9 is selected; a new note that sorts first arrives with the re-run.
    app_mut(window.hwnd)
        .sidebar
        .as_mut()
        .unwrap()
        .search
        .list
        .selected = Some(2);
    let added = scratch.note("a.md", "needle first");
    crate::window::library_host::with_state(window.hwnd, |state| state.add_note(&added));
    let before = search_generation(window.hwnd);
    crate::window::side_panel::refresh(window.hwnd);
    wait_for_search(window.hwnd, before);
    assert_eq!(search_rows(window.hwnd)[0], search_row("a", "needle first"));
    assert_eq!(selected_name(window.hwnd).as_deref(), Some("c9"));
    assert_eq!(search_selected(window.hwnd), Some(3));
}

#[test]
fn a_batch_from_an_older_generation_is_dropped() {
    // Break caught: a slow batch from the previous query landing after the new one began, so
    // the list flickers back to rows the new query never matched.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-generation");
    scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "needle");
    let shown = vec![search_row("a", "needle")];
    assert_eq!(search_rows(window.hwnd), shown);

    let current = search_generation(window.hwnd);
    let stale = crate::window::text_search_host::test_batch(
        current.wrapping_sub(1),
        vec![stray_hit("old")],
        None,
    );
    crate::window::text_search_host::batch_arrived(window.hwnd, stale);
    assert_eq!(
        search_rows(window.hwnd),
        shown,
        "dropped when handled directly"
    );
    let stale = crate::window::text_search_host::test_batch(
        current.wrapping_sub(1),
        vec![stray_hit("older")],
        Some(crate::library::text_search::RunEnd::Completed),
    );
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            window.hwnd,
            crate::window::WM_FASTPAD_TEXT_SEARCH_BATCH,
            0,
            stale,
        );
    }
    pump_posted_messages(window.hwnd);
    assert_eq!(
        search_rows(window.hwnd),
        shown,
        "and when it comes through the queue"
    );

    // A keystroke cancels the running search, so its late batches are stale too.
    type_into_search(window.hwnd, "needles");
    let late = crate::window::text_search_host::test_batch(current, vec![stray_hit("late")], None);
    crate::window::text_search_host::batch_arrived(window.hwnd, late);
    assert_eq!(search_rows(window.hwnd), shown);
    // The current generation's batch is the one that shows.
    let now = crate::window::text_search_host::test_batch(
        search_generation(window.hwnd),
        vec![stray_hit("b")],
        None,
    );
    crate::window::text_search_host::batch_arrived(window.hwnd, now);
    assert_eq!(search_rows(window.hwnd).len(), 2);
}

#[test]
fn a_dirty_tab_is_searched_as_the_editor_has_it() {
    // Break caught: the search reading an open note from disk, so a phrase typed only in the
    // editor is missed and a phrase deleted in the editor is still found, for the active tab
    // or a tab in the background; or reading a background tab leaving it in the editor.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-dirty");
    let a = scratch.note("a.md", "kept on disk only");
    let b = scratch.note("b.md", "plain b");
    scratch.note("c.md", "plain c");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    // No autosave may write the edits: opening `b` would save `a`, the tab being left.
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &a).unwrap();
    pump_posted_messages(window.hwnd);
    editor.set_text("typed in the editor only").unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    editor.set_text("b typed too").unwrap();
    let dirty = app_mut(window.hwnd)
        .tabs
        .documents()
        .filter(|document| document.dirty)
        .count();
    assert_eq!(dirty, 2);

    let overlays = crate::window::text_search_host::dirty_overlays(window.hwnd, &scratch.folder());
    assert_eq!(overlays.len(), 2);
    assert_eq!(
        overlays
            .get(std::path::Path::new("a.md"))
            .map(String::as_str),
        Some("typed in the editor only"),
        "a background tab"
    );
    assert_eq!(
        overlays
            .get(std::path::Path::new("b.md"))
            .map(String::as_str),
        Some("b typed too"),
        "the active tab"
    );
    assert_eq!(
        editor.text().unwrap(),
        "b typed too",
        "the active tab is back"
    );

    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "editor only");
    assert_eq!(
        search_rows(window.hwnd),
        vec![search_row("a", "typed in the editor only")]
    );
    search_for(window.hwnd, "on disk");
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some((crate::window::search_view::NO_MATCH.to_owned(), false))
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "kept on disk only");
}

#[test]
fn a_longer_plain_query_searches_only_the_previous_hits() {
    // Break caught: every keystroke re-reading the whole notebook, or narrowing kept after a
    // change of options.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-narrow");
    scratch.note("a.md", "needle");
    scratch.note("b.md", "needles");
    scratch.note("c.md", "other");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "need");
    assert_eq!(searched_total(window.hwnd), 3);
    search_for(window.hwnd, "needl");
    assert_eq!(
        searched_total(window.hwnd),
        2,
        "only the notes \"need\" found"
    );
    assert_eq!(search_rows(window.hwnd).len(), 2);
    let before = search_generation(window.hwnd);
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Case);
    wait_for_search(window.hwnd, before);
    assert_eq!(
        searched_total(window.hwnd),
        3,
        "new options search everything"
    );
}

#[test]
fn an_invalid_regex_shows_its_error_keeps_the_results_and_runs_nothing() {
    // Break caught: a regex typo blanking the list, running a search anyway, or showing no
    // reason.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-bad-regex");
    scratch.note("a.md", "ab here");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "ab");
    let before = search_generation(window.hwnd);
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
    wait_for_search(window.hwnd, before);
    assert!(crate::window::search_view::options(window.hwnd).regex);
    assert_eq!(search_rows(window.hwnd).len(), 1);

    type_into_search(window.hwnd, "(ab");
    pump_until(window.hwnd, || {
        matches!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::PatternError(_)
        )
    });
    let (message, error) = crate::window::search_view::summary(window.hwnd).unwrap();
    assert!(error, "shown as an error");
    assert!(!message.is_empty());
    assert_eq!(
        search_rows(window.hwnd).len(),
        1,
        "the previous results stay"
    );
    assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());

    search_for(window.hwnd, "(ab)");
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some(("1 note".to_owned(), false))
    );
}

#[test]
fn one_character_says_type_at_least_two_and_clears_the_results() {
    // Break caught: a one-letter query reading the whole notebook, or the last query's rows
    // left under a query they don't match.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-short");
    scratch.note("a.md", "ab");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "ab");
    assert_eq!(search_rows(window.hwnd).len(), 1);

    type_into_search(window.hwnd, "a");
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::TooShort
    );
    assert!(search_rows(window.hwnd).is_empty());
    assert_eq!(
        crate::window::search_view::summary(window.hwnd),
        Some((crate::window::search_view::TOO_SHORT.to_owned(), false))
    );
    assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());
    // A second character clears "Type at least 2 characters." at once, not after the debounce.
    type_into_search(window.hwnd, "ab");
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );
    assert_eq!(crate::window::search_view::summary(window.hwnd), None);
    type_into_search(window.hwnd, "  ");
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );
    assert_eq!(crate::window::search_view::summary(window.hwnd), None);
    // Run directly (as a toggle or Ctrl+Shift+F would), a query of spaces still runs nothing.
    let before = search_generation(window.hwnd);
    crate::window::text_search_host::run_now(window.hwnd);
    assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );
    pump_posted_messages(window.hwnd);
    assert_ne!(search_generation(window.hwnd), before, "it only cancelled");
    assert!(search_rows(window.hwnd).is_empty());
}

#[test]
fn show_with_query_fills_the_box_escapes_it_for_regex_and_runs_at_once() {
    // Break caught: Ctrl+Shift+F's selection waiting out the debounce, or "1+1" searched as
    // a regex (one or more 1s, then 1) when regex is on.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-prefill");
    scratch.note("a.md", "costs 1+1 here");
    scratch.note("b.md", "costs 11 here");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    let before = search_generation(window.hwnd);
    crate::window::search_view::show_with_query(window.hwnd, "1+1");
    assert!(
        matches!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::Running(_)
                | crate::window::search_view::SearchState::Done { .. }
        ),
        "running without the debounce"
    );
    wait_for_search(window.hwnd, before);
    assert_eq!(
        search_rows(window.hwnd),
        vec![search_row("a", "costs 1+1 here")]
    );

    let before = search_generation(window.hwnd);
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
    wait_for_search(window.hwnd, before);
    let before = search_generation(window.hwnd);
    crate::window::search_view::show_with_query(window.hwnd, "1+1");
    wait_for_search(window.hwnd, before);
    let (query, options) = crate::window::search_view::current_query(window.hwnd).unwrap();
    assert!(options.regex);
    assert_eq!(query, r"1\+1");
    assert_eq!(
        crate::window::search_view::run_query(window.hwnd),
        Some((query, options)),
        "the results are for the escaped query, with regex on"
    );
    assert_eq!(
        search_rows(window.hwnd),
        vec![search_row("a", "costs 1+1 here")]
    );
}

#[test]
fn a_rescan_during_a_search_keeps_its_end_from_narrowing_the_next_one() {
    // Break caught: a search still running when a rescan installs recording its pre-rescan
    // hits for narrowing, so the next longer query misses a note edited outside FastPad.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-rescan-narrow");
    scratch.note("a.md", "needle");
    scratch.note("b.md", "needles");
    scratch.note("c.md", "other");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "need");
    crate::window::text_search_host::run_now(window.hwnd);
    // The rescan lands while that search runs; then its end arrives.
    crate::window::text_search_host::notes_reloaded(window.hwnd);
    let end = crate::window::text_search_host::test_batch(
        search_generation(window.hwnd),
        Vec::new(),
        Some(crate::library::text_search::RunEnd::Completed),
    );
    crate::window::text_search_host::batch_arrived(window.hwnd, end);

    type_into_search(window.hwnd, "needl");
    let before = search_generation(window.hwnd);
    crate::window::text_search_host::run_now(window.hwnd);
    wait_for_search(window.hwnd, before);
    assert_eq!(
        searched_total(window.hwnd),
        3,
        "every note, not the old hits"
    );
}

#[test]
fn a_save_of_a_listed_note_ends_narrowing() {
    // Break caught: a longer query narrowed to the hits found before FastPad saved a new
    // phrase into a clean note, so the note is never found.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-save-narrow");
    let x = scratch.note("x.md", "nothing");
    scratch.note("y.md", "foo bar");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "foo");
    assert_eq!(search_rows(window.hwnd), vec![search_row("y", "foo bar")]);
    super::super::open_path(window.hwnd, &x).unwrap();
    pump_posted_messages(window.hwnd);
    editor.set_text("food here").unwrap();
    assert!(super::super::save_active_document(window.hwnd));
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);

    search_for(window.hwnd, "food");
    assert_eq!(search_rows(window.hwnd), vec![search_row("x", "food here")]);
}

#[test]
fn a_narrowed_search_still_counts_the_notes_the_last_one_skipped() {
    // Break caught: narrowing to the last hits only, so "1 note wasn't searched" disappears
    // and a note that couldn't be read is never tried again.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-narrow-skipped");
    scratch.note("a.md", "needle");
    scratch.note("b.md", "needles");
    let cloud = scratch.note("c.md", "needle in the cloud");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let relative = crate::library::record_path(&scratch.folder(), &cloud);
    crate::window::library_host::with_state(window.hwnd, |state| {
        for note in state.notes.iter_mut() {
            note.online_only = note.path == relative;
        }
    });
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    let skipped = |hwnd| match search_state(hwnd) {
        crate::window::search_view::SearchState::Done { progress, .. } => progress.skipped_total(),
        other => panic!("the search has not finished: {other:?}"),
    };
    search_for(window.hwnd, "need");
    assert_eq!((searched_total(window.hwnd), skipped(window.hwnd)), (3, 1));
    search_for(window.hwnd, "needl");
    assert_eq!(
        (searched_total(window.hwnd), skipped(window.hwnd)),
        (3, 1),
        "the two hits and the skipped note"
    );
}

#[test]
fn the_debounce_waits_out_a_modal_loop_and_a_file_population() {
    // Break caught: the timer firing inside a nested modal loop or while a file is being
    // populated, and swapping editor documents under it to read the dirty tabs.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-modal");
    scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    type_into_search(window.hwnd, "needle");
    let before = search_generation(window.hwnd);
    crate::window::answer_next_confirm(|hwnd| {
        crate::window::text_search_host::timer(hwnd);
        true
    });
    assert!(crate::window::modal::confirm(window.hwnd, "Go on?", "Go"));
    assert!(
        crate::window::text_search_host::cancel_flag(window.hwnd).is_none(),
        "nothing ran inside the modal loop"
    );
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );

    app_mut(window.hwnd).populating_file = true;
    crate::window::text_search_host::timer(window.hwnd);
    app_mut(window.hwnd).populating_file = false;
    assert!(
        crate::window::text_search_host::cancel_flag(window.hwnd).is_none(),
        "nothing ran during the population"
    );
    // The timer is still armed: the search runs once both are over.
    wait_for_search(window.hwnd, before);
    assert_eq!(search_rows(window.hwnd), vec![search_row("a", "needle")]);
}

#[test]
fn closing_the_window_cancels_a_running_search() {
    // Break caught: a worker reading a large notebook on after its window closed.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-destroy");
    for index in 0..200 {
        scratch.note(&format!("n{index}.md"), "needle");
    }
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    type_into_search(window.hwnd, "needle");
    crate::window::text_search_host::run_now(window.hwnd);
    let flag = crate::window::text_search_host::cancel_flag(window.hwnd).expect("running");
    unsafe {
        DestroyWindow(window.hwnd);
    }
    assert!(flag.load(Ordering::Relaxed));
}

/// The search field's toggle rectangles in the Search view's panel.
fn search_toggles(hwnd: HWND) -> (HWND, [RECT; 3]) {
    let panel = sidebar_panel(hwnd);
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let field = crate::window::search_view::SearchView::field_rect(client, dpi);
    (
        panel,
        crate::window::option_toggles::toggle_rects(field, dpi),
    )
}

const ALT_DOWN: LPARAM = 1 << 29;

#[test]
fn the_toggles_change_by_click_and_by_alt_keys_in_the_box_and_the_results() {
    // Break caught: toggles that paint but ignore clicks, Alt+C/W/R going to the menu band
    // instead of flipping the option, a toggle that flips it without searching again, or a
    // regex error with no line saying so.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SYSCHAR, WM_SYSKEYDOWN,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-toggles");
    scratch.note("A.md", "Needle");
    scratch.note("b.md", "needle");
    scratch.note("c.md", "needles");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "needle");
    assert_eq!(search_rows(window.hwnd).len(), 3);
    let dpi = unsafe { GetDpiForWindow(sidebar_panel(window.hwnd)) }.max(96);
    assert_eq!(
        app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .unwrap()
            .search
            .list
            .row_height,
        crate::window::design::metrics::scale(46, dpi),
        "two-line rows"
    );
    let options = || crate::window::search_view::options(window.hwnd);

    // A click on Match case.
    let (panel, rects) = search_toggles(window.hwnd);
    let center = |rect: RECT| {
        ((((rect.top + rect.bottom) / 2) as u32) << 16 | ((rect.left + rect.right) / 2) as u32)
            as LPARAM
    };
    let before = search_generation(window.hwnd);
    unsafe {
        SendMessageW(panel, WM_LBUTTONDOWN, 0, center(rects[0]));
        SendMessageW(panel, WM_LBUTTONUP, 0, center(rects[0]));
    }
    assert!(options().case);
    wait_for_search(window.hwnd, before);
    assert_eq!(
        search_rows(window.hwnd),
        vec![search_row("b", "needle"), search_row("c", "needles")]
    );

    // Alt+C in the box turns it off again.
    let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    let before = search_generation(window.hwnd);
    unsafe { SendMessageW(edit, WM_SYSKEYDOWN, usize::from(b'C'), ALT_DOWN) };
    assert!(!options().case);
    wait_for_search(window.hwnd, before);
    assert_eq!(search_rows(window.hwnd).len(), 3);
    // The character that follows is swallowed, not handed to the menu band.
    assert_eq!(
        unsafe { SendMessageW(edit, WM_SYSCHAR, usize::from(b'c'), ALT_DOWN) },
        0
    );
    assert!(app_mut(window.hwnd).menu_mode.is_none());

    // Alt+W in the results: whole word drops "needles".
    let before = search_generation(window.hwnd);
    unsafe { SendMessageW(panel, WM_SYSKEYDOWN, usize::from(b'W'), ALT_DOWN) };
    assert!(options().whole_word);
    wait_for_search(window.hwnd, before);
    assert_eq!(
        search_rows(window.hwnd),
        vec![search_row("A", "Needle"), search_row("b", "needle")]
    );
    // Without Alt held (F10 also sends WM_SYSKEYDOWN), nothing flips.
    unsafe { SendMessageW(panel, WM_SYSKEYDOWN, usize::from(b'W'), 0) };
    assert!(options().whole_word);

    // Alt+R; an invalid pattern shows its error in place of the summary and keeps the rows.
    let before = search_generation(window.hwnd);
    unsafe { SendMessageW(edit, WM_SYSKEYDOWN, usize::from(b'R'), ALT_DOWN) };
    assert!(options().regex);
    wait_for_search(window.hwnd, before);
    type_into_search(window.hwnd, "need(le");
    pump_until(window.hwnd, || {
        matches!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::PatternError(_)
        )
    });
    let (message, error) = crate::window::search_view::summary(window.hwnd).unwrap();
    assert!(error && !message.is_empty(), "{message}");
    assert_eq!(
        search_rows(window.hwnd).len(),
        2,
        "the previous results stay"
    );
}

#[test]
fn esc_in_the_search_box_clears_it_and_then_returns_to_the_editor() {
    // Break caught: Esc leaving the query in place, or jumping to the editor with text still
    // in the box.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-escape");
    scratch.note("a.md", "ab");
    let window = shown_window();
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "ab");
    let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    unsafe { SetFocus(edit) };

    unsafe { SendMessageW(edit, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    assert_eq!(unsafe { GetWindowTextLengthW(edit) }, 0);
    assert!(search_rows(window.hwnd).is_empty());
    assert_eq!(
        search_state(window.hwnd),
        crate::window::search_view::SearchState::Idle
    );
    assert_eq!(focused(), edit, "the first Esc only clears");

    unsafe { SendMessageW(edit, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    assert_eq!(
        focused(),
        editor.hwnd(),
        "Esc in the empty box goes to the editor"
    );
}

#[test]
fn with_no_notebook_the_search_box_and_its_toggles_still_work() {
    // Break caught: the options impossible to set until a notebook opens.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSKEYDOWN;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    assert_eq!(
        crate::window::search_view::status(window.hwnd),
        Some(crate::window::search_view::NO_NOTEBOOK)
    );
    let edit = crate::window::search_view::edit_hwnd(window.hwnd).expect("the box shows");
    unsafe { SendMessageW(edit, WM_SYSKEYDOWN, usize::from(b'C'), ALT_DOWN) };
    assert!(crate::window::search_view::options(window.hwnd).case);
}

#[test]
fn the_search_field_is_client_area_and_a_press_on_its_padding_focuses_the_box() {
    // Break caught: a press on the field's border or padding starting a window drag (the
    // whole Search header answered HTTRANSPARENT), so the box could only be focused by
    // hitting its text line exactly.
    use windows_sys::Win32::Foundation::{LPARAM, POINT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HTTRANSPARENT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCHITTEST,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-field-client");
    scratch.note("plan.md", "p");
    let window = shown_window();
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    let panel = sidebar_panel(window.hwnd);
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let field = crate::window::search_view::SearchView::field_rect(client, dpi);
    let hit_test = |x: i32, y: i32| {
        let mut point = POINT { x, y };
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(panel, &mut point) };
        let lparam = ((point.y as u16 as u32) << 16 | point.x as u16 as u32) as LPARAM;
        unsafe { SendMessageW(panel, WM_NCHITTEST, 0, lparam) }
    };
    // The field's top-left padding, outside the Edit, and the title band above the field.
    assert_ne!(
        hit_test(field.left + 1, field.top + 1),
        HTTRANSPARENT as LRESULT,
        "the field is client area"
    );
    assert_eq!(
        hit_test(field.left - 2, 1),
        HTTRANSPARENT as LRESULT,
        "the title band above the field still drags the window"
    );

    let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    unsafe { SetFocus(window.hwnd) };
    let lparam = (((field.top + 1) as u32) << 16 | (field.left + 1) as u32) as LPARAM;
    unsafe {
        SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
        SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
    }
    assert_eq!(
        unsafe { GetFocus() },
        edit,
        "the press put the caret in the box"
    );
}

#[test]
fn with_no_notebook_the_search_view_says_to_open_one() {
    // Break caught: an empty Search view with a live box that searches nothing.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    assert_eq!(
        crate::window::search_view::status(window.hwnd),
        Some(crate::window::search_view::NO_NOTEBOOK)
    );
}
