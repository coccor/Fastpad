//! The Notebook view's "+" in its title band: Open notebook…, New notebook…, and the recent
//! notebooks that aren't open, for a user who doesn't know more than one can be open.

use super::*;
use crate::window::library_host;
use crate::window::notebook_view::{self, AddChoice};
use windows_sys::Win32::UI::WindowsAndMessaging::{HTTRANSPARENT, WM_NCHITTEST};

fn write_recent(hwnd: HWND, data: &std::path::Path, folders: Vec<std::path::PathBuf>) {
    crate::library::local::write_folders(
        &crate::library::local::folders_file(data),
        &crate::library::local::RecentFolders {
            folders,
            ..Default::default()
        },
    )
    .unwrap();
    library_host::forget_folders_cache(hwnd);
}

/// One open notebook, A, with the sidebar's Notebook view shown.
fn one_open(label: &str) -> super::NotebooksAB {
    let ab = notebooks_a_and_b(label, "a", "b");
    assert!(library_host::close_notebook_of(ab.window.hwnd, ab.b_id));
    crate::window::side_panel::show_view(
        ab.window.hwnd,
        crate::config::SidebarView::Notebook,
        false,
    );
    crate::window::notebook_view::rebuild(ab.window.hwnd);
    ab
}

fn centre_of(rect: windows_sys::Win32::Foundation::RECT) -> (i32, i32) {
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

fn click_add(hwnd: HWND) {
    let panel = sidebar_windows(hwnd).1;
    let (x, y) = centre_of(notebook_view::add_button_rect(hwnd).unwrap());
    click(panel, x, y);
}

#[test]
fn the_add_button_sits_at_the_right_of_the_title_band_and_is_not_caption() {
    // Break caught: a button the window's drag area swallows, so clicks move the window.
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-place");
    let hwnd = ab.window.hwnd;
    let panel = sidebar_windows(hwnd).1;
    let button = notebook_view::add_button_rect(hwnd).unwrap();
    let (client_width, _) = client_size(panel);
    assert!(button.right <= client_width && button.left > client_width / 2);
    assert!(
        button.top >= 0 && button.bottom <= 38 * 2,
        "in the title band"
    );
    let hit = |x: i32, y: i32| unsafe {
        SendMessageW(panel, WM_NCHITTEST, 0, screen_lparam(panel, x, y))
    };
    let (x, y) = centre_of(button);
    assert_ne!(hit(x, y), HTTRANSPARENT as isize, "the button takes clicks");
    assert_eq!(
        hit(12, y),
        HTTRANSPARENT as isize,
        "the rest of the band still drags the window"
    );
}

#[test]
fn the_menu_lists_open_new_then_recent_notebooks_that_are_not_open() {
    // Break caught: a menu offering the notebook that is already open, or hiding the recent
    // ones a user would want back.
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-items");
    let hwnd = ab.window.hwnd;
    let (r1, r2) = (
        ab.scratch.root.join("Recipes"),
        ab.scratch.root.join("Archive"),
    );
    std::fs::create_dir_all(&r1).unwrap();
    std::fs::create_dir_all(&r2).unwrap();
    write_recent(hwnd, &ab.scratch.data(), vec![ab.a.clone(), r1, r2]);

    notebook_view::answer_next_add_menu(|_| None);
    click_add(hwnd);

    assert_eq!(
        notebook_view::last_add_menu(),
        [
            "Open notebook…\tCtrl+Shift+O",
            "New notebook…",
            "",
            "Recipes",
            "Archive",
        ]
    );
}

#[test]
fn with_nothing_recent_the_menu_has_only_open_and_new() {
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-nothing-recent");
    let hwnd = ab.window.hwnd;
    write_recent(hwnd, &ab.scratch.data(), vec![ab.a.clone()]);
    notebook_view::answer_next_add_menu(|_| None);
    click_add(hwnd);
    assert_eq!(
        notebook_view::last_add_menu(),
        ["Open notebook…\tCtrl+Shift+O", "New notebook…"]
    );
}

#[test]
fn more_than_five_recent_ends_with_more_recent_notebooks() {
    use crate::window::command_palette::PickerKind;
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-more");
    let hwnd = ab.window.hwnd;
    let recent: Vec<_> = (1..=7)
        .map(|n| {
            let folder = ab.scratch.root.join(format!("R{n}"));
            std::fs::create_dir_all(&folder).unwrap();
            folder
        })
        .collect();
    write_recent(hwnd, &ab.scratch.data(), recent);

    notebook_view::answer_next_add_menu(|items| {
        items.iter().position(|item| *item == AddChoice::MoreRecent)
    });
    click_add(hwnd);

    let items = notebook_view::last_add_menu();
    assert_eq!(items.len(), 2 + 1 + 5 + 1);
    assert_eq!(items.last().unwrap(), "More recent notebooks…");
    let kind = with_command_palette(hwnd, |p| p.picker().map(|picker| picker.kind)).flatten();
    assert_eq!(kind, Some(PickerKind::RecentFolder));
}

#[test]
fn open_notebook_from_the_menu_adds_a_notebook() {
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-open");
    let hwnd = ab.window.hwnd;
    let c = ab.scratch.root.join("C");
    std::fs::create_dir_all(&c).unwrap();
    crate::window::answer_next_folder_dialog({
        let c = c.clone();
        move |_| Some(c)
    });
    notebook_view::answer_next_add_menu(|items| {
        items.iter().position(|item| *item == AddChoice::Open)
    });
    click_add(hwnd);
    pump_until(hwnd, || {
        library_host::notebooks(hwnd)
            .iter()
            .any(|(_, folder)| crate::library::model::same_path(folder, &c))
    });
    assert_eq!(library_host::notebooks(hwnd).len(), 2);
}

#[test]
fn a_recent_notebook_from_the_menu_opens_beside_the_others() {
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-recent");
    let hwnd = ab.window.hwnd;
    write_recent(hwnd, &ab.scratch.data(), vec![ab.a.clone(), ab.b.clone()]);
    notebook_view::answer_next_add_menu(|items| {
        items
            .iter()
            .position(|item| matches!(item, AddChoice::Recent(_)))
    });
    click_add(hwnd);
    pump_until(hwnd, || library_host::notebooks(hwnd).len() == 2);
    assert!(
        library_host::notebooks(hwnd)
            .iter()
            .any(|(_, folder)| crate::library::model::same_path(folder, &ab.b))
    );
}

#[test]
fn new_notebook_from_the_menu_runs_new_notebook() {
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-new");
    let hwnd = ab.window.hwnd;
    let fresh = ab.scratch.root.join("Fresh");
    crate::window::modal::answer_next_new_notebook_dialog({
        let fresh = fresh.clone();
        move |_| Some(fresh)
    });
    notebook_view::answer_next_add_menu(|items| {
        items.iter().position(|item| *item == AddChoice::New)
    });
    click_add(hwnd);
    pump_until(hwnd, || {
        library_host::notebooks(hwnd)
            .iter()
            .any(|(_, folder)| crate::library::model::same_path(folder, &fresh))
    });
}

#[test]
fn the_button_is_a_named_accessible_button_with_a_tooltip() {
    // Break caught: a control a screen reader can't find or name.
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-a11y");
    let hwnd = ab.window.hwnd;
    let view = notebook_view(hwnd);
    let (client, dpi) = view.client_and_dpi_for_test();
    let buttons = view.buttons(client, dpi);
    assert_eq!(buttons[0].0, "Add notebook");
    let (rect, add) = (buttons[0].1, notebook_view::add_button_rect(hwnd).unwrap());
    assert_eq!(
        (rect.left, rect.top, rect.right, rect.bottom),
        (add.left, add.top, add.right, add.bottom)
    );
    assert_eq!(notebook_view::ADD_NOTEBOOK_TIP, "Open or create a notebook");
}

#[test]
fn the_button_shows_with_no_notebook_open_too() {
    let _scintilla = load_native_scintilla();
    let ab = one_open("add-button-none");
    let hwnd = ab.window.hwnd;
    assert!(library_host::close_notebook_of(hwnd, ab.a_id));
    crate::window::notebook_view::rebuild(hwnd);
    assert!(notebook_view::add_button_rect(hwnd).is_some());
    let view = notebook_view(hwnd);
    let (client, dpi) = view.client_and_dpi_for_test();
    assert_eq!(view.buttons(client, dpi)[0].0, "Add notebook");
}
