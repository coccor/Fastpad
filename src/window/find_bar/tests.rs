use super::{SearchDirection, SearchState};
use crate::editor::Editor;
use crate::editor::scintilla_constants::{
    SCI_GETTARGETEND, SCI_SEARCHINTARGET, SCI_SETSEARCHFLAGS, SCI_SETTARGETRANGE,
};
use std::collections::VecDeque;
use std::sync::Mutex;

#[test]
fn find_fields_center_their_text_and_replace_mode_splits_the_bar_without_overlap() {
    // Break caught: field text stuck to the top of its box, or a replacement field that
    // overlaps the query field or runs past the bar.
    use super::{FindBarMode, bar_layout, find_bar_height};
    for dpi in [96, 144] {
        let height = find_bar_height(dpi);
        let layout = bar_layout(800, dpi, 16, FindBarMode::Find);
        let (query, replace, close) = (layout.query, layout.replace, layout.close);
        assert!(replace.is_none());
        // Break caught: a close button overlapping the field or hanging off the bar.
        assert!(query.field.right < close.left && close.right < 800);
        assert_eq!(close.right - close.left, close.bottom - close.top);
        assert!(query.field.top > 0 && query.field.bottom < height - 1);
        let field_middle = (query.field.top + query.field.bottom) / 2;
        let edit_middle = (query.edit.top + query.edit.bottom) / 2;
        assert!((field_middle - edit_middle).abs() <= 1);

        let layout = bar_layout(800, dpi, 16, FindBarMode::Replace);
        let (query, replace) = (layout.query, layout.replace.unwrap());
        assert!(query.field.right < replace.field.left);
        assert!(replace.field.right < layout.close.left);
        assert_eq!(
            query.field.right - query.field.left,
            replace.field.right - replace.field.left
        );
    }
}

#[test]
fn next_match_wraps_once_then_stops() {
    let mut state = SearchState::new("one", SearchDirection::Forward, 8);
    assert_eq!(state.next_range("one two one"), Some(8..11));
    assert_eq!(state.next_range("one two one"), Some(0..3));
    assert_eq!(state.next_range("one two one"), None);
}

#[test]
fn backward_search_wraps_once_then_stops() {
    // Break caught: reusing forward-only bounds for Backward would search the wrong half of
    // the string, or never terminate once wrapped.
    let mut state = SearchState::new("one", SearchDirection::Backward, 3);
    assert_eq!(state.next_range("one two one"), Some(0..3));
    assert_eq!(state.next_range("one two one"), Some(8..11));
    assert_eq!(state.next_range("one two one"), None);
}

#[test]
fn absent_query_never_matches() {
    let mut state = SearchState::new("missing", SearchDirection::Forward, 0);
    assert_eq!(state.next_range("one two one"), None);
    assert_eq!(state.next_range("one two one"), None);
}

#[test]
fn repeated_forward_searches_over_a_single_match_stop_after_the_first_repeat() {
    // Break caught: not tracking `wrapped` across calls lets a lone match be reported forever.
    let mut state = SearchState::new("two", SearchDirection::Forward, 0);
    assert_eq!(state.next_range("one two one"), Some(4..7));
    assert_eq!(state.next_range("one two one"), None);
}

#[derive(Default)]
struct TargetLog {
    responses: VecDeque<isize>,
    ranges: Vec<(usize, isize)>,
    /// Scripted target ends; otherwise a hit ends one needle-length after it starts.
    ends: VecDeque<isize>,
    last_end: isize,
}

unsafe extern "C" fn target_range_stub(
    direct_ptr: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let shared = unsafe { &*(direct_ptr as *const Mutex<TargetLog>) };
    let mut log = shared.lock().unwrap();
    match message {
        SCI_SETTARGETRANGE => {
            log.ranges.push((wparam, lparam));
            0
        }
        SCI_SETSEARCHFLAGS => 0,
        SCI_SEARCHINTARGET => {
            let found = log.responses.pop_front().unwrap_or(-1);
            if found >= 0 {
                log.last_end = found + wparam as isize;
            }
            found
        }
        SCI_GETTARGETEND => {
            let scripted = log.ends.pop_front();
            scripted.unwrap_or(log.last_end)
        }
        _ => 0,
    }
}

#[test]
fn next_editor_match_drives_search_in_target_with_evolving_bounds_and_wraps_once() {
    // Break caught: reusing `str_bounds`' forward-ordered shape (instead of `scintilla_bounds`'
    // reversed one for Backward), or not retrying once after the first miss, would search the
    // wrong range or never find the wrapped match.
    let log = Mutex::new(TargetLog {
        responses: VecDeque::from([8_isize, -1, 0, -1]),
        ..TargetLog::default()
    });
    let editor = Editor::test_fixture(target_range_stub, &log as *const Mutex<TargetLog> as isize);
    let mut state = SearchState::new("one", SearchDirection::Forward, 8);

    assert_eq!(
        state.next_editor_match(&editor, 0, 11).unwrap(),
        Some(8..11)
    );
    assert_eq!(state.next_editor_match(&editor, 0, 11).unwrap(), Some(0..3));
    assert_eq!(state.next_editor_match(&editor, 0, 11).unwrap(), None);

    assert_eq!(
        log.lock().unwrap().ranges,
        vec![(8, 11), (11, 11), (0, 8), (3, 8)]
    );
}

#[test]
fn plain_options_map_to_scintilla_flags_and_regex_adds_none() {
    // Break caught: the toggles changing nothing in plain mode, or a Scintilla regex flag
    // left in, so regex mode would search with a second dialect.
    use super::search_flags;
    use crate::editor::scintilla_constants::{SCFIND_MATCHCASE, SCFIND_WHOLEWORD};
    use crate::search::MatchOptions;
    let plain = MatchOptions::default();
    let case = MatchOptions {
        case: true,
        ..plain
    };
    let word = MatchOptions {
        whole_word: true,
        ..plain
    };
    let all = MatchOptions {
        case: true,
        whole_word: true,
        regex: true,
    };
    assert_eq!(search_flags(plain), 0);
    assert_eq!(search_flags(case), SCFIND_MATCHCASE);
    assert_eq!(search_flags(word), SCFIND_WHOLEWORD);
    assert_eq!(search_flags(all), SCFIND_MATCHCASE | SCFIND_WHOLEWORD);
}

#[test]
fn regex_mode_wraps_once_each_way_and_a_pattern_error_is_no_matcher() {
    // Break caught: find next restarting at the caret's own match, find previous taking a
    // match that ends after the caret, no wrap, or `\d*` (empty-capable) or `(` searched
    // at all instead of shown as no match.
    use super::{regex_match, regex_matcher};
    use crate::search::MatchOptions;
    let options = MatchOptions::default();
    let matcher = regex_matcher(r"\d+", options).unwrap();
    let text = "a1 b22\nc333";
    let find = |origin, direction| regex_match(&matcher, text, origin, direction);
    assert_eq!(find(0, SearchDirection::Forward), Some(1..2));
    assert_eq!(find(2, SearchDirection::Forward), Some(4..6));
    assert_eq!(find(6, SearchDirection::Forward), Some(8..11), "next line");
    assert_eq!(find(11, SearchDirection::Forward), Some(1..2), "wrapped");
    assert_eq!(find(8, SearchDirection::Backward), Some(4..6));
    assert_eq!(find(5, SearchDirection::Backward), Some(1..2));
    assert_eq!(find(1, SearchDirection::Backward), Some(8..11), "wrapped");
    assert!(regex_matcher(r"\d*", options).is_none());
    assert!(regex_matcher("(", options).is_none());
    assert!(regex_matcher("", options).is_none());
}

#[test]
fn a_regex_find_next_in_a_megabyte_note_takes_well_under_a_frame() {
    // Break caught: a regex find next that copies, recompiles per step or rescans the text
    // more than once, putting F3 in a 1 MB note past one 16 ms frame. The worst case is a
    // miss: the whole text after the caret, then the whole text again after the wrap.
    // Measured in release only (`cargo test --release`), like the matcher's own budget test.
    use super::{regex_match, regex_matcher};
    use crate::search::MatchOptions;
    let line = "Plain text with a café, some numbers 12345 and Îndemn words.\r\n";
    let text = line.repeat(1_048_576 / line.len());
    let options = MatchOptions {
        whole_word: true,
        ..MatchOptions::default()
    };
    let started = std::time::Instant::now();
    let matcher = regex_matcher(r"invoice\s+\d{4}", options).unwrap();
    assert_eq!(
        regex_match(&matcher, &text, text.len() / 2, SearchDirection::Forward),
        None
    );
    let elapsed = started.elapsed();
    eprintln!("regex find next over {} bytes: {elapsed:?}", text.len());
    if !cfg!(debug_assertions) {
        assert!(elapsed < std::time::Duration::from_millis(8), "{elapsed:?}");
    }
}

#[test]
fn the_query_field_leaves_room_for_the_three_toggles() {
    // Break caught: typed text running under the toggles, or toggles outside the field.
    use super::{FindBarMode, bar_layout};
    use crate::window::option_toggles::toggle_rects;
    for dpi in [96, 144] {
        for mode in [FindBarMode::Find, FindBarMode::Replace] {
            let query = bar_layout(800, dpi, 16, mode).query;
            let rects = toggle_rects(query.field, dpi);
            assert!(query.edit.right <= rects[0].left, "{dpi} {mode:?}");
            assert!(rects[2].right <= query.field.right, "{dpi} {mode:?}");
            for rect in rects {
                assert!(rect.top >= query.field.top && rect.bottom <= query.field.bottom);
            }
        }
    }
}

#[test]
fn next_editor_match_with_an_empty_query_never_calls_scintilla() {
    let log = Mutex::new(TargetLog::default());
    let editor = Editor::test_fixture(target_range_stub, &log as *const Mutex<TargetLog> as isize);
    let mut state = SearchState::new("", SearchDirection::Forward, 0);

    assert_eq!(state.next_editor_match(&editor, 0, 11).unwrap(), None);
    assert!(log.lock().unwrap().ranges.is_empty());
}
