use super::{LiveFonts, LiveLayout};
use crate::live::motion::{self, Move};
use crate::live::text::{LiveText, MemoryText};
use crate::preview::dwrite::Graphics;
use std::rc::Rc;

const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps running past the \
                    river, the mill and the old stone bridge until night falls.";

fn layout(width: f32) -> LiveLayout {
    let graphics = Rc::new(Graphics::load().unwrap());
    let fonts = LiveFonts::new(&graphics, "Segoe UI", "Consolas", 16, 15, 11, 0);
    LiveLayout::new(graphics, fonts, width).unwrap()
}

fn laid(text: &MemoryText, width: f32) -> LiveLayout {
    let mut layout = layout(width);
    layout.reset(text);
    layout.ensure(text, 0..text.line_count()).unwrap();
    layout
}

/// Every caret position of `text`: the character boundaries and each line's end.
fn positions(text: &MemoryText) -> Vec<usize> {
    let mut all = text
        .text()
        .char_indices()
        .map(|(at, _)| at)
        .chain([text.len()])
        .filter(|&at| {
            let line = text.line_of(at);
            at <= text.line_end(line)
        })
        .collect::<Vec<_>>();
    all.dedup();
    all
}

#[test]
fn fonts_scale_with_the_editor_zoom() {
    let graphics = Graphics::load().unwrap();
    let plain = LiveFonts::new(&graphics, "Segoe UI", "Consolas", 16, 15, 10, 0);
    let zoomed = LiveFonts::new(&graphics, "Segoe UI", "Consolas", 16, 15, 10, 5);
    assert_eq!(plain.size, 16.0);
    assert_eq!(plain.line_height, 24.0);
    assert_eq!(zoomed.size, 24.0);
    let tiny = LiveFonts::new(&graphics, "Segoe UI", "Consolas", 16, 15, 10, -20);
    assert!(tiny.size > 0.0);
}

#[test]
fn a_long_line_wraps_into_rows() {
    let text = MemoryText::new(LONG);
    let mut layout = laid(&text, 200.0);
    let line_height = layout.fonts().line_height;
    let rows = layout.line(&text, 0).unwrap().row_count();
    assert!(rows > 2, "{rows} rows");
    assert_eq!(layout.total_height(), rows as f32 * line_height);
}

#[test]
fn empty_lines_have_one_row() {
    let text = MemoryText::new("a\n\nb\n");
    let mut layout = laid(&text, 300.0);
    let line_height = layout.fonts().line_height;
    for line in 0..text.line_count() {
        let laid = layout.line(&text, line).unwrap();
        assert_eq!(laid.row_count(), 1);
        assert_eq!(laid.height, line_height);
    }
    assert_eq!(layout.line_top(3), 3.0 * line_height);
}

#[test]
fn hit_test_round_trips_every_position_on_a_wrapped_line() {
    let text = MemoryText::new(&format!("{LONG}\nsecond liné\r\n\nend"));
    let mut layout = laid(&text, 180.0);
    for position in positions(&text) {
        let rect = layout.caret_rect(&text, position).unwrap();
        assert!(rect.bottom > rect.top);
        let hit = layout
            .position_at(&text, rect.left + 0.5, (rect.top + rect.bottom) / 2.0)
            .unwrap();
        assert_eq!(hit, position, "at {position}");
    }
}

#[test]
fn astral_chars_hit_test_to_their_start() {
    let text = MemoryText::new("a😀b");
    let mut layout = laid(&text, 300.0);
    for position in [0, 1, 5, 6] {
        let rect = layout.caret_rect(&text, position).unwrap();
        let hit = layout
            .position_at(&text, rect.left + 0.5, rect.top + 1.0)
            .unwrap();
        assert_eq!(hit, position);
    }
    // A byte inside the emoji draws its caret at the emoji's start.
    let inside = layout.caret_rect(&text, 3).unwrap();
    let start = layout.caret_rect(&text, 1).unwrap();
    assert_eq!(inside.left, start.left);
}

#[test]
fn a_click_past_a_rows_end_stays_on_that_row() {
    let text = MemoryText::new(LONG);
    let mut layout = laid(&text, 180.0);
    let line_height = layout.fonts().line_height;
    let hit = layout
        .position_at(&text, 10_000.0, line_height / 2.0)
        .unwrap();
    let below = layout.position_at(&text, 0.0, line_height * 1.5).unwrap();
    assert!(
        hit < below,
        "{hit} should be before the second row's start {below}"
    );
    let past_the_end = layout.position_at(&text, 10_000.0, 10_000.0).unwrap();
    assert_eq!(past_the_end, text.len());
}

#[test]
fn selection_across_lines_has_one_rect_per_row_and_line_ending() {
    let text = MemoryText::new("abc\ndef\nghi");
    let mut layout = laid(&text, 300.0);
    let rects = layout.selection_rects(&text, 1..9, 0..3).unwrap();
    // "bc", line 0's ending, "def", line 1's ending, "g".
    assert_eq!(rects.len(), 5);
    assert!(rects.windows(2).all(|pair| pair[0].top <= pair[1].top));
    let wrapped = MemoryText::new(LONG);
    let mut layout = laid(&wrapped, 180.0);
    let rows = layout.line(&wrapped, 0).unwrap().row_count();
    let rects = layout
        .selection_rects(&wrapped, 0..wrapped.len(), 0..1)
        .unwrap();
    assert_eq!(rects.len(), rows);
    assert!(
        layout
            .selection_rects(&wrapped, 3..3, 0..1)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn selection_rects_skip_lines_outside_the_visible_range() {
    let text = MemoryText::new("abc\ndef\nghi");
    let mut layout = laid(&text, 300.0);
    let rects = layout.selection_rects(&text, 0..11, 1..2).unwrap();
    // "def" and its line ending.
    assert_eq!(rects.len(), 2);
}

#[test]
fn editing_a_line_keeps_the_others_laid_out() {
    let before = MemoryText::new("one\ntwo\nthree");
    let mut layout = laid(&before, 300.0);
    let after = MemoryText::new("one\ntwo\nmore\nthree");
    let changed = layout.edit(&after, 1, 1);
    assert_eq!(changed, 1..3);
    assert_eq!(layout.line_count(), 4);
    assert!(layout.lines.get(0).is_some());
    assert!(layout.lines.get(1).is_none());
    assert!(layout.lines.get(2).is_none());
    assert!(layout.lines.get(3).is_some(), "\"three\" kept its layout");
    let line_height = layout.fonts().line_height;
    assert_eq!(layout.total_height(), 4.0 * line_height);
    let fewer = MemoryText::new("one\nthree");
    assert_eq!(layout.edit(&fewer, 0, -2), 0..1);
    assert_eq!(layout.line_count(), 2);
    assert!(layout.lines.get(1).is_some());
    layout.ensure(&fewer, 0..2).unwrap();
    assert_eq!(layout.total_height(), 2.0 * line_height);
}

#[test]
fn down_moves_to_the_next_row_of_a_wrapped_line() {
    let text = MemoryText::new(&format!("{LONG}\nnext"));
    let mut layout = laid(&text, 180.0);
    let first_row_end = {
        let mut geometry = layout.geometry(&text, 100.0);
        motion::apply(&mut geometry, 0, None, Move::RowEnd).caret
    };
    let mut geometry = layout.geometry(&text, 100.0);
    let down = motion::apply(&mut geometry, 2, None, Move::Down);
    assert_eq!(text.line_of(down.caret), 0, "still on the long line");
    assert!(down.caret > first_row_end);
    let back = motion::apply(&mut geometry, down.caret, down.goal_x, Move::Up);
    assert_eq!(back.caret, 2);
    let end = motion::apply(&mut geometry, LONG.len() - 1, None, Move::Down);
    assert!(text.line_of(end.caret) == 1);
}

mod styled {
    use super::{LONG, layout};
    use crate::live::blocks::LiveDocument;
    use crate::live::offsets::Show;
    use crate::live::styles::{Styled, StyledLines, no_images};
    use crate::live::text::{LiveText, MemoryText};
    use std::ops::Range;

    fn styled<'a>(
        text: &'a MemoryText,
        document: &'a LiveDocument,
        revealed: Range<usize>,
    ) -> Styled<'a, MemoryText> {
        Styled {
            text,
            document,
            revealed,
            images: &no_images,
        }
    }

    /// No line revealed.
    const NONE: Range<usize> = usize::MAX - 1..usize::MAX;

    fn shown(text: &impl StyledLines, layout: &mut super::LiveLayout, line: usize) -> String {
        String::from_utf16_lossy(layout.line(text, line).unwrap().offsets.layout_text())
    }

    #[test]
    fn a_revealed_heading_keeps_its_size() {
        let text = MemoryText::new("# Title\nbody");
        let document = LiveDocument::parse(text.text());
        let mut rendered = layout(400.0);
        let lines = styled(&text, &document, NONE);
        rendered.reset(&lines);
        let rendered_height = rendered.line(&lines, 0).unwrap().height;
        assert_eq!(shown(&lines, &mut rendered, 0), "Title");
        let mut revealed = layout(400.0);
        let lines = styled(&text, &document, 0..1);
        revealed.reset(&lines);
        let revealed_height = revealed.line(&lines, 0).unwrap().height;
        assert_eq!(shown(&lines, &mut revealed, 0), "# Title");
        assert_eq!(rendered_height, revealed_height);
        let body = revealed.line(&lines, 1).unwrap().height;
        assert!(revealed_height > body * 1.5, "{revealed_height} vs {body}");
    }

    #[test]
    fn hidden_markers_leave_the_layout_text() {
        let text = MemoryText::new("a **b** [c](u) `d` ~~e~~ [[N|f]]");
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE);
        let mut layout = layout(600.0);
        layout.reset(&lines);
        assert_eq!(shown(&lines, &mut layout, 0), "a b c d e f");
    }

    /// The byte ranges a rendered line hides.
    fn hidden(lines: &impl StyledLines, line: usize) -> Vec<Range<usize>> {
        lines
            .line_style(line)
            .pieces
            .into_iter()
            .filter(|(_, show)| *show == Show::Hidden)
            .map(|(range, _)| range)
            .collect()
    }

    #[test]
    fn hit_test_on_a_rendered_line_never_lands_in_a_marker() {
        let text = MemoryText::new("a **bold** [link](u) end");
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE);
        let mut layout = layout(600.0);
        layout.reset(&lines);
        let mid = layout.fonts().line_height / 2.0;
        let markers = hidden(&lines, 0);
        assert!(!markers.is_empty());
        for x in 0..400 {
            let hit = layout.position_at(&lines, x as f32, mid).unwrap();
            for marker in &markers {
                assert!(
                    hit <= marker.start || hit >= marker.end,
                    "x {x}: {hit} inside {marker:?}"
                );
            }
        }
        // The right half of the last bold letter: before the closing markers.
        let d = layout.caret_rect(&lines, 7).unwrap();
        let after_d = layout.caret_rect(&lines, 10).unwrap();
        let x = d.left + (after_d.left - d.left) * 0.75;
        assert_eq!(layout.position_at(&lines, x, mid).unwrap(), 8);
    }

    #[test]
    fn caret_rects_on_a_rendered_line_round_trip() {
        let text = MemoryText::new("x **bold** and [a link](u) 😀 end");
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE);
        let mut layout = layout(800.0);
        layout.reset(&lines);
        let markers = hidden(&lines, 0);
        let starts = text
            .text()
            .char_indices()
            .map(|(at, _)| at)
            .filter(|at| markers.iter().all(|marker| !marker.contains(at)))
            .collect::<Vec<_>>();
        for position in starts {
            let rect = layout.caret_rect(&lines, position).unwrap();
            let hit = layout
                .position_at(&lines, rect.left + 0.5, (rect.top + rect.bottom) / 2.0)
                .unwrap();
            assert_eq!(hit, position, "at {position}");
        }
    }

    #[test]
    fn restyle_keeps_lines_whose_style_did_not_change() {
        let text = MemoryText::new("# A\n**b**\nplain\n");
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, 2..3);
        let mut layout = layout(400.0);
        layout.reset(&lines);
        layout.ensure(&lines, 0..text.line_count()).unwrap();
        assert!(layout.restyle(&lines, 0..text.line_count()).is_empty());
    }

    #[test]
    fn restyle_drops_a_line_whose_reveal_changed() {
        let text = MemoryText::new("# A\n**b**\nplain\n");
        let document = LiveDocument::parse(text.text());
        let mut layout = layout(400.0);
        let before = styled(&text, &document, 2..3);
        layout.reset(&before);
        layout.ensure(&before, 0..text.line_count()).unwrap();
        let after = styled(&text, &document, 1..2);
        assert_eq!(layout.restyle(&after, 0..text.line_count()), vec![1, 2]);
        assert!(layout.laid_line(0).is_some(), "the heading kept its layout");
    }

    #[test]
    fn inline_code_uses_the_code_family() {
        let text = MemoryText::new("a `code` b");
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE);
        let mut layout = layout(400.0);
        layout.reset(&lines);
        let laid = layout.line(&lines, 0).unwrap();
        assert_eq!(laid.backgrounds.len(), 1);
        let family = |position: u32| {
            let mut length = 0_u32;
            unsafe {
                laid.layout
                    .GetFontFamilyNameLength(position, &mut length, None)
            }
            .unwrap();
            let mut name = vec![0u16; length as usize + 1];
            unsafe { laid.layout.GetFontFamilyName(position, &mut name, None) }.unwrap();
            String::from_utf16_lossy(&name[..length as usize])
        };
        assert_eq!(family(3), "Consolas");
        assert_eq!(family(0), "Segoe UI");
    }

    #[test]
    fn a_long_rendered_heading_wraps_at_its_own_size() {
        let source = format!("# {LONG}");
        let text = MemoryText::new(&source);
        let document = LiveDocument::parse(text.text());
        let mut rendered = layout(300.0);
        let lines = styled(&text, &document, NONE);
        rendered.reset(&lines);
        let heading_rows = rendered.line(&lines, 0).unwrap().row_count();
        // The same words as a paragraph wrap into fewer rows.
        let plain = MemoryText::new(LONG);
        let document = LiveDocument::parse(plain.text());
        let mut paragraph = layout(300.0);
        let lines = styled(&plain, &document, NONE);
        paragraph.reset(&lines);
        let paragraph_rows = paragraph.line(&lines, 0).unwrap().row_count();
        assert!(
            heading_rows > paragraph_rows,
            "{heading_rows} vs {paragraph_rows}"
        );
    }
}

mod blocks {
    use super::{LONG, layout};
    use crate::live::blocks::LiveDocument;
    use crate::live::layout::{Decoration, LineLayout, LiveLayout};
    use crate::live::styles::{ImageInfo, Paint, Styled, StyledLines, no_images};
    use crate::live::text::MemoryText;
    use std::ops::Range;
    use std::path::PathBuf;

    /// No line revealed.
    const NONE: Range<usize> = usize::MAX - 1..usize::MAX;

    fn styled<'a>(
        text: &'a MemoryText,
        document: &'a LiveDocument,
        revealed: Range<usize>,
        images: &'a dyn Fn(&str) -> ImageInfo,
    ) -> Styled<'a, MemoryText> {
        Styled {
            text,
            document,
            revealed,
            images,
        }
    }

    /// `source` laid out at `width`, with `line` laid out; returns the layout and the text size.
    fn laid_out(
        source: &str,
        width: f32,
        line: usize,
        revealed: Range<usize>,
    ) -> (LiveLayout, f32) {
        let text = MemoryText::new(source);
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, revealed, &no_images);
        let mut layout = layout(width);
        layout.reset(&lines);
        layout.ensure(&lines, line..line + 1).unwrap();
        let em = layout.fonts().size;
        (layout, em)
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }

    fn line(layout: &LiveLayout, line: usize) -> &LineLayout {
        layout.laid_line(line).unwrap()
    }

    #[test]
    fn an_item_is_laid_out_at_its_indent() {
        let text = MemoryText::new("- item\n\nplain");
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE, &no_images);
        let mut layout = layout(400.0);
        layout.reset(&lines);
        let em = layout.fonts().size;
        let laid = layout.line(&lines, 0).unwrap();
        assert!(near(laid.indent, em * 1.5), "{}", laid.indent);
        assert_eq!(String::from_utf16_lossy(laid.offsets.layout_text()), "item");
        let bullet = laid
            .decorations
            .iter()
            .find_map(|decoration| match decoration {
                Decoration::Bullet { center, depth, .. } => Some((*center, *depth)),
                _ => None,
            })
            .unwrap();
        assert!(bullet.0.0 < em * 1.5 && bullet.1 == 1, "{bullet:?}");
        let caret = layout.caret_rect(&lines, 2).unwrap();
        assert!(near(caret.left, em * 1.5), "{}", caret.left);
        assert_eq!(layout.line(&lines, 2).unwrap().indent, 0.0);
    }

    #[test]
    fn hits_and_carets_round_trip_on_an_indented_line() {
        let source = format!("> - {LONG}");
        let text = MemoryText::new(&source);
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE, &no_images);
        let mut layout = layout(260.0);
        layout.reset(&lines);
        assert!(layout.line(&lines, 0).unwrap().row_count() > 2);
        for (position, _) in source.char_indices().filter(|(at, _)| *at >= 4) {
            let rect = layout.caret_rect(&lines, position).unwrap();
            let hit = layout
                .position_at(&lines, rect.left + 0.5, (rect.top + rect.bottom) / 2.0)
                .unwrap();
            assert_eq!(hit, position, "at {position}");
        }
    }

    #[test]
    fn wrapped_item_rows_start_at_the_indent() {
        let source = format!("- {LONG}");
        let text = MemoryText::new(&source);
        let document = LiveDocument::parse(text.text());
        let lines = styled(&text, &document, NONE, &no_images);
        let mut layout = layout(260.0);
        layout.reset(&lines);
        let em = layout.fonts().size;
        let lefts = (2..source.len())
            .map(|position| layout.caret_rect(&lines, position).unwrap().left)
            .collect::<Vec<_>>();
        let leftmost = lefts.iter().copied().fold(f32::MAX, f32::min);
        assert!(near(leftmost, em * 1.5), "{leftmost}");
        // Several rows start there.
        assert!(lefts.iter().filter(|left| near(**left, leftmost)).count() > 2);
    }

    #[test]
    fn quote_bars_cover_the_line_height() {
        let (layout, em) = laid_out(&format!("> {LONG}"), 260.0, 0, NONE);
        let laid = line(&layout, 0);
        assert!(laid.row_count() > 1);
        assert!(laid.decorations.iter().any(|decoration| matches!(
            decoration,
            Decoration::Fill { rect, paint: Paint::QuoteBar }
                if rect.top == 0.0 && near(rect.bottom, laid.height) && near(rect.right, em * 0.25)
        )));
    }

    #[test]
    fn nested_quote_and_list_indents_add_up() {
        let (layout, em) = laid_out("> 1. x", 400.0, 0, NONE);
        assert!(near(line(&layout, 0).indent, em * 3.0));
        let number = line(&layout, 0)
            .decorations
            .iter()
            .find_map(|decoration| match decoration {
                Decoration::Number { rect, text } => Some((near(rect.left, em), text.clone())),
                _ => None,
            });
        assert_eq!(number, Some((true, "1.".to_owned())));
        let (layout, em) = laid_out("- a\n  - b", 400.0, 1, NONE);
        assert!(near(line(&layout, 1).indent, em * 3.0));
    }

    #[test]
    fn a_task_draws_a_checkbox_for_its_inner_byte() {
        let (layout, _) = laid_out("- [x] done", 400.0, 0, NONE);
        let laid = line(&layout, 0);
        assert!(laid.decorations.iter().any(|decoration| matches!(
            decoration,
            Decoration::Checkbox { checked: true, inner: 3, rect } if rect.right < laid.indent
        )));
        assert!(
            !laid
                .decorations
                .iter()
                .any(|decoration| matches!(decoration, Decoration::Bullet { .. }))
        );
    }

    #[test]
    fn a_revealed_item_has_no_indent() {
        let (layout, _) = laid_out("> - item", 400.0, 0, 0..1);
        let laid = line(&layout, 0);
        assert_eq!(laid.indent, 0.0);
        assert!(laid.decorations.is_empty());
        assert_eq!(
            String::from_utf16_lossy(laid.offsets.layout_text()),
            "> - item"
        );
    }

    #[test]
    fn code_lines_use_the_code_font_and_fill_the_width() {
        let (layout, _) = laid_out("```\nlet x;\n```", 400.0, 1, NONE);
        let laid = line(&layout, 1);
        assert!(laid.decorations.iter().any(|decoration| matches!(
            decoration,
            Decoration::Fill { rect, paint: Paint::CodeBackground }
                if rect.left == 0.0 && rect.right == f32::MAX && near(rect.bottom, laid.height)
        )));
        let mut length = 0_u32;
        unsafe { laid.layout.GetFontFamilyNameLength(0, &mut length, None) }.unwrap();
        let mut name = vec![0u16; length as usize + 1];
        unsafe { laid.layout.GetFontFamilyName(0, &mut name, None) }.unwrap();
        assert_eq!(
            String::from_utf16_lossy(&name[..length as usize]),
            "Consolas"
        );
        // The fences keep a code row too.
        let (layout, _) = laid_out("```\nlet x;\n```", 400.0, 2, NONE);
        assert!(
            line(&layout, 2)
                .decorations
                .iter()
                .any(|decoration| matches!(decoration, Decoration::Fill { .. }))
        );
    }

    #[test]
    fn a_rule_is_one_row_with_a_line() {
        let (layout, _) = laid_out("a\n\n---", 400.0, 2, NONE);
        let laid = line(&layout, 2);
        assert_eq!(laid.row_count(), 1);
        assert!(laid.offsets.layout_text().is_empty());
        assert!(laid.decorations.iter().any(|decoration| matches!(
            decoration,
            Decoration::Line { from, paint: Paint::Rule, .. } if near(from.1, laid.height / 2.0)
        )));
    }

    /// An image line whose picture is `size` pixels, at `width`: its height, its image
    /// decoration and the text size.
    fn image_line(size: (u32, u32), width: f32) -> (f32, Option<Decoration>, f32) {
        let text = MemoryText::new("![cat](c.png)");
        let document = LiveDocument::parse(text.text());
        let images = move |_: &str| ImageInfo::Ready {
            path: PathBuf::from(r"C:\c.png"),
            size,
        };
        let lines = styled(&text, &document, NONE, &images);
        let mut layout = layout(width);
        layout.reset(&lines);
        let em = layout.fonts().size;
        let laid = layout.line(&lines, 0).unwrap();
        let image = laid
            .decorations
            .iter()
            .find(|decoration| matches!(decoration, Decoration::Image { .. }))
            .cloned();
        (laid.height, image, em)
    }

    #[test]
    fn a_ready_image_line_is_as_tall_as_its_scaled_image() {
        let (height, image, em) = image_line((200, 100), 600.0);
        let Some(Decoration::Image { rect, path }) = image else {
            panic!("{image:?}");
        };
        assert_eq!(path, PathBuf::from(r"C:\c.png"));
        assert!(near(rect.right - rect.left, 200.0) && near(rect.bottom - rect.top, 100.0));
        assert!(near(height, 100.0 + em * 0.5), "{height}");
        // Wider than the text: scaled to its width, the aspect kept.
        let (_, image, _) = image_line((800, 400), 400.0);
        let Some(Decoration::Image { rect, .. }) = image else {
            panic!("{image:?}");
        };
        assert!(near(rect.right - rect.left, 400.0) && near(rect.bottom - rect.top, 200.0));
    }

    #[test]
    fn a_tall_image_is_capped_at_300_dips_times_zoom() {
        let (_, image, _) = image_line((100, 900), 600.0);
        let Some(Decoration::Image { rect, .. }) = image else {
            panic!("{image:?}");
        };
        assert!(near(rect.bottom - rect.top, 300.0));
        assert!(near(rect.right - rect.left, 100.0 / 3.0));
    }

    #[test]
    fn a_pending_image_line_shows_its_alt_text() {
        let text = MemoryText::new("![cat](c.png)");
        let document = LiveDocument::parse(text.text());
        let images = |_: &str| ImageInfo::Pending(PathBuf::from(r"C:\c.png"));
        let lines = styled(&text, &document, NONE, &images);
        let mut layout = layout(400.0);
        layout.reset(&lines);
        let laid = layout.line(&lines, 0).unwrap();
        assert_eq!(String::from_utf16_lossy(laid.offsets.layout_text()), "cat");
        assert!(laid.decorations.is_empty());
        // It keeps its image style, so it is laid out again once the image is ready.
        assert_eq!(laid.style, lines.line_style(0));
    }
}

mod tables {
    use super::layout;
    use crate::live::blocks::LiveDocument;
    use crate::live::layout::{Decoration, LiveLayout};
    use crate::live::styles::{Paint, Styled, no_images};
    use crate::live::text::{LiveText, MemoryText};
    use std::ops::Range;

    /// No line revealed.
    const NONE: Range<usize> = usize::MAX - 1..usize::MAX;

    /// `source`, its document and the layout at `width` with every line laid out, given to
    /// `check` with the styled lines.
    fn with_table(
        source: &str,
        width: f32,
        revealed: Range<usize>,
        check: impl FnOnce(&mut LiveLayout, &Styled<'_, MemoryText>),
    ) {
        let text = MemoryText::new(source);
        let document = LiveDocument::parse(text.text());
        let lines = Styled {
            text: &text,
            document: &document,
            revealed,
            images: &no_images,
        };
        let mut layout = layout(width);
        layout.reset(&lines);
        layout.ensure(&lines, 0..text.line_count()).unwrap();
        check(&mut layout, &lines);
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }

    /// The x of the caret at the first byte of `needle` on `line`.
    fn x_of(
        layout: &mut LiveLayout,
        lines: &Styled<'_, MemoryText>,
        line: usize,
        needle: &str,
    ) -> f32 {
        let start = lines.line_start(line);
        let at = start + lines.line(line).find(needle).unwrap();
        layout.caret_rect(lines, at).unwrap().left
    }

    #[test]
    fn cells_line_up_across_rows() {
        let source = "| a | bbbbbb | x |\n|---|---|--:|\n| cccccc | d | yyyy |\n";
        with_table(source, 600.0, NONE, |layout, lines| {
            assert!(near(
                x_of(layout, lines, 0, "bbb"),
                x_of(layout, lines, 2, "d")
            ));
            assert!(near(
                x_of(layout, lines, 0, "a"),
                x_of(layout, lines, 2, "ccc")
            ));
            // The third column is right-aligned: its cells end together.
            let end = |layout: &mut LiveLayout, line: usize, cell: &str| {
                let at = lines.line_start(line) + lines.line(line).find(cell).unwrap() + cell.len();
                layout.caret_rect(lines, at).unwrap().left
            };
            assert!(near(end(layout, 0, "x"), end(layout, 2, "yyyy")));
            let header = layout.laid_line(0).unwrap();
            assert_eq!(header.columns.len(), 3);
            assert!(header.decorations.iter().any(|decoration| matches!(
                decoration,
                Decoration::Fill {
                    paint: Paint::CodeBackground,
                    ..
                }
            )));
            let delimiter = layout.laid_line(1).unwrap();
            assert!(delimiter.height < header.height);
            assert_eq!(
                delimiter
                    .decorations
                    .iter()
                    .filter(|decoration| matches!(decoration, Decoration::Line { .. }))
                    .count(),
                4
            );
        });
    }

    /// Lays `before` out, then restyles every line for `after` (same line count); returns the
    /// lines dropped and how many rows were measured for it.
    fn restyled(before: &str, after: &str) -> (Vec<usize>, usize, f32, f32) {
        let old_text = MemoryText::new(before);
        let old_document = LiveDocument::parse(old_text.text());
        let old = Styled {
            text: &old_text,
            document: &old_document,
            revealed: NONE,
            images: &no_images,
        };
        let mut layout = layout(600.0);
        layout.reset(&old);
        layout.ensure(&old, 0..old_text.line_count()).unwrap();
        let x_before = x_of(&mut layout, &old, 2, "d");
        let new_text = MemoryText::new(after);
        let new_document = LiveDocument::parse(new_text.text());
        let new = Styled {
            text: &new_text,
            document: &new_document,
            revealed: NONE,
            images: &no_images,
        };
        let (_, measured) = layout.counts.get();
        let dropped = layout.restyle(&new, 0..new_text.line_count());
        let (_, measured_after) = layout.counts.get();
        layout.ensure(&new, 0..new_text.line_count()).unwrap();
        let x_after = x_of(&mut layout, &new, 2, "d");
        (dropped, measured_after - measured, x_before, x_after)
    }

    #[test]
    fn widening_a_cell_moves_its_column_in_every_row() {
        let (dropped, measured, before, after) = restyled(
            "| a | b |\n|---|---|\n| c | d |\n",
            "| aaaaaaaaaaaa | b |\n|---|---|\n| c | d |\n",
        );
        assert_eq!(dropped, vec![0, 1, 2]);
        assert_eq!(measured, 1);
        assert!(after > before + 20.0, "{before} -> {after}");
    }

    #[test]
    fn typing_in_a_cell_that_stays_narrow_lays_out_one_row() {
        let (dropped, measured, before, after) = restyled(
            "| aaaaaaaa | b |\n|---|---|\n| c | d |\n| e | f |\n",
            "| aaaaaaaa | b |\n|---|---|\n| cc | d |\n| e | f |\n",
        );
        assert_eq!(dropped, vec![2]);
        assert_eq!(measured, 1);
        assert!(near(before, after));
    }

    #[test]
    fn a_table_wider_than_the_view_shows_as_source() {
        let row = format!("| {} | {} |", "w".repeat(60), "v".repeat(60));
        let source = format!("{row}\n|---|---|\n| a | b |\n");
        with_table(&source, 300.0, NONE, |layout, _| {
            let laid = layout.laid_line(0).unwrap();
            assert_eq!(String::from_utf16_lossy(laid.offsets.layout_text()), row);
            assert!(!laid.columns.is_empty());
            assert!(laid.decorations.is_empty());
        });
    }

    #[test]
    fn a_revealed_row_is_source_and_the_rest_keep_the_grid() {
        let source = "| a | b |\n|---|---|\n| c | d |\n";
        with_table(source, 600.0, 2..3, |layout, _| {
            let revealed = layout.laid_line(2).unwrap();
            assert_eq!(
                String::from_utf16_lossy(revealed.offsets.layout_text()),
                "| c | d |"
            );
            assert!(revealed.columns.is_empty());
            let header = layout.laid_line(0).unwrap();
            assert_eq!(header.offsets.objects().len(), 3);
            assert_eq!(header.columns.len(), 2);
        });
    }

    #[test]
    fn hits_on_a_spacer_land_on_a_cell_edge() {
        let source = "| a | bbbbbbbb |\n|---|---|\n| cccccccccc | d |\n";
        with_table(source, 600.0, NONE, |layout, lines| {
            // Between the end of `a` and the start of `bbbbbbbb`: inside the spacer.
            let a = x_of(layout, lines, 0, "a");
            let b = x_of(layout, lines, 0, "bbb");
            let top = layout.line_top(0);
            let hit = layout.position_at(lines, (a + b) / 2.0, top + 2.0).unwrap();
            assert!(hit == 4 || hit == 5, "{hit}");
        });
    }

    #[test]
    fn changing_the_width_remeasures() {
        let source = "| a | b |\n|---|---|\n| c | d |\n";
        with_table(source, 600.0, NONE, |layout, lines| {
            let (_, measured) = layout.counts.get();
            assert_eq!(measured, 2);
            layout.set_width(lines, 500.0);
            layout.ensure(lines, 0..3).unwrap();
            let (_, measured) = layout.counts.get();
            assert_eq!(measured, 4);
        });
    }
}
