use super::*;

/// `marked` with `|` marking the caret, or `«` and `»` around a selection.
fn parse(marked: &str) -> (String, Range<usize>) {
    if let Some(start) = marked.find('«') {
        let end = marked.find('»').expect("a selection end") - '«'.len_utf8();
        return (marked.replace(['«', '»'], ""), start..end);
    }
    let caret = marked.find('|').expect("a caret");
    (marked.replacen('|', "", 1), caret..caret)
}

/// The text after `plan`, with the first selection marked the way `parse` reads it.
fn show(text: &str, plan: &EditPlan) -> String {
    let mut out = plan.apply_to(text);
    let selection = plan.selections[0].clone();
    if selection.is_empty() {
        out.insert(selection.start, '|');
    } else {
        out.insert(selection.end, '»');
        out.insert(selection.start, '«');
    }
    out
}

fn typed(marked: &str, ch: char) -> Option<String> {
    let (text, selection) = parse(marked);
    type_char(&text, selection, ch).map(|plan| show(&text, &plan))
}

#[test]
fn typing_a_marker_over_a_selection_wraps_it_and_keeps_it_selected() {
    assert_eq!(typed("a «word» b", '*').as_deref(), Some("a *«word»* b"));
    assert_eq!(
        typed("a *«word»* b", '*').as_deref(),
        Some("a **«word»** b")
    );
    assert_eq!(typed("«x»", '`').as_deref(), Some("`«x»`"));
    assert_eq!(typed("«x»", '~').as_deref(), Some("~«x»~"));
    assert_eq!(typed("«x»", '[').as_deref(), Some("[«x»]"));
    assert_eq!(typed("«x»", '(').as_deref(), Some("(«x»)"));
    // Other characters, and selections over lines, replace the selection as usual.
    assert_eq!(typed("«x»", 'a'), None);
    assert_eq!(typed("«a\nb»", '*'), None);
}

#[test]
fn brackets_close_themselves_before_space_or_the_line_end() {
    assert_eq!(typed("a |", '(').as_deref(), Some("a (|)"));
    assert_eq!(typed("|", '[').as_deref(), Some("[|]"));
    assert_eq!(typed("[|]", '[').as_deref(), Some("[[|]]"));
    assert_eq!(typed("see |.", '(').as_deref(), Some("see (|)."));
    // Before a word they don't: the bracket is meant for it.
    assert_eq!(typed("|word", '('), None);
    // Emphasis markers never pair on a caret: `*` starts bullets and `_` joins words.
    assert_eq!(typed("|", '*'), None);
    assert_eq!(typed("|", '_'), None);
}

#[test]
fn a_closer_steps_over_the_one_an_opener_left() {
    assert_eq!(typed("(a|)", ')').as_deref(), Some("(a)|"));
    assert_eq!(typed("- [ |]", ']').as_deref(), Some("- [ ]|"));
    // No opener left open on the line: the `)` is typed.
    assert_eq!(typed("a|)", ')'), None);
}

#[test]
fn backticks_pair_step_over_and_make_a_fence() {
    assert_eq!(typed("|", '`').as_deref(), Some("`|`"));
    assert_eq!(typed("`|`", '`').as_deref(), Some("``|"));
    // The third backtick of a fence is typed as it is.
    assert_eq!(typed("``|", '`'), None);
    assert_eq!(typed("`code|`", '`').as_deref(), Some("`code`|"));
    // Closing a span typed without the pair: an odd count before is an open span.
    assert_eq!(typed("use `x|", '`'), None);
    // Right after a word it is a closer or an apostrophe, not an opener.
    assert_eq!(typed("x|", '`'), None);
}

#[test]
fn backspace_in_an_empty_pair_removes_both() {
    for marked in ["(|)", "[|]", "`|`"] {
        let (text, caret) = parse(marked);
        let plan = backspace_pair(&text, caret.start).unwrap();
        assert_eq!(show(&text, &plan), "|");
    }
    let (text, caret) = parse("(a|)");
    assert!(backspace_pair(&text, caret.start).is_none());
}

#[test]
fn a_url_pasted_over_text_links_it() {
    let (text, selection) = parse("see «the docs» now");
    let plan = link_from_paste(&text, selection.clone(), "https://example.com/a").unwrap();
    assert_eq!(
        show(&text, &plan),
        "see [the docs](https://example.com/a)| now"
    );
    // Parentheses in the URL would end the link early: it goes in angle brackets.
    let plan = link_from_paste(&text, selection.clone(), "https://e.com/a_(b)").unwrap();
    assert_eq!(
        plan.apply_to(&text),
        "see [the docs](<https://e.com/a_(b)>) now"
    );
    // Not a URL, no selection, or a URL selected: an ordinary paste.
    assert!(link_from_paste(&text, selection.clone(), "just words").is_none());
    assert!(link_from_paste(&text, 4..4, "https://e.com").is_none());
    let (text, selection) = parse("«https://old.com»");
    assert!(link_from_paste(&text, selection, "https://new.com").is_none());
}

#[test]
fn urls_are_one_link_with_a_known_scheme() {
    assert!(is_url("https://example.com"));
    assert!(is_url("  HTTP://x.y\r\n"));
    assert!(is_url("mailto:a@b.c"));
    assert!(!is_url("https://"));
    assert!(!is_url("example.com"));
    assert!(!is_url("https://a.com and more"));
}

#[test]
fn home_moves_between_the_text_and_the_line_start() {
    let home = |marked: &str| {
        let (text, caret) = parse(marked);
        home_target(&text, caret.start).map(|at| {
            let mut out = text.clone();
            out.insert(at, '|');
            out
        })
    };
    assert_eq!(home("- item te|xt").as_deref(), Some("- |item text"));
    assert_eq!(home("- |item").as_deref(), Some("|- item"));
    assert_eq!(home("|- item").as_deref(), Some("- |item"));
    assert_eq!(home("  1. [ ] do |it").as_deref(), Some("  1. [ ] |do it"));
    assert_eq!(home("> quo|ted").as_deref(), Some("> |quoted"));
    assert_eq!(home("## Ti|tle").as_deref(), Some("## |Title"));
    // Plain lines keep the editor's own Home.
    assert_eq!(home("plain te|xt"), None);
}

fn enter(marked: &str, run: fn(&str, usize, &'static str) -> Option<EditPlan>) -> Option<String> {
    let (text, caret) = parse(marked);
    run(&text, caret.start, "\n").map(|plan| show(&text, &plan))
}

#[test]
fn enter_after_an_unclosed_fence_closes_it() {
    assert_eq!(
        enter("```rust|", enter_after_fence).as_deref(),
        Some("```rust\n|\n```")
    );
    assert_eq!(
        enter("text\n  ~~~~|\nmore", enter_after_fence).as_deref(),
        Some("text\n  ~~~~\n  |\n  ~~~~\nmore")
    );
    // Already closed below: a plain Enter.
    assert_eq!(enter("```|\ncode\n```", enter_after_fence), None);
    // Above an existing block it takes that block's opener as its closer, leaving the block's
    // closer open: it is closed here.
    assert_eq!(
        enter("```|\n\n```\nold\n```", enter_after_fence).as_deref(),
        Some("```\n|\n```\n\n```\nold\n```")
    );
    // Not at the line end, or not a fence.
    assert_eq!(enter("``|`", enter_after_fence), None);
    assert_eq!(enter("``|", enter_after_fence), None);
}

#[test]
fn enter_in_a_quote_continues_it() {
    assert_eq!(enter("> a|", enter_in_quote).as_deref(), Some("> a\n> |"));
    assert_eq!(
        enter("> > a|", enter_in_quote).as_deref(),
        Some("> > a\n> > |")
    );
    assert_eq!(enter(">a|", enter_in_quote).as_deref(), Some(">a\n> |"));
    assert_eq!(enter("> a|b", enter_in_quote).as_deref(), Some("> a\n> |b"));
    // An empty quote line drops a level, and ends the quote after the last.
    assert_eq!(enter("> > |", enter_in_quote).as_deref(), Some("> |"));
    assert_eq!(enter("> |", enter_in_quote).as_deref(), Some("|"));
    // A list inside the quote continues inside it; its empty item ends the list, not the quote.
    assert_eq!(
        enter("> - a|", enter_in_quote).as_deref(),
        Some("> - a\n> - |")
    );
    assert_eq!(
        enter("> 2. a|", enter_in_quote).as_deref(),
        Some("> 2. a\n> 3. |")
    );
    assert_eq!(
        enter("> - [x] a|", enter_in_quote).as_deref(),
        Some("> - [x] a\n> - [ ] |")
    );
    assert_eq!(enter("> - |", enter_in_quote).as_deref(), Some("> |"));
    assert_eq!(enter("not a quote|", enter_in_quote), None);
}

#[test]
fn enter_in_a_numbered_list_renumbers_the_items_after() {
    let run = |marked: &str| {
        let (text, caret) = parse(marked);
        crate::editor::markdown_edit::enter_in_list(&text, caret.start, "\n")
            .map(|plan| show(&text, &plan))
    };
    assert_eq!(
        run("1. a|\n2. b\n   more\n   - child\n3. c\n\ntext").as_deref(),
        Some("1. a\n2. |\n3. b\n   more\n   - child\n4. c\n\ntext")
    );
    // A list of all `1.` stays that way after the new item.
    assert_eq!(run("1. a|\n1. b").as_deref(), Some("1. a\n2. |\n1. b"));
    // Another delimiter is another list.
    assert_eq!(run("1. a|\n2) b").as_deref(), Some("1. a\n2. |\n2) b"));
}

fn command(marked: &str, run: impl Fn(&str, &[Range<usize>]) -> EditPlan) -> String {
    let (text, selection) = parse(marked);
    show(&text, &run(&text, &[selection]))
}

#[test]
fn heading_commands_set_toggle_and_clear_the_level() {
    let heading =
        |level| move |text: &str, selections: &[Range<usize>]| set_heading(text, selections, level);
    assert_eq!(command("ti|tle", heading(2)), "## ti|tle");
    assert_eq!(command("|title", heading(1)), "# |title");
    assert_eq!(command("### ti|tle", heading(1)), "# ti|tle");
    assert_eq!(command("## ti|tle", heading(2)), "ti|tle");
    assert_eq!(command("## ti|tle", heading(0)), "ti|tle");
    assert_eq!(command("> quo|te", heading(3)), "> ### quo|te");
    assert_eq!(command("«a\n\nb»", heading(1)), "«# a\n\n# b»");
}

#[test]
fn list_commands_make_switch_and_remove_markers() {
    let list =
        |kind| move |text: &str, selections: &[Range<usize>]| toggle_list(text, selections, kind);
    assert_eq!(command("it|em", list(ListKind::Bullet)), "- it|em");
    assert_eq!(command("- it|em", list(ListKind::Bullet)), "it|em");
    assert_eq!(command("1. it|em", list(ListKind::Bullet)), "- it|em");
    assert_eq!(
        command("«a\n  b\n\nc»", list(ListKind::Numbered)),
        "«1. a\n  2. b\n\n3. c»"
    );
    assert_eq!(command("«1. a\n2. b»", list(ListKind::Numbered)), "«a\nb»");
    // A single line under a numbered item counts on from it.
    assert_eq!(command("1. a\n|b", list(ListKind::Numbered)), "1. a\n2. |b");
    // The task box survives a change of marker.
    assert_eq!(
        command("1. [x] d|one", list(ListKind::Bullet)),
        "- [x] d|one"
    );
}

#[test]
fn the_task_command_adds_ticks_and_unticks_boxes() {
    let task =
        |text: &str, selections: &[Range<usize>]| toggle_list(text, selections, ListKind::Task);
    assert_eq!(command("bu|y", task), "- [ ] bu|y");
    assert_eq!(command("- bu|y", task), "- [ ] bu|y");
    assert_eq!(command("  2. bu|y", task), "  2. [ ] bu|y");
    assert_eq!(command("- [ ] bu|y", task), "- [x] bu|y");
    assert_eq!(command("- [X] bu|y", task), "- [ ] bu|y");
    // Mixed: every box is ticked first, then all untick together.
    assert_eq!(command("«- [x] a\n- [ ] b»", task), "«- [x] a\n- [x] b»");
    assert_eq!(command("«- [x] a\n- [x] b»", task), "«- [ ] a\n- [ ] b»");
    assert_eq!(command("«- [ ] a\nb»", task), "«- [ ] a\n- [ ] b»");
}

const TABLE: &str = "| a | b |\n| - | - |\n| 1 | 2 |";

#[test]
fn tab_in_the_last_cell_adds_a_row() {
    // The caret after the `2`, in the last cell.
    let caret = TABLE.len() - 2;
    let plan = append_row(TABLE, caret, "\n").unwrap();
    assert_eq!(plan.apply_to(TABLE), format!("{TABLE}\n|  |  |"));
    assert_eq!(plan.selections, vec![TABLE.len() + 3..TABLE.len() + 3]);
    // Any other cell moves on instead.
    assert!(append_row(TABLE, TABLE.find('1').unwrap(), "\n").is_none());
    // A header and delimiter only: no body row to add after.
    assert!(append_row("| a |\n| - |", 9, "\n").is_none());
}

#[test]
fn insert_table_puts_a_table_on_a_blank_line_or_after_the_text() {
    let (text, caret) = parse("|");
    let plan = insert_table(&text, caret.start, "\n");
    assert_eq!(
        show(&text, &plan),
        "| «Column 1» | Column 2 | Column 3 |\n| -------- | -------- | -------- |\n|          |          |          |"
    );
    let (text, caret) = parse("text|\nnext");
    let plan = insert_table(&text, caret.start, "\n");
    let out = plan.apply_to(&text);
    assert!(out.starts_with("text\n\n| Column 1 |"), "{out}");
    assert!(
        out.ends_with("|          |          |          |\n\nnext"),
        "{out}"
    );
}

#[test]
fn table_columns_are_added_after_the_caret_and_deleted_under_it() {
    // The caret after the `a`, in the first column.
    let plan = table_column(TABLE, 3, true).unwrap();
    assert_eq!(
        plan.apply_to(TABLE),
        "| a   |     | b   |\n| --- | --- | --- |\n| 1   |     | 2   |"
    );
    assert_eq!(plan.selections, vec![8..8]);
    let plan = table_column(TABLE, 3, false).unwrap();
    assert_eq!(plan.apply_to(TABLE), "| b   |\n| --- |\n| 2   |");
    assert_eq!(plan.selections, vec![2..2]);
    assert!(table_column("| a |\n| - |", 2, false).is_none());
    // Alignment markers stay with their column.
    let plan = table_column("| a | b |\n| :- | -: |", 2, true).unwrap();
    assert_eq!(
        plan.apply_to("| a | b |\n| :- | -: |"),
        "| a   |     |   b |\n| :-- | --- | --: |"
    );
}

#[test]
fn completion_queries_start_where_the_list_does() {
    assert_eq!(wiki_query("see [[No"), Some(6));
    assert_eq!(wiki_query("[["), Some(2));
    assert_eq!(wiki_query("[[Note]] and"), None);
    assert_eq!(wiki_query("[[Note|al"), None);
    assert_eq!(wiki_query("[[Note#he"), None);
    assert_eq!(tag_query("#pro"), Some(1));
    assert_eq!(tag_query("a (#pro/fa"), Some(4));
    assert_eq!(tag_query("#"), None);
    assert_eq!(tag_query("a#b"), None);
    assert_eq!(tag_query("# Heading"), None);
    assert_eq!(slash_query("/"), Some(0));
    assert_eq!(slash_query("  /ta"), Some(2));
    assert_eq!(slash_query("> /co"), Some(2));
    assert_eq!(slash_query("a /b"), None);
    assert_eq!(slash_query("/usr/bin"), None);
    assert_eq!(slash_query("/a b"), None);
}

#[test]
fn slash_commands_rank_by_label_then_words_then_letters() {
    assert_eq!(rank_slash("").len(), SLASH_COMMANDS.len());
    assert_eq!(rank_slash("ta")[..2], ["Task list", "Table"]);
    assert_eq!(rank_slash("tab")[0], "Table");
    assert_eq!(rank_slash("todo"), vec!["Task list"]);
    assert_eq!(rank_slash("h2")[0], "Heading 2");
    assert_eq!(rank_slash("cb")[0], "Code block");
    assert!(rank_slash("zzz").is_empty());
}

#[test]
fn every_slash_command_writes_something() {
    for (label, _) in SLASH_COMMANDS {
        let (text, selection) = slash_snippet(label, "\n", "2026-11-04", "09:30", false)
            .unwrap_or_else(|| panic!("{label} has no snippet"));
        assert!(selection.end <= text.len(), "{label}");
    }
    assert_eq!(
        slash_snippet("Divider", "\n", "", "", true).unwrap().0,
        "\n---\n"
    );
}

#[test]
fn names_rank_prefix_first_and_keep_their_order() {
    let names: Vec<String> = ["project/fastpad", "fast", "breakfast", "faster"]
        .map(String::from)
        .to_vec();
    assert_eq!(
        rank_names(&names, "fast", 10),
        vec!["fast", "faster", "project/fastpad", "breakfast"]
    );
    assert_eq!(rank_names(&names, "FAST", 2), vec!["fast", "faster"]);
}
