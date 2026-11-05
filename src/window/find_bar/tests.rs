use super::{SearchDirection, SearchState};
use crate::editor::Editor;
use crate::editor::scintilla_constants::{
    SCI_GETLENGTH, SCI_GETTARGETEND, SCI_SEARCHINTARGET, SCI_SETSEARCHFLAGS, SCI_SETTARGETRANGE,
};
use std::collections::VecDeque;
use std::sync::Mutex;

#[test]
fn find_fields_center_their_text_and_replace_mode_adds_a_second_row_below() {
    // Break caught: field text stuck to the top of its box, a replacement field beside the query
    // field instead of under it (VS Code's layout), or a Replace row that overflows the widget.
    use super::{FindBarMode, bar_layout, find_bar_height};
    for dpi in [96, 144] {
        let width = 460 * dpi as i32 / 96;
        let height = find_bar_height(dpi, FindBarMode::Find);
        let layout = bar_layout(width, dpi, 16, FindBarMode::Find, 0);
        let (query, replace, close) = (layout.query, layout.replace, layout.close);
        assert!(replace.is_none());
        assert!(layout.replace_one.is_none() && layout.replace_all.is_none());
        // Break caught: a close button overlapping the field or hanging off the widget.
        assert!(query.field.right < close.left && close.right < width);
        assert!(query.field.top > 0 && query.field.bottom < height);
        let field_middle = (query.field.top + query.field.bottom) / 2;
        let edit_middle = (query.edit.top + query.edit.bottom) / 2;
        assert!((field_middle - edit_middle).abs() <= 1);
        // The chevron runs down the left edge, left of the fields.
        assert!(layout.chevron.right <= query.field.left);
        assert!(layout.chevron.left >= 0 && layout.chevron.bottom <= height);

        let replace_height = find_bar_height(dpi, FindBarMode::Replace);
        assert!(replace_height > height);
        let layout = bar_layout(width, dpi, 16, FindBarMode::Replace, 0);
        let (query, replace) = (layout.query, layout.replace.unwrap());
        assert!(query.field.bottom < replace.field.top, "{dpi}: second row");
        assert!(replace.field.bottom < replace_height);
        assert_eq!(query.field.left, replace.field.left);
        assert_eq!(query.field.right, replace.field.right);
        let (one, all) = (layout.replace_one.unwrap(), layout.replace_all.unwrap());
        assert!(replace.field.right < one.left && one.right <= all.left);
        assert!(all.right < width);
        assert_eq!(one.top, replace.field.top);
        assert!(layout.chevron.bottom > replace.field.top, "spans both rows");
    }
}

#[test]
fn the_widget_keeps_its_width_and_narrows_only_for_a_small_editor() {
    use super::find_bar_width;
    assert_eq!(find_bar_width(2000, 96), 460);
    assert_eq!(find_bar_width(400, 96), 392);
    assert_eq!(
        find_bar_width(100, 96),
        260,
        "never narrower than the minimum"
    );
    assert_eq!(find_bar_width(3000, 144), 690);
}

#[test]
fn the_find_bar_and_its_fields_hold_the_body_text_at_every_text_size() {
    let _factor = crate::window::design::text_scale::FactorGuard::new();
    // Break caught: a find field that clips the larger Windows text size inside a 28 px box.
    use super::{FIELD_HEIGHT_AT_96_DPI, find_bar_height};
    use crate::window::design::text_scale::{scale_text, set_factor_for_test};
    use crate::window::design::type_ramp::{TextStyle, create};
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetTextMetricsW, ReleaseDC, SelectObject, TEXTMETRICW,
    };
    for factor in [100, 150, 225] {
        set_factor_for_test(factor);
        let (field, bar, tallest) = unsafe {
            let font = create(TextStyle::Body, 96);
            let dc = GetDC(std::ptr::null_mut());
            let previous = SelectObject(dc, font);
            let mut metrics = TEXTMETRICW::default();
            GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, previous);
            ReleaseDC(std::ptr::null_mut(), dc);
            DeleteObject(font);
            (
                scale_text(FIELD_HEIGHT_AT_96_DPI, 96),
                find_bar_height(96, super::FindBarMode::Find),
                metrics.tmHeight,
            )
        };
        set_factor_for_test(100);
        assert!(field >= tallest + 2, "{factor}%: {field} < {tallest} + 2");
        assert!(bar > field, "{factor}%");
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
    /// The document length the stub reports; the tests that count set it.
    length: isize,
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
        SCI_GETLENGTH => log.length,
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
            let query = bar_layout(800, dpi, 16, mode, 0).query;
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

#[test]
fn the_counter_and_nav_buttons_fit_between_the_query_field_and_close_button() {
    // Break caught: typed text running under the toggles, the counter over the field (VS Code
    // keeps it outside, right of the field), nav buttons overlapping the counter or each other,
    // or the row not scaling on a 150% monitor.
    use super::{FindBarMode, bar_layout};
    use crate::window::option_toggles::toggle_rects;
    for dpi in [96, 144] {
        let width = 460 * dpi as i32 / 96;
        for mode in [FindBarMode::Find, FindBarMode::Replace] {
            let layout = bar_layout(width, dpi, 16, mode, 0);
            let rects = toggle_rects(layout.query.field, dpi);
            let label = format!("{dpi} {mode:?}");
            assert!(layout.query.edit.right <= rects[0].left, "{label}");
            assert!(layout.query.field.right < layout.counter.left, "{label}");
            assert!(layout.counter.left < layout.counter.right, "{label}: room");
            assert_eq!(layout.counter.top, layout.query.field.top);
            assert!(layout.counter.right <= layout.previous.left, "{label}");
            assert!(layout.previous.right <= layout.next.left, "{label}");
            assert!(layout.next.right <= layout.close.left, "{label}");
            assert!(layout.close.right < width, "{label}");
            for rect in [layout.previous, layout.next] {
                assert_eq!(
                    rect.bottom - rect.top,
                    layout.close.bottom - layout.close.top
                );
                assert_eq!(rect.top, layout.close.top);
            }
        }
    }
    let narrow = bar_layout(100, 96, 16, FindBarMode::Find, 0);
    assert!(
        narrow.counter.right >= narrow.counter.left,
        "never inverted"
    );
}

#[test]
fn buttons_name_their_keys_in_the_tooltips() {
    // Break caught: tips that name keys the fields don't take (in the Find field Enter is next and
    // Shift+Enter previous; in the Replace field Enter replaces and Shift+Enter replaces all).
    use super::BarButton;
    assert_eq!(
        BarButton::Previous.tooltip(),
        "Previous match (Shift+Enter)"
    );
    assert_eq!(BarButton::Next.tooltip(), "Next match (Enter)");
    assert_eq!(BarButton::ReplaceOne.tooltip(), "Replace (Enter)");
    assert_eq!(BarButton::ReplaceAll.tooltip(), "Replace All (Shift+Enter)");
}

#[test]
fn a_count_reads_as_its_index_of_its_total_and_marks_a_capped_total() {
    // Break caught: "0 of 0", a capped total shown as exact, or an unknown index shown as 0.
    use super::MatchCount;
    let count = |total, capped, index| MatchCount {
        total,
        capped,
        index,
    };
    assert_eq!(count(12, false, Some(3)).label(), "3 of 12");
    assert_eq!(count(1000, true, Some(7)).label(), "7 of 1000+");
    assert_eq!(count(12, false, None).label(), "12 results");
    assert_eq!(count(1, false, None).label(), "1 result");
    assert_eq!(count(1000, true, None).label(), "1000+ results");
    assert_eq!(count(0, false, None).label(), "No results");
}

#[test]
fn tally_finds_the_selected_match_and_caps_the_count_without_reading_past_it() {
    // Break caught: an off-by-one index, an uncapped scan of a huge match list, a selection that
    // is not exactly a match given an index, or "1000+" shown for exactly 1000.
    use super::count::tally as tally_matches;
    fn tally(
        matches: impl Iterator<Item = std::ops::Range<usize>>,
        selection: &std::ops::Range<usize>,
        cap: usize,
    ) -> super::MatchCount {
        tally_matches(matches, selection, cap).count
    }
    let ranges = |n: usize| (0..n).map(|i| i * 2..i * 2 + 1);
    let found = tally(ranges(12), &(4..5), 1000);
    assert_eq!(
        (found.total, found.capped, found.index),
        (12, false, Some(3))
    );
    // Break caught: the highlighted ranges differing from the counted ones, the current match
    // not the selected one, or more than the cap kept to highlight.
    let kept = tally_matches(ranges(12), &(4..5), 1000);
    assert_eq!(kept.ranges, ranges(12).collect::<Vec<_>>());
    assert_eq!(kept.current(), Some(4..5));
    assert_eq!(tally_matches(ranges(12), &(4..6), 1000).current(), None);
    let capped = tally_matches(ranges(5000), &(0..1), 1000);
    assert_eq!(capped.ranges.len(), 1000);
    let found = tally(ranges(12), &(4..6), 1000);
    assert_eq!(found.index, None, "the selection is not exactly a match");
    let found = tally(ranges(12), &(0..0), 1000);
    assert_eq!(found.index, None, "a caret");

    let exactly = tally(ranges(1000), &(0..1), 1000);
    assert_eq!((exactly.total, exactly.capped), (1000, false));
    let read = std::cell::Cell::new(0);
    let counted = ranges(50_000).inspect(|_| read.set(read.get() + 1));
    let over = tally(counted, &(1998..1999), 1000);
    assert_eq!(
        (over.total, over.capped, over.index),
        (1000, true, Some(1000))
    );
    assert_eq!(read.get(), 1001, "reads one past the cap, no more");
    let beyond = tally(ranges(50_000), &(2500..2501), 1000);
    assert_eq!(
        beyond.index, None,
        "a selection past the cap has no known index"
    );
    assert_eq!(tally(std::iter::empty(), &(0..0), 1000).total, 0);
}

#[test]
fn plain_counting_walks_the_matches_through_scintilla_and_finds_the_selection() {
    // Break caught: counting from the caret instead of the document start, not advancing past a
    // match, a miss counted as a match, or an empty query searched at all.
    use crate::search::MatchOptions;
    let count_matches =
        |editor: &Editor, query: &str, options, selection: &std::ops::Range<usize>| {
            super::find_matches(editor, query, options, selection).map(|found| found.count)
        };
    let log = Mutex::new(TargetLog {
        responses: VecDeque::from([0_isize, 8, -1]),
        length: 11,
        ..TargetLog::default()
    });
    let editor = Editor::test_fixture(target_range_stub, &log as *const Mutex<TargetLog> as isize);
    let found = count_matches(&editor, "one", MatchOptions::default(), &(8..11)).unwrap();
    assert_eq!(
        (found.total, found.capped, found.index),
        (2, false, Some(2))
    );
    assert_eq!(log.lock().unwrap().ranges, vec![(0, 11), (3, 11)]);

    let none = Mutex::new(TargetLog {
        length: 11,
        ..TargetLog::default()
    });
    let editor = Editor::test_fixture(target_range_stub, &none as *const Mutex<TargetLog> as isize);
    let found = count_matches(&editor, "zzz", MatchOptions::default(), &(0..0)).unwrap();
    assert_eq!(found.label(), "No results");
    assert_eq!(
        count_matches(&editor, "", MatchOptions::default(), &(0..0)),
        None
    );
    assert_eq!(none.lock().unwrap().ranges, vec![(0, 11)]);
}

#[test]
fn regex_counting_stops_scanning_at_the_limit() {
    // Break caught: collecting every match of a huge text before capping it.
    use super::regex_matcher;
    use crate::search::MatchOptions;
    let matcher = regex_matcher(r"\d", MatchOptions::default()).unwrap();
    let text = "1 ".repeat(5000);
    assert_eq!(matcher.find_up_to(&text, 1001).len(), 1001);
    assert_eq!(matcher.find_up_to(&text, 0).len(), 0);
    assert_eq!(matcher.find_up_to("1 2", 1001), vec![0..1, 2..3]);
}

#[test]
fn the_counter_fits_its_label_so_the_field_reaches_close_to_the_arrows() {
    // Break caught: a fixed-width counter leaving a wide gap between "1 of 3" and the arrows, or
    // a long label running under the arrows.
    use super::{FindBarMode, bar_layout};
    let short = bar_layout(460, 96, 16, FindBarMode::Find, 30);
    let long = bar_layout(460, 96, 16, FindBarMode::Find, 80);
    assert_eq!(short.counter.right, long.counter.right);
    assert!(short.counter.right <= short.previous.left);
    assert!(short.previous.left - short.counter.right <= 4);
    assert!(long.counter.right - long.counter.left >= 80);
    assert!(
        short.query.field.right > long.query.field.right,
        "the field takes the room"
    );
    // A short count keeps a minimum, so the field doesn't jump as counts come and go.
    let none = bar_layout(460, 96, 16, FindBarMode::Find, 0);
    assert_eq!(none.counter.left, short.counter.left);
}
