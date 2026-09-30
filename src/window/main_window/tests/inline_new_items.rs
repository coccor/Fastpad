//! Inline naming of new folders and notes in the tree.

use super::*;

#[test]
fn new_folder_from_the_header_names_it_in_an_empty_draft_row_and_selects_the_new_row() {
    // Break caught: the header button opening the name bar, a "New folder" prefill, a
    // folder made before Enter or on Escape, the draft left behind, or the new folder not
    // selected with the focus in the tree (inline naming spec §3.2, §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-folder");
    scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    select_row(window.hwnd, &RowKind::Note("top.md".into()));
    let panel = sidebar_windows(window.hwnd).1;
    let count = crate::window::side_panel::accessible_item_count(panel);
    assert!(
        (0..count)
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .any(|item| item.name == "New folder"),
        "the header button has its accessible name"
    );
    let press = || {
        crate::window::notebook_view::header_clicked(
            window.hwnd,
            crate::window::notebook_view::HeaderButton::NewFolder,
        );
    };

    press();
    assert_eq!(draft_row(window.hwnd), Some((0, 0)), "first at the root");
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(crate::window::inline_name::Purpose::NewFolder(
            std::path::PathBuf::new()
        ))
    );
    assert_eq!(field_text(window.hwnd), "", "the field starts empty");
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert!(
        !app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.is_visible())
    );
    type_into_field(window.hwnd, "Plans");
    field_key(window.hwnd, VK_ESCAPE);
    assert!(!inline_open(window.hwnd));
    assert_eq!(
        draft_row(window.hwnd),
        None,
        "Escape takes the draft row away"
    );
    assert!(
        !scratch.folder().join("Plans").exists(),
        "Escape creates nothing"
    );
    assert_eq!(unsafe { GetFocus() }, panel, "Escape returns to the tree");

    press();
    type_into_field(window.hwnd, "Plans");
    assert!(
        !scratch.folder().join("Plans").exists(),
        "nothing before Enter"
    );
    field_key(window.hwnd, VK_RETURN);

    assert!(!inline_open(window.hwnd));
    assert!(scratch.folder().join("Plans").is_dir());
    assert_eq!(draft_row(window.hwnd), None);
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder("Plans".into()))
    );
    assert_eq!(unsafe { GetFocus() }, panel, "the focus stays in the tree");
}

#[test]
fn new_folder_here_drafts_inside_that_folder_and_a_taken_name_keeps_the_field_open() {
    // Break caught: "New folder here" drafting at the root, the folder left collapsed, a
    // name taken by a listed note missed while typing, Enter accepted over the message, or
    // a clash with an unlisted file closing the field (spec §4.4, §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-folder-here");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    std::fs::write(scratch.folder().join(r"sub\notes.bin"), "x").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), false);
    crate::window::notebook_view::rebuild(window.hwnd);
    let index = row_of(window.hwnd, &RowKind::Folder("sub".into()));
    crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteNewFolder));
    crate::window::notebook_view::open_context_menu(window.hwnd, index, None);

    assert_eq!(
        draft_row(window.hwnd),
        Some((index + 1, 1)),
        "its first child"
    );
    assert!(
        crate::window::library_host::expanded(window.hwnd)
            .contains(&std::path::PathBuf::from("sub"))
    );
    type_into_field(window.hwnd, "A.md");
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("A.md already exists here.")
    );
    field_key(window.hwnd, VK_RETURN);
    assert!(
        inline_open(window.hwnd),
        "Enter is refused while a problem shows"
    );
    assert!(!scratch.folder().join(r"sub\A.md").is_dir());

    type_into_field(window.hwnd, "notes.bin");
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd),
        None,
        "not listed"
    );
    field_key(window.hwnd, VK_RETURN);
    assert!(inline_open(window.hwnd));
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("A folder or file named \u{201c}notes.bin\u{201d} already exists")
    );

    type_into_field(window.hwnd, "Plans");
    assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
    field_key(window.hwnd, VK_RETURN);
    assert!(!inline_open(window.hwnd));
    assert!(scratch.folder().join(r"sub\Plans").is_dir());
    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Folder(r"sub\Plans".into()))
    );
}

#[test]
fn a_typed_folder_name_is_sanitized_hidden_names_are_refused_and_an_empty_one_cancels() {
    // Break caught: a name Windows refuses failing with a path error, "..." showing a
    // message instead of cancelling, or a .git or node_modules folder that the next rescan
    // hides (spec §4.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-folder-names");
    scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    let create = |typed: &str| {
        crate::window::inline_name::new_folder(window.hwnd, Some(std::path::PathBuf::new()));
        type_into_field(window.hwnd, typed);
        field_key(window.hwnd, VK_RETURN);
    };

    create(" a/b: c?. ");
    assert!(scratch.folder().join("ab c").is_dir());
    create("CON");
    assert!(scratch.folder().join("CON_").is_dir());
    assert!(!inline_open(window.hwnd));

    create("...");
    assert!(
        !inline_open(window.hwnd),
        "nothing left of the name cancels"
    );
    assert_eq!(draft_row(window.hwnd), None);

    for hidden in [".git", "node_modules"] {
        create(hidden);
        assert!(inline_open(window.hwnd), "{hidden}");
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd),
            Some(format!(
                "FastPad hides folders named \u{201c}{hidden}\u{201d}. Choose another name."
            ))
        );
        assert!(!scratch.folder().join(hidden).exists());
        crate::window::inline_name::cancel(window.hwnd);
    }
}

#[test]
fn a_new_folder_draft_survives_a_rescan_but_goes_with_its_folder_or_notebook() {
    // Break caught: a rescan dropping the draft row and what was typed, a draft left in a
    // folder deleted in Explorer, or one outliving its notebook (spec §5.4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-folder-rescan");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::notebook_view::rebuild(window.hwnd);
    crate::window::inline_name::new_folder(window.hwnd, Some("sub".into()));
    type_into_field(window.hwnd, "Typed");

    rescan_and_wait(window.hwnd);
    assert!(inline_open(window.hwnd));
    assert_eq!(field_text(window.hwnd), "Typed");
    let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
    assert_eq!(draft_row(window.hwnd), Some((sub + 1, 1)));

    std::fs::remove_dir_all(scratch.folder().join("sub")).unwrap();
    rescan_and_wait(window.hwnd);
    assert!(!inline_open(window.hwnd));
    assert_eq!(draft_row(window.hwnd), None);

    crate::window::inline_name::new_folder(window.hwnd, None);
    assert!(inline_open(window.hwnd));
    crate::window::library_host::close_notebook(window.hwnd);
    assert!(!inline_open(window.hwnd));
}

#[test]
fn the_field_sits_over_its_row_scrolls_with_it_and_is_left_out_of_the_tree_for_screen_readers() {
    // Break caught: the field drawn away from its row or over the header, left behind
    // when the list scrolls, losing the typing when scrolled out of view, or the draft row
    // read out as an empty tree item (spec §5.4, §6).
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::MapWindowPoints;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, GetWindowRect, WM_MOUSEWHEEL, WS_VISIBLE,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-placement");
    for index in 0..80 {
        scratch.note(&format!("n{index:02}.md"), "x");
    }
    let (window, _editor) = notebook_window(&scratch);
    let panel = sidebar_windows(window.hwnd).1;
    let names = || {
        let count = crate::window::side_panel::accessible_item_count(panel);
        (0..count)
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .map(|item| item.name)
            .collect::<Vec<_>>()
    };
    let before = names();
    // The test window is never shown, so the field's own style says whether it shows.
    let shown =
        |field: HWND| (unsafe { GetWindowLongPtrW(field, GWL_STYLE) } as u32) & WS_VISIBLE != 0;

    crate::window::inline_name::new_folder(window.hwnd, None);
    let field = inline_field(window.hwnd);
    assert_eq!(names(), before, "the draft row is no MSAA item");
    assert!(shown(field));
    let draft = draft_row(window.hwnd).unwrap().0;
    let row = notebook_view(window.hwnd).row_rect_at(draft).unwrap();
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(field, &mut rect);
        MapWindowPoints(
            std::ptr::null_mut(),
            panel,
            &mut rect as *mut RECT as *mut POINT,
            2,
        );
    }
    assert!(
        rect.top >= row.top && rect.bottom <= row.bottom,
        "{}..{} in {}..{}",
        rect.top,
        rect.bottom,
        row.top,
        row.bottom
    );

    let down = ((-(120_i16 * 20)) as u16 as usize) << 16;
    unsafe { SendMessageW(panel, WM_MOUSEWHEEL, down, 0) };
    assert!(notebook_view(window.hwnd).list.top > 0, "the list scrolled");
    assert!(!shown(field), "out of view, hidden");
    assert_eq!(unsafe { GetFocus() }, field, "and still editing");
    type_into_field(window.hwnd, "Kept");
    let up = ((120_i16 * 20) as u16 as usize) << 16;
    unsafe { SendMessageW(panel, WM_MOUSEWHEEL, up, 0) };
    assert!(shown(field), "back in view");
    assert_eq!(field_text(window.hwnd), "Kept");
}

#[test]
fn the_name_field_is_named_for_screen_readers_and_its_problem_is_its_description() {
    // Break caught: a field a screen reader announces as a bare "edit", or a problem it
    // never hears (spec §6).
    use crate::window::accessibility::{
        AccessibleVtable, IID_IACCESSIBLE, RawVariant, VariantValue,
    };
    use crate::window::sidebar_accessibility::take_raised;
    use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};
    use windows_sys::Win32::UI::Accessibility::AccessibleObjectFromWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_DESCRIPTIONCHANGE, OBJID_CLIENT,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-accessible");
    std::fs::create_dir_all(scratch.folder().join(r"sub\Taken")).unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::inline_name::new_folder(window.hwnd, Some("sub".into()));
    let field = inline_field(window.hwnd);
    let read = |description: bool| -> String {
        let com = unsafe {
            windows_sys::Win32::System::Com::CoInitializeEx(
                std::ptr::null(),
                windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
            )
        };
        let mut object = std::ptr::null_mut();
        let result = unsafe {
            AccessibleObjectFromWindow(field, OBJID_CLIENT as u32, &IID_IACCESSIBLE, &mut object)
        };
        assert!(result >= 0 && !object.is_null(), "{result:#x}");
        let vtable = unsafe { &**(object as *const *const AccessibleVtable) };
        let get = if description {
            vtable.get_acc_description
        } else {
            vtable.get_acc_name
        };
        let mut text = std::ptr::null();
        unsafe { get(object, RawVariant::integer(0), &mut text) };
        let value = if text.is_null() {
            String::new()
        } else {
            let units = unsafe { std::slice::from_raw_parts(text, SysStringLen(text) as usize) };
            let value = String::from_utf16_lossy(units);
            unsafe { SysFreeString(text) };
            value
        };
        unsafe { (vtable.release)(object) };
        if com >= 0 {
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
        value
    };
    assert_eq!(read(false), "New folder name, in sub");

    take_raised();
    type_into_field(window.hwnd, "taken");
    assert!(
        take_raised().contains(&(field as usize, EVENT_OBJECT_DESCRIPTIONCHANGE, 0)),
        "the problem is announced"
    );
    assert_eq!(read(true), "taken already exists here.");
}

#[test]
fn ctrl_a_selects_the_name_and_ctrl_backspace_deletes_a_word_in_the_field() {
    // Break caught: Ctrl+A doing nothing in the field, or Ctrl+Backspace typing a box
    // character instead of deleting the word before the caret (spec §5.1).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_BACK, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-keys");
    scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    crate::window::inline_name::new_folder(window.hwnd, None);
    type_into_field(window.hwnd, "my note.md");
    let field = inline_field(window.hwnd);
    unsafe { SendMessageW(field, windows_sys::Win32::UI::Controls::EM_SETSEL, 10, 10) };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };

    field_key(window.hwnd, VK_BACK);
    unsafe { SendMessageW(field, WM_CHAR, 0x7f, 0) };
    let after_backspace = field_text(window.hwnd);
    field_key(window.hwnd, u16::from(b'A'));
    unsafe { SendMessageW(field, WM_CHAR, 0x01, 0) };
    let selection = field_selection(window.hwnd);
    unsafe { SetKeyboardState(original.as_ptr()) };

    assert_eq!(after_backspace, "my note.");
    assert_eq!(selection, (0, 8));
    assert_eq!(
        field_text(window.hwnd),
        "my note.",
        "no control characters typed"
    );
}

#[test]
fn an_empty_folder_made_on_disk_appears_after_a_rescan_and_goes_with_it() {
    // Break caught: a folder made in Explorer staying invisible until it holds a note, or a
    // folder deleted in Explorer keeping its row (spec §3.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("empty-folder-rescan");
    scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);

    std::fs::create_dir(scratch.folder().join("Fresh")).unwrap();
    rescan_and_wait(window.hwnd);
    row_of(window.hwnd, &RowKind::Folder("Fresh".into()));

    std::fs::remove_dir(scratch.folder().join("Fresh")).unwrap();
    rescan_and_wait(window.hwnd);
    assert!(
        crate::library::tree::row_index(
            &notebook_view(window.hwnd).rows,
            &RowKind::Folder("Fresh".into())
        )
        .is_none()
    );
}

#[test]
fn the_palette_offers_new_note_and_new_folder_only_while_a_notebook_is_open() {
    // Break caught: "Notebook: New note…" or "Notebook: New folder…" listed with no
    // notebook, where they can only say "Open a notebook first." (inline naming spec §3.1).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("palette-new-note");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let listed = |command: CommandId| {
        execute_command(window.hwnd, CommandId::CommandPalette);
        let listed = app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .shown()
            .iter()
            .any(|entry| entry.command == command);
        super::super::close_command_palette(window.hwnd, false);
        listed
    };
    assert!(crate::window::library_host::folder(window.hwnd).is_none());
    assert!(!listed(CommandId::NoteNew));
    assert!(!listed(CommandId::NoteNewFolder));
    scratch.install(window.hwnd);
    assert!(listed(CommandId::NoteNew));
    assert!(listed(CommandId::NoteNewFolder));
}

#[test]
fn plus_new_note_here_and_the_palette_each_draft_a_note_in_the_right_folder() {
    // Break caught: "+" or "New note here" opening an untitled tab instead, a draft in the
    // wrong folder or at the wrong depth, a field not focused, or the palette ignoring the
    // selected row's folder (inline naming spec §3.1).
    use crate::window::inline_name::Purpose;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-note-starts");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "b");
    scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    let tabs = super::super::tab_count(window.hwnd);
    select_row(window.hwnd, &RowKind::Note("top.md".into()));

    crate::window::notebook_view::header_clicked(
        window.hwnd,
        crate::window::notebook_view::HeaderButton::NewNote,
    );
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(Purpose::NewNote(std::path::PathBuf::new()))
    );
    assert_eq!(draft_row(window.hwnd), Some((0, 0)));
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert_eq!(
        super::super::tab_count(window.hwnd),
        tabs,
        "no untitled tab"
    );
    field_key(window.hwnd, VK_ESCAPE);
    assert_eq!(draft_row(window.hwnd), None);

    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), false);
    crate::window::notebook_view::rebuild(window.hwnd);
    let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
    crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteNew));
    crate::window::notebook_view::open_context_menu(window.hwnd, sub, None);
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(Purpose::NewNote("sub".into()))
    );
    assert_eq!(draft_row(window.hwnd), Some((sub + 1, 1)), "sub expanded");
    field_key(window.hwnd, VK_ESCAPE);

    select_row(window.hwnd, &RowKind::Note(r"sub\b.md".into()));
    execute_command(window.hwnd, CommandId::CommandPalette);
    let query = app_mut(window.hwnd)
        .command_palette
        .as_ref()
        .unwrap()
        .query_hwnd();
    let typed = crate::platform::wide_null("Notebook: New note");
    unsafe { SetWindowTextW(query, typed.as_ptr()) };
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };
    assert_eq!(
        crate::window::inline_name::purpose(window.hwnd),
        Some(Purpose::NewNote("sub".into())),
        "the selected note's folder"
    );
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert_eq!(super::super::tab_count(window.hwnd), tabs);
}

#[test]
fn enter_on_a_new_note_creates_the_file_and_opens_it_with_focus_in_the_editor() {
    // Break caught: the note left unsaved in an untitled tab, created with text or over a
    // file, opened as the preview, not listed in the tree, or the focus left in the tree
    // (spec §4.1, §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_RETURN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-note-enter");
    scratch.note("top.md", "t");
    let (window, editor) = notebook_window(&scratch);

    crate::window::inline_name::new_note(window.hwnd, None);
    type_into_field(window.hwnd, "todo");
    field_key(window.hwnd, VK_RETURN);

    let todo = scratch.folder().join("todo.md");
    assert_eq!(std::fs::read(&todo).unwrap(), b"", "an empty file");
    assert!(!inline_open(window.hwnd));
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(todo.as_path()));
    assert!(!active.preview);
    assert_eq!(unsafe { GetFocus() }, editor.hwnd());
    row_of(window.hwnd, &RowKind::Note("todo.md".into()));

    crate::window::inline_name::new_note(window.hwnd, Some(std::path::PathBuf::new()));
    type_into_field(window.hwnd, "data.json");
    field_key(window.hwnd, VK_RETURN);
    assert!(
        scratch.folder().join("data.json").exists(),
        "a typed note extension is kept"
    );
    assert!(!scratch.folder().join("data.json.md").exists());
}

#[test]
fn a_new_note_with_a_listed_name_shows_the_message_and_enter_keeps_the_field() {
    // Break caught: a clash with a listed note missed until Enter, the message shown in
    // another case than typed, or Enter going ahead (spec §4.4).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-note-taken");
    scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    crate::window::inline_name::new_note(window.hwnd, None);

    type_into_field(window.hwnd, "A");
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("A.md already exists here.")
    );
    field_key(window.hwnd, VK_RETURN);
    assert!(inline_open(window.hwnd));
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("a.md")).unwrap(),
        "a"
    );
    type_into_field(window.hwnd, "b");
    assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
}

#[test]
fn a_new_note_clashing_with_an_unlisted_file_or_a_vanished_folder_says_so_after_enter() {
    // Break caught: a file written after the scan overwritten, a note created somewhere
    // else when its folder was deleted in Explorer, or the field closing on the failure
    // (spec §4.4, §5.2).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-note-disk");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note("top.md", "t");
    let (window, _editor) = notebook_window(&scratch);
    std::fs::write(scratch.folder().join("fresh.md"), "theirs").unwrap();

    crate::window::inline_name::new_note(window.hwnd, None);
    type_into_field(window.hwnd, "fresh");
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd),
        None,
        "not listed"
    );
    field_key(window.hwnd, VK_RETURN);
    assert!(inline_open(window.hwnd));
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("fresh.md already exists. Try fresh 2.md.")
    );
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("fresh.md")).unwrap(),
        "theirs"
    );
    crate::window::inline_name::cancel(window.hwnd);

    crate::window::inline_name::new_note(window.hwnd, Some("sub".into()));
    std::fs::remove_dir(scratch.folder().join("sub")).unwrap();
    type_into_field(window.hwnd, "x");
    field_key(window.hwnd, VK_RETURN);
    assert!(inline_open(window.hwnd));
    let problem = crate::window::inline_name::problem(window.hwnd).unwrap();
    assert!(
        problem.starts_with("FastPad could not create x.md: "),
        "{problem}"
    );
    assert!(!scratch.folder().join("x.md").exists());
}

#[test]
fn a_rescan_that_lists_the_typed_name_shows_the_problem_without_a_keystroke() {
    // Break caught: the live check run only on keystrokes, so a note that appeared on disk
    // while the user typed its name is only caught by the disk call (spec §4.4, §5.4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-note-rescan");
    scratch.note("top.md", "t");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::notebook_view::rebuild(window.hwnd);
    crate::window::inline_name::new_note(window.hwnd, None);
    type_into_field(window.hwnd, "idea");
    assert_eq!(crate::window::inline_name::problem(window.hwnd), None);

    scratch.note("idea.md", "made elsewhere");
    rescan_and_wait(window.hwnd);

    assert!(inline_open(window.hwnd));
    assert_eq!(
        crate::window::inline_name::problem(window.hwnd).as_deref(),
        Some("idea.md already exists here.")
    );
}

#[test]
fn the_first_note_of_an_empty_notebook_gets_a_draft_row() {
    // Break caught: "+" in a notebook with no notes doing nothing, because the empty state
    // has no tree to put the draft row in (spec §3.1).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-new-note-empty");
    let (window, _editor) = notebook_window(&scratch);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);

    crate::window::notebook_view::header_clicked(
        window.hwnd,
        crate::window::notebook_view::HeaderButton::NewNote,
    );
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);
    assert_eq!(draft_row(window.hwnd), Some((0, 0)));
    field_key(window.hwnd, VK_ESCAPE);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);
}

#[test]
fn a_draft_whose_folder_goes_in_a_notebook_left_empty_shows_the_empty_state() {
    // Break caught: the tree forced on for a draft that the rebuild then ends (its folder
    // gone, nothing else listed), leaving a blank tree without the empty state's New note
    // button (spec §3.1, §5.4).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-draft-empty-gone");
    std::fs::create_dir(scratch.folder().join("Fresh")).unwrap();
    let (window, _editor) = notebook_window(&scratch);
    crate::window::inline_name::new_note(window.hwnd, Some("Fresh".into()));
    assert!(inline_open(window.hwnd));
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);

    // The library drops the folder (deleted in Explorer, say): nothing is left to list.
    std::fs::remove_dir(scratch.folder().join("Fresh")).unwrap();
    crate::window::library_host::with_state(window.hwnd, |state| {
        state.remove_folder(std::path::Path::new("Fresh"), 0)
    });
    crate::window::notebook_view::rebuild(window.hwnd);

    assert!(!inline_open(window.hwnd));
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);
}

#[test]
fn the_empty_notebooks_new_note_button_drafts_a_note_instead_of_opening_a_tab() {
    // Break caught: the empty state's own "New note" button opening an untitled tab
    // (`CommandId::New`) instead of drafting a note in the tree, which is the only way an
    // empty notebook can name its own first note (inline naming spec §3.1).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("inline-empty-state-button");
    let (window, _editor) = notebook_window(&scratch);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);
    let tabs = super::super::tab_count(window.hwnd);

    crate::window::notebook_view::state_button(window.hwnd);

    assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);
    assert_eq!(draft_row(window.hwnd), Some((0, 0)));
    assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
    assert_eq!(
        super::super::tab_count(window.hwnd),
        tabs,
        "no untitled tab"
    );
}
