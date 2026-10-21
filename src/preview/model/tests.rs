use super::*;

fn kinds(source: &str) -> Vec<BlockKind> {
    parse_document(source)
        .0
        .into_iter()
        .map(|block| block.kind)
        .collect()
}

fn plain(text: &str) -> RichText {
    RichText::plain(text)
}

fn styled(text: &str, spans: Vec<Span>) -> RichText {
    RichText {
        spans,
        ..plain(text)
    }
}

fn span(range: Range<u32>, style: InlineStyle) -> Span {
    Span { range, style }
}

fn para(text: RichText) -> BlockKind {
    BlockKind::Paragraph {
        align: TextAlign::Inherit,
        text,
    }
}

fn heading(level: u8, text: RichText) -> BlockKind {
    BlockKind::Heading {
        level,
        align: TextAlign::Inherit,
        anchor: None,
        text,
    }
}

fn container(align: TextAlign, children: Vec<BlockKind>) -> BlockKind {
    BlockKind::Container { align, children }
}

fn image(dest: &str, alt: &str) -> ImageRef {
    ImageRef {
        dest: dest.into(),
        alt: alt.into(),
        ..ImageRef::default()
    }
}

fn with_images(text: &str, images: Vec<(u32, ImageRef)>) -> RichText {
    RichText {
        images: images
            .into_iter()
            .map(|(position, image)| InlineImage { position, image })
            .collect(),
        ..plain(text)
    }
}

#[test]
fn headings_and_paragraphs_carry_source_ranges() {
    let source = "# Title\n\nHello *world*\n";
    let (blocks, _) = parse_document(source);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].kind, heading(1, plain("Title")));
    assert_eq!(source[blocks[0].bytes.clone()].trim_end(), "# Title");
    assert_eq!(blocks[0].lines, 0..1);
    assert_eq!(
        blocks[1].kind,
        para(styled(
            "Hello world",
            vec![span(6..11, InlineStyle::Emphasis)]
        ))
    );
    assert_eq!(source[blocks[1].bytes.clone()].trim_end(), "Hello *world*");
    assert_eq!(blocks[1].lines, 2..3);
}

#[test]
fn setext_headings_are_headings() {
    assert_eq!(kinds("Title\n=====\n"), vec![heading(1, plain("Title"))]);
}

#[test]
fn task_lists_nest_and_record_checked_state() {
    let expected = BlockKind::List {
        start: None,
        items: vec![
            ListItem {
                task: Some(true),
                blocks: vec![para(plain("done"))],
            },
            ListItem {
                task: Some(false),
                blocks: vec![
                    para(plain("todo")),
                    BlockKind::List {
                        start: None,
                        items: vec![ListItem {
                            task: None,
                            blocks: vec![para(plain("nested"))],
                        }],
                    },
                ],
            },
        ],
    };
    assert_eq!(
        kinds("- [x] done\n- [ ] todo\n  - nested\n"),
        vec![expected]
    );
}

#[test]
fn loose_task_lists_keep_their_checkboxes() {
    let expected = BlockKind::List {
        start: None,
        items: vec![
            ListItem {
                task: Some(true),
                blocks: vec![para(plain("a"))],
            },
            ListItem {
                task: Some(false),
                blocks: vec![para(plain("b"))],
            },
        ],
    };
    assert_eq!(kinds("- [x] a\n\n- [ ] b\n"), vec![expected]);
}

#[test]
fn ordered_lists_keep_their_start_number() {
    let kinds = kinds("3. a\n4. b\n");
    let [BlockKind::List { start, items }] = kinds.as_slice() else {
        panic!("expected one list, got {kinds:?}");
    };
    assert_eq!(*start, Some(3));
    assert_eq!(items.len(), 2);
}

#[test]
fn quotes_nest() {
    assert_eq!(
        kinds("> quote\n>\n> > inner\n"),
        vec![BlockKind::Quote(vec![
            para(plain("quote")),
            BlockKind::Quote(vec![para(plain("inner"))]),
        ])]
    );
}

#[test]
fn fenced_and_indented_code_blocks() {
    assert_eq!(
        kinds("```rust\nfn main() {}\n```\n"),
        vec![BlockKind::Code {
            language: "rust".into(),
            text: "fn main() {}".into()
        }]
    );
    assert_eq!(
        kinds("    x = 1\n"),
        vec![BlockKind::Code {
            language: String::new(),
            text: "x = 1".into()
        }]
    );
}

#[test]
fn tables_keep_alignment_head_and_styled_cells() {
    assert_eq!(
        kinds("| a | b |\n|:--|--:|\n| 1 | **2** |\n"),
        vec![BlockKind::Table {
            alignments: vec![CellAlign::Left, CellAlign::Right],
            head: vec![plain("a"), plain("b")],
            rows: vec![vec![
                plain("1"),
                styled("2", vec![span(0..1, InlineStyle::Strong)])
            ]],
        }]
    );
}

#[test]
fn strikethrough_links_and_inline_code_become_spans() {
    assert_eq!(
        kinds("~~old~~ [site](https://x.dev) `code`\n"),
        vec![para(styled(
            "old site code",
            vec![
                span(0..3, InlineStyle::Strikethrough),
                span(4..8, InlineStyle::Link("https://x.dev".into())),
                span(9..13, InlineStyle::Code),
            ],
        ))]
    );
}

#[test]
fn markdown_images_flow_inline_with_their_title() {
    assert_eq!(
        kinds("![logo](img/logo.png)\n"),
        vec![para(with_images(
            "\u{FFFC}",
            vec![(0, image("img/logo.png", "logo"))]
        ))]
    );
    let titled = ImageRef {
        title: "Logo".into(),
        ..image("a.png", "logo")
    };
    assert_eq!(
        kinds("See ![logo](a.png \"Logo\") here\n"),
        vec![para(with_images("See \u{FFFC} here", vec![(4, titled)]))]
    );
}

#[test]
fn badge_rows_are_linked_inline_images_in_markdown_and_html() {
    let expected = vec![para(RichText {
        spans: vec![
            span(0..1, InlineStyle::Link("https://a".into())),
            span(2..3, InlineStyle::Link("https://b".into())),
        ],
        ..with_images(
            "\u{FFFC} \u{FFFC}",
            vec![(0, image("a.svg", "a")), (2, image("b.svg", "b"))],
        )
    })];
    assert_eq!(
        kinds("[![a](a.svg)](https://a) [![b](b.svg)](https://b)\n"),
        expected
    );
    assert_eq!(
        kinds(
            "<a href=\"https://a\"><img src=\"a.svg\" alt=\"a\"></a> <a href=\"https://b\"><img src=\"b.svg\" alt=\"b\"></a>\n"
        ),
        expected
    );
}

#[test]
fn span_offsets_are_utf16() {
    assert_eq!(
        kinds("**é😀**\n"),
        vec![para(styled("é😀", vec![span(0..3, InlineStyle::Strong)]))]
    );
}

#[test]
fn slices_are_offset_by_their_base_position() {
    let blocks = parse_blocks("para\n", 100, 7, &[]);
    assert_eq!(blocks[0].bytes.start, 100);
    assert_eq!(blocks[0].lines, 7..8);
}

#[test]
fn reference_definitions_are_collected_and_resolve_in_slices() {
    let (blocks, refdefs) = parse_document("[site]\n\n[Site]: https://x.dev\n");
    assert_eq!(refdefs.len(), 1);
    assert_eq!(refdefs[0].key, "site");
    assert_eq!(refdefs[0].dest, "https://x.dev");
    let link = para(styled(
        "site",
        vec![span(0..4, InlineStyle::Link("https://x.dev".into()))],
    ));
    assert_eq!(blocks[0].kind, link);
    assert_eq!(parse_blocks("[site]\n", 0, 0, &refdefs)[0].kind, link);
}

#[test]
fn footnote_syntax_stays_literal() {
    assert_eq!(kinds("a[^1]\n"), vec![para(plain("a[^1]"))]);
}

#[test]
fn a_centred_div_wraps_the_markdown_blocks_inside_it() {
    let source = "<div align=\"center\">\n\n# FastPad\n\nFast.\n\n</div>\n\nAfter\n";
    let (blocks, _) = parse_document(source);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            container(
                TextAlign::Center,
                vec![heading(1, plain("FastPad")), para(plain("Fast."))]
            ),
            para(plain("After")),
        ]
    );
    assert_eq!(blocks[0].lines, 0..7);
    assert!(
        source[blocks[0].bytes.clone()]
            .trim_end()
            .ends_with("</div>")
    );
}

#[test]
fn rules_and_html_blocks() {
    assert_eq!(
        kinds("---\n\n<div>hi</div>\n"),
        vec![
            BlockKind::Rule,
            container(TextAlign::Inherit, vec![para(plain("hi"))])
        ]
    );
}

#[test]
fn html_images_carry_their_attributes() {
    let expected = ImageRef {
        dest: "icon.svg".into(),
        alt: "FastPad".into(),
        title: "Logo".into(),
        width: Some(Length::Pixels(96)),
        height: Some(Length::Pixels(96)),
        sources: Vec::new(),
    };
    assert_eq!(
        kinds(
            "<img src=\"icon.svg\" alt=\"FastPad\" width=\"96\" height=\"96px\" title=\"Logo\">\n"
        ),
        vec![para(with_images("\u{FFFC}", vec![(0, expected)]))]
    );
    assert_eq!(kinds("<img alt=\"no source\">\n"), vec![]);
}

#[test]
fn inline_html_styles_become_spans() {
    assert_eq!(
        kinds(
            "Press <kbd>Ctrl</kbd>+<kbd>S</kbd>, H<sub>2</sub>O, x<sup>2</sup>, <mark>hot</mark>, <ins>new</ins>, <small>fine</small>, <b>b</b><i>i</i><s>s</s><code>c</code>\n"
        ),
        vec![para(styled(
            "Press Ctrl+S, H2O, x2, hot, new, fine, bisc",
            vec![
                span(6..10, InlineStyle::Keyboard),
                span(11..12, InlineStyle::Keyboard),
                span(15..16, InlineStyle::Subscript),
                span(20..21, InlineStyle::Superscript),
                span(23..26, InlineStyle::Mark),
                span(28..31, InlineStyle::Underline),
                span(33..37, InlineStyle::Small),
                span(39..40, InlineStyle::Strong),
                span(40..41, InlineStyle::Emphasis),
                span(41..42, InlineStyle::Strikethrough),
                span(42..43, InlineStyle::Code),
            ],
        ))]
    );
}

#[test]
fn quotes_and_grouping_tags_keep_their_text() {
    assert_eq!(
        kinds("<q>hi</q> <cite>c</cite> <span>s</span> <abbr title=\"x\">a</abbr>\n"),
        vec![para(styled(
            "\u{201C}hi\u{201D} c s a",
            vec![span(5..6, InlineStyle::Emphasis)]
        ))]
    );
}

#[test]
fn html_text_collapses_whitespace_and_br_breaks_lines() {
    assert_eq!(
        kinds("<p align=\"right\">\n  one   two<br>\n  three\n</p>\n"),
        vec![BlockKind::Paragraph {
            align: TextAlign::Right,
            text: plain("one two\nthree"),
        }]
    );
}

#[test]
fn details_hold_a_summary_and_markdown_children() {
    assert_eq!(
        kinds(
            "<details>\n<summary><b>More</b> ways</summary>\n\n| a |\n|---|\n| 1<br>2 |\n\n</details>\n"
        ),
        vec![BlockKind::Details {
            open: false,
            summary: styled("More ways", vec![span(0..4, InlineStyle::Strong)]),
            children: vec![BlockKind::Table {
                alignments: vec![CellAlign::None],
                head: vec![plain("a")],
                rows: vec![vec![plain("1\n2")]],
            }],
        }]
    );
}

#[test]
fn details_can_start_open_and_default_their_summary() {
    assert_eq!(
        kinds("<details open>\n\nBody\n\n</details>\n"),
        vec![BlockKind::Details {
            open: true,
            summary: plain("Details"),
            children: vec![para(plain("Body"))],
        }]
    );
}

#[test]
fn unclosed_tags_close_at_the_end_of_their_container_or_document() {
    let source = "<div align=\"center\">\n\nA\n\nB\n";
    let (blocks, _) = parse_document(source);
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].kind,
        container(TextAlign::Center, vec![para(plain("A")), para(plain("B"))])
    );
    assert_eq!(blocks[0].bytes, 0..source.len());
    assert_eq!(
        kinds("> <div>\n> quoted\n\nafter\n"),
        vec![
            BlockKind::Quote(vec![container(
                TextAlign::Inherit,
                vec![para(plain("quoted"))]
            )]),
            para(plain("after")),
        ]
    );
}

#[test]
fn deep_html_nesting_is_capped() {
    let source = "<mark>".repeat(1_000) + "x\n";
    let styled = kinds(&source);
    let [BlockKind::Paragraph { text, .. }] = styled.as_slice() else {
        panic!("expected one paragraph");
    };
    assert_eq!(text.spans.len(), MAX_OPEN_STYLES);
    fn depth(kind: &BlockKind) -> usize {
        match kind {
            BlockKind::Details { children, .. } => {
                1 + children.iter().map(depth).max().unwrap_or(0)
            }
            _ => 0,
        }
    }
    let nested = "<details>".repeat(10_000) + "\n";
    let blocks = kinds(&nested);
    assert!(blocks.iter().map(depth).max().unwrap_or(0) <= MAX_FRAME_DEPTH);
}

#[test]
fn stray_end_tags_are_ignored() {
    assert_eq!(kinds("</div></p>text</b>\n"), vec![para(plain("text"))]);
}

#[test]
fn block_tags_imply_the_end_of_paragraphs_and_headings() {
    assert_eq!(
        kinds("<p>one<div>two</div>\n"),
        vec![container(
            TextAlign::Inherit,
            vec![
                para(plain("one")),
                container(TextAlign::Inherit, vec![para(plain("two"))]),
            ]
        )]
    );
    assert_eq!(
        kinds("<h1>a<h2>b</h2>\n"),
        vec![container(
            TextAlign::Inherit,
            vec![heading(1, plain("a")), heading(2, plain("b"))]
        )]
    );
    assert_eq!(kinds("<h7>\nx\n</h7>\n"), vec![heading(6, plain("x"))]);
}

#[test]
fn block_tags_inside_markdown_inline_content_do_nothing() {
    assert_eq!(
        kinds("| <div>x</div> |\n|---|\n| <p>y |\n"),
        vec![BlockKind::Table {
            alignments: vec![CellAlign::None],
            head: vec![plain("x")],
            rows: vec![vec![plain("y")]],
        }]
    );
}

#[test]
fn picture_sources_record_their_colour_scheme() {
    let source = "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"dark.png 1x, dark@2x.png 2x\">\n  <source media=\"( PREFERS-COLOR-SCHEME : light )\" srcset=\"light.png\">\n  <source media=\"(min-width: 600px)\" srcset=\"wide.png\">\n  <img src=\"plain.png\" alt=\"Logo\">\n</picture>\n";
    let expected = ImageRef {
        sources: vec![
            ImageSource {
                url: "dark.png".into(),
                scheme: Some(ColorScheme::Dark),
            },
            ImageSource {
                url: "light.png".into(),
                scheme: Some(ColorScheme::Light),
            },
            ImageSource {
                url: "wide.png".into(),
                scheme: None,
            },
        ],
        ..image("plain.png", "Logo")
    };
    assert_eq!(
        kinds(source),
        vec![para(with_images("\u{FFFC}", vec![(0, expected)]))]
    );
}

#[test]
fn scripts_are_removed_and_unknown_tags_keep_their_text() {
    assert_eq!(
        kinds("a <script>x</script> b <center>c</center>\n"),
        vec![para(plain("a  b c"))]
    );
    let (blocks, _) = parse_document("<script>\nalert(1)\n</script>\n\nafter\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, para(plain("after")));
    assert_eq!(blocks[0].lines, 4..5);
}

#[test]
fn comments_produce_no_block() {
    let (blocks, _) = parse_document("<!-- note -->\n\ntext\n");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, para(plain("text")));
    assert_eq!(blocks[0].bytes.start, 15);
}

#[test]
fn ids_and_named_links_anchor_headings() {
    assert_eq!(
        kinds("<h2 id=\"Install\">Install it</h2>\n\n## <a name=\"usage\"></a>Usage\n"),
        vec![
            BlockKind::Heading {
                level: 2,
                align: TextAlign::Inherit,
                anchor: Some("Install".into()),
                text: plain("Install it"),
            },
            BlockKind::Heading {
                level: 2,
                align: TextAlign::Inherit,
                anchor: Some("usage".into()),
                text: plain("Usage"),
            },
        ]
    );
}

#[test]
fn sizes_alignments_and_media_parse_like_github() {
    assert_eq!(Length::parse("96"), Some(Length::Pixels(96)));
    assert_eq!(Length::parse(" 96px "), Some(Length::Pixels(96)));
    assert_eq!(Length::parse("50%"), Some(Length::Percent(50)));
    assert_eq!(Length::parse("12.7"), Some(Length::Pixels(12)));
    for invalid in ["0", "", "auto", "12em", ".5"] {
        assert_eq!(Length::parse(invalid), None, "{invalid}");
    }
    assert_eq!(TextAlign::parse("CENTER"), TextAlign::Center);
    assert_eq!(TextAlign::parse("middle"), TextAlign::Center);
    assert_eq!(TextAlign::parse("justify"), TextAlign::Justify);
    assert_eq!(TextAlign::parse("top"), TextAlign::Inherit);
    assert_eq!(
        ColorScheme::from_media("(prefers-color-scheme:dark)"),
        Some(ColorScheme::Dark)
    );
    assert_eq!(ColorScheme::from_media("print"), None);
}

#[test]
fn the_readme_fixture_renders_no_literal_markup() {
    fn collect(kind: &BlockKind, texts: &mut Vec<String>) {
        match kind {
            BlockKind::Heading { text, .. } | BlockKind::Paragraph { text, .. } => {
                texts.push(text.text.clone())
            }
            BlockKind::List { items, .. } => {
                for block in items.iter().flat_map(|item| &item.blocks) {
                    collect(block, texts);
                }
            }
            BlockKind::Quote(children)
            | BlockKind::Alert {
                blocks: children, ..
            }
            | BlockKind::Container { children, .. } => {
                for child in children {
                    collect(child, texts);
                }
            }
            BlockKind::Details {
                summary, children, ..
            } => {
                texts.push(summary.text.clone());
                for child in children {
                    collect(child, texts);
                }
            }
            BlockKind::Table { head, rows, .. } => texts.extend(
                head.iter()
                    .chain(rows.iter().flatten())
                    .map(|cell| cell.text.clone()),
            ),
            BlockKind::Code { text, .. } => texts.push(text.clone()),
            BlockKind::Rule => {}
        }
    }
    let source = include_str!("../../../tests/fixtures/html-readme.md");
    let mut texts = Vec::new();
    for block in parse_document(source).0 {
        collect(&block.kind, &mut texts);
    }
    for text in &texts {
        for tag in [
            "div", "img", "details", "summary", "kbd", "b", "br", "picture", "source", "p", "a",
        ] {
            assert!(
                !text.contains(&format!("<{tag}")) && !text.contains(&format!("</{tag}")),
                "literal <{tag}> in {text:?}"
            );
        }
        assert!(!text.contains("<!--"), "literal comment in {text:?}");
    }
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Other ways to install"))
    );
    assert!(texts.iter().any(|text| text == "Back to top"));
}

#[test]
fn front_matter_shows_as_a_table_of_its_properties() {
    // Break caught: front matter drawn as a rule and a setext heading, its keys run together.
    assert_eq!(
        kinds("---\ntitle: Plan\ntags: [a, b]\n---\n# Heading\n"),
        vec![
            BlockKind::Table {
                alignments: vec![CellAlign::Left, CellAlign::Left],
                head: vec![plain("Property"), plain("Value")],
                rows: vec![
                    vec![plain("title"), plain("Plan")],
                    vec![plain("tags"), plain("a, b")],
                ],
            },
            heading(1, plain("Heading")),
        ]
    );
    // A rule further down is still a rule.
    assert_eq!(kinds("text\n\n---\n\nmore\n").len(), 3);
}

#[test]
fn inline_tags_become_tag_spans_outside_links_and_code() {
    assert_eq!(
        kinds("Plan #project/atlas, (#todo) and C#.\n"),
        vec![para(styled(
            "Plan #project/atlas, (#todo) and C#.",
            vec![
                span(5..19, InlineStyle::Tag("project/atlas".to_owned())),
                span(22..27, InlineStyle::Tag("todo".to_owned())),
            ]
        ))]
    );
    assert_eq!(
        kinds("[#linked](x.md) `#code`\n"),
        vec![para(styled(
            "#linked #code",
            vec![
                span(0..7, InlineStyle::Link("x.md".to_owned())),
                span(8..13, InlineStyle::Code),
            ]
        ))]
    );
    // After an emphasis run, the next text event starts mid-word: no tag there.
    assert_eq!(
        kinds("*a*#b\n"),
        vec![para(styled("a#b", vec![span(0..1, InlineStyle::Emphasis)]))]
    );
}

#[test]
fn wikilinks_become_links_the_notebook_resolves() {
    assert_eq!(
        kinds("See [[Plan#Goals|the goals]] and [[Atlas]].\n"),
        vec![para(styled(
            "See the goals and Atlas.",
            vec![
                span(
                    4..13,
                    InlineStyle::Link("fastpad-wiki:Plan#Goals".to_owned())
                ),
                span(18..23, InlineStyle::Link("fastpad-wiki:Atlas".to_owned())),
            ]
        ))]
    );
}

/// The extended autolinks in the first block's text, as (linked text, destination).
fn autolinks(source: &str) -> Vec<(String, String)> {
    let kinds = kinds(source);
    let (BlockKind::Paragraph { text, .. } | BlockKind::Heading { text, .. }) = &kinds[0] else {
        panic!("expected text, got {kinds:?}");
    };
    let units = text.text.encode_utf16().collect::<Vec<_>>();
    let mut links = text
        .spans
        .iter()
        .filter_map(|span| match &span.style {
            InlineStyle::Link(dest) => Some((
                String::from_utf16_lossy(
                    &units[span.range.start as usize..span.range.end as usize],
                ),
                dest.clone(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    links.sort();
    links
}

fn link(text: &str, dest: &str) -> (String, String) {
    (text.to_owned(), dest.to_owned())
}

#[test]
fn bare_urls_and_emails_become_links() {
    assert_eq!(
        kinds("Visit https://example.com/a?b=1 now\n"),
        vec![para(styled(
            "Visit https://example.com/a?b=1 now",
            vec![span(
                6..31,
                InlineStyle::Link("https://example.com/a?b=1".to_owned())
            )]
        ))]
    );
    assert_eq!(
        autolinks("Go to www.commonmark.org/help or http://x.example.\n"),
        vec![
            link("http://x.example", "http://x.example"),
            link("www.commonmark.org/help", "http://www.commonmark.org/help"),
        ]
    );
    assert_eq!(
        autolinks("Mail foo.bar+baz@example.com.\n"),
        vec![link(
            "foo.bar+baz@example.com",
            "mailto:foo.bar+baz@example.com"
        )]
    );
    // Case-insensitive schemes, and after `*_~(`.
    assert_eq!(
        autolinks("(WWW.example.com) and ~HTTPS://a.b~\n"),
        vec![
            link("HTTPS://a.b", "HTTPS://a.b"),
            link("WWW.example.com", "http://WWW.example.com"),
        ]
    );
    // Inside emphasis and across a line break; UTF-16 offsets after non-ASCII text.
    assert_eq!(
        autolinks("**www.a.com**\nnaïve — https://b.org\n"),
        vec![
            link("https://b.org", "https://b.org"),
            link("www.a.com", "http://www.a.com"),
        ]
    );
    assert_eq!(
        autolinks("# Docs at www.docs.dev\n"),
        vec![link("www.docs.dev", "http://www.docs.dev")]
    );
}

#[test]
fn autolinks_need_a_valid_domain() {
    // No period, or an underscore in the last two segments.
    assert_eq!(autolinks("www. and http://localhost\n"), vec![]);
    assert_eq!(autolinks("www.ex_ample.com and http://a.b_c\n"), vec![]);
    // An underscore before the last two segments is fine.
    assert_eq!(
        autolinks("www.sub_domain.example.com\n"),
        vec![link(
            "www.sub_domain.example.com",
            "http://www.sub_domain.example.com"
        )]
    );
    // Not a word's middle, and an email needs a dotted domain and a local part.
    assert_eq!(
        autolinks("xwww.example.com and a@localhost and @b.com\n"),
        vec![]
    );
    assert_eq!(autolinks("a@b.c_\n"), vec![]);
}

#[test]
fn autolink_paths_stop_at_spaces_angles_and_trailing_punctuation() {
    assert_eq!(
        autolinks("www.a.com/path<b> end\n"),
        vec![link("www.a.com/path", "http://www.a.com/path")]
    );
    for trailing in ["?", "!", ".", ",", ":", "*", "_", "~", "'", "\"", "?!."] {
        assert_eq!(
            autolinks(&format!("see https://a.com/x{trailing} next\n")),
            vec![link("https://a.com/x", "https://a.com/x")],
            "{trailing}"
        );
    }
    // Balanced parentheses stay; an unbalanced closing one does not.
    assert_eq!(
        autolinks("www.a.com/q(1) and (www.b.com/q(2))\n"),
        vec![
            link("www.a.com/q(1)", "http://www.a.com/q(1)"),
            link("www.b.com/q(2)", "http://www.b.com/q(2)"),
        ]
    );
    // An email address inside a URL's path is part of the URL.
    assert_eq!(
        autolinks("https://a.com/u@b.com\n"),
        vec![link("https://a.com/u@b.com", "https://a.com/u@b.com")]
    );
}

#[test]
fn autolinks_skip_code_html_links_footnotes_and_math() {
    assert_eq!(
        kinds("`www.a.com` x\n"),
        vec![para(styled(
            "www.a.com x",
            vec![span(0..9, InlineStyle::Code)]
        ))]
    );
    assert_eq!(
        kinds("[site](https://x.dev) www.y.dev\n"),
        vec![para(styled(
            "site www.y.dev",
            vec![
                span(0..4, InlineStyle::Link("https://x.dev".to_owned())),
                span(5..14, InlineStyle::Link("http://www.y.dev".to_owned())),
            ]
        ))]
    );
    // A link's text is not linked again, nor an angle-bracket autolink.
    assert_eq!(
        autolinks("[www.a.com](https://b.com) <https://c.com>\n"),
        vec![
            link("https://c.com", "https://c.com"),
            link("www.a.com", "https://b.com"),
        ]
    );
    // An HTML block's text and a tag's attributes stay as written; Markdown text between inline
    // tags is prose.
    assert_eq!(autolinks("<p>www.a.com</p>\n"), vec![]);
    assert_eq!(autolinks("x <abbr title=\"www.a.com\">y</abbr>\n"), vec![]);
    assert_eq!(
        autolinks("<span>www.a.com</span> x\n"),
        vec![link("www.a.com", "http://www.a.com")]
    );
    // Footnote references and definitions, and math.
    assert_eq!(autolinks("x[^www.a.com] y\n"), vec![]);
    assert_eq!(autolinks("[^1]: see www.a.com and more\n"), vec![]);
    assert_eq!(autolinks("$www.a.com$ and $$https://b.com$$\n"), vec![]);
    assert_eq!(
        autolinks("$5 and www.a.com $6\n"),
        vec![link("www.a.com", "http://www.a.com")]
    );
}

#[test]
fn an_autolink_wins_over_a_tag_in_its_fragment() {
    assert_eq!(
        kinds("https://a.com/ #b https://a.com/x#c\n"),
        vec![para(styled(
            "https://a.com/ #b https://a.com/x#c",
            vec![
                span(15..17, InlineStyle::Tag("b".to_owned())),
                span(0..14, InlineStyle::Link("https://a.com/".to_owned())),
                span(18..35, InlineStyle::Link("https://a.com/x#c".to_owned())),
            ]
        ))]
    );
}

#[test]
fn github_alerts_become_alert_blocks() {
    assert_eq!(
        kinds("> [!NOTE]\n> Useful *info*.\n"),
        vec![BlockKind::Alert {
            kind: AlertKind::Note,
            blocks: vec![para(styled(
                "Useful info.",
                vec![span(7..11, InlineStyle::Emphasis)]
            ))],
        }]
    );
    for (marker, kind) in [
        ("[!tip]", AlertKind::Tip),
        ("[!Important]", AlertKind::Important),
        ("[!WARNING]", AlertKind::Warning),
        ("[!caution]  ", AlertKind::Caution),
    ] {
        assert_eq!(
            kinds(&format!("> {marker}\n> body\n")),
            vec![BlockKind::Alert {
                kind,
                blocks: vec![para(plain("body"))],
            }],
            "{marker}"
        );
    }
    // A marker alone is an empty alert.
    assert_eq!(
        kinds("> [!NOTE]\n"),
        vec![BlockKind::Alert {
            kind: AlertKind::Note,
            blocks: vec![],
        }]
    );
    assert_eq!(AlertKind::Important.title(), "Important");
}

#[test]
fn other_quotes_are_not_alerts() {
    // Text after the marker, an unknown kind, a marker not on the first line, no `!`.
    for source in [
        "> [!NOTE] inline\n",
        "> [!DANGER]\n> body\n",
        "> body\n> [!NOTE]\n",
        "> [NOTE]\n> body\n",
    ] {
        assert!(
            matches!(kinds(source).as_slice(), [BlockKind::Quote(_)]),
            "{source}: {:?}",
            kinds(source)
        );
    }
}
