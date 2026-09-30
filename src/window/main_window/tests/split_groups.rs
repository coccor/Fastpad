//! Split editors: second groups, sashes, per-group strips and views, and moving between groups.

use super::*;

#[test]
fn a_second_group_gets_its_own_editor_find_bar_and_preview() {
    // Break caught: a second group sharing the first one's editor or find bar, so a find in
    // one group moves the caret in the other, or its preview state leaking across.
    use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    let second = super::super::create_group(window.hwnd).expect("second group");
    assert_ne!(first, second);
    let (first_editor, second_editor) = (
        super::super::group_editor(window.hwnd, first).unwrap(),
        super::super::group_editor(window.hwnd, second).unwrap(),
    );
    assert_ne!(first_editor.hwnd(), second_editor.hwnd());
    let second_window = app_mut(window.hwnd).group(second).unwrap().hwnd;
    assert_eq!(unsafe { GetParent(second_editor.hwnd()) }, second_window);
    assert!(second_editor.shares_documents_with(&first_editor));

    // Find needs a tab, so group 2 shows the document too.
    let document = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert!(
        app_mut(window.hwnd)
            .tabs
            .add_view(second, document, Default::default())
    );
    app_mut(window.hwnd).tabs.set_active_group(second);
    execute_command(window.hwnd, CommandId::Find);
    let panel = app_mut(window.hwnd).find_bar().unwrap().panel_hwnd();
    assert_eq!(unsafe { GetParent(panel) }, second_window);
    assert!(
        app_mut(window.hwnd)
            .group(first)
            .unwrap()
            .find_bar
            .is_none()
    );
    assert_eq!(
        app_mut(window.hwnd).group_containing(first_editor.hwnd()),
        Some(first)
    );

    app_mut(window.hwnd).tabs.set_active_group(first);
    assert!(app_mut(window.hwnd).tabs.move_view(second, document, first));
    super::super::destroy_group(window.hwnd, second);
    assert!(app_mut(window.hwnd).group(second).is_none());
    assert_eq!(app_mut(window.hwnd).tabs.group_ids(), vec![first]);
}

fn second_group_showing_the_active_document(hwnd: HWND) -> (GroupId, GroupId) {
    let first = app_mut(hwnd).tabs.active_group();
    let second = super::super::create_group(hwnd).expect("second group");
    let id = app_mut(hwnd).tabs.active().unwrap().id;
    assert!(
        app_mut(hwnd)
            .tabs
            .add_view(second, id, crate::editor::ViewState::default())
    );
    super::super::show_group_view(hwnd, second);
    (first, second)
}

/// A point on group `id`'s strip past its last tab, in the group's client coordinates.
fn empty_strip_point(hwnd: HWND, id: GroupId) -> LPARAM {
    let layout = super::super::strip_layout_of(hwnd, id).unwrap();
    let count = app_mut(hwnd).tabs.group(id).unwrap().len();
    let last = layout.tab(count - 1).unwrap();
    client_lparam(last.right + 10, layout.height / 2)
}

#[test]
fn an_edit_in_one_group_shows_in_the_other_and_counts_once() {
    // Break caught: document-level effects run by every editor showing the document, so one
    // keystroke bumps the generation twice, or the other group's caret jumps to the edit.
    use crate::editor::ViewState;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor
        .set_text(
            "one
two
three",
        )
        .unwrap();
    let (_, second) = second_group_showing_the_active_document(window.hwnd);
    let other = super::super::group_editor(window.hwnd, second).unwrap();
    let at = |caret| ViewState {
        caret,
        anchor: caret,
        first_line: 0,
        x_offset: 0,
    };
    editor.apply_view_state(at(0)).unwrap();
    other.apply_view_state(at(8)).unwrap();
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    let before = app_mut(window.hwnd).tabs.document(id).unwrap().generation;

    editor.replace_target(0..0, "x").unwrap();

    assert_eq!(
        other.text().unwrap(),
        "xone
two
three"
    );
    let document = app_mut(window.hwnd).tabs.document(id).unwrap();
    assert_eq!(document.generation, before + 1);
    assert!(document.dirty);
    assert_eq!(
        other.view_state().unwrap().caret,
        9,
        "the other caret moves with its text only"
    );
}

#[test]
fn focus_in_a_groups_editor_or_find_bar_makes_that_group_active() {
    // Break caught: a click into group 2 leaving group 1 active, so Ctrl+F, the status bar
    // and the title act on the group the user left.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = second_group_showing_the_active_document(window.hwnd);
    let other = super::super::group_editor(window.hwnd, second).unwrap();

    unsafe { SetFocus(other.hwnd()) };
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    execute_command(window.hwnd, CommandId::Find);
    let query = app_mut(window.hwnd).find_bar().unwrap().query_hwnd();

    let first_editor = super::super::group_editor(window.hwnd, first).unwrap();
    unsafe { SetFocus(first_editor.hwnd()) };
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    unsafe { SetFocus(query) };
    super::super::pump_posted_messages(window.hwnd);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
}

#[test]
fn a_strip_menu_command_acts_on_the_group_that_was_right_clicked() {
    // Break caught: the strip's menu running New in the active group instead of the group
    // under the pointer.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = second_group_showing_the_active_document(window.hwnd);
    super::super::layout_editor_and_find_bar(window.hwnd);
    let group = app_mut(window.hwnd).group(second).unwrap().hwnd;
    // Until groups are laid out side by side, the second one gets its size here.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::MoveWindow(group, 0, 0, 600, 400, 0) };
    let empty = empty_strip_point(window.hwnd, second);
    answer_next_popup_menu(|_| Some(CommandId::New));
    unsafe {
        SendMessageW(group, WM_RBUTTONDOWN, 0, empty);
        SendMessageW(group, WM_RBUTTONUP, 0, empty);
    }
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    assert_eq!(app_mut(window.hwnd).tabs.group(second).unwrap().len(), 2);
    assert_eq!(app_mut(window.hwnd).tabs.group(first).unwrap().len(), 1);
}

#[test]
fn a_preview_opened_in_one_group_leaves_the_other_group_alone() {
    // Break caught: one preview mode shared by every group, so Full preview in group 2 hides
    // group 1's editor or opens a preview there too.
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_STYLE, GetWindowLongPtrW, WS_VISIBLE};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("# title").unwrap();
    app_mut(window.hwnd)
        .tabs
        .set_active_language(crate::document::Language::Markdown);
    let (first, second) = second_group_showing_the_active_document(window.hwnd);
    super::super::activate_group(window.hwnd, second);
    execute_command(window.hwnd, CommandId::MarkdownPreviewFull);
    let mode =
        |id| crate::window::preview_host::with_group_host(window.hwnd, id, |host| host.mode());
    assert_eq!(mode(second), Some(crate::preview::PreviewMode::Full));
    assert_eq!(mode(first), Some(crate::preview::PreviewMode::Off));
    // The test window is never shown, so the style says whether the editor would show.
    let style = unsafe { GetWindowLongPtrW(editor.hwnd(), GWL_STYLE) } as u32;
    assert!(style & WS_VISIBLE != 0, "group 1's editor still shows");
    let other = super::super::group_editor(window.hwnd, second).unwrap();
    let style = unsafe { GetWindowLongPtrW(other.hwnd(), GWL_STYLE) } as u32;
    assert!(
        style & WS_VISIBLE == 0,
        "group 2's Full preview hides its editor"
    );
}

fn split_for_test(hwnd: HWND, direction: crate::window::split_tree::Direction) -> GroupId {
    let active = app_mut(hwnd).tabs.active_group();
    let new = super::super::create_group(hwnd).expect("group");
    assert!(app_mut(hwnd).layout.split(active, direction, new));
    let id = app_mut(hwnd).tabs.active().unwrap().id;
    app_mut(hwnd)
        .tabs
        .add_view(new, id, crate::editor::ViewState::default());
    super::super::show_group_view(hwnd, new);
    super::super::layout_editor_and_find_bar(hwnd);
    new
}

fn window_rect_in_main(hwnd: HWND, child: HWND) -> RECT {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::MapWindowPoints;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(child, &mut rect);
        MapWindowPoints(
            std::ptr::null_mut(),
            hwnd,
            &mut rect as *mut RECT as *mut POINT,
            2,
        );
    }
    rect
}

#[test]
fn groups_side_by_side_both_reach_into_the_title_row_with_a_sash_between() {
    // Break caught: a second column pushed below the title row (wasting a row), overlapping
    // the first, or leaving no gap for the sash.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    let second = split_for_test(window.hwnd, crate::window::split_tree::Direction::Right);
    let (a, b) = (
        window_rect_in_main(window.hwnd, app_mut(window.hwnd).group(first).unwrap().hwnd),
        window_rect_in_main(
            window.hwnd,
            app_mut(window.hwnd).group(second).unwrap().hwnd,
        ),
    );
    assert_eq!((a.top, b.top), (0, 0));
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window.hwnd) };
    assert_eq!(
        b.left - a.right,
        crate::window::titlebar::scale(crate::window::split_tree::SASH_96, dpi)
    );
    assert_eq!(
        super::super::tree_layout(window.hwnd).unwrap().sashes.len(),
        1
    );
    assert_eq!(super::super::group_strip_bounds(window.hwnd).len(), 2);
}

#[test]
fn a_group_below_another_draws_its_strip_at_its_own_top() {
    // Break caught: a lower group's strip hit-tested as caption, so clicking its tabs drags
    // the window.
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLIENT, WM_NCHITTEST};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let second = split_for_test(window.hwnd, crate::window::split_tree::Direction::Down);
    let group = app_mut(window.hwnd).group(second).unwrap().hwnd;
    let rect = window_rect_in_main(window.hwnd, group);
    assert!(rect.top > 0);
    let mut screen = windows_sys::Win32::Foundation::POINT {
        x: rect.right - 20,
        y: rect.top + 5,
    };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut screen) };
    let packed = ((screen.y as u32 & 0xffff) << 16 | (screen.x as u32 & 0xffff)) as LPARAM;
    assert_eq!(
        unsafe { SendMessageW(group, WM_NCHITTEST, 0, packed) },
        HTCLIENT as LRESULT
    );
    assert_eq!(super::super::group_strip_bounds(window.hwnd).len(), 1);
}

#[test]
fn dragging_a_sash_resizes_both_groups_and_stops_at_the_minimum() {
    // Break caught: a sash that doesn't follow the pointer, or one dragged over the edge
    // collapsing a group to nothing.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    split_for_test(window.hwnd, crate::window::split_tree::Direction::Right);
    let sash = super::super::tree_layout(window.hwnd).unwrap().sashes[0].clone();
    let (x, y) = (sash.rect.left + 1, (sash.rect.top + sash.rect.bottom) / 2);
    let area = super::super::tree_area(window.hwnd).unwrap();
    unsafe {
        SendMessageW(window.hwnd, WM_LBUTTONDOWN, 1, client_lparam(x, y));
        SendMessageW(window.hwnd, WM_MOUSEMOVE, 1, client_lparam(area.left, y));
        SendMessageW(window.hwnd, WM_LBUTTONUP, 0, client_lparam(area.left, y));
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window.hwnd) };
    let a = super::super::tree_layout(window.hwnd)
        .unwrap()
        .rect_of(first)
        .unwrap();
    assert_eq!(
        a.right - a.left,
        crate::window::titlebar::scale(crate::window::split_tree::MIN_WIDTH_96, dpi)
    );
    let group = app_mut(window.hwnd).group(first).unwrap().hwnd;
    let rect = window_rect_in_main(window.hwnd, group);
    assert_eq!(rect.right - rect.left, a.right - a.left);
}

#[test]
fn a_double_click_on_a_sash_equalizes() {
    // Break caught: a sash double-click read as two presses and ignored.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    split_for_test(window.hwnd, crate::window::split_tree::Direction::Right);
    let sash = super::super::tree_layout(window.hwnd).unwrap().sashes[0].clone();
    app_mut(window.hwnd)
        .layout
        .set_sash(&sash, sash.rect.left - 100, 96);
    super::super::layout_editor_and_find_bar(window.hwnd);
    let sash = super::super::tree_layout(window.hwnd).unwrap().sashes[0].clone();
    let (x, y) = (sash.rect.left + 1, (sash.rect.top + sash.rect.bottom) / 2);
    for _ in 0..2 {
        unsafe {
            SendMessageW(window.hwnd, WM_LBUTTONDOWN, 1, client_lparam(x, y));
            SendMessageW(window.hwnd, WM_LBUTTONUP, 0, client_lparam(x, y));
        }
    }
    let layout = super::super::tree_layout(window.hwnd).unwrap();
    let area = super::super::tree_area(window.hwnd).unwrap();
    let a = layout.rect_of(first).unwrap();
    assert!(((a.right - area.left) - (area.right - area.left) / 2).abs() <= 3);
}

#[test]
fn only_the_group_under_the_caption_buttons_gives_them_up() {
    // Break caught: every top group cutting the caption area out of its region, leaving a
    // hole in the left group's strip.
    use windows_sys::Win32::Graphics::Gdi::{
        CreateRectRgn, DeleteObject, GetWindowRgn, PtInRegion,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    let second = split_for_test(window.hwnd, crate::window::split_tree::Direction::Right);
    let region_has = |group: HWND, x: i32, y: i32| unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        let kind = GetWindowRgn(group, region);
        let inside =
            kind == 0 /* ERROR: no region, the whole window */ || PtInRegion(region, x, y) != 0;
        DeleteObject(region);
        inside
    };
    let a = app_mut(window.hwnd).group(first).unwrap().hwnd;
    let b = app_mut(window.hwnd).group(second).unwrap().hwnd;
    let a_rect = window_rect_in_main(window.hwnd, a);
    let b_rect = window_rect_in_main(window.hwnd, b);
    assert!(region_has(a, a_rect.right - a_rect.left - 2, 2));
    assert!(!region_has(b, b_rect.right - b_rect.left - 2, 2));
}

#[test]
fn split_right_opens_the_active_document_in_a_new_group_to_the_right() {
    // Break caught: Split Right opening an empty group, a copy of the document instead of a
    // second view, or the new group landing on the left.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("shared").unwrap();
    let first = app_mut(window.hwnd).tabs.active_group();
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::SplitRight);
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 2);
    assert_eq!(order[0], first);
    let second = order[1];
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    assert_eq!(app_mut(window.hwnd).tabs.views_of(id), vec![first, second]);
    assert_eq!(
        super::super::group_editor(window.hwnd, second)
            .unwrap()
            .text()
            .unwrap(),
        "shared"
    );
}

/// The foreground group `id`'s editor draws JSON strings in.
fn json_string_colour(hwnd: HWND, id: GroupId) -> isize {
    const SCI_STYLEGETFORE: u32 = 2481;
    let editor = super::super::group_editor(hwnd, id).unwrap();
    unsafe {
        SendMessageW(
            editor.hwnd(),
            SCI_STYLEGETFORE,
            crate::editor::scintilla_constants::SCE_JSON_STRING as usize,
            0,
        )
    }
}

fn theme_json_string_colour(hwnd: HWND) -> isize {
    crate::languages::style_table(
        crate::document::Language::Json,
        super::super::effective_theme(hwnd),
    )
    .iter()
    .find(|style| style.style == crate::editor::scintilla_constants::SCE_JSON_STRING)
    .unwrap()
    .foreground as isize
}

#[test]
fn a_split_shows_the_documents_syntax_colours_in_the_new_group() {
    // Break caught: a new group's editor keeps only the base colour in every style, so a JSON
    // or Markdown document shows monochrome there (styles belong to each Scintilla view).
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("{\"a\": \"b\"}").unwrap();
    app_mut(window.hwnd)
        .tabs
        .set_active_language(crate::document::Language::Json);
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert_eq!(
        json_string_colour(window.hwnd, second),
        theme_json_string_colour(window.hwnd)
    );
}

#[test]
fn a_theme_change_recolours_the_syntax_in_every_group() {
    // Break caught: a theme change re-applies the language to the active editor only, and the
    // other groups keep the base colour in every style.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("{\"a\": \"b\"}").unwrap();
    app_mut(window.hwnd)
        .tabs
        .set_active_language(crate::document::Language::Json);
    let (first, second) = second_group_showing_the_active_document(window.hwnd);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    execute_command(window.hwnd, CommandId::ThemeCatppuccinMocha);
    assert_eq!(
        json_string_colour(window.hwnd, second),
        theme_json_string_colour(window.hwnd)
    );
}

#[test]
fn hovering_a_strip_that_is_not_active_highlights_its_own_tab() {
    // Break caught: pointer messages on a group's strip hit-test and highlight the active
    // group's strip instead of their own.
    use crate::window::group_strip::StripTarget;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert!(super::super::activate_group(window.hwnd, first));
    let tab = super::super::strip_layout_of(window.hwnd, second)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();
    let group = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    unsafe {
        SendMessageW(
            group,
            super::super::WM_MOUSEMOVE,
            0,
            client_lparam(tab.x, tab.y),
        )
    };
    let hovered =
        |id| super::super::with_group_id(window.hwnd, id, |state| state.pointer.hovered).unwrap();
    assert_eq!(hovered(second), Some(StripTarget::Tab(0)));
    assert_eq!(hovered(first), None);
}

#[test]
fn the_wheel_over_a_strip_that_is_not_active_leaves_the_active_strip_alone() {
    // Break caught: the wheel over one group's strip scrolls the active group's tabs.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    for _ in 0..40 {
        execute_command(window.hwnd, CommandId::New);
    }
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert!(super::super::activate_group(window.hwnd, first));
    let _ = app_mut(window.hwnd).tabs.set_scroll_offset(0);
    let tab = super::super::strip_layout_of(window.hwnd, second)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();
    let group = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let mut point = windows_sys::Win32::Foundation::POINT { x: tab.x, y: tab.y };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(group, &mut point) };
    let wheel_down = ((-120_i16 as u16 as usize) << 16) as super::super::WPARAM;
    unsafe {
        SendMessageW(
            group,
            super::super::WM_MOUSEWHEEL,
            wheel_down,
            client_lparam(point.x, point.y),
        )
    };
    assert_eq!(
        app_mut(window.hwnd)
            .tabs
            .group(first)
            .unwrap()
            .scroll_offset(),
        0
    );
}

#[test]
fn a_tab_click_in_another_group_moves_the_focus_there() {
    // Break caught: the click makes the other group active but the caret stays in the first
    // group's editor, so typing edits one document while Ctrl+S saves another.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    unsafe { SetFocus(editor.hwnd()) };
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    let tab = super::super::strip_layout_of(window.hwnd, second)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();
    let group = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let point = client_lparam(tab.x, tab.y);
    unsafe {
        SendMessageW(group, super::super::WM_LBUTTONDOWN, 1, point);
        SendMessageW(group, super::super::WM_LBUTTONUP, 0, point);
    }
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    assert_eq!(
        unsafe { GetFocus() },
        super::super::group_editor(window.hwnd, second)
            .unwrap()
            .hwnd()
    );
}

#[test]
fn closing_a_document_shown_only_elsewhere_leaves_the_focused_group_active() {
    // Break caught: a file deleted on disk closes its view in another group by activating
    // that group, and leaves it active while the caret stays in the first group.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    execute_command(window.hwnd, CommandId::New);
    let elsewhere = app_mut(window.hwnd).tabs.active().unwrap().id;
    unsafe { SetFocus(editor.hwnd()) };
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);

    super::super::close_document_without_prompt(window.hwnd, elsewhere);

    assert!(app_mut(window.hwnd).tabs.document(elsewhere).is_none());
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    assert_eq!(unsafe { GetFocus() }, editor.hwnd());
}

#[test]
fn an_accessible_tab_selection_in_another_group_selects_that_tab() {
    // Break caught: the strip's MSAA selection is resolved against the active group, so a
    // screen-reader user selecting a tab in another group's list gets no response.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    let shared = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::New);
    assert!(super::super::activate_group(window.hwnd, first));
    let request = super::super::AccessibleSelectRequest {
        document_id: shared,
        revision: app_mut(window.hwnd)
            .tabs
            .group(second)
            .unwrap()
            .view()
            .snapshot()
            .revision,
    };
    let group = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let selected = unsafe {
        SendMessageW(
            group,
            super::super::WM_FASTPAD_ACCESSIBLE_SELECT,
            0,
            &request as *const super::super::AccessibleSelectRequest as LPARAM,
        )
    };
    assert_eq!(selected, 1);
    assert_eq!(
        app_mut(window.hwnd)
            .tabs
            .group(second)
            .unwrap()
            .active_document(),
        Some(shared)
    );
}

#[test]
fn a_split_without_room_is_refused_with_a_notice() {
    // Break caught: a split into a sliver narrower than the minimum group.
    use windows_sys::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOZORDER, SetWindowPos};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    unsafe {
        SetWindowPos(
            window.hwnd,
            std::ptr::null_mut(),
            0,
            0,
            360,
            400,
            SWP_NOMOVE | SWP_NOZORDER,
        )
    };
    super::super::layout_editor_and_find_bar(window.hwnd);
    execute_command(window.hwnd, CommandId::SplitRight);
    assert_eq!(super::super::group_order(window.hwnd).len(), 1);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice == super::super::NO_ROOM_TO_SPLIT)
    );
}

#[test]
fn a_failed_group_window_leaves_the_layout_and_tabs_unchanged() {
    // Break caught: a half-made group left in the tree when its Scintilla can't be created.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    super::super::fail_next_group_creation();
    execute_command(window.hwnd, CommandId::SplitRight);
    assert_eq!(super::super::group_order(window.hwnd).len(), 1);
    assert_eq!(app_mut(window.hwnd).tabs.group_ids().len(), 1);
    assert!(!notices(window.hwnd).is_empty());
}

#[test]
fn closing_a_group_with_a_dirty_document_shown_elsewhere_does_not_prompt() {
    // Break caught: Close Group asking to save (or discarding) a document another group
    // still shows, or leaving it clean.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("unsaved").unwrap();
    let first = app_mut(window.hwnd).tabs.active_group();
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert!(app_mut(window.hwnd).tabs.document(id).unwrap().dirty);
    execute_command(window.hwnd, CommandId::SplitRight);
    let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = std::rc::Rc::clone(&prompted);
    answer_next_close_prompt(move |_| {
        seen.set(true);
        CloseDecision::Cancel
    });
    execute_command(window.hwnd, CommandId::CloseGroup);
    assert!(!prompted.get(), "another group still shows the document");
    assert_eq!(super::super::group_order(window.hwnd), vec![first]);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    let document = app_mut(window.hwnd).tabs.document(id).unwrap();
    assert!(document.dirty);
    assert_eq!(
        super::super::group_editor(window.hwnd, first)
            .unwrap()
            .text()
            .unwrap(),
        "unsaved"
    );
}

#[test]
fn closing_a_groups_last_tab_removes_the_group_but_never_the_only_one() {
    // Break caught: an empty second group left on screen, or the only group destroyed.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitDown);
    execute_command(window.hwnd, CommandId::CloseTab);
    assert_eq!(super::super::group_order(window.hwnd), vec![first]);
    execute_command(window.hwnd, CommandId::CloseAllTabs);
    assert_eq!(super::super::group_order(window.hwnd), vec![first]);
    assert!(app_mut(window.hwnd).groups.len() == 1);
}

#[test]
fn ctrl_2_focuses_group_two_and_ctrl_3_without_one_splits_right_of_the_last() {
    // Break caught: Ctrl+N still selecting tabs, or a missing group N doing nothing instead
    // of VS Code's split to the right of the last group.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitDown);
    let second = super::super::group_order(window.hwnd)[1];
    execute_command(window.hwnd, CommandId::FocusGroup1);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    execute_command(window.hwnd, CommandId::FocusGroup2);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    execute_command(window.hwnd, CommandId::FocusGroup1);
    execute_command(window.hwnd, CommandId::FocusGroup3);
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 3);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), order[2]);
    let area = super::super::tree_area(window.hwnd).unwrap();
    let layout = super::super::tree_layout(window.hwnd).unwrap();
    assert_eq!(layout.rect_of(order[2]).unwrap().right, area.right);
    assert_eq!(
        layout.rect_of(order[2]).unwrap().top,
        layout.rect_of(second).unwrap().top,
        "beside the last group, as in VS Code"
    );
    execute_command(window.hwnd, CommandId::FocusGroup1);
    execute_command(window.hwnd, CommandId::FocusLastGroup);
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), order[2]);
}

#[test]
fn moving_a_tab_to_the_next_group_creates_one_and_the_empty_source_closes() {
    // Break caught: Ctrl+Alt+Right doing nothing with one group, copying instead of moving,
    // or leaving the emptied group behind.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("moved").unwrap();
    let first = app_mut(window.hwnd).tabs.active_group();
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::MoveTabToNextGroup);
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 1, "the emptied first group closed");
    assert_ne!(order[0], first);
    assert_eq!(app_mut(window.hwnd).tabs.views_of(id), vec![order[0]]);
    assert_eq!(
        super::super::group_editor(window.hwnd, order[0])
            .unwrap()
            .text()
            .unwrap(),
        "moved"
    );
    execute_command(window.hwnd, CommandId::MoveTabToPreviousGroup);
    assert_eq!(
        super::super::group_order(window.hwnd),
        order,
        "nothing before group 1"
    );
}

#[test]
fn alt_digits_select_tabs_and_ctrl_digits_focus_groups_through_the_table() {
    // Break caught: Alt+2 eaten by the menu band's mnemonic handling, or Ctrl+2 still bound
    // to Select Tab 2.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    assert_eq!(
        translate_key_with(window.hwnd, editor.hwnd(), b'2', true, false, false),
        Some(CommandId::FocusGroup2)
    );
    assert_eq!(
        translate_key_with(window.hwnd, editor.hwnd(), b'2', false, false, true),
        Some(CommandId::SelectTab2)
    );
}

#[test]
fn right_clicking_a_tab_activates_it_and_runs_the_chosen_item() {
    // Break caught: the tab menu acting on the previously active tab.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let first_tab = app_mut(window.hwnd).tabs.ids().collect::<Vec<_>>()[0];
    let group = app_mut(window.hwnd).active_group().unwrap().hwnd;
    let center = super::super::strip_layout(window.hwnd)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();
    let point = client_lparam(center.x, center.y);
    answer_next_popup_menu(|_| Some(CommandId::SplitRight));
    unsafe {
        SendMessageW(group, WM_RBUTTONDOWN, 0, point);
        SendMessageW(group, WM_RBUTTONUP, 0, point);
    }
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 2);
    assert_eq!(app_mut(window.hwnd).tabs.views_of(first_tab), order);
}

#[test]
fn opening_a_file_open_in_another_group_adds_a_view_in_the_active_group() {
    // Break caught: the open jumping back to group 1 (leaving group 2 where the user is
    // working) or opening a second copy of the file.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("groups-open");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    let first = app_mut(window.hwnd).tabs.active_group();
    super::super::open_path(window.hwnd, &b).unwrap();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    super::super::open_path(window.hwnd, &a).unwrap();
    let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    assert_eq!(app_mut(window.hwnd).tabs.views_of(id), vec![first, second]);
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, id);
    super::super::open_path(window.hwnd, &a).unwrap();
    assert_eq!(
        app_mut(window.hwnd).tabs.group(second).unwrap().len(),
        2,
        "no second view in one group"
    );
}

#[test]
fn a_tree_click_replaces_only_the_active_groups_preview() {
    // Break caught: a click in the tree replacing group 1's italic tab while the user works
    // in group 2.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("groups-preview");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let c = scratch.note("c.md", "c");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_note(window.hwnd, &a, super::super::OpenMode::Preview, false).unwrap();
    let first = app_mut(window.hwnd).tabs.active_group();
    // An empty group 2, so group 1's preview is the only view of `a`.
    let second = super::super::split_group(
        window.hwnd,
        first,
        crate::window::split_tree::Direction::Right,
    )
    .unwrap();
    super::super::activate_group(window.hwnd, second);
    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Preview, false).unwrap();
    super::super::open_note(window.hwnd, &c, super::super::OpenMode::Preview, false).unwrap();
    let a_id = app_mut(window.hwnd).tabs.find_path(&a);
    assert!(a_id.is_some(), "group 1's preview kept");
    assert!(
        app_mut(window.hwnd).tabs.find_path(&b).is_none(),
        "group 2's preview replaced"
    );
    assert_eq!(
        app_mut(window.hwnd).tabs.views_of(a_id.unwrap()),
        vec![first]
    );
    assert_eq!(app_mut(window.hwnd).tabs.group(second).unwrap().len(), 1);
}

#[test]
fn deleting_a_note_open_in_two_groups_closes_both_views() {
    // Break caught: a view left open on a deleted file in the group that wasn't active.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("groups-delete");
    let a = scratch.note("a.md", "a");
    scratch.note("keep.md", "k");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &scratch.folder().join("keep.md")).unwrap();
    super::super::open_path(window.hwnd, &a).unwrap();
    execute_command(window.hwnd, CommandId::SplitRight);
    let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
    crate::window::answer_next_confirm(|_| true);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert!(!a.exists());
    assert!(app_mut(window.hwnd).tabs.document(id).is_none());
    assert!(app_mut(window.hwnd).tabs.views_of(id).is_empty());
}

#[test]
fn a_background_replace_in_a_document_shown_in_the_other_group_counts_once() {
    // Break caught: a replace through the document host double-counting a document another
    // group's editor shows (host edit plus that editor's notifications), or moving the
    // active group's caret.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("groups-replace");
    let a = scratch.note("a.md", "alpha beta");
    let b = scratch.note("b.md", "other");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    execute_command(window.hwnd, CommandId::SplitRight);
    super::super::open_path(window.hwnd, &b).unwrap();
    let first = super::super::group_order(window.hwnd)[0];
    let second = super::super::group_order(window.hwnd)[1];
    execute_command(window.hwnd, CommandId::FocusGroup2);
    let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
    let before = app_mut(window.hwnd).tabs.document(id).unwrap().generation;
    let caret = super::super::group_editor(window.hwnd, second)
        .unwrap()
        .view_state()
        .unwrap();
    let matcher = crate::search::Matcher::new("alpha", Default::default()).unwrap();
    assert_eq!(
        super::super::replace_in_document(window.hwnd, id, &matcher, "ALPHA"),
        Some(1)
    );
    let document = app_mut(window.hwnd).tabs.document(id).unwrap();
    assert!(document.dirty);
    assert!(document.generation > before);
    assert_eq!(
        super::super::group_editor(window.hwnd, first)
            .unwrap()
            .text()
            .unwrap(),
        "ALPHA beta"
    );
    assert_eq!(
        super::super::group_editor(window.hwnd, second)
            .unwrap()
            .view_state()
            .unwrap(),
        caret
    );
}

#[test]
fn clicking_an_open_editors_row_focuses_that_group_and_a_header_does_nothing() {
    // Break caught: a click on group 2's row opening a copy in the active group, or a header
    // row acting like a tab.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editors-groups");
    let a = scratch.note("a.md", "a");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    execute_command(window.hwnd, CommandId::SplitRight);
    execute_command(window.hwnd, CommandId::New);
    let first = super::super::group_order(window.hwnd)[0];
    let panel = sidebar_windows(window.hwnd).1;
    let header = notebook_view(window.hwnd).editor_rect_at(0).unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(header));
    mouse(panel, WM_LBUTTONUP, 0, centre(header));
    assert_ne!(app_mut(window.hwnd).tabs.active_group(), first);
    let row = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
    mouse(panel, WM_LBUTTONDOWN, 1, centre(row));
    mouse(panel, WM_LBUTTONUP, 0, centre(row));
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
    let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
    assert_eq!(
        app_mut(window.hwnd).tabs.views_of(id).len(),
        2,
        "no copy made"
    );
}

#[test]
fn ctrl_p_lists_views_in_every_group_and_picking_one_focuses_it() {
    // Break caught: the MRU rows only covering the active group, or a pick adding a view to
    // the active group instead of going to the one listed.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-groups");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &a).unwrap();
    execute_command(window.hwnd, CommandId::FocusGroup2);
    super::super::open_path(window.hwnd, &b).unwrap();
    execute_command(window.hwnd, CommandId::FocusGroup1);
    let (rows, _) = super::super::quick_open_rows(window.hwnd, "");
    let groups: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            crate::window::command_palette::PickerRow::View { number, found, .. } => {
                Some((found.name.clone(), *number))
            }
            _ => None,
        })
        .collect();
    assert!(groups.contains(&("b.md".to_owned(), Some(2))), "{groups:?}");
    let b_id = app_mut(window.hwnd).tabs.find_path(&b).unwrap();
    let second = super::super::group_order(window.hwnd)[1];
    assert!(super::super::focus_view(window.hwnd, second, b_id));
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    assert_eq!(app_mut(window.hwnd).tabs.views_of(b_id), vec![second]);
}
