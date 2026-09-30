use super::*;
use crate::library::tree::{RowKind, TreeRow};
use crate::window::row_list::RowListState;
use std::cell::Cell;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};

thread_local! {
    static ITEMS_BUILT: Cell<usize> = const { Cell::new(0) };
}

const ROW: RECT = RECT {
    left: 0,
    top: 0,
    right: 100,
    bottom: 26,
};

fn fake_container(_: HWND) -> (String, u32) {
    ("Fake notes".to_owned(), ROLE_SYSTEM_LIST)
}
fn fake_count(_: HWND) -> usize {
    10_000
}
fn fake_item(_: HWND, index: usize) -> Option<AccessibleItem> {
    ITEMS_BUILT.set(ITEMS_BUILT.get() + 1);
    (index < 10_000).then(|| list_item(&format!("Note {index}"), index == 3, false, ROW, true))
}
fn fake_hit(_: HWND, _: POINT) -> Option<usize> {
    Some(7)
}
fn fake_current(_: HWND) -> Option<usize> {
    Some(3)
}
fn fake_select(_: HWND, _: usize) {}
fn fake_activate(_: HWND, _: usize) {}
fn fake_identity(_: HWND, _: usize) -> Option<u64> {
    None
}
fn fake_generation(_: HWND) -> u64 {
    0
}

static FAKE: AccessibleSource = AccessibleSource {
    container: fake_container,
    count: fake_count,
    item: fake_item,
    hit: fake_hit,
    current: fake_current,
    select: fake_select,
    activate: fake_activate,
    identity: fake_identity,
    generation: fake_generation,
};

fn read_bstr(value: BSTR) -> String {
    let text = unsafe { std::slice::from_raw_parts(value, SysStringLen(value) as usize) };
    let result = String::from_utf16_lossy(text);
    unsafe { SysFreeString(value) };
    result
}

fn row(kind: RowKind, name: &str, depth: u16, pinned: bool, expanded: bool) -> TreeRow {
    TreeRow {
        kind,
        depth,
        name: name.to_owned(),
        pinned,
        expanded,
    }
}

#[test]
fn a_ten_thousand_row_list_is_counted_without_building_its_items() {
    // Break caught: accChildCount building every row's name, making each screen-reader call
    // cost O(rows) on a 10,000-note notebook.
    let provider = create_provider(std::ptr::null_mut(), &FAKE);
    ITEMS_BUILT.set(0);
    let mut count = 0;
    unsafe {
        assert_eq!(
            (SIDEBAR_VTABLE.get_acc_child_count)(provider, &mut count),
            S_OK
        );
    }
    assert_eq!(count, 10_000);
    assert_eq!(ITEMS_BUILT.get(), 0);
    let mut name: BSTR = std::ptr::null();
    unsafe {
        assert_eq!(
            (SIDEBAR_VTABLE.get_acc_name)(provider, RawVariant::integer(10_000), &mut name),
            S_OK
        );
    }
    assert_eq!(read_bstr(name), "Note 9999");
    assert_eq!(ITEMS_BUILT.get(), 1);
    unsafe {
        assert_eq!(
            (SIDEBAR_VTABLE.get_acc_name)(provider, RawVariant::integer(10_001), &mut name),
            E_INVALIDARG
        );
        (SIDEBAR_VTABLE.release)(provider);
    }
}

#[test]
fn the_container_and_children_report_their_roles_states_and_selection() {
    // Break caught: the list announced as a generic client area, or the selected row not
    // reported through accSelection.
    let provider = create_provider(std::ptr::null_mut(), &FAKE);
    let table = &SIDEBAR_VTABLE;
    unsafe {
        let mut value = RawVariant::empty();
        assert_eq!(
            (table.get_acc_role)(provider, RawVariant::integer(0), &mut value),
            S_OK
        );
        assert_eq!(value.child_id(), Some(ROLE_SYSTEM_LIST as i32));
        assert_eq!(
            (table.get_acc_role)(provider, RawVariant::integer(4), &mut value),
            S_OK
        );
        assert_eq!(value.child_id(), Some(ROLE_SYSTEM_LISTITEM as i32));
        assert_eq!(
            (table.get_acc_state)(provider, RawVariant::integer(4), &mut value),
            S_OK
        );
        let state = value.child_id().unwrap() as u32;
        assert_ne!(state & STATE_SELECTED, 0);
        assert_ne!(state & STATE_SELECTABLE, 0);
        assert_eq!((table.get_acc_selection)(provider, &mut value), S_OK);
        assert_eq!(value.child_id(), Some(4));
        // Nothing has the focus in a window-less fixture.
        assert_eq!((table.get_acc_focus)(provider, &mut value), S_FALSE);
        let mut name: BSTR = std::ptr::null();
        assert_eq!(
            (table.get_acc_name)(provider, RawVariant::integer(0), &mut name),
            S_OK
        );
        assert_eq!(read_bstr(name), "Fake notes");
        let mut next = RawVariant::empty();
        assert_eq!(
            (table.acc_navigate)(
                provider,
                NAVDIR_NEXT as i32,
                RawVariant::integer(1),
                &mut next
            ),
            S_OK
        );
        assert_eq!(next.child_id(), Some(2));
        assert_eq!(
            (table.acc_navigate)(
                provider,
                NAVDIR_NEXT as i32,
                RawVariant::integer(10_000),
                &mut next
            ),
            S_FALSE
        );
        (table.release)(provider);
    }
}

#[test]
fn tree_rows_are_outline_items_with_expansion_and_pin_in_their_names() {
    // Break caught: a folder's expanded state missing, or a pin conveyed only by the filled
    // icon.
    let folder = tree_item(
        &row(
            RowKind::Folder(PathBuf::from("Work")),
            "Work",
            0,
            false,
            true,
        ),
        false,
        false,
        ROW,
        true,
    );
    assert_eq!(folder.role, ROLE_SYSTEM_OUTLINEITEM);
    assert_ne!(folder.state & STATE_EXPANDED, 0);
    assert_eq!(folder.state & STATE_COLLAPSED, 0);
    assert_eq!(folder.value, "1", "one level under the notebook's root row");
    let closed = tree_item(
        &row(
            RowKind::Folder(PathBuf::from("Old")),
            "Old",
            1,
            false,
            false,
        ),
        false,
        false,
        ROW,
        true,
    );
    assert_ne!(closed.state & STATE_COLLAPSED, 0);
    assert_eq!(closed.value, "2");

    let pinned = tree_item(
        &row(RowKind::Note(PathBuf::from("a.md")), "a.md", 1, true, false),
        true,
        true,
        ROW,
        false,
    );
    assert_eq!(
        pinned.name, "a.md, Markdown, pinned",
        "the type, then the pin"
    );
    assert_eq!(pinned.state & (STATE_EXPANDED | STATE_COLLAPSED), 0);
    assert_ne!(pinned.state & STATE_SELECTED, 0);
    assert_ne!(pinned.state & STATE_FOCUSED, 0);
    assert_ne!(pinned.state & STATE_OFFSCREEN, 0);

    // Break caught: a note's type conveyed by its coloured icon alone (spec §5.4).
    let csv = tree_item(
        &row(
            RowKind::Note(PathBuf::from(r"Work\budget.CSV")),
            "budget.CSV",
            1,
            false,
            false,
        ),
        false,
        false,
        ROW,
        true,
    );
    assert_eq!(csv.name, "budget.CSV, CSV");
    let markdown = tree_item(
        &row(
            RowKind::Note(PathBuf::from("meeting notes.md")),
            "meeting notes.md",
            0,
            false,
            false,
        ),
        false,
        false,
        ROW,
        true,
    );
    assert_eq!(markdown.name, "meeting notes.md, Markdown");
    assert_eq!(folder.name, "Work", "folder rows are unchanged");
}

#[test]
fn rows_scrolled_out_of_the_list_are_offscreen() {
    // Break caught: a screen reader told that row 5,000 sits at the top of the list.
    let mut list = RowListState::new(26);
    list.set_count(100);
    list.top = 10;
    let area = RECT {
        left: 0,
        top: 38,
        right: 200,
        bottom: 38 + 26 * 5,
    };
    let (rect, visible) = row_rect(area, &list, 10);
    assert_eq!((rect.top, rect.bottom, visible), (38, 64, true));
    assert!(!row_rect(area, &list, 9).1);
    assert!(!row_rect(area, &list, 15).1);
    assert_eq!(row_rect(area, &list, 12).0.top, 38 + 52);
}

#[test]
fn events_announce_selection_focus_state_and_reorders() {
    // Break caught: no event when the selection moves, so a screen reader keeps reading the
    // old row, or no state change when a folder expands or a note is pinned in place.
    let mark = |current, state, name: &str, count| AccessibleMark {
        current,
        state,
        name: name.to_owned(),
        count,
        ..AccessibleMark::default()
    };
    assert_eq!(
        events_between(&mark(Some(1), 0, "a", 5), &mark(Some(2), 0, "b", 5), true),
        vec![(EVENT_OBJECT_SELECTION, 3), (EVENT_OBJECT_FOCUS, 3)]
    );
    assert_eq!(
        events_between(&mark(Some(1), 0, "a", 5), &mark(Some(2), 0, "b", 5), false),
        vec![(EVENT_OBJECT_SELECTION, 3)]
    );
    assert_eq!(
        events_between(
            &mark(Some(0), STATE_COLLAPSED, "Work", 5),
            &mark(Some(0), STATE_EXPANDED, "Work", 9),
            true
        ),
        vec![(EVENT_OBJECT_REORDER, 0), (EVENT_OBJECT_STATECHANGE, 1)]
    );
    assert_eq!(
        events_between(
            &mark(Some(0), 0, "a", 5),
            &mark(Some(0), 0, "a, pinned", 5),
            true
        ),
        vec![(EVENT_OBJECT_STATECHANGE, 1), (EVENT_OBJECT_NAMECHANGE, 1)]
    );
    assert!(events_between(&mark(None, 0, "", 0), &mark(None, 0, "", 0), true).is_empty());
}

#[test]
fn a_pin_that_re_sorts_the_row_raises_reorder_selection_and_state_change() {
    // Break caught: pinning a note that is not first moves it to the top at the same count,
    // announcing only a new selection, with no state change and no reorder of its siblings.
    let before = AccessibleMark {
        current: Some(3),
        name: "b".to_owned(),
        count: 5,
        identity: Some(7),
        generation: 1,
        ..AccessibleMark::default()
    };
    let after = AccessibleMark {
        current: Some(0),
        name: "b, pinned".to_owned(),
        generation: 2,
        ..before.clone()
    };
    assert_eq!(
        events_between(&before, &after, true),
        vec![
            (EVENT_OBJECT_REORDER, 0),
            (EVENT_OBJECT_SELECTION, 1),
            (EVENT_OBJECT_FOCUS, 1),
            (EVENT_OBJECT_STATECHANGE, 1),
            (EVENT_OBJECT_NAMECHANGE, 1),
        ]
    );
    // Another row now at the same index is a new selection, not a state change.
    let other = AccessibleMark {
        identity: Some(8),
        ..before.clone()
    };
    assert_eq!(
        events_between(&before, &other, false),
        vec![(EVENT_OBJECT_SELECTION, 4)]
    );
}

#[test]
fn a_query_message_with_an_unknown_pointer_is_ignored() {
    // Break caught: any process sending WM_APP + 0x60 with a junk lParam crashing FastPad by
    // having it written through as a `Call`.
    assert_eq!(unsafe { answer(std::ptr::null_mut(), 0x10) }, 0);
    assert_eq!(unsafe { answer(std::ptr::null_mut(), -1) }, 0);
}

#[test]
fn the_chevron_says_whether_it_is_expanded_and_an_unavailable_button_says_so() {
    // Break caught: a chevron read as a plain button with no hint of what it opens, or
    // Replace all read as pressable while a search still runs.
    let open = expander_item("Toggle replace", true, ROW);
    assert_eq!(open.role, ROLE_SYSTEM_PUSHBUTTON);
    assert_ne!(open.state & STATE_EXPANDED, 0);
    assert_eq!(open.state & (STATE_COLLAPSED | STATE_FOCUSABLE), 0);
    assert_eq!(default_action(&open), "Press");
    let closed = expander_item("Toggle replace", false, ROW);
    assert_ne!(closed.state & STATE_COLLAPSED, 0);
    assert_eq!(closed.state & STATE_EXPANDED, 0);

    let ready = action_item("Replace all", true, ROW);
    assert_eq!(ready.role, ROLE_SYSTEM_PUSHBUTTON);
    assert_eq!(ready.state & (STATE_UNAVAILABLE | STATE_FOCUSABLE), 0);
    assert_ne!(
        action_item("Replace all", false, ROW).state & STATE_UNAVAILABLE,
        0
    );
}

#[test]
fn default_actions_follow_the_item_kind() {
    // Break caught: a folder offering "Open", or a button offering nothing to a screen
    // reader's default-action command.
    let button = button_item("Search", true, false, ROW);
    assert_eq!(default_action(&button), "Press");
    assert_ne!(button.state & STATE_PRESSED, 0);
    let folder = tree_item(
        &row(RowKind::Folder(PathBuf::from("w")), "w", 0, false, false),
        false,
        false,
        ROW,
        true,
    );
    assert_eq!(default_action(&folder), "Expand");
    let note = list_item("n", false, false, ROW, true);
    assert_eq!(default_action(&note), "Open");
}

#[test]
fn toggles_are_check_buttons_fields_are_text_and_lines_are_static_text() {
    // Break caught: a toggle read as a push button with no checked state, or a status line
    // offering "Open" as its default action.
    let on = check_item("Match case", true, ROW);
    assert_eq!(on.role, ROLE_SYSTEM_CHECKBUTTON);
    assert_ne!(on.state & STATE_CHECKED, 0);
    assert_eq!(default_action(&on), "Uncheck");
    let off = check_item("Match case", false, ROW);
    assert_eq!(off.state & STATE_CHECKED, 0);
    assert_eq!(default_action(&off), "Check");

    let field = field_item("Find", "abc".to_owned(), true, ROW, std::ptr::null_mut());
    assert_eq!(field.role, ROLE_SYSTEM_TEXT);
    assert_eq!(field.value, "abc");
    assert_ne!(field.state & STATE_FOCUSED, 0);
    assert_eq!(default_action(&field), "");

    let line = text_item("3 notes", ROW);
    assert_eq!(line.role, ROLE_SYSTEM_STATICTEXT);
    assert_ne!(line.state & STATE_READONLY, 0);
    assert_eq!(default_action(&line), "");
    assert_eq!(
        default_action(&button_item("Close", false, false, ROW)),
        "Press"
    );
}
