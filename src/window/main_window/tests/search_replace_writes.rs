//! Replace keys and the write worker: stale plans, reloads, counts and closing.

use super::*;

/// Runs `key` with Ctrl (and Shift) held through the accelerator table, as the message loop
/// does for a key sent to `target`, and returns whether the table translated it.
fn translate_key(hwnd: HWND, target: HWND, key: u8, shift: bool) -> bool {
    translate_key_with(hwnd, target, key, true, shift, false).is_some()
}

#[test]
fn ctrl_shift_h_in_the_replace_field_closes_it_and_the_chevron_action_toggles_it() {
    // Break caught (final review FR6): a keyboard user unable to close the replace field
    // once it is open (Ctrl+Shift+H only opening it), the caret left in the hidden field,
    // Ctrl+Shift+H elsewhere closing it, or the chevron's default action doing nothing.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-close-key");
    scratch.note("a.md", "alpha needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let open = || crate::window::search_view::replace_open(window.hwnd);

    execute_command(window.hwnd, CommandId::ReplaceInNotes);
    let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
    let search_box = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    assert!(open());
    assert_eq!(unsafe { GetFocus() }, replace);

    assert!(translate_key(window.hwnd, replace, b'H', true));
    assert!(!open(), "closed from inside the field");
    assert!(!is_shown(replace));
    assert_eq!(
        unsafe { GetFocus() },
        search_box,
        "the caret goes to the box"
    );

    assert!(translate_key(window.hwnd, search_box, b'H', true));
    assert!(open(), "from the box it opens the field");
    assert_eq!(unsafe { GetFocus() }, replace);
    unsafe { SetFocus(search_box) };
    assert!(translate_key(window.hwnd, search_box, b'H', true));
    assert!(open(), "with the caret in the box it stays open");
    assert_eq!(unsafe { GetFocus() }, replace);

    let panel = sidebar_panel(window.hwnd);
    let source = &crate::window::side_panel::PANEL_ACCESSIBLE;
    let chevron = (0..(source.count)(panel))
        .position(|index| {
            (source.item)(panel, index).is_some_and(|item| item.name == "Toggle replace")
        })
        .unwrap();
    (source.activate)(panel, chevron);
    assert!(!open(), "the chevron's default action closes the field");
    (source.activate)(panel, chevron);
    assert!(open(), "and opens it again");
    assert_eq!(unsafe { GetFocus() }, replace);
}

#[test]
fn ctrl_shift_1_is_not_an_accelerator() {
    // Break caught (Task 5 review Minor 1): a Ctrl+Shift+1 accelerator, or a change to how
    // the table is built, eating the key before the Search view's row replace sees it.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-ctrl-shift-1");
    scratch.note("a.md", "alpha needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    let panel = sidebar_panel(window.hwnd);
    assert!(
        !translate_key(window.hwnd, panel, b'1', true),
        "in the results"
    );
    assert!(
        !translate_key(window.hwnd, editor.hwnd(), b'1', true),
        "in the editor"
    );
    assert!(
        translate_key(window.hwnd, panel, b'H', true),
        "the harness translates a real accelerator"
    );
}

#[test]
fn the_replace_field_never_takes_the_caret_without_a_search_box() {
    // Break caught (Task 5 review Minor 2): with the search box not made, Ctrl+Shift+H
    // making the replace field (which `layout` never places or shows) and focusing it, so
    // keystrokes go into an invisible control.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-no-box");
    scratch.note("a.md", "alpha needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    ensure_sidebar(window.hwnd);
    assert_eq!(crate::window::search_view::edit_hwnd(window.hwnd), None);
    assert!(crate::window::search_view::fail_search_box(window.hwnd));

    execute_command(window.hwnd, CommandId::ReplaceInNotes);
    assert_eq!(crate::window::search_view::edit_hwnd(window.hwnd), None);
    assert_eq!(
        crate::window::search_view::replace_edit_hwnd(window.hwnd),
        None,
        "no field made, so none focused"
    );
    assert!(!crate::window::search_view::replace_open(window.hwnd));
}

#[test]
fn a_tab_closed_before_an_unasked_row_replace_applies_is_not_written() {
    // Break caught: a note saved without the question ever saying so, because its tab (whose
    // text the count read, so no question was asked) closed while the count ran.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-closed-meanwhile");
    let b = scratch.note("b.md", "b needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    crate::window::modal::take_last_confirm();

    crate::window::text_search_host::replace_in(window.hwnd, std::path::Path::new("b.md"));
    execute_command(window.hwnd, CommandId::CloseTab);
    assert!(
        tab_paths(window.hwnd)
            .iter()
            .all(|path| path.as_deref() != Some(b.as_path()))
    );
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 0 matches in 0 notes. 1 note was skipped because it changed since the search. (b)"
    );
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        None,
        "never asked"
    );
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b needle");
}

#[test]
fn a_hit_from_a_dirty_tab_that_has_closed_is_never_written() {
    // Break caught (R-nostamp): a note whose hit came from a tab's unsaved text (no stamp,
    // so no check that the file is what the search read) written from its file after the
    // tab closed, or its skip left out of the report.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-no-stamp");
    let a = scratch.note("a.md", "needle");
    let b = scratch.note("b.md", "b needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    editor.set_text("b needle typed").unwrap();
    search_to_replace(window.hwnd, "needle", "pin");
    assert_eq!(search_rows(window.hwnd).len(), 2);
    answer_next_close_prompt(|_| CloseDecision::Discard);
    execute_command(window.hwnd, CommandId::CloseTab);
    assert!(
        tab_paths(window.hwnd)
            .iter()
            .all(|path| path.as_deref() != Some(b.as_path()))
    );

    crate::window::answer_next_confirm(|_| true);
    crate::window::text_search_host::replace_all(window.hwnd);
    assert_eq!(
        wait_for_report(window.hwnd),
        "Replaced 1 match in 1 note. 1 note was skipped because it changed since the search. (b)"
    );
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        Some(format!(
            "Replace 1 match in 1 note with \"pin\"?{SAVED_LINE}"
        )),
        "b is never read, so the question doesn't count it"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b needle");
}

#[test]
fn a_new_search_drops_a_replace_that_has_not_asked_yet() {
    // Break caught (review Important 1): a count still running or held when the user types
    // another query asking its question anyway, so a Yes saves the old query's replacement
    // while the results show the new one.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-new-search");
    let a = scratch.note("a.md", "needle other");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    crate::window::modal::take_last_confirm();
    let asked = decline_next_confirm();

    crate::window::text_search_host::replace_all(window.hwnd);
    assert!(crate::window::text_search_host::replacing(window.hwnd));
    type_into_search(window.hwnd, "other");
    assert!(
        !crate::window::text_search_host::replacing(window.hwnd),
        "the keystroke dropped the replace"
    );
    // A count for the current generation, made for the old results: the box shows another
    // query, so it is not asked about either.
    crate::window::text_search_host::replace_counted(
        window.hwnd,
        crate::window::text_search_host::test_counted(
            window.hwnd,
            crate::window::text_search_host::replace_generation(window.hwnd),
            one_closed_match(),
        ),
    );
    assert!(!asked.get(), "nothing asked");
    assert_eq!(crate::window::modal::take_last_confirm(), None);
    assert!(!crate::window::text_search_host::replacing(window.hwnd));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle other");
    // The queued answer was never used: take it, so no later test gets it.
    assert!(!crate::window::modal::confirm(window.hwnd, "drain"));
    crate::window::modal::take_last_confirm();
}

#[test]
fn a_reload_leaves_a_tab_edited_and_saved_since_its_file_was_read() {
    // Break caught (review Minor 1): the user's edit, saved (by Ctrl+S or autosave) after the
    // reload read the file but before its text arrived, replaced in the editor by the older
    // file text, with the tab then claiming the older disk stamp.
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-reload-edited");
    let b = scratch.note("b.md", "b needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    std::fs::write(&b, "b pin").unwrap();
    let report = crate::library::text_replace::ReplaceReport {
        matches: 1,
        written: vec![(
            PathBuf::from("b.md"),
            crate::library::text_search::Stamp { size: 5, mtime: 1 },
        )],
        ..Default::default()
    };
    crate::window::text_search_host::replace_written(
        window.hwnd,
        crate::window::text_search_host::test_written(
            crate::window::text_search_host::replace_generation(window.hwnd),
            report,
        ),
    );
    // The reload has read "b pin" and posted it; it is not dispatched yet.
    let message = crate::window::WM_FASTPAD_REPLACE_RELOADED;
    pump_until_queued(window.hwnd, message);

    editor.set_text("b mine").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b mine");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
    pump_posted_messages(window.hwnd);
    let mut queued = MSG::default();
    assert_eq!(
        unsafe { PeekMessageW(&mut queued, window.hwnd, message, message, PM_NOREMOVE) },
        0,
        "the reload was dispatched"
    );
    assert_eq!(editor.text().unwrap(), "b mine", "the saved edit stays");
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().disk_stamp,
        crate::library::disk_stamp(&b)
    );
}

/// Waits, without dispatching anything, until `message` is queued for `hwnd`.
fn pump_until_queued(hwnd: HWND, message: u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW};
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut queued = MSG::default();
    while unsafe { PeekMessageW(&mut queued, hwnd, message, message, PM_NOREMOVE) } == 0 {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a_write_report_and_its_reloads_wait_for_a_file_population_to_end() {
    // Break caught (review Minor 6): a report's reloads swapping documents in the middle of a
    // file population or a modal loop, or a held report or reload lost so the tab keeps the
    // text from before the write.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-held-written");
    let a = scratch.note("a.md", "a needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    pump_posted_messages(window.hwnd);
    std::fs::write(&a, "a pin").unwrap();
    let report = crate::library::text_replace::ReplaceReport {
        matches: 1,
        written: vec![(
            PathBuf::from("a.md"),
            crate::library::text_search::Stamp { size: 5, mtime: 1 },
        )],
        ..Default::default()
    };
    let reported = || {
        notices(window.hwnd)
            .iter()
            .any(|notice| notice.starts_with("Replaced "))
    };
    let held = || crate::window::text_search_host::held_after_write(window.hwnd);

    app_mut(window.hwnd).populating_file = true;
    crate::window::text_search_host::replace_written(
        window.hwnd,
        crate::window::text_search_host::test_written(
            crate::window::text_search_host::replace_generation(window.hwnd),
            report,
        ),
    );
    assert_eq!(held(), (true, 0));
    crate::window::text_search_host::replace_timer(window.hwnd);
    assert_eq!(held(), (true, 0), "still populating");
    assert!(!reported());
    app_mut(window.hwnd).populating_file = false;
    crate::window::text_search_host::replace_timer(window.hwnd);
    assert_eq!(held(), (false, 0));
    assert!(reported());

    app_mut(window.hwnd).populating_file = true;
    pump_until(window.hwnd, || held().1 == 1);
    assert_eq!(
        editor.text().unwrap(),
        "a needle",
        "no reload during the population"
    );
    app_mut(window.hwnd).populating_file = false;
    crate::window::text_search_host::replace_timer(window.hwnd);
    assert_eq!(held(), (false, 0));
    assert_eq!(editor.text().unwrap(), "a pin");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

fn one_closed_match() -> crate::library::text_replace::ReplaceCount {
    crate::library::text_replace::ReplaceCount {
        matches: 1,
        notes: 1,
        closed_notes: 1,
    }
}

#[test]
fn a_count_of_an_earlier_replace_or_notebook_is_dropped() {
    // Break caught: the question asked, or the old notebook's notes written, for a count
    // that arrived after the user switched notebooks or after its replace was cancelled.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-stale");
    let a = scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    crate::window::modal::take_last_confirm();
    let host = |lparam| crate::window::text_search_host::replace_counted(window.hwnd, lparam);
    let generation = || crate::window::text_search_host::replace_generation(window.hwnd);
    let counted = |generation| {
        crate::window::text_search_host::test_counted(window.hwnd, generation, one_closed_match())
    };

    host(counted(generation().wrapping_sub(1)));
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        None,
        "an older one"
    );
    // What a notebook change does (`library_host`'s notebook switch calls it).
    let before_forget = generation();
    crate::window::text_search_host::forget(window.hwnd);
    host(counted(before_forget));
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        None,
        "from before a notebook change"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
    assert!(!crate::window::text_search_host::replacing(window.hwnd));

    // The same count with the current generation is asked about: the generation dropped it.
    let asked = decline_next_confirm();
    host(counted(generation()));
    assert!(asked.get());
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        Some(format!(
            "Replace 1 match in 1 note with \"pin\"?{SAVED_LINE}"
        ))
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
}

#[test]
fn a_count_that_arrives_while_a_file_is_populated_asks_once_it_ends() {
    // Break caught: the question (a nested modal loop) or a background-tab swap run in the
    // middle of a file population, or a held count lost so the replace never asks.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-held");
    let a = scratch.note("a.md", "needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    crate::window::modal::take_last_confirm();
    let counted = crate::window::text_search_host::test_counted(
        window.hwnd,
        crate::window::text_search_host::replace_generation(window.hwnd),
        one_closed_match(),
    );

    app_mut(window.hwnd).populating_file = true;
    crate::window::text_search_host::replace_counted(window.hwnd, counted);
    assert!(crate::window::text_search_host::replace_held(window.hwnd));
    crate::window::text_search_host::replace_timer(window.hwnd);
    assert!(
        crate::window::text_search_host::replace_held(window.hwnd),
        "still populating"
    );
    assert_eq!(
        crate::window::modal::take_last_confirm(),
        None,
        "no question during the population"
    );

    app_mut(window.hwnd).populating_file = false;
    let asked = decline_next_confirm();
    crate::window::text_search_host::replace_timer(window.hwnd);
    assert!(asked.get());
    assert!(!crate::window::text_search_host::replace_held(window.hwnd));
    assert!(!crate::window::text_search_host::replacing(window.hwnd));
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
}

#[test]
fn a_clean_tab_on_a_written_note_reloads_from_disk_and_a_dirty_one_keeps_its_text() {
    // Break caught: a clean tab opened while the write ran left showing the text from before
    // it (its next save would undo the replace), a dirty tab's unsaved edits dropped for the
    // file's text, or a reload that leaves the tab dirty or with an undo back to the old
    // text.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-reload");
    let a = scratch.note("a.md", "a needle");
    let b = scratch.note("b.md", "b needle");
    let c = scratch.note("c.md", "c needle");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    super::super::open_path(window.hwnd, &c).unwrap();
    pump_posted_messages(window.hwnd);
    editor.set_text("c typed").unwrap();
    let c_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    super::super::open_path(window.hwnd, &a).unwrap();
    pump_posted_messages(window.hwnd);
    let a_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    super::super::open_path(window.hwnd, &b).unwrap();
    pump_posted_messages(window.hwnd);
    let b_id = app_mut(window.hwnd).tabs.active().unwrap().id;
    // What the write did while those tabs opened.
    let mut written = Vec::new();
    for (path, text) in [(&a, "a pin"), (&b, "b pin"), (&c, "c pin")] {
        std::fs::write(path, text).unwrap();
        let relative = PathBuf::from(path.file_name().unwrap());
        let stamp = crate::library::text_search::Stamp {
            size: text.len() as u64,
            mtime: 1,
        };
        written.push((relative, stamp));
    }
    let report = crate::library::text_replace::ReplaceReport {
        matches: 3,
        written,
        ..Default::default()
    };

    crate::window::text_search_host::replace_written(
        window.hwnd,
        crate::window::text_search_host::test_written(
            crate::window::text_search_host::replace_generation(window.hwnd),
            report,
        ),
    );
    assert_eq!(
        notices(window.hwnd).last().map(String::as_str),
        Some("Replaced 3 matches in 3 notes.")
    );
    pump_until(window.hwnd, || editor.text().unwrap() == "b pin");
    let dirty = |id| app_mut(window.hwnd).tabs.document(id).unwrap().dirty;
    assert!(!dirty(b_id), "the active tab stays clean");
    assert_eq!(
        app_mut(window.hwnd).tabs.document(b_id).unwrap().disk_stamp,
        crate::library::disk_stamp(&b)
    );
    assert!(!editor.can_undo().unwrap(), "no undo back to the old text");

    assert!(super::super::activate_document_by_id(window.hwnd, a_id));
    assert_eq!(editor.text().unwrap(), "a pin", "a background tab too");
    assert!(!dirty(a_id));
    assert!(super::super::activate_document_by_id(window.hwnd, c_id));
    assert_eq!(
        editor.text().unwrap(),
        "c typed",
        "a dirty tab keeps its text"
    );
    assert!(dirty(c_id));
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "c pin");
}

#[test]
fn closing_the_window_waits_for_the_write_worker_to_end() {
    // Break caught (R-join): the write worker left running past the window's end, so a note
    // is written (or half written) after FastPad has closed.
    use crate::window::text_search_host::writer_hooks;
    struct Unpause;
    impl Drop for Unpause {
        fn drop(&mut self) {
            writer_hooks::set_pause(0);
        }
    }
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-join");
    let paths = (0..50)
        .map(|index| scratch.note(&format!("n{index:03}.md"), "needle"))
        .collect::<Vec<_>>();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    search_to_replace(window.hwnd, "needle", "pin");
    let asked = std::rc::Rc::new(std::cell::Cell::new(false));
    let answered = std::rc::Rc::clone(&asked);
    crate::window::answer_next_confirm(move |_| {
        answered.set(true);
        true
    });
    // The worker is still running when the window closes: it waits after its notes.
    let _unpause = Unpause;
    writer_hooks::set_pause(300);
    let ended = writer_hooks::ended();
    crate::window::text_search_host::replace_all(window.hwnd);
    pump_until(window.hwnd, || asked.get());
    assert_eq!(writer_hooks::ended(), ended, "still writing");

    drop(window);
    assert_eq!(
        writer_hooks::ended(),
        ended + 1,
        "WM_DESTROY waited for the worker"
    );
    let texts = || {
        paths
            .iter()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .collect::<Vec<_>>()
    };
    assert!(
        texts().iter().all(|text| text == "needle" || text == "pin"),
        "every note whole"
    );
}

#[test]
fn tab_cycles_the_search_box_the_replace_field_and_the_results() {
    // Break caught: Tab beeping in the box, never reaching the replace field or the results,
    // landing in the replace field while it is closed, or Shift+Tab not going back.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_TAB};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-tab");
    scratch.note("a.md", "alpha needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let panel = sidebar_panel(window.hwnd);

    execute_command(window.hwnd, CommandId::ReplaceInNotes);
    search_for(window.hwnd, "needle");
    let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
    let search_box = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    let focus = || unsafe { GetFocus() };
    let tab = |back: bool| press_with(focus(), VK_TAB, false, back, false);

    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(search_box) };
    tab(false);
    assert_eq!(focus(), replace, "box -> replace");
    tab(false);
    assert_eq!(focus(), panel, "replace -> results");
    tab(false);
    assert_eq!(focus(), search_box, "results -> box, wrapping");
    tab(true);
    assert_eq!(focus(), panel, "Shift+Tab: box -> results, wrapping");
    tab(true);
    assert_eq!(focus(), replace, "Shift+Tab: results -> replace");
    tab(true);
    assert_eq!(focus(), search_box, "Shift+Tab: replace -> box");

    crate::window::search_view::toggle_replace(window.hwnd);
    assert!(!crate::window::search_view::replace_open(window.hwnd));
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(search_box) };
    tab(false);
    assert_eq!(focus(), panel, "closed: box -> results");
    tab(false);
    assert_eq!(focus(), search_box, "closed: results -> box");
    tab(true);
    assert_eq!(focus(), panel, "closed: Shift+Tab box -> results");
    tab(true);
    assert_eq!(focus(), search_box, "closed: Shift+Tab results -> box");
}

#[test]
fn the_replace_buttons_and_their_keys_route_to_the_replace_and_open_nothing() {
    // Break caught: Ctrl+Shift+1 or a press on a row's replace button opening the result
    // instead, Ctrl+Alt+Enter in a field opening one, either running with the replace field
    // closed, or the row button and its key reaching different places.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("replace-routes");
    scratch.note("a.md", "alpha needle");
    scratch.note("b.md", "beta needle");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let panel = sidebar_panel(window.hwnd);
    let requests = || crate::window::search_view::replace_requests(window.hwnd);
    // Each request that runs starts a real replace of closed notes, whose question is
    // declined before the next request: while one runs, Replace all and the row buttons are
    // unavailable and a request is refused (below).
    let declined = |asked: std::rc::Rc<std::cell::Cell<bool>>| {
        pump_until(window.hwnd, || asked.get());
        assert!(!crate::window::text_search_host::replacing(window.hwnd));
        crate::window::modal::take_last_confirm()
    };
    let row_question =
        "Replace 1 match in \"b\" with \"\"? The note is saved and this can't be undone.";
    let all_question = format!("Replace 2 matches in 2 notes with \"\"?{SAVED_LINE}");

    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    search_for(window.hwnd, "needle");
    assert_eq!(search_rows(window.hwnd).len(), 2);
    unsafe { SetFocus(panel) };
    press_with(panel, u16::from(b'1'), true, true, false);
    assert_eq!(requests(), (Vec::new(), 0), "the replace field is closed");

    crate::window::search_view::toggle_replace(window.hwnd);
    unsafe { SetFocus(panel) };
    app_mut(window.hwnd)
        .sidebar
        .as_mut()
        .unwrap()
        .search
        .list
        .select(1, 400);
    let asked = decline_next_confirm();
    press_with(panel, u16::from(b'1'), true, true, false);
    assert_eq!(requests(), (vec![1], 0), "Ctrl+Shift+1 on the selected row");
    assert_eq!(
        app_mut(window.hwnd).tabs.preview_id(),
        None,
        "nothing opened"
    );
    // While that replace runs, another request is refused.
    assert!(crate::window::text_search_host::replacing(window.hwnd));
    press_with(panel, u16::from(b'1'), true, true, false);
    assert_eq!(requests(), (vec![1], 0), "refused while a replace runs");
    assert_eq!(
        declined(asked).as_deref(),
        Some(row_question),
        "the row's request ran the replace of that row's note"
    );

    let (width, height) = client_size(panel);
    let client = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
    let button = {
        let app = app_mut(window.hwnd);
        let view = &app.sidebar.as_ref().unwrap().search;
        let (row, _) = crate::window::sidebar_accessibility::row_rect(
            view.list_area(client, dpi),
            &view.list,
            1,
        );
        crate::window::search_view::SearchView::row_replace_rect(row, dpi)
    };
    let asked = decline_next_confirm();
    click(
        panel,
        (button.left + button.right) / 2,
        (button.top + button.bottom) / 2,
    );
    assert_eq!(requests(), (vec![1, 1], 0), "the row's button");
    assert_eq!(
        app_mut(window.hwnd).tabs.preview_id(),
        None,
        "nothing opened"
    );
    assert_eq!(declined(asked).as_deref(), Some(row_question));

    let all = crate::window::search_view::SearchView::replace_all_rect(client, dpi);
    let asked = decline_next_confirm();
    click(
        panel,
        (all.left + all.right) / 2,
        (all.top + all.bottom) / 2,
    );
    assert_eq!(requests(), (vec![1, 1], 1), "Replace all");
    assert_eq!(declined(asked), Some(all_question.clone()));

    let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
    let search_box = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    let asked = decline_next_confirm();
    press_with(replace, VK_RETURN, true, false, true);
    assert_eq!(declined(asked), Some(all_question.clone()));
    let asked = decline_next_confirm();
    press_with(search_box, VK_RETURN, true, false, true);
    assert_eq!(declined(asked), Some(all_question.clone()));
    // With Ctrl held Windows usually sends Ctrl+Alt+Enter as WM_KEYDOWN, not WM_SYSKEYDOWN.
    for field in [replace, search_box] {
        let asked = decline_next_confirm();
        press_as_keydown(field, VK_RETURN, true, true);
        assert_eq!(declined(asked), Some(all_question.clone()));
    }
    assert_eq!(
        requests(),
        (vec![1, 1], 5),
        "Ctrl+Alt+Enter in either field, as either message"
    );
    assert_eq!(
        app_mut(window.hwnd).tabs.preview_id(),
        None,
        "nothing opened"
    );
}

/// Sends `key` to `window` as `WM_KEYDOWN` with Ctrl and Alt held as given (Shift up).
fn press_as_keydown(window: HWND, key: u16, ctrl: bool, alt: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = if ctrl { 0x80 } else { 0 };
    keys[VK_SHIFT as usize] = 0;
    keys[VK_MENU as usize] = if alt { 0x80 } else { 0 };
    unsafe { SetKeyboardState(keys.as_ptr()) };
    unsafe { SendMessageW(window, WM_KEYDOWN, usize::from(key), 0) };
    unsafe { SetKeyboardState(original.as_ptr()) };
}

#[test]
fn ctrl_shift_f_escapes_the_selection_while_regex_is_on() {
    // Break caught: "a.b" searched as a pattern that also matches "axb".
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-prefill-regex");
    scratch.note("a.md", "see a.b here");
    scratch.note("x.md", "see axb here");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
    editor.populate_clean("see a.b here").unwrap();
    editor.set_selection(4..7).unwrap();

    execute_command(window.hwnd, CommandId::ShowSearchView);

    assert_eq!(
        crate::window::search_view::current_query(window.hwnd).map(|(query, _)| query),
        Some(r"a\.b".to_owned())
    );
    pump_until(window.hwnd, || {
        !crate::window::search_view::shown_results(window.hwnd).is_empty()
    });
    assert_eq!(
        crate::window::search_view::shown_results(window.hwnd),
        vec![("a".to_owned(), "see a.b here".to_owned())]
    );
}

#[test]
fn a_regex_prefill_with_hash_and_dash_opens_to_its_match_in_the_find_bar() {
    // Break caught (final review issue 1): `regex::escape` writes `\#` and `\-`, which the
    // find bar's old ECMAScript regex rejected, so the result opened to no match.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-prefill-regex-open");
    scratch.note("a.md", "see a-b#c here");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
    editor.populate_clean("x a-b#c").unwrap();
    editor.set_selection(2..7).unwrap();

    execute_command(window.hwnd, CommandId::ShowSearchView);
    assert_eq!(
        crate::window::search_view::current_query(window.hwnd).map(|(query, _)| query),
        Some(r"a\-b\#c".to_owned())
    );
    pump_until(window.hwnd, || {
        crate::window::search_view::shown_results(window.hwnd).len() == 1
    });

    crate::window::search_view::open_selected(window.hwnd, super::super::OpenMode::Preview, true);

    assert_eq!(editor.text().unwrap(), "see a-b#c here");
    assert_eq!(editor.selection().unwrap(), 4..9);
    let bar = app_mut(window.hwnd).find_bar().unwrap();
    assert!(bar.options().regex);
    assert!(!bar.no_match());
}
