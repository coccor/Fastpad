use super::{LiveFonts, LiveLayout};
use crate::live::motion::{self, Move};
use crate::live::text::{LiveText, MemoryText};
use crate::preview::dwrite::Graphics;
use std::rc::Rc;

const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps running past the \
                    river, the mill and the old stone bridge until night falls.";

fn layout(width: f32) -> LiveLayout {
    let graphics = Rc::new(Graphics::load().unwrap());
    let fonts = LiveFonts::new(&graphics, "Segoe UI", 16, 15, 11, 0);
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
    let plain = LiveFonts::new(&graphics, "Segoe UI", 16, 15, 10, 0);
    let zoomed = LiveFonts::new(&graphics, "Segoe UI", 16, 15, 10, 5);
    assert_eq!(plain.size, 16.0);
    assert_eq!(plain.line_height, 24.0);
    assert_eq!(zoomed.size, 24.0);
    let tiny = LiveFonts::new(&graphics, "Segoe UI", 16, 15, 10, -20);
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
