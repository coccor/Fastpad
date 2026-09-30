use super::{
    MIN_QUERY_CHARS, Narrowing, confirm_text, list_mark, narrows, note_mark, overlay_mark,
    report_text, row_confirm_text, searchable,
};
use crate::library::NoteEntry;
use crate::library::text_replace::ReplaceCount;
use crate::search::MatchOptions;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn a_query_runs_from_two_characters_and_never_when_it_is_all_white_space() {
    // Break caught: a one-letter query reading every note in the notebook, a two-byte letter
    // counted as two characters, or a query of spaces running at all.
    assert_eq!(MIN_QUERY_CHARS, 2);
    assert!(!searchable(""));
    assert!(!searchable("a"));
    assert!(!searchable("é"), "one character in two bytes");
    assert!(searchable("ab"));
    assert!(searchable("éa"));
    assert!(searchable(" a"), "a leading space is part of the phrase");
    assert!(!searchable("   "));
    assert!(!searchable("\t\t"));
}

fn note(path: &str, online_only: bool) -> NoteEntry {
    NoteEntry {
        path: PathBuf::from(path),
        size: 10,
        mtime: 20,
        online_only,
    }
}

#[test]
fn the_list_mark_follows_the_note_paths_and_not_their_sizes() {
    // Break caught: a save of a listed note re-running the search on the next refresh, or a
    // note added, renamed or made online-only leaving the old results in place.
    let base = list_mark(&[note("a.md", false), note("b.md", false)]);
    assert_eq!(base, list_mark(&[note("a.md", false), note("b.md", false)]));
    assert_ne!(base, list_mark(&[note("a.md", false)]), "a note removed");
    assert_ne!(
        base,
        list_mark(&[note("a.md", false), note("c.md", false)]),
        "a note renamed"
    );
    assert_ne!(
        base,
        list_mark(&[note("a.md", false), note("b.md", true)]),
        "a note went online-only"
    );
    let mut saved = note("b.md", false);
    saved.size = 99;
    saved.mtime = 7;
    assert_eq!(
        base,
        list_mark(&[note("a.md", false), saved]),
        "a save of a listed note is no list change"
    );
}

#[test]
fn the_overlay_mark_changes_with_any_tab_text_and_not_with_order() {
    // Break caught: narrowing kept after an edit in a dirty tab, so a phrase typed there after
    // the last search is never found.
    let mut first = HashMap::new();
    first.insert(PathBuf::from("a.md"), "one".to_owned());
    first.insert(PathBuf::from("b.md"), "two".to_owned());
    let mut second = HashMap::new();
    second.insert(PathBuf::from("b.md"), "two".to_owned());
    second.insert(PathBuf::from("a.md"), "one".to_owned());
    assert_eq!(overlay_mark(&first), overlay_mark(&second));
    second.insert(PathBuf::from("a.md"), "one!".to_owned());
    assert_ne!(overlay_mark(&first), overlay_mark(&second));
    assert_ne!(overlay_mark(&HashMap::new()), overlay_mark(&first));
}

#[test]
fn the_note_mark_also_follows_sizes_and_times() {
    // Break caught: narrowing kept after FastPad saved a new phrase into a listed note, which
    // changes its size and time but not the list of paths.
    let base = note_mark(&[note("a.md", false)]);
    assert_eq!(base, note_mark(&[note("a.md", false)]));
    let mut saved = note("a.md", false);
    saved.size = 99;
    assert_ne!(base, note_mark(std::slice::from_ref(&saved)), "a new size");
    saved.size = 10;
    saved.mtime = 7;
    assert_ne!(base, note_mark(&[saved]), "a new time");
    assert_ne!(base, note_mark(&[note("a.md", true)]), "online-only");
    assert_ne!(base, note_mark(&[note("b.md", false)]), "another path");
}

#[test]
fn narrowing_needs_a_longer_plain_query_with_the_same_options_and_tab_texts() {
    // Break caught: a whole-word or regex query narrowed to an earlier query's hits ("xfoo"
    // contains "foo", but "foo" is not a whole word in "xfoo"), a shorter query narrowed, or
    // narrowing across a change of options.
    let plain = MatchOptions::default();
    let previous = Narrowing {
        query: "foo".to_owned(),
        options: plain,
        paths: Vec::new(),
        notes: 1,
        overlays: 2,
    };
    assert!(narrows(&previous, "foo", plain, 2), "the same query again");
    assert!(narrows(&previous, "food", plain, 2));
    assert!(narrows(&previous, "a foo", plain, 2));
    assert!(
        !narrows(&previous, "fo", plain, 2),
        "a shorter query can match more"
    );
    assert!(
        !narrows(&previous, "Foo", plain, 2),
        "contained only after folding"
    );
    let case = MatchOptions {
        case: true,
        ..plain
    };
    assert!(!narrows(&previous, "food", case, 2), "the options changed");
    let whole = MatchOptions {
        whole_word: true,
        ..plain
    };
    let previous_whole = Narrowing {
        options: whole,
        ..previous.clone()
    };
    assert!(!narrows(&previous_whole, "xfoo", whole, 2));
    let regex = MatchOptions {
        regex: true,
        ..plain
    };
    let previous_regex = Narrowing {
        options: regex,
        ..previous.clone()
    };
    assert!(!narrows(&previous_regex, "foo|bar", regex, 2));
    assert!(
        !narrows(&previous, "food", plain, 3),
        "a dirty tab's text changed"
    );
}
#[test]
fn the_question_counts_matches_and_notes_and_warns_only_when_a_note_is_saved() {
    // Break caught: "1 matches", a count without its thousands separator, the capped
    // question claiming only the notes that matched, the warning missing when a note's file
    // will be written or shown when every match is in an open tab, or a per-row question
    // for a note it can't undo (spec §11, §12a).
    let count = |matches, notes, closed_notes| ReplaceCount {
        matches,
        notes,
        closed_notes,
    };
    assert_eq!(
        confirm_text(count(1, 1, 0), 1, false, "x"),
        "Replace 1 match in 1 note with \"x\"?"
    );
    assert_eq!(
        confirm_text(count(1_234, 12, 1), 12, false, "y"),
        "Replace 1,234 matches in 12 notes with \"y\"?\nNotes that aren't open are saved and can't be undone."
    );
    assert_eq!(
        confirm_text(count(2_000, 480, 0), 500, true, ""),
        "Replace 2,000 matches in the 500 listed notes with \"\"? More notes match; search again to replace in the rest."
    );
    assert_eq!(
        confirm_text(count(2_000, 480, 479), 500, true, "z"),
        "Replace 2,000 matches in the 500 listed notes with \"z\"? More notes match; search again to replace in the rest.\nNotes that aren't open are saved and can't be undone."
    );
    assert_eq!(
        row_confirm_text(3, "Q1 budget", "z"),
        "Replace 3 matches in \"Q1 budget\" with \"z\"? The note is saved and this can't be undone."
    );
    assert_eq!(
        row_confirm_text(1, "a", "b"),
        "Replace 1 match in \"a\" with \"b\"? The note is saved and this can't be undone."
    );
}

#[test]
fn the_report_is_one_line_that_counts_what_was_replaced_and_names_what_was_not() {
    // Break caught: a skipped note reported as replaced, "1 notes were skipped", a note that
    // couldn't be written left unnamed, a line break in the one-line status bar, or a list
    // of 400 names in one notification.
    let names = |names: &[&str]| {
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(report_text(1, 1, &[], &[]), "Replaced 1 match in 1 note.");
    assert_eq!(
        report_text(5, 2, &names(&["b"]), &[]),
        "Replaced 5 matches in 2 notes. 1 note was skipped because it changed since the search. (b)"
    );
    assert_eq!(
        report_text(0, 0, &[], &names(&["c", "d"])),
        "Replaced 0 matches in 0 notes. 2 notes couldn't be written. (c, d)"
    );
    assert_eq!(
        report_text(3_000, 1, &names(&["x"]), &names(&["y", "z"])),
        "Replaced 3,000 matches in 1 note. 1 note was skipped because it changed since the search. 2 notes couldn't be written. (x, y, z)"
    );
    let changed = (0..1_200)
        .map(|index| format!("n{index}"))
        .collect::<Vec<_>>();
    let report = report_text(9, 9, &changed, &names(&["f"]));
    assert_eq!(
        report,
        "Replaced 9 matches in 9 notes. 1,200 notes were skipped because they changed since the search. 1 note couldn't be written. (n0, n1, n2, and 1,198 more)"
    );
    assert!(!report.contains('\n'));
}
