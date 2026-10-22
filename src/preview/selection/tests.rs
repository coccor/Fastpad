use super::*;
use crate::preview::model::parse_document;

fn blocks(source: &str) -> Vec<Block> {
    parse_document(source).0
}

/// Every run of every block, with sections as their `open` attribute says.
fn runs_of(blocks: &[Block]) -> Vec<Vec<String>> {
    blocks
        .iter()
        .map(|block| {
            text_runs(&block.kind, &mut |_, attribute| attribute)
                .iter()
                .map(|run| String::from_utf16_lossy(&run.text.units()))
                .collect()
        })
        .collect()
}

fn all(blocks: &[Block]) -> Selection {
    let last = blocks.len() - 1;
    let runs = text_runs(&blocks[last].kind, &mut |_, attribute| attribute);
    let end = runs.last().map_or(TextPoint::new(last, 0, 0), |run| {
        TextPoint::new(last, runs.len() as u32 - 1, run.text.utf16_len())
    });
    Selection {
        anchor: TextPoint::default(),
        focus: end,
    }
}

fn copy_all(source: &str) -> Clip {
    let blocks = blocks(source);
    copy(&blocks, &all(&blocks), &mut |_, _, attribute| attribute)
}

#[test]
fn runs_follow_reading_order_without_decoration() {
    // Break caught: list markers, alert titles or the rule landing in the copy, or cells out of
    // row order.
    let blocks = blocks(
        "# Title\n\n- one\n- two\n\n> [!NOTE]\n> said\n\n---\n\n| a | b |\n|---|---|\n| 1 |\n\n```\ncode\n```\n",
    );
    assert_eq!(
        runs_of(&blocks),
        vec![
            vec!["Title".to_owned()],
            vec!["one".to_owned(), "two".to_owned()],
            vec!["said".to_owned()],
            vec![],
            // A short row still has every column, as layout draws it.
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "1".to_owned(),
                String::new()
            ],
            vec!["code".to_owned()],
        ]
    );
}

#[test]
fn a_collapsed_section_gives_its_summary_and_keeps_counting_sections() {
    // Break caught: hidden text copied, or a section after a collapsed one asking about the
    // wrong index (layout counts the nested ones it skips).
    let source = "<details><summary>Outer</summary>\n\n<details open><summary>Inner</summary>\n\nhidden\n\n</details>\n\n</details>\n\n";
    let blocks = blocks(source);
    let mut asked = Vec::new();
    let runs = text_runs(&blocks[0].kind, &mut |index, attribute| {
        asked.push(index);
        attribute
    });
    let texts = runs
        .iter()
        .map(|run| String::from_utf16_lossy(&run.text.units()))
        .collect::<Vec<_>>();
    assert_eq!(texts, vec!["Outer"]);
    assert_eq!(asked, vec![0], "only the visible section is asked about");
    // Opened by the user, both summaries and the text show.
    let runs = text_runs(&blocks[0].kind, &mut |_, _| true);
    assert_eq!(runs.len(), 3);
}

#[test]
fn points_order_by_block_then_run_then_offset() {
    let a = TextPoint::new(1, 0, 9);
    let b = TextPoint::new(1, 1, 0);
    let c = TextPoint::new(2, 0, 0);
    assert!(a < b && b < c);
    let backwards = Selection {
        anchor: c,
        focus: a,
    };
    assert_eq!(backwards.ordered(), (a, c));
}

#[test]
fn run_range_covers_the_middle_runs_whole_and_cuts_the_ends() {
    let selection = Selection {
        anchor: TextPoint::new(0, 1, 3),
        focus: TextPoint::new(2, 0, 4),
    };
    assert_eq!(selection.run_range(0, 0, 10), None);
    assert_eq!(selection.run_range(0, 1, 10), Some((3, 10)));
    assert_eq!(selection.run_range(1, 5, 7), Some((0, 7)));
    assert_eq!(selection.run_range(2, 0, 10), Some((0, 4)));
    assert_eq!(selection.run_range(2, 1, 10), None);
    // An end past a shorter run is clamped to it.
    assert_eq!(selection.run_range(2, 0, 2), Some((0, 2)));
}

#[test]
fn words_spaces_and_punctuation_select_as_a_double_click_does() {
    let units = "foo_bar  baz, 😀x".encode_utf16().collect::<Vec<_>>();
    assert_eq!(word_range(&units, 2), (0, 7));
    assert_eq!(word_range(&units, 7), (7, 9));
    assert_eq!(word_range(&units, 12), (12, 13));
    // At the end, the last character's class.
    assert_eq!(word_range(&units, units.len() as u32), (16, 17));
    // An emoji is one character, two units.
    assert_eq!(word_range(&units, 14), (14, 16));
    assert_eq!(word_range(&[], 0), (0, 0));
}

#[test]
fn plain_text_drops_markers_and_lays_out_blocks() {
    let clip = copy_all(
        "# Title\n\nSome **bold** and `code`.\n\n- one\n- [x] done\n  - nested\n\n1. first\n2. second\n\n> quoted\n>\n> again\n",
    );
    assert_eq!(
        clip.plain,
        "Title\n\nSome bold and code.\n\n- one\n- [x] done\n  - nested\n\n1. first\n2. second\n\n> quoted\n>\n> again"
    );
}

#[test]
fn plain_text_separates_cells_with_tabs_and_keeps_code_exactly() {
    let clip = copy_all("| a | b |\n|---|---|\n| 1 | 2 |\n\n```\nfn main() {\n    x\n}\n```\n");
    assert_eq!(clip.plain, "a\tb\n1\t2\n\nfn main() {\n    x\n}");
}

#[test]
fn a_partial_selection_copies_only_its_part() {
    let blocks = blocks("Hello **brave** world\n\nSecond paragraph\n");
    let selection = Selection {
        anchor: TextPoint::new(0, 0, 6),
        focus: TextPoint::new(1, 0, 6),
    };
    let clip = copy(&blocks, &selection, &mut |_, _, attribute| attribute);
    assert_eq!(clip.plain, "brave world\n\nSecond");
    assert_eq!(
        clip.html,
        "<p><strong>brave</strong> world</p><p>Second</p>"
    );
}

#[test]
fn a_drag_ending_at_the_start_of_a_run_does_not_copy_it() {
    // Break caught: a trailing blank line pasted after a paragraph selected to its end.
    let blocks = blocks("One\n\nTwo\n");
    let selection = Selection {
        anchor: TextPoint::new(0, 0, 0),
        focus: TextPoint::new(1, 0, 0),
    };
    let clip = copy(&blocks, &selection, &mut |_, _, attribute| attribute);
    assert_eq!(clip.plain, "One");
}

#[test]
fn html_uses_semantic_elements_and_escapes_text() {
    let clip = copy_all(
        "## A < B\n\n- [x] done\n- [link](https://x.dev/?a=1&b=2)\n\n| h |\n|---|\n| c |\n\n```\n<tag>\n```\n",
    );
    assert_eq!(
        clip.html,
        concat!(
            "<h2>A &lt; B</h2>",
            "<ul><li><p><input type=\"checkbox\" disabled checked> done</p></li>",
            "<li><p><a href=\"https://x.dev/?a=1&amp;b=2\">link</a></p></li></ul>",
            "<table><thead><tr><th>h</th></tr></thead><tbody><tr><td>c</td></tr></tbody></table>",
            "<pre><code>&lt;tag&gt;</code></pre>",
        )
    );
}

#[test]
fn a_numbered_list_keeps_its_start_and_quotes_nest() {
    let clip = copy_all("3. three\n4. four\n\n> outer\n>\n> > inner\n");
    assert_eq!(
        clip.html,
        concat!(
            "<ol start=\"3\"><li><p>three</p></li><li><p>four</p></li></ol>",
            "<blockquote><p>outer</p><blockquote><p>inner</p></blockquote></blockquote>",
        )
    );
    assert_eq!(clip.plain, "3. three\n4. four\n\n> outer\n>\n> > inner");
}

#[test]
fn crossing_spans_reopen_instead_of_misnesting() {
    let blocks = blocks("***both** only*\n");
    let clip = copy(&blocks, &all(&blocks), &mut |_, _, attribute| attribute);
    // Whatever the parser's span order, every tag closes in the order it opened.
    let mut stack = Vec::new();
    for tag in clip.html.split('<').skip(1) {
        let name = tag.split('>').next().unwrap();
        if let Some(closing) = name.strip_prefix('/') {
            assert_eq!(stack.pop(), Some(closing.to_owned()), "{}", clip.html);
        } else {
            stack.push(name.split(' ').next().unwrap().to_owned());
        }
    }
    assert!(stack.is_empty(), "{}", clip.html);
    assert_eq!(clip.plain, "both only");
}

#[test]
fn an_inline_image_copies_as_its_alt_text() {
    let clip = copy_all("Look ![a cat](cat.png) here\n");
    assert_eq!(clip.plain, "Look a cat here");
    assert_eq!(clip.html, "<p>Look a cat here</p>");
}

#[test]
fn cf_html_offsets_point_at_the_fragment() {
    // Break caught: Word pasting nothing, or the header, because an offset counted characters
    // instead of UTF-8 bytes.
    let fragment = "<p>é and 😀</p>";
    let wrapped = cf_html(fragment);
    let field = |name: &str| -> usize {
        let start = wrapped.find(name).unwrap() + name.len();
        wrapped[start..start + 10].parse().unwrap()
    };
    let bytes = wrapped.as_bytes();
    let (start, end) = (field("StartFragment:"), field("EndFragment:"));
    assert_eq!(&bytes[start..end], fragment.as_bytes());
    assert_eq!(field("EndHTML:"), bytes.len());
    assert!(wrapped[field("StartHTML:")..].starts_with("<html>"));
}
