//! Daily notes, templates and `notebook.ini` (second brain phase 1 spec §9).

use super::*;
use crate::library::dates::{Date, Time};

fn friday(window: &ProductionWindow) {
    let _ = window;
    crate::window::library_host::set_now(
        Date::new(2026, 10, 16).unwrap(),
        Time {
            hour: 9,
            minute: 30,
        },
    );
}

fn active_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    app_mut(hwnd).tabs.active()?.path.clone()
}

fn caret(editor: &crate::editor::Editor) -> usize {
    editor.selection().unwrap().start
}

#[test]
fn ctrl_alt_d_makes_todays_note_once_and_then_opens_it() {
    // Break caught: a second press making another note, or today's note left unlisted.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("daily-today");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");
    friday(&window);

    execute_command(window.hwnd, CommandId::NotesToday);

    let today = scratch.folder().join(r"journal\2026-10-16.md");
    let text = "# Friday, 16 October 2026\n\n";
    assert_eq!(std::fs::read_to_string(&today).unwrap(), text);
    assert_eq!(active_path(window.hwnd).as_deref(), Some(today.as_path()));
    assert_eq!(caret(&editor), text.len(), "the caret starts at the end");
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_listed(std::path::Path::new(r"journal\2026-10-16.md")));
    });

    editor.set_text("# Friday\n\nwrote this").unwrap();
    assert!(super::super::save_active_document(window.hwnd));
    open_note(&window, &scratch, "b.md", "");
    execute_command(window.hwnd, CommandId::NotesToday);
    assert_eq!(active_path(window.hwnd).as_deref(), Some(today.as_path()));
    assert_eq!(
        std::fs::read_to_string(&today).unwrap(),
        "# Friday\n\nwrote this"
    );
}

#[test]
fn a_daily_template_and_folder_from_notebook_ini_shape_todays_note() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("daily-template");
    std::fs::create_dir_all(scratch.folder().join(r"templates")).unwrap();
    std::fs::create_dir_all(scratch.folder().join(".fastpad")).unwrap();
    scratch.note(
        r".fastpad\notebook.ini",
        "daily_folder=days\ndaily_format=YYYY-MM-DD ddd\ndaily_template=templates/day.md\n",
    );
    scratch.note(
        r"templates\day.md",
        "---\ncreated: {{date:YYYY-MM-DD}}\n---\n# {{title}}\n\n- {{time}} {{cursor}}\n",
    );
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");
    friday(&window);

    execute_command(window.hwnd, CommandId::NotesToday);

    let today = scratch.folder().join(r"days\2026-10-16 Fri.md");
    let text = std::fs::read_to_string(&today).unwrap();
    assert_eq!(
        text,
        "---\ncreated: 2026-10-16\n---\n# 2026-10-16 Fri\n\n- 09:30 \n"
    );
    assert_eq!(caret(&editor), text.len() - 1, "the caret is at {{cursor}}");
    let ours: Vec<String> = notices(window.hwnd)
        .into_iter()
        .filter(|notice| notice.contains("template") || notice.contains("notebook.ini"))
        .collect();
    assert!(ours.is_empty(), "{ours:?}");
}

#[test]
fn previous_and_next_walk_the_daily_notes_by_date() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("daily-walk");
    std::fs::create_dir_all(scratch.folder().join("journal")).unwrap();
    let tenth = scratch.note(r"journal\2026-10-10.md", "10");
    let fourteenth = scratch.note(r"journal\2026-10-14.md", "14");
    scratch.note(r"journal\ideas.md", "not a day");
    let window = ProductionWindow::new(make_app());
    install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");
    friday(&window);

    execute_command(window.hwnd, CommandId::NotesPreviousDaily);
    assert_eq!(
        active_path(window.hwnd).as_deref(),
        Some(fourteenth.as_path()),
        "from today"
    );
    execute_command(window.hwnd, CommandId::NotesPreviousDaily);
    assert_eq!(active_path(window.hwnd).as_deref(), Some(tenth.as_path()));
    app_mut(window.hwnd).notifications.dismiss_all();
    execute_command(window.hwnd, CommandId::NotesPreviousDaily);
    assert_eq!(active_path(window.hwnd).as_deref(), Some(tenth.as_path()));
    assert_eq!(notices(window.hwnd), ["There is no earlier daily note."]);
    execute_command(window.hwnd, CommandId::NotesNextDaily);
    assert_eq!(
        active_path(window.hwnd).as_deref(),
        Some(fourteenth.as_path())
    );
}

#[test]
fn daily_notes_need_a_notebook() {
    let window = ProductionWindow::new(make_app());
    execute_command(window.hwnd, CommandId::NotesToday);
    assert_eq!(
        notices(window.hwnd),
        ["Open a notebook to use daily notes."]
    );
}

#[test]
fn a_bad_notebook_ini_line_is_named_and_its_default_used() {
    // Break caught: a typo in notebook.ini silently ignored, or breaking the notebook.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("notebook-ini-bad");
    std::fs::create_dir_all(scratch.folder().join(".fastpad")).unwrap();
    scratch.note(r".fastpad\notebook.ini", "version=1\ndaily_format=MM-DD\n");
    let window = ProductionWindow::new(make_app());
    install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");

    assert!(
        notices(window.hwnd)
            .contains(&"notebook.ini line 2: invalid value for daily_format: \"MM-DD\"".to_owned())
    );
    assert_eq!(
        crate::window::library_host::notebook_settings(window.hwnd).daily_format,
        "YYYY-MM-DD"
    );
    app_mut(window.hwnd).notifications.dismiss_all();
    rescan_and_wait(window.hwnd);
    assert!(notices(window.hwnd).is_empty(), "said once per change");
}

#[test]
fn editing_notebook_settings_makes_a_commented_file_and_opens_it() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("notebook-ini-edit");
    let window = ProductionWindow::new(make_app());
    install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");

    execute_command(window.hwnd, CommandId::NotesEditNotebookSettings);

    let ini = scratch.folder().join(r".fastpad\notebook.ini");
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        crate::library::notebook_ini::starter_text()
    );
    assert_eq!(active_path(window.hwnd).as_deref(), Some(ini.as_path()));
}

#[test]
fn a_new_note_from_a_template_fills_it_in_where_the_folder_menu_asked() {
    // Break caught: one page type per template having to be copied and edited by hand.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("template-new");
    std::fs::create_dir_all(scratch.folder().join(r".fastpad\templates")).unwrap();
    std::fs::create_dir_all(scratch.folder().join("people")).unwrap();
    scratch.note(
        r".fastpad\templates\Person.md",
        "# {{title}}\n\nRole: {{cursor}}\n",
    );
    scratch.note(r".fastpad\templates\Meeting.md", "# {{title}}\n");
    scratch.note(r"people\Ann.md", "");
    let (window, editor) = notebook_window(&scratch);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    pump_until(window.hwnd, || {
        crate::window::library_host::index_loaded(window.hwnd)
    });
    friday(&window);

    crate::window::library_host::new_note_from_template(window.hwnd, Some("people".into()));
    let shown = app_mut(window.hwnd)
        .library
        .shown_templates
        .as_ref()
        .map(|(_, found)| found.len());
    assert_eq!(shown, Some(2), "both templates are offered");
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::Template,
        crate::window::command_palette::PickerChoice::Item(1),
    );
    type_into_field(window.hwnd, "Bob");
    field_key(window.hwnd, VK_RETURN);

    let bob = scratch.folder().join(r"people\Bob.md");
    let text = "# Bob\n\nRole: \n";
    assert_eq!(std::fs::read_to_string(&bob).unwrap(), text);
    assert_eq!(active_path(window.hwnd).as_deref(), Some(bob.as_path()));
    assert_eq!(caret(&editor), text.len() - 1);
}

#[test]
fn an_empty_templates_folder_says_where_templates_go() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("template-none");
    let window = ProductionWindow::new(make_app());
    install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");
    app_mut(window.hwnd).notifications.dismiss_all();

    execute_command(window.hwnd, CommandId::NotesNewFromTemplate);

    assert_eq!(
        notices(window.hwnd),
        ["No templates in .fastpad\\templates. Add .md files there to use them."]
    );
}
