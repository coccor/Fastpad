use super::{
    CLEAR_TOOL, FocusStop, HeaderButton, LOADING, NO_MATCH, NO_NOTEBOOK, PADDING_AT_96_DPI,
    REPLACE_ROW_AT_96_DPI, ROW_AT_96_DPI, ROW_INSET_AT_96_DPI, ROW_LINE_AT_96_DPI, SearchState,
    SearchView, TOO_SHORT, fit_before, next_stop, notice_text, placeholder, skipped_tooltip,
    status_text, summary_text,
};
use crate::library::text_search::{Progress, RunEnd, TextHit};
use crate::search::{MatchOptions, SearchOption, Snippet};
use crate::window::design::metrics::scale;
use crate::window::notebook_view::LOAD_FAILED;
use crate::window::text_search_host::SearchBatch;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{POINT, RECT};

#[test]
fn the_notice_explains_an_empty_list() {
    // Break caught: a blank Search view with no notebook open, or "No notes match." while the
    // notebook is still loading.
    assert_eq!(notice_text(false, false, false), Some(NO_NOTEBOOK));
    assert_eq!(notice_text(true, false, false), Some(LOADING));
    assert_eq!(notice_text(true, false, true), Some(LOAD_FAILED));
    assert_eq!(notice_text(true, true, false), None);
}

#[test]
fn the_placeholder_says_it_searches_text_in_the_open_notebook() {
    // Break caught: the box saying "Search" with no hint that it searches the notes' text,
    // or of which notebook.
    assert_eq!(
        placeholder(Some(Path::new(r"C:\Users\me\Work"))),
        "Search text in Work"
    );
    assert_eq!(placeholder(None), "Search text");
}

#[test]
fn a_long_prefix_is_cut_from_the_start_so_the_match_stays_in_view() {
    // Break caught: in a narrow panel, forty characters before the match pushing it off the
    // row, or a cut inside a multi-byte character.
    let measure = |text: &str| text.chars().count() as i32 * 10;
    assert_eq!(fit_before("abcdef", 100, measure), "abcdef");
    assert_eq!(fit_before("abcdef", 40, measure), "\u{2026}def");
    assert_eq!(fit_before("\u{2026}été ab", 50, measure), "\u{2026}é ab");
    assert_eq!(
        fit_before("abc", 5, measure),
        "",
        "not even the ellipsis fits"
    );
    assert_eq!(fit_before("", 0, measure), "");
}

#[test]
fn the_skipped_tooltip_has_one_line_per_reason_that_skipped_a_note() {
    let progress = Progress {
        visited: 9,
        total: 9,
        skipped: [2, 0, 1, 1_500],
    };
    assert_eq!(
        skipped_tooltip(&progress),
        "2 online only\r\n1 couldn't be read\r\n1,500 not text"
    );
    let large = Progress {
        skipped: [0, 3, 0, 0],
        ..progress
    };
    assert_eq!(skipped_tooltip(&large), "3 larger than 4 MB");
    assert_eq!(skipped_tooltip(&Progress::default()), "");
}

#[test]
fn a_result_row_holds_two_lines_of_the_sidebar_text_at_every_dpi() {
    // Break caught: the snippet line clipped at 150% or 200%, or the bold match taller than
    // its slot.
    use crate::window::titlebar::create_ui_font;
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, FW_BOLD, FW_NORMAL, GetDC, GetTextMetricsW, ReleaseDC, SelectObject,
        TEXTMETRICW,
    };
    for dpi in [96, 120, 144, 192] {
        let mut tallest = 0;
        for weight in [FW_NORMAL, FW_BOLD] {
            let font = create_ui_font(
                scale(
                    crate::window::design::type_ramp::TextStyle::Body.spec().px,
                    dpi,
                ),
                "Segoe UI",
                weight as i32,
                false,
            );
            unsafe {
                let dc = GetDC(std::ptr::null_mut());
                let previous = SelectObject(dc, font);
                let mut metrics = TEXTMETRICW::default();
                assert_ne!(GetTextMetricsW(dc, &mut metrics), 0);
                tallest = tallest.max(metrics.tmHeight);
                SelectObject(dc, previous);
                ReleaseDC(std::ptr::null_mut(), dc);
                DeleteObject(font);
            }
        }
        assert!(
            scale(ROW_LINE_AT_96_DPI, dpi) >= tallest,
            "{dpi}: {tallest}"
        );
        assert!(
            scale(ROW_AT_96_DPI, dpi)
                >= 2 * scale(ROW_LINE_AT_96_DPI, dpi) + 2 * scale(ROW_INSET_AT_96_DPI, dpi) - 1,
            "{dpi}"
        );
    }
}

#[test]
fn a_result_reads_its_name_folder_and_snippet() {
    // Break caught: a screen reader hearing only the note name, with no hint of why it
    // matched, or a stray ", " for a note at the root.
    use super::result_name;
    let hit = |folder: &str| TextHit {
        path: PathBuf::from("q1.md"),
        name: "Q1 budget".to_owned(),
        folder: folder.to_owned(),
        snippet: Snippet {
            text: "\u{2026}paid the invoice march 3\u{2026}".to_owned(),
            highlight: 12..25,
        },
        stamp: None,
    };
    assert_eq!(
        result_name(&hit("work")),
        "Q1 budget, work: \u{2026}paid the invoice march 3\u{2026}"
    );
    assert_eq!(
        result_name(&hit("")),
        "Q1 budget: \u{2026}paid the invoice march 3\u{2026}"
    );
}

#[test]
fn the_summary_counts_notes_and_says_when_nothing_matches() {
    // Break caught: "No notes match." before the search finished, a count without its
    // thousands separator, "1 notes", the cap shown as "500 notes", or a regex error shown as
    // ordinary text.
    let done = |capped| SearchState::Done {
        progress: Progress::default(),
        capped,
    };
    let running = SearchState::Running(Progress::default());
    let line = |text: &str, error| Some((text.to_owned(), error));
    assert_eq!(summary_text(&SearchState::Idle, 0), None);
    assert_eq!(
        summary_text(&SearchState::TooShort, 0),
        line(TOO_SHORT, false)
    );
    assert_eq!(summary_text(&running, 0), None, "nothing found yet");
    assert_eq!(summary_text(&running, 1), line("1 note", false));
    assert_eq!(summary_text(&done(false), 0), line(NO_MATCH, false));
    assert_eq!(
        summary_text(&done(false), 1_234),
        line("1,234 notes", false)
    );
    assert_eq!(summary_text(&done(true), 500), line("500+ notes", false));
    assert_eq!(
        summary_text(&SearchState::PatternError("Unclosed group".to_owned()), 3),
        line("Unclosed group", true)
    );
}

#[test]
fn the_status_line_shows_progress_and_what_was_skipped() {
    // Break caught: no progress while a big notebook is searched, a status line left up after
    // a clean search, or a skipped count with the wrong grammar.
    let progress = Progress {
        visited: 4_120,
        total: 9_800,
        skipped: [0; 4],
    };
    assert_eq!(
        status_text(&SearchState::Running(progress)).as_deref(),
        Some("Searching\u{2026} 4,120 of 9,800")
    );
    let done = |skipped| SearchState::Done {
        progress: Progress {
            visited: 9,
            total: 9,
            skipped,
        },
        capped: false,
    };
    assert_eq!(
        status_text(&done([0; 4])),
        None,
        "hidden when nothing was skipped"
    );
    assert_eq!(
        status_text(&done([0, 1, 0, 0])).as_deref(),
        Some("1 note wasn't searched")
    );
    assert_eq!(
        status_text(&done([2, 0, 1, 1])).as_deref(),
        Some("4 notes weren't searched")
    );
    assert_eq!(status_text(&SearchState::Idle), None);
    assert_eq!(
        status_text(&SearchState::PatternError("x".to_owned())),
        None
    );
}

fn hit(name: &str, folder: &str) -> TextHit {
    let file = format!("{name}.md");
    let path = if folder.is_empty() {
        PathBuf::from(file)
    } else {
        Path::new(folder).join(file)
    };
    TextHit {
        path,
        name: name.to_owned(),
        folder: folder.to_owned(),
        snippet: Snippet {
            text: format!("{name} needle"),
            highlight: name.len() + 1..name.len() + 7,
        },
        stamp: None,
    }
}

fn batch(hits: Vec<TextHit>, visited: usize, end: Option<RunEnd>) -> SearchBatch {
    SearchBatch {
        generation: 1,
        hits,
        progress: Progress {
            visited,
            total: 10,
            skipped: [0; 4],
        },
        end,
        skipped: Vec::new(),
    }
}

fn rows(view: &SearchView) -> Vec<(String, String)> {
    view.results
        .iter()
        .map(|result| (result.name.clone(), result.folder.clone()))
        .collect()
}

fn row(name: &str, folder: &str) -> (String, String) {
    (name.to_owned(), folder.to_owned())
}

const HEIGHT: i32 = 400;

#[test]
fn batches_are_inserted_in_order_and_keep_the_selection_by_path() {
    // Break caught: rows appended in arrival order, or a row arriving above the selection
    // moving it, so Enter opens a different note than the one highlighted.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    assert!(view.apply(batch(vec![hit("m", ""), hit("c", "")], 4, None), HEIGHT));
    assert_eq!(rows(&view), [row("c", ""), row("m", "")]);
    assert_eq!(view.list.selected, Some(0), "the first result is selected");
    view.list.select(1, HEIGHT);
    let more = vec![hit("a", ""), hit("b", "sub"), hit("b", "")];
    assert!(view.apply(batch(more, 8, None), HEIGHT));
    assert_eq!(
        rows(&view),
        [
            row("a", ""),
            row("b", ""),
            row("b", "sub"),
            row("c", ""),
            row("m", "")
        ],
        "natural name order, then the root before a folder"
    );
    assert_eq!(view.list.selected, Some(4), "still m");
    assert!(view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT));
    assert_eq!(
        view.search,
        SearchState::Done {
            progress: Progress {
                visited: 10,
                total: 10,
                skipped: [0; 4]
            },
            capped: false
        }
    );
}

#[test]
fn a_rerun_keeps_the_results_until_its_first_batch_and_a_new_query_starts_empty() {
    // Break caught: the list blanking on every library change, a new query showing the old
    // query's rows until its own arrive, or the selected note lost across either.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
    view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
    view.list.select(2, HEIGHT);

    view.begin("needle", 10);
    assert_eq!(rows(&view).len(), 3, "kept until the first batch");
    assert!(matches!(view.search, SearchState::Running(_)));
    view.apply(batch(vec![hit("b", ""), hit("c", "")], 5, None), HEIGHT);
    assert_eq!(
        rows(&view),
        [row("b", ""), row("c", "")],
        "the first batch replaces them"
    );
    assert_eq!(view.list.selected, Some(1), "c is still selected");

    view.begin("needles", 10);
    assert!(view.results.is_empty(), "a new query starts empty");
    assert_eq!(view.list.selected, None);
    view.apply(batch(vec![hit("a", ""), hit("c", "")], 10, None), HEIGHT);
    assert_eq!(
        view.list.selected,
        Some(1),
        "the remembered note is selected again when it arrives"
    );
}

#[test]
fn a_reruns_empty_progress_batch_keeps_the_old_results() {
    // Break caught: the list blanking for a moment on every re-run, because the first
    // interval batch (progress only, no hits) replaced the rows before any hit arrived.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
    view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
    view.list.select(1, HEIGHT);
    view.begin("needle", 10);
    view.apply(batch(Vec::new(), 3, None), HEIGHT);
    assert_eq!(rows(&view).len(), 3, "an empty batch replaces nothing");
    assert_eq!(view.list.selected, Some(1));
    view.apply(batch(vec![hit("a", ""), hit("b", "")], 6, None), HEIGHT);
    assert_eq!(
        rows(&view),
        [row("a", ""), row("b", "")],
        "the first hit replaces them"
    );
    assert_eq!(view.list.selected, Some(1), "b is still remembered");

    view.begin("needle", 10);
    view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT);
    assert!(
        view.results.is_empty(),
        "a re-run that ends without hits empties the list"
    );
}

#[test]
fn the_options_a_search_ran_with_are_kept_and_new_options_start_empty() {
    // Break caught: the find bar seeded (Task 7) with options toggled after the search ran,
    // or rows found without match case still listed while the match-case run starts.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    view.apply(
        batch(vec![hit("a", "")], 10, Some(RunEnd::Completed)),
        HEIGHT,
    );
    assert_eq!(view.run_options, MatchOptions::default());
    view.options = view.options.toggled(SearchOption::Case);
    assert!(!view.run_options.case, "not until a search runs with it");
    view.begin("needle", 10);
    assert!(view.run_options.case);
    assert!(view.results.is_empty(), "other options are a new search");
}

#[test]
fn a_selection_the_user_moves_during_a_search_wins_over_the_remembered_one() {
    // Break caught: the remembered note arriving late and snatching the selection from the
    // row the user just moved to.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
    view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
    view.list.select(2, HEIGHT);
    view.begin("needles", 10);
    view.apply(batch(vec![hit("a", ""), hit("b", "")], 5, None), HEIGHT);
    assert_eq!(
        view.list.selected,
        Some(0),
        "c has not arrived: the first row"
    );
    view.list.select(1, HEIGHT);
    view.apply(batch(vec![hit("c", "")], 10, None), HEIGHT);
    assert_eq!(view.list.selected, Some(1), "b, which the user picked");
}

#[test]
fn a_batch_that_changes_nothing_shown_asks_for_no_repaint() {
    // Break caught: an InvalidateRect for every empty batch of a long search.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    assert!(view.apply(batch(vec![hit("a", "")], 3, None), HEIGHT));
    assert!(!view.apply(batch(Vec::new(), 3, None), HEIGHT));
    assert!(
        view.apply(batch(Vec::new(), 4, None), HEIGHT),
        "the progress moved"
    );
}

#[test]
fn the_replace_row_sits_under_the_box_with_replace_all_at_its_right() {
    // Break caught: the chevron drawn over the box, the replace field overlapping the search
    // field or its button, or the summary and results left under the replace row.
    for dpi in [96, 144, 192] {
        let client = RECT {
            left: 0,
            top: 0,
            right: 320,
            bottom: 600,
        };
        let chevron = SearchView::chevron_rect(client, dpi);
        let field = SearchView::field_rect(client, dpi);
        assert!(chevron.left > client.left, "{dpi}");
        assert!(chevron.right < field.left, "{dpi}");
        assert_eq!((chevron.top, chevron.bottom), (field.top, field.bottom));

        let replace = SearchView::replace_field_rect(client, dpi);
        let all = SearchView::replace_all_rect(client, dpi);
        assert_eq!(replace.left, field.left);
        assert!(replace.top > field.bottom, "{dpi}");
        assert_eq!(replace.bottom - replace.top, field.bottom - field.top);
        assert!(replace.right < all.left, "{dpi}");
        assert_eq!(all.right, field.right);
        assert_eq!((all.top, all.bottom), (replace.top, replace.bottom));

        let mut view = SearchView::new(std::ptr::null_mut(), dpi);
        let closed = view.list_area(client, dpi);
        assert!(view.summary_rect(client, dpi).top >= field.bottom, "{dpi}");
        view.replace_open = true;
        let open = view.list_area(client, dpi);
        assert_eq!(open.top - closed.top, scale(REPLACE_ROW_AT_96_DPI, dpi));
        assert!(
            view.summary_rect(client, dpi).top >= replace.bottom,
            "{dpi}"
        );
    }
}

#[test]
fn replace_all_waits_for_a_finished_search_with_results() {
    // Break caught: Replace all pressable while results stream in (the list and its stamps
    // aren't final), after a pattern error, or with nothing listed (spec §11).
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.notebook = Some(PathBuf::from(r"C:\notes"));
    view.loaded = true;
    assert!(!view.replace_all_enabled(), "idle");
    view.begin("needle", 10);
    view.apply(batch(vec![hit("a", "")], 4, None), HEIGHT);
    assert!(!view.replace_all_enabled(), "running");
    view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT);
    assert!(view.replace_all_enabled());
    view.search = SearchState::PatternError("Unclosed group".to_owned());
    assert!(!view.replace_all_enabled(), "a pattern error");
    view.begin("zzz", 10);
    view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT);
    assert!(!view.replace_all_enabled(), "nothing listed");
    view.begin("needle", 10);
    view.apply(
        batch(vec![hit("a", "")], 10, Some(RunEnd::Completed)),
        HEIGHT,
    );
    assert!(view.replace_all_enabled());
    view.replacing = true;
    assert!(!view.replace_all_enabled(), "a replace runs");
    view.replacing = false;
    assert!(view.replace_all_enabled(), "and has ended");
}

#[test]
fn a_disabled_replace_button_draws_dim_and_stays_legible_on_the_focused_selection() {
    // Break caught (final review FR3, Task 5 review Minor 3): a disabled button drawn as
    // pressable, or drawn in the line-number color over the focused selection, where it may
    // have little contrast.
    use super::{button_color, row_button_color};
    use crate::window::palette::Palette;
    use crate::window::row_list::RowLook;
    let palette = Palette {
        selection_foreground: Some(0x00AB_CDEF),
        ..Palette::neutral()
    };
    let look = |selected, hover, focused| RowLook {
        selected,
        hover,
        focused,
    };
    let normal = palette.editor_foreground;
    let dim = palette.line_number_foreground;
    assert_eq!(button_color(false, false, normal, &palette), dim);
    assert_eq!(button_color(false, true, normal, &palette), dim, "no hover");
    assert_eq!(button_color(true, false, normal, &palette), normal);
    assert_eq!(
        button_color(true, true, normal, &palette),
        palette.hover_foreground
    );
    for (row, enabled_color) in [
        (look(false, true, true), normal),
        (look(true, false, false), normal),
    ] {
        assert_eq!(
            row_button_color(false, false, row, &palette),
            dim,
            "{row:?}"
        );
        assert_eq!(
            row_button_color(true, false, row, &palette),
            enabled_color,
            "{row:?}"
        );
    }
    let focused = look(true, false, true);
    let enabled = row_button_color(true, false, focused, &palette);
    let disabled = row_button_color(false, false, focused, &palette);
    assert_eq!(enabled, 0x00AB_CDEF, "the selection's text color");
    assert_ne!(disabled, enabled, "visibly dimmer than the enabled button");
    assert_eq!(
        disabled,
        crate::catppuccin::blend(0x00AB_CDEF, palette.selection_background, 128),
        "halfway into the selection"
    );
    assert_eq!(
        row_button_color(true, true, focused, &palette),
        palette.hover_foreground
    );
    // Every themed palette: the disabled button differs from the enabled one there.
    for theme in crate::platform::theme::Theme::ALL {
        let themed = Palette::for_theme(theme, false);
        assert_ne!(
            row_button_color(false, false, focused, &themed),
            row_button_color(true, false, focused, &themed),
            "{theme:?}"
        );
    }
    // High contrast: no blend, only the system pair.
    let system = Palette {
        high_contrast: true,
        ..palette
    };
    assert_eq!(
        row_button_color(false, false, focused, &system),
        0x00AB_CDEF
    );
}

#[test]
fn a_row_shows_its_replace_button_while_hovered_or_selected_and_the_field_is_open() {
    // Break caught: a replace button on every row (a stray click replaces in the wrong
    // note), none on the selected row for keyboard users, or buttons with the field closed.
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.begin("needle", 10);
    view.apply(
        batch(
            vec![hit("a", ""), hit("b", ""), hit("c", "")],
            10,
            Some(RunEnd::Completed),
        ),
        HEIGHT,
    );
    view.list.select(0, HEIGHT);
    view.list.set_hover(Some(2));
    assert!((0..3).all(|index| !view.row_button_shown(index)));
    view.replace_open = true;
    assert!(view.row_button_shown(0), "selected");
    assert!(view.row_button_shown(2), "hovered");
    assert!(!view.row_button_shown(1));

    let row = RECT {
        left: 0,
        top: 84,
        right: 300,
        bottom: 126,
    };
    let button = SearchView::row_replace_rect(row, 96);
    assert_eq!(button.right, 300 - scale(PADDING_AT_96_DPI, 96));
    assert!(button.left > row.left);
    assert!(button.top > row.top && button.bottom < row.bottom);
    assert_eq!(button.bottom - button.top, button.right - button.left);
}

#[test]
fn tab_cycles_the_box_the_replace_field_and_the_results_and_wraps() {
    // Break caught: Tab stuck in the box, landing in a closed (hidden) replace field or in an
    // empty result list, or Shift+Tab not the exact reverse.
    use FocusStop::{Box, Replace, Results};
    // The replace field open, results listed: Box -> Replace -> Results -> Box.
    assert_eq!(next_stop(Box, false, true, true), Replace);
    assert_eq!(next_stop(Replace, false, true, true), Results);
    assert_eq!(next_stop(Results, false, true, true), Box);
    assert_eq!(next_stop(Box, true, true, true), Results);
    assert_eq!(next_stop(Results, true, true, true), Replace);
    assert_eq!(next_stop(Replace, true, true, true), Box);
    // The replace field closed: Box <-> Results.
    assert_eq!(next_stop(Box, false, false, true), Results);
    assert_eq!(next_stop(Results, false, false, true), Box);
    assert_eq!(next_stop(Box, true, false, true), Results);
    assert_eq!(next_stop(Results, true, false, true), Box);
    // No results: the fields only, or the box stays put alone.
    assert_eq!(next_stop(Box, false, true, false), Replace);
    assert_eq!(next_stop(Replace, false, true, false), Box);
    assert_eq!(next_stop(Replace, true, true, false), Box);
    assert_eq!(next_stop(Box, false, false, false), Box);
    assert_eq!(next_stop(Box, true, false, false), Box);
}

#[test]
fn the_title_band_sits_above_the_field_and_the_clear_button_left_of_the_toggles() {
    // Break caught: the field drawn inside the title band (it would be caption, not a field), or
    // the clear button over a toggle or outside the field.
    for dpi in [96, 144, 192] {
        let client = RECT {
            left: 0,
            top: 0,
            right: 320,
            bottom: 600,
        };
        let title = SearchView::title_rect(client, dpi);
        let field = SearchView::field_rect(client, dpi);
        let clear = SearchView::clear_rect(client, dpi);
        let toggles = crate::window::option_toggles::toggle_rects(field, dpi);
        assert_eq!(title.bottom, scale(38, dpi));
        assert!(field.top >= title.bottom, "{dpi}");
        assert!(
            clear.left >= field.left && clear.right <= toggles[0].left,
            "{dpi}"
        );
        assert!(
            clear.top >= field.top && clear.bottom <= field.bottom,
            "{dpi}"
        );
        let view = SearchView::new(std::ptr::null_mut(), dpi);
        assert!(view.summary_rect(client, dpi).top >= field.bottom, "{dpi}");
    }
}

#[test]
fn the_clear_button_shows_and_hits_only_while_the_box_has_text() {
    // Break caught: an x on an empty field, or one that can't be clicked once text is typed.
    let client = RECT {
        left: 0,
        top: 0,
        right: 320,
        bottom: 600,
    };
    let mut view = SearchView::new(std::ptr::null_mut(), 96);
    view.edit = Some(std::ptr::null_mut());
    let clear = SearchView::clear_rect(client, 96);
    let center = POINT {
        x: (clear.left + clear.right) / 2,
        y: (clear.top + clear.bottom) / 2,
    };
    assert!(!view.clear_shown());
    assert_eq!(view.header_button_at(center, client, 96), None);
    let tool = |view: &SearchView| {
        view.tooltip_tools(client, 96)
            .into_iter()
            .find(|tool| tool.0 == CLEAR_TOOL)
            .map(|tool| tool.2)
    };
    assert_eq!(tool(&view).as_deref(), Some(""));
    view.box_text = "needle".to_owned();
    assert!(view.clear_shown());
    assert_eq!(
        view.header_button_at(center, client, 96),
        Some(HeaderButton::Clear)
    );
    assert_eq!(tool(&view).as_deref(), Some("Clear search"));
}

mod painting {
    use super::super::paint::{paint_field, paint_hover};
    use crate::platform::theme::Theme;
    use crate::window::palette::Palette;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel, HDC,
        ReleaseDC, SelectObject,
    };

    const BACKGROUND: u32 = 0x0080_8080;
    const BOX: RECT = RECT {
        left: 10,
        top: 10,
        right: 110,
        bottom: 40,
    };

    /// Fills a 200x80 memory bitmap with `BACKGROUND`, runs `draw`, then `read`.
    fn with_canvas(draw: impl FnOnce(HDC), read: impl FnOnce(&dyn Fn(i32, i32) -> u32)) {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 200, 80);
            let previous = SelectObject(dc, bitmap);
            let all = RECT {
                left: 0,
                top: 0,
                right: 200,
                bottom: 80,
            };
            crate::window::panel::fill(dc, all, BACKGROUND);
            draw(dc);
            read(&|x, y| GetPixel(dc, x, y));
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }

    #[test]
    fn a_field_is_a_rounded_box_with_an_accent_border() {
        // Break caught: a square field, or a rounded one that lost its border.
        let colors = Palette::for_theme(Theme::ALL[0], false);
        with_canvas(
            |dc| unsafe { paint_field(dc, BOX, &colors, BACKGROUND, 96) },
            |pixel| {
                assert_eq!(pixel(10, 10), BACKGROUND, "corner");
                assert_eq!(pixel(60, 10), colors.selection_background, "border");
                assert_eq!(pixel(60, 11), colors.editor_background, "inside");
            },
        );
    }

    #[test]
    fn a_hover_fill_is_rounded() {
        // Break caught: a hovered button shaded with square corners.
        let colors = Palette::for_theme(Theme::ALL[0], false);
        with_canvas(
            |dc| unsafe { paint_hover(dc, BOX, &colors, BACKGROUND, 96) },
            |pixel| {
                assert_eq!(pixel(10, 10), BACKGROUND, "corner");
                assert_eq!(pixel(60, 25), colors.hover_background, "middle");
            },
        );
    }

    #[test]
    fn high_contrast_keeps_square_fields_and_hover_fills() {
        let colors = Palette::for_theme(Theme::ALL[0], true);
        with_canvas(
            |dc| unsafe {
                paint_field(dc, BOX, &colors, BACKGROUND, 96);
                paint_hover(
                    dc,
                    RECT {
                        left: 120,
                        right: 180,
                        ..BOX
                    },
                    &colors,
                    BACKGROUND,
                    96,
                );
            },
            |pixel| {
                assert_eq!(pixel(10, 10), colors.selection_background, "field corner");
                assert_eq!(pixel(120, 10), colors.hover_background, "hover corner");
            },
        );
    }
}
