//! The find bar: options, toggles, replace, regex and whole-word matching.

use super::*;

fn set_find_query(hwnd: HWND, text: &str) {
    let edit = app_mut(hwnd).find_bar().unwrap().query_hwnd();
    let wide = crate::platform::wide_null(text);
    // Sends EN_CHANGE, handled with nothing of the App borrowed here.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
    }
}

#[test]
fn the_find_bar_passes_its_options_to_scintilla_and_a_bad_regex_is_a_miss() {
    // Break caught: toggles that change nothing, whole word matching inside foo_bar, regex
    // mode not reaching the `regex` crate (no `{2}`), or an invalid pattern reported as an
    // error or leaving no trace.
    use crate::search::SearchOption;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor
        .populate_clean("Foo foo foobar foo_bar foo. a1 b22")
        .unwrap();
    execute_command(window.hwnd, CommandId::Find);
    let bar = || app_mut(window.hwnd).find_bar().unwrap();

    set_find_query(window.hwnd, "foo");
    super::super::toggle_find_option(window.hwnd, SearchOption::Case);
    editor.set_selection(0..0).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 4..7, "match case skips Foo");

    super::super::toggle_find_option(window.hwnd, SearchOption::Case);
    super::super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
    editor.set_selection(7..7).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(
        editor.selection().unwrap(),
        23..26,
        "whole word skips foobar and foo_bar"
    );

    super::super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    set_find_query(window.hwnd, r"b\d{2}");
    editor.set_selection(0..0).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 31..34);
    assert!(!bar().no_match());

    set_find_query(window.hwnd, "(");
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 31..34, "the selection stays");
    assert!(bar().no_match());
    assert!(notices(window.hwnd).is_empty());
    set_find_query(window.hwnd, "a1");
    assert!(!bar().no_match(), "typing clears the no-match state");
}

#[test]
fn alt_keys_and_clicks_flip_the_find_bar_toggles() {
    // Break caught: Alt+C opening a menu instead of flipping match case, a toggle click that
    // does nothing, or the letter reaching the menu band after the flip.
    use crate::search::MatchOptions;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_SYSCHAR, WM_SYSKEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::Find);
    let bar = || app_mut(window.hwnd).find_bar().unwrap();
    let (query, panel) = (bar().query_hwnd(), bar().panel_hwnd());
    let alt = 1 << 29;

    unsafe { SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'C'), alt) };
    assert!(bar().options().case);
    unsafe { SendMessageW(query, WM_SYSCHAR, usize::from(b'c'), alt) };
    assert_eq!(app_mut(window.hwnd).menu_mode, None);
    unsafe {
        SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'W'), alt);
        SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'R'), alt);
    }
    assert_eq!(
        bar().options(),
        MatchOptions {
            case: true,
            whole_word: true,
            regex: true
        }
    );

    let rect = bar().toggle_rects()[0];
    let x = (rect.left + rect.right) / 2;
    let y = (rect.top + rect.bottom) / 2;
    let point = ((y as u32) << 16 | (x as u32 & 0xffff)) as super::super::LPARAM;
    super::super::panel_pointer(window.hwnd, panel, WM_LBUTTONUP, 0, point);
    assert!(!bar().options().case, "a click on Aa turns match case off");
}

#[test]
fn replace_current_replaces_a_selection_that_matches_under_the_options() {
    // Break caught: Enter in Replace comparing the selection to the query byte for byte, so
    // a case-insensitive "CAT" is skipped instead of replaced.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("CAT cat").unwrap();
    execute_command(window.hwnd, CommandId::Replace);
    set_find_query(window.hwnd, "cat");
    let replace = app_mut(window.hwnd).find_bar().unwrap().replace_hwnd();
    let dog = crate::platform::wide_null("dog");
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(replace, dog.as_ptr());
    }
    editor.set_selection(0..3).unwrap();

    super::super::replace_current(window.hwnd);

    assert_eq!(editor.text().unwrap(), "dog cat");
    assert_eq!(editor.selection().unwrap(), 4..7);
}

#[test]
fn opening_a_result_seeds_the_find_bar_with_search_options_and_f3_steps_on() {
    // Break caught: the find bar keeping its own options (so match case is lost), the first
    // match not selected, or F3 and Shift+F3 not reaching the next and previous matches.
    use crate::search::{MatchOptions, SearchOption};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("result-seed");
    scratch.note("a.md", "beta Beta beta Beta");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    crate::window::search_view::toggle_option(window.hwnd, SearchOption::Case);
    type_into_search(window.hwnd, "Beta");
    pump_until(window.hwnd, || {
        crate::window::search_view::shown_results(window.hwnd).len() == 1
    });

    crate::window::search_view::open_selected(window.hwnd, super::super::OpenMode::Preview, true);

    assert_eq!(editor.text().unwrap(), "beta Beta beta Beta");
    assert_eq!(editor.selection().unwrap(), 5..9);
    let bar = app_mut(window.hwnd).find_bar().unwrap();
    assert!(bar.is_visible());
    assert_eq!(bar.query_text(), "Beta");
    assert_eq!(
        bar.options(),
        MatchOptions {
            case: true,
            ..MatchOptions::default()
        }
    );
    assert!(!bar.no_match());
    execute_command(window.hwnd, CommandId::FindNext);
    assert_eq!(editor.selection().unwrap(), 15..19);
    execute_command(window.hwnd, CommandId::FindPrevious);
    assert_eq!(editor.selection().unwrap(), 5..9);
}

#[test]
fn opening_a_result_whose_text_changed_shows_no_match() {
    // Break caught (review focus 5): a stale result opening nothing, panicking on its
    // snippet, selecting text that no longer matches, or reporting the miss as an error.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("result-stale");
    let note = scratch.note("a.md", "alpha beta");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
    type_into_search(window.hwnd, "beta");
    pump_until(window.hwnd, || {
        crate::window::search_view::shown_results(window.hwnd).len() == 1
    });
    std::fs::write(&note, "alpha gamma").unwrap();
    let before = notices(window.hwnd).len();

    crate::window::search_view::open_selected(window.hwnd, super::super::OpenMode::Preview, false);

    assert_eq!(editor.text().unwrap(), "alpha gamma");
    assert_eq!(editor.selection().unwrap(), 0..0);
    let bar = app_mut(window.hwnd).find_bar().unwrap();
    assert!(bar.is_visible());
    assert_eq!(bar.query_text(), "beta");
    assert!(bar.no_match());
    assert_eq!(notices(window.hwnd).len(), before);
}

fn set_replace_text(hwnd: HWND, text: &str) {
    let edit = app_mut(hwnd).find_bar().unwrap().replace_hwnd();
    let wide = crate::platform::wide_null(text);
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
    }
}

#[test]
fn a_regex_that_can_match_empty_text_shows_no_match_and_replaces_nothing() {
    // Break caught: `\d*` searched at all (a hang stepping past empty matches, or the empty
    // match selected at the caret), or Replace All inserting the replacement between
    // characters. Search rejects such a pattern too (spec §6).
    use crate::search::SearchOption;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("abc 123").unwrap();
    execute_command(window.hwnd, CommandId::Find);
    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    set_find_query(window.hwnd, r"\d*");
    let bar = || app_mut(window.hwnd).find_bar().unwrap();

    editor.set_selection(1..1).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 1..1, "F3");
    assert!(bar().no_match());
    super::super::find_previous(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 1..1, "Shift+F3");
    assert!(bar().no_match());

    execute_command(window.hwnd, CommandId::Replace);
    set_find_query(window.hwnd, "a*");
    set_replace_text(window.hwnd, "y");
    super::super::replace_all_matches(window.hwnd);
    assert_eq!(editor.text().unwrap(), "abc 123");
    assert!(bar().no_match());
}

#[test]
fn the_find_bars_regex_replace_expands_groups_and_plain_replace_is_literal() {
    // Break caught (spec §12a): the find bar's regex Replace inserting "$2/${year}" as
    // typed, Replace All expanding every match with the first match's groups, Enter using
    // another match's captures, group numbers shifted by the whole-word wrapper, or plain
    // mode expanding `$1`.
    use crate::search::SearchOption;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor
        .populate_clean("2024-09 and 1999-01\r\n2001-12")
        .unwrap();
    execute_command(window.hwnd, CommandId::Replace);
    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    set_find_query(window.hwnd, r"(?<year>\d{4})-(\d{2})");
    set_replace_text(window.hwnd, "$2/${year} $$");

    editor.set_selection(12..19).unwrap();
    super::super::replace_current(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "2024-09 and 01/1999 $\r\n2001-12",
        "Enter expands the selected match's own groups"
    );
    assert_eq!(
        editor.selection().unwrap(),
        23..30,
        "then moves to the next"
    );

    super::super::replace_all_matches(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "09/2024 $ and 01/1999 $\r\n12/2001 $"
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.text().unwrap(),
        "2024-09 and 01/1999 $\r\n2001-12",
        "Replace All is one undo step"
    );

    super::super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
    set_find_query(window.hwnd, "(fo+)");
    set_replace_text(window.hwnd, "<$1>");
    editor.populate_clean("foo foobar fooo").unwrap();
    super::super::replace_all_matches(window.hwnd);
    assert_eq!(editor.text().unwrap(), "<foo> foobar <fooo>");

    super::super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    set_find_query(window.hwnd, "$1");
    set_replace_text(window.hwnd, "$2$$");
    editor.populate_clean("a $1 b $1").unwrap();
    editor.set_selection(2..4).unwrap();
    super::super::replace_current(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "a $2$$ b $1",
        "plain Enter is literal"
    );
    super::super::replace_all_matches(window.hwnd);
    assert_eq!(
        editor.text().unwrap(),
        "a $2$$ b $2$$",
        "plain Replace All too"
    );
}

#[test]
fn a_case_insensitive_regex_folds_accented_capitals_and_f3_wraps() {
    // Break caught: case folding limited to ASCII (MSVC std::wregex), so "îndemn" never
    // finds "Îndemn" though Search does, or F3 and Shift+F3 not wrapping at the ends.
    use crate::search::SearchOption;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor
        .populate_clean("Îndemn la drum, cu Élan\nîndemn")
        .unwrap();
    execute_command(window.hwnd, CommandId::Find);
    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    let bar = || app_mut(window.hwnd).find_bar().unwrap();

    set_find_query(window.hwnd, "élan");
    editor.set_selection(0..0).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 20..25);
    assert!(!bar().no_match());

    set_find_query(window.hwnd, "îndemn");
    editor.set_selection(0..0).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 0..7);
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 26..33, "the next line");
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 0..7, "F3 wraps");
    super::super::find_previous(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 26..33, "Shift+F3 wraps");
    super::super::find_previous(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 0..7);
    assert!(!bar().no_match());
}

#[test]
fn typing_a_replacement_keeps_the_no_match_outline_and_typing_a_query_clears_it() {
    // Break caught: the replacement field's EN_CHANGE clearing the query's no-match
    // outline, though the query still matches nothing.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("alpha").unwrap();
    execute_command(window.hwnd, CommandId::Replace);
    set_find_query(window.hwnd, "zeta");
    super::super::find_next(window.hwnd);
    let bar = || app_mut(window.hwnd).find_bar().unwrap();
    assert!(bar().no_match());

    set_replace_text(window.hwnd, "beta");
    assert!(bar().no_match(), "the replacement doesn't change the match");
    set_find_query(window.hwnd, "alp");
    assert!(!bar().no_match());
}

#[test]
fn a_whole_word_regex_matches_a_non_ascii_word_only_as_a_whole_word() {
    // Break caught: a whole-word regex whose `\b` counts ă as a non-word character (as MSVC
    // std::wregex's does), so "mașină" is missed as a whole word and found inside
    // "mașinării". The `regex` crate's Unicode `\b` agrees with plain whole word here.
    use crate::search::SearchOption;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::Find);
    super::super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
    set_find_query(window.hwnd, "mașină");
    let bar = || app_mut(window.hwnd).find_bar().unwrap();
    let find = |text: &str, backward: bool| {
        editor.populate_clean(text).unwrap();
        let end = editor.length().unwrap();
        editor
            .set_selection(if backward { end..end } else { 0..0 })
            .unwrap();
        if backward {
            super::super::find_previous(window.hwnd);
        } else {
            super::super::find_next(window.hwnd);
        }
        (!bar().no_match()).then(|| editor.selection().unwrap())
    };

    // Plain whole word (SCFIND_WHOLEWORD) is the reference.
    assert_eq!(find("o mașină nouă", false), Some(2..10), "plain");
    assert_eq!(find("mașinării", false), None, "plain");

    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    for backward in [false, true] {
        assert_eq!(find("o mașină nouă", backward), Some(2..10), "{backward}");
        assert_eq!(find("mașină", backward), Some(0..8), "{backward}");
        assert_eq!(find("mașinării", backward), None, "{backward}");
        assert_eq!(
            find("mașinării mașină", backward),
            Some(12..20),
            "past the longer word, {backward}"
        );
    }
}

#[test]
fn a_whole_word_regex_skips_longer_words_forward_backward_and_in_replace() {
    // Break caught: a hit inside foobar or foo_bar accepted, a rejected hit ending the
    // search instead of being stepped past, Shift+F3 stopping at the rejected hit, or
    // Replace All replacing inside longer words.
    use crate::search::SearchOption;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean("foobar foo_bar foo x").unwrap();
    execute_command(window.hwnd, CommandId::Find);
    super::super::toggle_find_option(window.hwnd, SearchOption::Regex);
    super::super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
    set_find_query(window.hwnd, "fo+");
    let bar = || app_mut(window.hwnd).find_bar().unwrap();

    editor.set_selection(0..0).unwrap();
    super::super::find_next(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 15..18, "F3");
    editor.set_selection(20..20).unwrap();
    super::super::find_previous(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 15..18, "Shift+F3");
    editor.set_selection(15..15).unwrap();
    super::super::find_previous(window.hwnd);
    assert_eq!(editor.selection().unwrap(), 15..18, "Shift+F3 wraps to it");
    assert!(!bar().no_match());

    editor.set_selection(0..0).unwrap();
    execute_command(window.hwnd, CommandId::Replace);
    set_find_query(window.hwnd, "fo+");
    set_replace_text(window.hwnd, "X");
    super::super::replace_all_matches(window.hwnd);
    assert_eq!(editor.text().unwrap(), "foobar foo_bar X x");

    // Several whole words go in one undo step.
    editor.populate_clean("fooo foobar fo").unwrap();
    super::super::replace_all_matches(window.hwnd);
    assert_eq!(editor.text().unwrap(), "X foobar X");
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), "fooo foobar fo");

    // Enter in Replace replaces a selected whole word, and not a selection inside one.
    editor.populate_clean("foo foobar").unwrap();
    editor.set_selection(4..7).unwrap();
    super::super::replace_current(window.hwnd);
    assert_eq!(editor.text().unwrap(), "foo foobar", "inside foobar");
    editor.set_selection(0..3).unwrap();
    super::super::replace_current(window.hwnd);
    assert_eq!(editor.text().unwrap(), "X foobar");
}
