//! F6 focus cycling, the activity bar, and what screen readers see.

use super::*;

#[test]
fn f6_order_skips_a_closed_panel_and_a_missing_sidebar() {
    // Break caught: F6 landing in a hidden panel, or getting stuck when notes mode is off.
    use super::super::{FocusPart, next_focus_part};
    assert_eq!(
        next_focus_part(FocusPart::Group(0), false, true, true, 1),
        FocusPart::ActivityBar
    );
    assert_eq!(
        next_focus_part(FocusPart::ActivityBar, false, true, true, 1),
        FocusPart::Panel
    );
    assert_eq!(
        next_focus_part(FocusPart::Panel, false, true, true, 1),
        FocusPart::Group(0)
    );
    assert_eq!(
        next_focus_part(FocusPart::ActivityBar, true, true, true, 1),
        FocusPart::Group(0)
    );
    assert_eq!(
        next_focus_part(FocusPart::ActivityBar, false, true, false, 1),
        FocusPart::Group(0)
    );
    assert_eq!(
        next_focus_part(FocusPart::Group(0), false, false, false, 1),
        FocusPart::Group(0)
    );
}

#[test]
fn f6_visits_every_group_in_order() {
    // Break caught: F6 skipping every group after the first.
    use super::super::FocusPart::*;
    assert_eq!(
        super::super::next_focus_part(Group(0), false, true, true, 3),
        Group(1)
    );
    assert_eq!(
        super::super::next_focus_part(Group(2), false, true, true, 3),
        ActivityBar
    );
    assert_eq!(
        super::super::next_focus_part(ActivityBar, true, true, true, 3),
        Group(2)
    );
    assert_eq!(
        super::super::next_focus_part(Group(0), false, false, false, 1),
        Group(0)
    );
}

#[test]
fn f6_cycles_activity_bar_panel_and_editor_and_shift_f6_goes_back() {
    // Break caught: F6 doing nothing, skipping the panel, or leaving the focus in a closed
    // panel.
    let _scintilla = load_native_scintilla();
    let window = shown_window();
    let _editor = install_test_editor(&window);
    let (bar, panel) = crate::window::side_panel::windows(window.hwnd).unwrap();
    use crate::config::SidebarView;
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
    super::super::return_focus_to_editor(window.hwnd);
    let editor = focused();

    execute_command(window.hwnd, CommandId::FocusNextPane);
    assert_eq!(focused(), bar);
    execute_command(window.hwnd, CommandId::FocusNextPane);
    assert_eq!(focused(), panel);
    execute_command(window.hwnd, CommandId::FocusNextPane);
    assert_eq!(focused(), editor);
    execute_command(window.hwnd, CommandId::FocusPreviousPane);
    assert_eq!(focused(), panel);

    crate::window::side_panel::toggle(window.hwnd);
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        SidebarView::Hidden
    );
    super::super::return_focus_to_editor(window.hwnd);
    execute_command(window.hwnd, CommandId::FocusNextPane);
    assert_eq!(focused(), bar);
    execute_command(window.hwnd, CommandId::FocusNextPane);
    assert_eq!(focused(), editor, "a closed panel is skipped");
}

#[test]
fn escape_in_the_panel_returns_the_focus_to_the_editor() {
    // Break caught: Esc in the tree leaving the keyboard stuck in the sidebar.
    let _scintilla = load_native_scintilla();
    let window = shown_window();
    let _editor = install_test_editor(&window);
    super::super::return_focus_to_editor(window.hwnd);
    let editor = focused();
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, true);
    let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
    assert_eq!(focused(), panel);
    unsafe {
        SendMessageW(
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE as usize,
            0,
        );
    }
    assert_eq!(focused(), editor);
}

#[test]
fn the_activity_bar_moves_with_arrows_and_presses_with_enter() {
    // Break caught: activity-bar buttons reachable only with the mouse, or their pressed
    // state not following the shown view.
    let _scintilla = load_native_scintilla();
    let window = shown_window();
    let _editor = install_test_editor(&window);
    use crate::config::SidebarView;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_DOWN, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
    let bar = crate::window::side_panel::windows(window.hwnd).unwrap().0;
    unsafe {
        SetFocus(bar);
    }
    assert_eq!(crate::window::side_panel::bar_focus(window.hwnd), 0);
    let items = crate::window::activity_bar::accessible_items(bar);
    assert_eq!(items.len(), 4);
    assert_ne!(
        items[0].state & crate::window::sidebar_accessibility::STATE_PRESSED,
        0
    );
    assert_ne!(
        items[0].state & crate::window::sidebar_accessibility::STATE_FOCUSED,
        0
    );
    assert_eq!(items[3].name, "Settings");

    unsafe {
        SendMessageW(bar, WM_KEYDOWN, VK_DOWN as usize, 0);
    }
    assert_eq!(crate::window::side_panel::bar_focus(window.hwnd), 1);
    unsafe {
        SendMessageW(bar, WM_KEYDOWN, VK_RETURN as usize, 0);
    }
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        SidebarView::Search
    );
    let items = crate::window::activity_bar::accessible_items(bar);
    assert_ne!(
        items[1].state & crate::window::sidebar_accessibility::STATE_PRESSED,
        0
    );
    assert_eq!(
        items[0].state & crate::window::sidebar_accessibility::STATE_PRESSED,
        0
    );
}

#[test]
fn the_panel_exposes_the_tree_as_an_outline_with_pinned_and_folder_states() {
    // Break caught: the tree invisible to screen readers, the child count not matching the
    // visible rows, or a pin and a collapsed folder not reported.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("tree-msaa");
    let a = scratch.note("a.md", "a");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::library_host::toggle_pin(window.hwnd, &a);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
    let count = crate::window::side_panel::accessible_item_count(panel);
    let items = (0..count)
        .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
        .collect::<Vec<_>>();
    assert_eq!(items.len(), count);
    let outline = items
        .iter()
        .filter(|item| item.role == windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINEITEM)
        .collect::<Vec<_>>();
    // The tree's rows are the outline items after the notebook's root row: the section rows
    // (the Open Editors header and the root row) are at level 0, each with its rows under it.
    let root = outline
        .iter()
        .position(|item| item.value == "0" && !item.name.starts_with("Open editors, "))
        .expect("the notebook's root row");
    let rows = &outline[root + 1..];
    // "sub" is collapsed, so b is not a row: pinned a first, then the folder.
    assert_eq!(rows.len(), 2, "{items:?}");
    assert_eq!(rows[0].name, "a.md, Markdown, pinned");
    assert_eq!(rows[1].name, "sub");
    assert!(
        rows.iter().all(|row| row.value == "1"),
        "top-level rows sit one level under the root row: {items:?}"
    );
    assert_ne!(
        rows[1].state & crate::window::sidebar_accessibility::STATE_COLLAPSED,
        0
    );

    let provider = crate::window::sidebar_accessibility::create_for_test(
        panel,
        &crate::window::side_panel::PANEL_ACCESSIBLE,
    );
    let table = &crate::window::sidebar_accessibility::SIDEBAR_VTABLE;
    use crate::window::accessibility::{RawVariant, VariantValue};
    unsafe {
        let mut children = 0;
        (table.get_acc_child_count)(provider, &mut children);
        assert_eq!(children as usize, count);
        let mut role = RawVariant::empty();
        (table.get_acc_role)(provider, RawVariant::integer(0), &mut role);
        assert_eq!(
            role.child_id(),
            Some(windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINE as i32)
        );
        (table.release)(provider);
    }
}

#[test]
fn the_search_view_exposes_its_box_toggles_summary_and_results() {
    // Break caught (spec §10): the search box missing from the panel's children (the
    // sidebar PR's known limitation), toggles read as push buttons or without their checked
    // state, or results named without their snippet.
    use crate::window::accessibility::{AccessibleVtable, RawVariant, VariantValue};
    use crate::window::sidebar_accessibility::{
        SIDEBAR_VTABLE, STATE_CHECKED, create_for_test, take_raised,
    };
    use windows_sys::Win32::UI::Accessibility::{
        ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_LISTITEM, ROLE_SYSTEM_STATICTEXT, ROLE_SYSTEM_TEXT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-msaa");
    scratch.note("a.md", "one beta");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "beta two");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    type_into_search(window.hwnd, "beta");
    pump_until(window.hwnd, || {
        crate::window::search_view::shown_results(window.hwnd).len() == 2
            && matches!(
                search_state(window.hwnd),
                crate::window::search_view::SearchState::Done { .. }
            )
    });
    let panel = sidebar_panel(window.hwnd);
    let items = || {
        (0..crate::window::side_panel::accessible_item_count(panel))
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .collect::<Vec<_>>()
    };

    let shown = items();
    let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
    assert_eq!(shown[0].role, ROLE_SYSTEM_TEXT, "{shown:?}");
    assert_eq!(shown[0].window, edit);
    assert_eq!(shown[0].value, "beta");
    let toggles = shown[1..4]
        .iter()
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        toggles,
        ["Match case", "Match whole word", "Use regular expression"]
    );
    assert!(
        shown[1..4]
            .iter()
            .all(|item| item.role == ROLE_SYSTEM_CHECKBUTTON && item.state & STATE_CHECKED == 0)
    );
    // The chevron comes after the toggles, so the box and the toggles keep IDs 1 to 4.
    assert_eq!(shown[4].name, "Toggle replace");
    assert_eq!(shown[5].role, ROLE_SYSTEM_STATICTEXT);
    assert_eq!(shown[5].name, "2 notes");
    let results = shown
        .iter()
        .filter(|item| item.role == ROLE_SYSTEM_LISTITEM)
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(results, ["a: one beta", "b, sub: beta two"]);

    take_raised();
    crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Case);
    assert_ne!(items()[1].state & STATE_CHECKED, 0);
    assert!(
        take_raised().contains(&(panel as usize, EVENT_OBJECT_STATECHANGE, 2)),
        "the Match case child (ID 2) raises a state change"
    );

    // The box's full object is the Edit's own.
    let com = unsafe {
        windows_sys::Win32::System::Com::CoInitializeEx(
            std::ptr::null(),
            windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
        )
    };
    let provider = create_for_test(panel, &crate::window::side_panel::PANEL_ACCESSIBLE);
    unsafe {
        let mut object = std::ptr::null_mut();
        let result = (SIDEBAR_VTABLE.get_acc_child)(provider, RawVariant::integer(1), &mut object);
        assert_eq!(result, 0, "S_OK");
        assert!(!object.is_null());
        let vtable = *(object as *const *const AccessibleVtable);
        ((*vtable).release)(object);
        let mut none = std::ptr::null_mut();
        assert_eq!(
            (SIDEBAR_VTABLE.get_acc_child)(provider, RawVariant::integer(2), &mut none),
            windows_sys::Win32::Foundation::S_FALSE
        );
        (SIDEBAR_VTABLE.release)(provider);
    }
    if com >= 0 {
        unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
    }
}

#[test]
fn the_summary_speaks_at_most_once_a_second_while_a_search_runs() {
    // Break caught: a name change per batch (up to 20 a second) flooding the screen reader,
    // or the final count swallowed by the limit.
    use crate::library::text_search::{Progress, RunEnd, TextHit};
    use crate::window::sidebar_accessibility::take_raised;
    use crate::window::text_search_host::SearchBatch;
    use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_NAMECHANGE;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("search-speak");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
    let panel = sidebar_panel(window.hwnd);
    let hit = |name: &str| TextHit {
        path: std::path::PathBuf::from(format!("{name}.md")),
        name: name.to_owned(),
        folder: String::new(),
        snippet: crate::search::Snippet {
            text: "beta".to_owned(),
            highlight: 0..4,
        },
        stamp: None,
    };
    let progress = |visited| Progress {
        visited,
        total: 100,
        skipped: [0; 4],
    };
    let name_changes = || {
        take_raised()
            .into_iter()
            .filter(|&(hwnd, event, _)| hwnd == panel as usize && event == EVENT_OBJECT_NAMECHANGE)
            .count()
    };
    take_raised();

    crate::window::search_view::begin_search(window.hwnd, "beta", 100);
    for (visited, name) in [(10, "a"), (20, "b"), (30, "c")] {
        crate::window::search_view::apply_batch(
            window.hwnd,
            SearchBatch {
                generation: 0,
                hits: vec![hit(name)],
                progress: progress(visited),
                end: None,
                skipped: Vec::new(),
            },
        );
    }
    assert!(
        name_changes() <= 2,
        "one announcement (summary and status) at most"
    );

    crate::window::search_view::apply_batch(
        window.hwnd,
        SearchBatch {
            generation: 0,
            hits: Vec::new(),
            progress: progress(100),
            end: Some(RunEnd::Completed),
            skipped: Vec::new(),
        },
    );
    assert!(name_changes() >= 1, "the finished search is announced");
}

#[test]
fn the_find_bar_exposes_its_fields_toggles_and_close_button() {
    // Break caught: the find bar's toggles invisible to screen readers, a field's value read
    // with WM_GETTEXT under the App borrow instead of kept, or a default action that doesn't
    // flip a toggle.
    use crate::window::find_bar::FIND_BAR_ACCESSIBLE;
    use crate::window::sidebar_accessibility::{STATE_CHECKED, take_raised};
    use windows_sys::Win32::UI::Accessibility::{
        ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_PUSHBUTTON, ROLE_SYSTEM_TEXT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_STATECHANGE, SetWindowTextW, WM_SYSKEYDOWN,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::Find);
    let (panel, query, replace) = {
        let bar = app_mut(window.hwnd).find_bar().unwrap();
        (bar.panel_hwnd(), bar.query_hwnd(), bar.replace_hwnd())
    };
    let items = || {
        (0..(FIND_BAR_ACCESSIBLE.count)(panel))
            .filter_map(|index| (FIND_BAR_ACCESSIBLE.item)(panel, index))
            .collect::<Vec<_>>()
    };

    let shown = items();
    let roles = shown.iter().map(|item| item.role).collect::<Vec<_>>();
    assert_eq!(
        roles,
        [
            ROLE_SYSTEM_TEXT,
            ROLE_SYSTEM_CHECKBUTTON,
            ROLE_SYSTEM_CHECKBUTTON,
            ROLE_SYSTEM_CHECKBUTTON,
            ROLE_SYSTEM_PUSHBUTTON,
            ROLE_SYSTEM_PUSHBUTTON,
            ROLE_SYSTEM_PUSHBUTTON
        ]
    );
    assert_eq!(shown[4].name, "Previous match");
    assert_eq!(shown[5].name, "Next match");
    assert_eq!(shown[6].name, "Close");
    assert_eq!(shown[0].window, query);
    assert_eq!(shown[2].name, "Match whole word");
    let typed = crate::platform::wide_null("needle");
    unsafe { SetWindowTextW(query, typed.as_ptr()) };
    assert_eq!(items()[0].value, "needle", "the field's kept text");

    take_raised();
    unsafe { SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'W'), 1 << 29) };
    assert_ne!(items()[2].state & STATE_CHECKED, 0);
    assert!(take_raised().contains(&(panel as usize, EVENT_OBJECT_STATECHANGE, 3)));

    // The default action clicks the toggle, as the mouse does.
    (FIND_BAR_ACCESSIBLE.activate)(panel, 1);
    assert!(app_mut(window.hwnd).find_bar().unwrap().options().case);

    execute_command(window.hwnd, CommandId::Replace);
    assert_eq!((FIND_BAR_ACCESSIBLE.count)(panel), 8);
    let typed = crate::platform::wide_null("pin");
    unsafe { SetWindowTextW(replace, typed.as_ptr()) };
    let shown = items();
    assert_eq!(shown[4].name, "Replace");
    assert_eq!(shown[4].window, replace);
    assert_eq!(shown[4].value, "pin");
}

#[test]
fn pinning_a_note_that_is_not_first_raises_reorder_and_state_change() {
    // Break caught: a pin re-sorting the selected row to the top at the same count, so screen
    // readers hear only a new selection: no state change and no reorder of its siblings.
    use crate::window::sidebar_accessibility::take_raised;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_REORDER, EVENT_OBJECT_SELECTION, EVENT_OBJECT_STATECHANGE,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("pin-msaa");
    scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
    let kind = RowKind::Note(crate::library::record_path(&scratch.folder(), &b));
    let before = row_of(window.hwnd, &kind);
    assert!(before > 0, "{:?}", notebook_view(window.hwnd).rows);
    notebook_view(window.hwnd).list.selected = Some(before);
    let source = &crate::window::side_panel::PANEL_ACCESSIBLE;
    let count = (source.count)(panel);
    take_raised();

    crate::window::library_host::toggle_pin(window.hwnd, &b);

    let after = row_of(window.hwnd, &kind);
    assert!(after < before, "the pinned note moves up");
    assert_eq!(
        (source.count)(panel),
        count + 2,
        "the Pinned header and the pinned note join the Open editors list"
    );
    let id = (source.current)(panel).unwrap() as i32 + 1;
    let raised = take_raised()
        .into_iter()
        .filter(|&(hwnd, _, _)| hwnd == panel as usize)
        .map(|(_, event, child)| (event, child))
        .collect::<Vec<_>>();
    assert!(raised.contains(&(EVENT_OBJECT_REORDER, 0)), "{raised:?}");
    assert!(raised.contains(&(EVENT_OBJECT_SELECTION, id)), "{raised:?}");
    assert!(
        raised.contains(&(EVENT_OBJECT_STATECHANGE, id)),
        "{raised:?}"
    );
    assert!(
        (source.item)(panel, id as usize - 1)
            .unwrap()
            .name
            .ends_with(", pinned")
    );
}

#[test]
fn arrowing_through_recent_notebooks_selects_and_announces_them() {
    // Break caught: the no-notebook state's RECENT list reporting no selection, so arrowing
    // through it raises no events and no item is ever STATE_SYSTEM_SELECTED.
    use crate::window::sidebar_accessibility::{STATE_FOCUSED, STATE_SELECTED, take_raised};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DOWN;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EVENT_OBJECT_FOCUS, EVENT_OBJECT_SELECTION, WM_KEYDOWN,
    };
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("recent-msaa-a");
    let other = LibraryScratch::new("recent-msaa-b");
    scratch.note("a.md", "a");
    let window = shown_window();
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &crate::library::local::RecentFolders {
            folders: vec![scratch.folder(), other.folder()],
            ..Default::default()
        },
    )
    .unwrap();
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::CloseNotebook);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::NoNotebook);
    let recent = notebook_view(window.hwnd).recent.clone();
    assert!(recent.len() >= 2, "{recent:?}");
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, true);
    let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
    assert_eq!(focused(), panel);
    let source = &crate::window::side_panel::PANEL_ACCESSIBLE;
    let buttons = (source.count)(panel) - recent.len();

    for _ in 0..2 {
        take_raised();
        unsafe {
            SendMessageW(panel, WM_KEYDOWN, VK_DOWN as usize, 0);
        }
        let selected = notebook_view(window.hwnd).list.selected.unwrap();
        let id = (buttons + selected) as i32 + 1;
        assert_eq!((source.current)(panel), Some(buttons + selected));
        let raised = take_raised();
        let panel_id = panel as usize;
        assert!(
            raised.contains(&(panel_id, EVENT_OBJECT_SELECTION, id)),
            "{raised:?}"
        );
        assert!(
            raised.contains(&(panel_id, EVENT_OBJECT_FOCUS, id)),
            "{raised:?}"
        );
        let item = (source.item)(panel, id as usize - 1).unwrap();
        assert_ne!(item.state & STATE_SELECTED, 0, "{item:?}");
        assert_ne!(item.state & STATE_FOCUSED, 0, "{item:?}");
        assert_eq!(
            item.name,
            notebook_view(window.hwnd).recent_name(selected),
            "indexed by list position"
        );
    }
    assert_eq!(notebook_view(window.hwnd).list.selected, Some(1));
}

#[test]
fn a_query_from_another_thread_is_answered_on_the_window_thread() {
    // Break caught: an MSAA client's RPC thread reading App directly, or its marshalled query
    // rejected by the pointer check meant for foreign senders.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
    let expected = crate::window::side_panel::accessible_item_count(panel);
    assert!(expected > 0);
    let provider = crate::window::sidebar_accessibility::create_for_test(
        panel,
        &crate::window::side_panel::PANEL_ACCESSIBLE,
    ) as usize;
    let worker = std::thread::spawn(move || {
        let provider = provider as *mut std::ffi::c_void;
        let table = &crate::window::sidebar_accessibility::SIDEBAR_VTABLE;
        let mut count = 0;
        unsafe {
            assert_eq!(
                (table.get_acc_child_count)(provider, &mut count),
                windows_sys::Win32::Foundation::S_OK
            );
            (table.release)(provider);
        }
        count
    });
    // Messages sent from the worker are delivered while this thread peeks.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !worker.is_finished() {
        assert!(
            std::time::Instant::now() < deadline,
            "the query was never answered"
        );
        let mut message = windows_sys::Win32::UI::WindowsAndMessaging::MSG::default();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                0,
                0,
                windows_sys::Win32::UI::WindowsAndMessaging::PM_NOREMOVE,
            );
        }
    }
    assert_eq!(worker.join().unwrap() as usize, expected);
}
