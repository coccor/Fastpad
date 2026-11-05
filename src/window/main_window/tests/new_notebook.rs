//! File: New notebook… creates a folder from a Save-style dialog and opens it as the notebook.

use super::*;
use crate::window::modal::{answer_next_new_notebook_dialog, take_last_new_notebook_start};

#[test]
fn a_new_notebook_is_created_opened_and_left_empty() {
    // Break caught: the folder not created, the new notebook not added beside the open one (or
    // replacing it), the new one missing from recents, or FastPad writing files into a notebook
    // nobody has used yet (multi-root notebooks spec §3.3: opening adds a root).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-notebook");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let created = scratch.root.join("Recipes");
    answer_next_new_notebook_dialog({
        let created = created.clone();
        move |hwnd| {
            assert!(crate::window::modal::modal_active(hwnd));
            Some(created)
        }
    });

    execute_command(window.hwnd, CommandId::NewNotebook);

    assert_eq!(
        take_last_new_notebook_start(),
        Some(scratch.root.clone()),
        "the dialog starts beside the open notebook"
    );
    assert!(created.is_dir());
    let open: Vec<_> = crate::window::library_host::notebooks(window.hwnd)
        .into_iter()
        .map(|(_, folder)| folder)
        .collect();
    assert_eq!(open, vec![scratch.folder(), created.clone()]);
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(scratch.folder()),
        "the open notebook stays the default"
    );
    let recent =
        crate::library::local::read_folders(&crate::library::local::folders_file(&scratch.data()));
    assert_eq!(recent.folders.first(), Some(&created));
    let created_id = crate::window::library_host::notebooks(window.hwnd)[1].0;
    pump_until(window.hwnd, || {
        crate::window::library_host::with_notebook(window.hwnd, created_id, |_| ()).is_some()
    });
    assert_eq!(std::fs::read_dir(&created).unwrap().count(), 0);
}

#[test]
fn naming_an_existing_folder_asks_before_opening_it() {
    // Break caught: an existing folder opened without asking, or opened after Cancel.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-notebook-exists");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    let existing = scratch.root.join("Work");
    std::fs::create_dir_all(&existing).unwrap();

    answer_next_new_notebook_dialog({
        let existing = existing.clone();
        move |_| Some(existing)
    });
    crate::window::answer_next_confirm(|_| false);
    execute_command(window.hwnd, CommandId::NewNotebook);
    assert!(
        crate::window::modal::take_last_confirm()
            .is_some_and(|question| question.contains("already exists"))
    );
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);

    answer_next_new_notebook_dialog({
        let existing = existing.clone();
        move |_| Some(existing)
    });
    crate::window::answer_next_confirm(|_| true);
    execute_command(window.hwnd, CommandId::NewNotebook);
    assert_eq!(
        crate::window::modal::take_last_confirm_action().as_deref(),
        Some("Open")
    );
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(existing)
    );
    pump_until(window.hwnd, || {
        app_mut(window.hwnd).library.default_state().is_some()
    });
}

#[test]
fn naming_a_file_or_an_unreachable_place_explains_why_and_changes_nothing() {
    // Break caught: a file opened as a notebook, or a failed create that says nothing.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-notebook-file");
    let window = ProductionWindow::new(make_app());
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    let file = scratch.root.join("plain.txt");
    std::fs::write(&file, "x").unwrap();

    answer_next_new_notebook_dialog(move |_| Some(file));
    execute_command(window.hwnd, CommandId::NewNotebook);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("plain.txt") && n.contains("is a file")),
        "{:?}",
        notices(window.hwnd)
    );

    let unreachable = scratch.root.join("missing").join("Deeper");
    answer_next_new_notebook_dialog(move |_| Some(unreachable));
    execute_command(window.hwnd, CommandId::NewNotebook);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("could not create") && n.contains("Deeper")),
        "{:?}",
        notices(window.hwnd)
    );
    assert!(!scratch.root.join("missing").exists());
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
}

#[test]
fn cancel_changes_nothing_and_notes_mode_off_shows_no_dialog() {
    // Break caught: Cancel creating or opening anything, or the dialog shown while notes mode
    // is off and the folder could not be opened anyway.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-notebook-cancel");
    let window = ProductionWindow::new(make_app());
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());

    answer_next_new_notebook_dialog(|_| None);
    execute_command(window.hwnd, CommandId::NewNotebook);
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    assert!(
        notices(window.hwnd).is_empty(),
        "{:?}",
        notices(window.hwnd)
    );
    assert!(
        take_last_new_notebook_start().is_some_and(|start| start.is_dir()),
        "with no notebook open the dialog still starts somewhere real"
    );

    app_mut(window.hwnd).settings.notes_mode = false;
    execute_command(window.hwnd, CommandId::NewNotebook);
    assert_eq!(take_last_new_notebook_start(), None, "no dialog");
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("Notes mode is off"))
    );
}

#[test]
fn the_no_notebook_panel_offers_new_notebook_under_open_notebook() {
    // Break caught: the button missing for screen readers, or a click on it opening the folder
    // picker instead of the New notebook dialog.
    use windows_sys::Win32::Foundation::LPARAM;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-notebook-panel");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::notebook_view::rebuild(window.hwnd);
    assert_eq!(
        notebook_view(window.hwnd).mode,
        crate::window::notebook_view::Mode::NoNotebook
    );
    let panel = notebook_view(window.hwnd).panel;
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let buttons = notebook_view(window.hwnd).buttons(client, dpi);
    let names: Vec<&str> = buttons.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        names.ends_with(&["Open notebook…", "New notebook…"]),
        "{names:?}"
    );

    let created = scratch.root.join("Fresh");
    answer_next_new_notebook_dialog({
        let created = created.clone();
        move |_| Some(created)
    });
    let new = buttons[buttons.len() - 1].1;
    let lparam = ((((new.top + new.bottom) / 2) as u32) << 16 | ((new.left + new.right) / 2) as u32)
        as LPARAM;
    unsafe {
        SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
        SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
    }
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(created)
    );
    pump_until(window.hwnd, || {
        app_mut(window.hwnd).library.default_state().is_some()
    });
}
