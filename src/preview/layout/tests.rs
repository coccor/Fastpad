use super::*;
use crate::platform::theme::Theme;
use crate::preview::colors::preview_colors;
use crate::preview::model::parse_document;
use crate::preview::outline;
use crate::preview::render::{TestWindow, create_hwnd_target};
use windows::core::BOOL;

fn with_setup<R>(
    image_size: &dyn Fn(&Path) -> Option<(u32, u32)>,
    document_dir: Option<&Path>,
    dark: bool,
    details: &HashMap<DetailsKey, bool>,
    test: impl FnOnce(&LayoutContext<'_>) -> R,
) -> R {
    let graphics = Graphics::load().unwrap();
    let window = TestWindow::new(400, 300);
    let target = create_hwnd_target(&graphics, window.0, 400, 300, 96).unwrap();
    let brushes = Brushes::create(&target, &preview_colors(Theme::Light, false)).unwrap();
    let fonts = PreviewFonts::from_settings("Segoe UI", "Consolas", 12);
    let context = LayoutContext::new(
        &graphics,
        &brushes,
        &fonts,
        document_dir,
        image_size,
        dark,
        details,
    )
    .unwrap();
    test(&context)
}

fn with_context<R>(
    image_size: &dyn Fn(&Path) -> Option<(u32, u32)>,
    document_dir: Option<&Path>,
    test: impl FnOnce(&LayoutContext<'_>) -> R,
) -> R {
    with_setup(image_size, document_dir, false, &HashMap::new(), test)
}

fn no_images(_: &Path) -> Option<(u32, u32)> {
    None
}

fn laid(context: &LayoutContext<'_>, source: &str, width: f32) -> LaidBlock {
    let (blocks, _) = parse_document(source);
    let outline = outline::build(&blocks);
    layout_block(context, &blocks[0].kind, width, &outline.details[0]).unwrap()
}

fn flatten(ops: &[DrawOp]) -> Vec<&DrawOp> {
    ops.iter()
        .flat_map(|op| match op {
            DrawOp::Scrollable { ops, .. } => {
                let mut nested = vec![op];
                nested.extend(flatten(ops));
                nested
            }
            _ => vec![op],
        })
        .collect()
}

fn first_text_layout(block: &LaidBlock) -> (IDWriteTextLayout, f32) {
    flatten(&block.ops)
        .into_iter()
        .find_map(|op| match op {
            DrawOp::Text { layout, x, .. } => Some((layout.clone(), *x)),
            _ => None,
        })
        .expect("a text op")
}

fn first_glyph_x(block: &LaidBlock) -> f32 {
    let (layout, x) = first_text_layout(block);
    let (mut point_x, mut point_y) = (0.0, 0.0);
    let mut hit = DWRITE_HIT_TEST_METRICS::default();
    unsafe { layout.HitTestTextPosition(0, false, &mut point_x, &mut point_y, &mut hit) }.unwrap();
    x + point_x
}

#[test]
fn headings_are_taller_than_paragraphs_with_the_same_text() {
    with_context(&no_images, None, |context| {
        let heading = laid(context, "# Same\n", 400.0);
        let paragraph = laid(context, "Same\n", 400.0);
        assert!(heading.height > paragraph.height);
        assert_eq!(heading.headings, vec![(0, 0.0)]);
    });
}

#[test]
fn narrow_widths_wrap_paragraphs() {
    with_context(&no_images, None, |context| {
        let text = "word ".repeat(60) + "\n";
        assert!(laid(context, &text, 100.0).height > laid(context, &text, 1000.0).height);
    });
}

#[test]
fn links_get_hit_rectangles() {
    with_context(&no_images, None, |context| {
        let block = laid(context, "go [here](https://x.dev)\n", 400.0);
        assert_eq!(block.targets.len(), 1);
        assert_eq!(block.targets[0].dest(), Some("https://x.dev"));
        assert_eq!(block.targets[0].text, "here");
        let rect = block.targets[0].rects[0];
        assert!(rect.left > 0.0 && rect.width() > 0.0 && rect.height() > 0.0);
        assert!(!block.targets[0].scrolls);
    });
}

#[test]
fn wide_tables_and_long_code_lines_scroll_horizontally() {
    with_context(&no_images, None, |context| {
        let wide = "wide ".repeat(30);
        let table = laid(
            context,
            &format!("| {wide} | {wide} |\n|---|---|\n| a | b |\n"),
            200.0,
        );
        assert!(table.scroll_width > 200.0);
        assert!(
            flatten(&table.ops)
                .iter()
                .any(|op| matches!(op, DrawOp::Scrollable { .. }))
        );
        let code = laid(context, &format!("```\n{wide}{wide}\n```\n"), 200.0);
        assert!(code.scroll_width > 200.0);
    });
}

#[test]
fn scrolled_link_rects_are_shifted_and_clipped() {
    let rects = [RectF::new(300.0, 0.0, 360.0, 20.0)];
    let clip = Some(RectF::new(0.0, 0.0, 200.0, 40.0));
    assert!(visible_link_rects(&rects, true, clip, 0.0).is_empty());
    assert_eq!(
        visible_link_rects(&rects, true, clip, 130.0),
        vec![RectF::new(170.0, 0.0, 200.0, 20.0)]
    );
    assert_eq!(
        visible_link_rects(&rects, true, clip, 200.0),
        vec![RectF::new(100.0, 0.0, 160.0, 20.0)]
    );
    assert_eq!(
        visible_link_rects(&rects, false, None, 500.0),
        rects.to_vec()
    );
}

#[test]
fn wide_table_links_carry_the_table_clip() {
    with_context(&no_images, None, |context| {
        let wide = "wide ".repeat(30);
        let block = laid(
            context,
            &format!("| {wide} | [far](https://far.dev) |\n|---|---|\n| a | b |\n"),
            200.0,
        );
        let link = &block.targets[0];
        assert!(link.scrolls);
        assert_eq!(
            link.clip.map(|clip| (clip.left, clip.right)),
            Some((0.0, 200.0))
        );
        assert!(link.visible_rects(0.0).is_empty());
        let far = link.rects[0].left;
        assert!(!link.visible_rects(far).is_empty());
    });
}

#[test]
fn target_search_sees_links_and_sections_only_from_the_model() {
    let has = |source: &str| {
        let (blocks, _) = parse_document(source);
        block_has_target(&blocks[0].kind)
    };
    assert!(has("see [a](b)\n"));
    assert!(has("# [a](b)\n"));
    assert!(has("- item\n  - [a](b)\n"));
    assert!(has("> quote\n>\n> - [a](b)\n"));
    assert!(has("| h |\n|---|\n| [a](b) |\n"));
    assert!(has("<div>\n\n[a](b)\n\n</div>\n"));
    assert!(has("<details>\n<summary>S</summary>\n\nx\n\n</details>\n"));
    assert!(!has("plain *text*\n"));
    assert!(!has("```\n[a](b)\n```\n"));
    assert!(!has("![alt](a.png)\n"));
}

#[test]
fn task_items_draw_checkboxes() {
    with_context(&no_images, None, |context| {
        let block = laid(context, "- [x] done\n- [ ] todo\n", 400.0);
        let checks = flatten(&block.ops)
            .into_iter()
            .filter_map(|op| match op {
                DrawOp::Checkbox { checked, .. } => Some(*checked),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(checks, vec![true, false]);
    });
}

#[test]
fn images_scale_down_to_width_and_use_placeholders_when_unknown() {
    let dir = PathBuf::from(r"C:\docs");
    let known = |_: &Path| Some((800, 400));
    with_context(&known, Some(&dir), |context| {
        let block = laid(context, "![a](a.png)\n", 400.0);
        assert_eq!(
            block.images[0].path.as_deref(),
            Some(Path::new(r"C:\docs\a.png"))
        );
        assert_eq!(block.images[0].rect.width(), 400.0);
        assert_eq!(block.images[0].rect.height(), 200.0);
    });
    with_context(&no_images, Some(&dir), |context| {
        let block = laid(context, "![a](a.png)\n", 400.0);
        assert!(block.images[0].rect.height() > 0.0);
        assert!(block.images[0].rect.width() <= 400.0);
    });
}

#[test]
fn inline_images_share_a_line_and_wrap_when_they_do_not_fit() {
    let dir = PathBuf::from(r"C:\docs");
    let known = |_: &Path| Some((100, 50));
    with_context(&known, Some(&dir), |context| {
        let source = "![a](a.png) ![b](b.png) ![c](c.png)\n";
        let wide = laid(context, source, 400.0);
        let tops = wide
            .images
            .iter()
            .map(|slot| slot.rect.top)
            .collect::<Vec<_>>();
        assert_eq!(tops.len(), 3);
        assert!(tops.iter().all(|top| *top == tops[0]), "{tops:?}");
        assert!(wide.images[1].rect.left > wide.images[0].rect.right);
        let narrow = laid(context, source, 250.0);
        assert!(narrow.images[2].rect.top > narrow.images[0].rect.top);
        assert_eq!(narrow.images[0].rect.width(), 100.0);
    });
}

#[test]
fn images_without_a_size_are_one_line_chips_labelled_with_alt_text() {
    with_context(&no_images, None, |context| {
        let block = laid(
            context,
            "![build status](https://x.dev/badge.svg) text\n",
            400.0,
        );
        let slot = &block.images[0];
        assert!(slot.path.is_none());
        assert_eq!(slot.rect.height(), context.line_height());
        assert!(slot.rect.width() > 0.0 && slot.rect.width() < 400.0);
        assert!(slot.alt_origin.0 > 0.0);
    });
}

#[test]
fn missing_images_show_their_whole_alt_text_inside_the_placeholder() {
    // Break caught: a placeholder sized by attributes drew its alt text 8 DIPs below the top
    // of a box only one line tall, so the text was clipped halfway down.
    with_context(&no_images, None, |context| {
        for source in [
            "<img src=\"missing.png\" alt=\"FastPad logo\" width=\"160\">\n",
            "<img src=\"missing.png\" alt=\"FastPad logo\" width=\"160\" height=\"24\">\n",
            "<img src=\"missing.png\" alt=\"FastPad logo\" width=\"40\" height=\"40\">\n",
            "![FastPad logo](missing.png)\n",
        ] {
            let block = laid(context, source, 400.0);
            let slot = &block.images[0];
            let alt = metrics(&slot.alt).unwrap();
            assert!(slot.alt_origin.1 >= 0.0, "{source}: {:?}", slot.alt_origin);
            assert!(
                slot.alt_origin.1 + alt.height <= slot.rect.height() + 0.5,
                "{source}: alt text {} tall at {} in a box {} tall",
                alt.height,
                slot.alt_origin.1,
                slot.rect.height()
            );
        }
    });
}

#[test]
fn image_sizes_follow_attributes_then_natural_size_and_never_exceed_the_width() {
    let dir = PathBuf::from(r"C:\docs");
    let known = |_: &Path| Some((800, 400));
    with_context(&known, Some(&dir), |context| {
        let sized = |width, height| {
            let image = ImageRef {
                dest: "a.png".into(),
                width,
                height,
                ..ImageRef::default()
            };
            let placed = context.image_box(&image, 0, 400.0).unwrap();
            (placed.width, placed.height, placed.chip)
        };
        assert_eq!(
            sized(Some(Length::Pixels(96)), Some(Length::Pixels(48))),
            (96.0, 48.0, false)
        );
        assert_eq!(
            sized(Some(Length::Percent(50)), None),
            (200.0, 100.0, false)
        );
        assert_eq!(
            sized(None, Some(Length::Pixels(100))),
            (200.0, 100.0, false)
        );
        assert_eq!(sized(None, None), (400.0, 200.0, false));
        assert_eq!(
            sized(Some(Length::Pixels(1000)), Some(Length::Pixels(100))),
            (400.0, 40.0, false)
        );
    });
    with_context(&no_images, None, |context| {
        let image = ImageRef {
            dest: "https://x.dev/badge.svg".into(),
            alt: "build".into(),
            ..ImageRef::default()
        };
        let placed = context.image_box(&image, 0, 400.0).unwrap();
        assert!(placed.chip && placed.path.is_none());
        assert_eq!(placed.height, context.line_height());
    });
}

#[test]
fn image_only_links_are_named_by_alt_text_then_title_then_destination() {
    with_context(&no_images, None, |context| {
        let name = |source: &str| laid(context, source, 400.0).targets[0].text.clone();
        assert_eq!(name("[![Build](b.png)](https://ci)\n"), "Build");
        assert_eq!(name("[![](b.png \"Status\")](https://ci)\n"), "Status");
        assert_eq!(name("[![](b.png)](https://ci)\n"), "https://ci");
        assert_eq!(name("[see ![Build](b.png)](https://ci)\n"), "see");
    });
}

#[test]
fn pictures_pick_the_source_for_the_colour_scheme() {
    let dir = PathBuf::from(r"C:\docs");
    let source = "<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"dark.png\">\n<source media=\"(prefers-color-scheme: light)\" srcset=\"light.png\">\n<img src=\"plain.png\">\n</picture>\n";
    for (dark, expected) in [(true, "dark.png"), (false, "light.png")] {
        with_setup(&no_images, Some(&dir), dark, &HashMap::new(), |context| {
            let block = laid(context, source, 400.0);
            assert_eq!(
                block.images[0].path.as_deref(),
                Some(dir.join(expected).as_path())
            );
        });
    }
    with_setup(&no_images, Some(&dir), true, &HashMap::new(), |context| {
        let block = laid(
            context,
            "<picture><source media=\"print\" srcset=\"any.png\"><img src=\"plain.png\"></picture>\n",
            400.0,
        );
        assert_eq!(
            block.images[0].path.as_deref(),
            Some(dir.join("any.png").as_path())
        );
    });
}

#[test]
fn alignment_centres_text_and_inherits_through_containers() {
    with_context(&no_images, None, |context| {
        assert!(first_glyph_x(&laid(context, "<p align=\"center\">Hi</p>\n", 400.0)) > 150.0);
        assert!(
            first_glyph_x(&laid(
                context,
                "<div align=\"center\">\n\nHi\n\n</div>\n",
                400.0
            )) > 150.0
        );
        assert!(
            first_glyph_x(&laid(
                context,
                "<div align=\"center\">\n\n<p align=\"left\">Hi</p>\n\n</div>\n",
                400.0
            )) < 10.0
        );
        assert!(first_glyph_x(&laid(context, "Hi\n", 400.0)) < 10.0);
    });
    assert_eq!(
        text_alignment(TextAlign::Justify),
        DWRITE_TEXT_ALIGNMENT_JUSTIFIED
    );
}

#[test]
fn mark_and_keyboard_draw_fills_and_a_border() {
    with_context(&no_images, None, |context| {
        let block = laid(context, "a <mark>b</mark> <kbd>Ctrl</kbd>\n", 400.0);
        let ops = flatten(&block.ops);
        assert!(ops.iter().any(|op| matches!(
            op,
            DrawOp::RoundedFill {
                role: ColorRole::Mark,
                ..
            }
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            DrawOp::RoundedStroke {
                role: ColorRole::KbdBorder,
                ..
            }
        )));
    });
}

#[test]
fn scripts_and_small_text_shrink_and_insertions_are_underlined() {
    with_context(&no_images, None, |context| {
        let block = laid(
            context,
            "x<sup>2</sup> y<sub>3</sub> <small>s</small> <ins>n</ins>\n",
            400.0,
        );
        let (layout, _) = first_text_layout(&block);
        let size_at = |position| {
            let mut size = 0.0;
            unsafe { layout.GetFontSize(position, &mut size, None) }.unwrap();
            size
        };
        let body = context.fonts.body_size;
        assert_eq!(size_at(0), body);
        assert_eq!(size_at(1), body * SCRIPT_SCALE);
        assert_eq!(size_at(4), body * SCRIPT_SCALE);
        assert_eq!(size_at(6), body * SMALL_SCALE);
        let mut underline = BOOL::default();
        unsafe { layout.GetUnderline(8, &mut underline, None) }.unwrap();
        assert!(underline.as_bool());
    });
}

#[test]
fn quotes_draw_a_bar_and_indent_their_content() {
    with_context(&no_images, None, |context| {
        let block = laid(context, "> quoted\n", 400.0);
        assert!(block.ops.iter().any(|op| matches!(
            op,
            DrawOp::Fill {
                role: ColorRole::QuoteBar,
                ..
            }
        )));
        let text_x = block.ops.iter().find_map(|op| match op {
            DrawOp::Text { x, .. } => Some(*x),
            _ => None,
        });
        assert!(text_x.unwrap() > 0.0);
    });
}

#[test]
fn details_start_collapsed_and_open_through_their_state() {
    let source =
        "<details>\n<summary>More</summary>\n\n# Inside\n\n[x](https://x.dev)\n\n</details>\n";
    let key = DetailsKey {
        summary: "More".into(),
        occurrence: 0,
    };
    let kinds = |block: &LaidBlock| {
        block
            .targets
            .iter()
            .map(|target| target.kind.clone())
            .collect::<Vec<_>>()
    };
    let collapsed = with_context(&no_images, None, |context| {
        let block = laid(context, source, 400.0);
        assert_eq!(
            kinds(&block),
            vec![TargetKind::Disclosure {
                key: key.clone(),
                expanded: false
            }]
        );
        assert_eq!(block.targets[0].text, "More");
        assert!(block.headings.is_empty());
        block.height
    });
    let opened = HashMap::from([(key.clone(), true)]);
    let open = with_setup(&no_images, None, false, &opened, |context| {
        let block = laid(context, source, 400.0);
        assert_eq!(
            kinds(&block),
            vec![
                TargetKind::Disclosure {
                    key: key.clone(),
                    expanded: true
                },
                TargetKind::Link("https://x.dev".into()),
            ]
        );
        assert_eq!(block.headings.len(), 1);
        block.height
    });
    assert!(open > collapsed);
}

#[test]
fn headings_after_a_collapsed_section_keep_their_document_index() {
    with_context(&no_images, None, |context| {
        let block = laid(
            context,
            "<div>\n\n<details>\n<summary>S</summary>\n\n# A\n\n</details>\n\n# B\n\n</div>\n",
            400.0,
        );
        assert_eq!(
            block
                .headings
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>(),
            vec![1]
        );
    });
}

#[test]
fn the_preview_font_changes_the_body_text_layout() {
    // Break caught: preview_font read and saved but never reaching the text the preview draws.
    let width_with = |preview_font: &str| {
        let graphics = Graphics::load().unwrap();
        let window = TestWindow::new(400, 300);
        let target = create_hwnd_target(&graphics, window.0, 400, 300, 96).unwrap();
        let brushes = Brushes::create(&target, &preview_colors(Theme::Light, false)).unwrap();
        let fonts = PreviewFonts::from_settings(preview_font, "Consolas", 12);
        let details = HashMap::new();
        let context = LayoutContext::new(
            &graphics, &brushes, &fonts, None, &no_images, false, &details,
        )
        .unwrap();
        let block = laid(
            &context,
            "The quick brown fox jumps over the lazy dog",
            2000.0,
        );
        let (layout, _) = first_text_layout(&block);
        let mut metrics = windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics) }.unwrap();
        metrics.widthIncludingTrailingWhitespace
    };
    assert_ne!(width_with("Segoe UI"), width_with("Times New Roman"));
}
