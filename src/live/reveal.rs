//! Which source lines show as source (2026-10-18 Live editor design spec §5): every block the
//! selection touches, whole, from the blank line before it to the blank line after it (phase 3
//! review: the caret's block, not just its line). A selection ending at the very start of a line
//! covers the previous line's ending, not a character of that line, so that line's block stays
//! rendered (phase 2 plan, deviation 8). Live has one selection (phase 1 plan, deviation 5).

use std::ops::Range;

/// The lines shown as source for the selection `anchor..caret`: the blocks holding the first
/// and last lines it touches (`block_of` gives a line's block, itself for a blank line), and every
/// line between them.
pub fn revealed_blocks(
    anchor: usize,
    caret: usize,
    line_of: impl Fn(usize) -> usize,
    block_of: impl Fn(usize) -> Range<usize>,
) -> Range<usize> {
    let lines = revealed_lines(anchor, caret, line_of);
    let first = block_of(lines.start);
    let last = block_of(lines.end - 1);
    first.start.min(lines.start)..last.end.max(lines.end)
}

/// The source lines the selection `anchor..caret` (either order) touches.
pub fn revealed_lines(
    anchor: usize,
    caret: usize,
    line_of: impl Fn(usize) -> usize,
) -> Range<usize> {
    let (start, end) = (anchor.min(caret), anchor.max(caret));
    let first = line_of(start);
    let end_line = line_of(end);
    let last = if end > start && end_line > first && line_of(end - 1) < end_line {
        end_line - 1
    } else {
        end_line
    };
    first..last.max(first) + 1
}

/// `revealed` after an edit on `first_line` that added (`lines_delta > 0`) or removed lines.
pub fn shift(revealed: Range<usize>, first_line: usize, lines_delta: isize) -> Range<usize> {
    let move_line = |line: usize| {
        if line <= first_line {
            line
        } else {
            line.saturating_add_signed(lines_delta).max(first_line)
        }
    };
    let start = move_line(revealed.start);
    let end = move_line(revealed.end).max(start + 1);
    start..end
}

/// The lines revealed in exactly one of `old` and `new`: at most two ranges.
pub fn changed(old: Range<usize>, new: Range<usize>) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    for (from, other) in [(&old, &new), (&new, &old)] {
        if from.start < other.start {
            lines.push(from.start..from.end.min(other.start));
        }
        if from.end > other.end {
            lines.push(from.start.max(other.end)..from.end);
        }
    }
    lines.retain(|range| range.start < range.end);
    lines.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in lines {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    // Lines of "aa\nbb\ncc\n": 0..3, 3..6, 6..9.
    fn line_of(at: usize) -> usize {
        at / 3
    }

    #[test]
    fn a_caret_reveals_its_whole_block() {
        // Lines 2..5 are one block; line 6 is blank.
        let block_of = |line: usize| {
            if (2..5).contains(&line) {
                2..5
            } else {
                line..line + 1
            }
        };
        assert_eq!(revealed_blocks(30, 30, |at| at / 10, block_of), 2..5);
        assert_eq!(revealed_blocks(60, 60, |at| at / 10, block_of), 6..7);
        // A selection from the blank line into the block reveals both.
        assert_eq!(revealed_blocks(61, 25, |at| at / 10, block_of), 2..7);
        // One ending at the blank line's start covers only the block.
        assert_eq!(revealed_blocks(60, 25, |at| at / 10, block_of), 2..6);
    }

    #[test]
    fn a_caret_reveals_its_line() {
        assert_eq!(revealed_lines(4, 4, line_of), 1..2);
    }

    #[test]
    fn a_selection_reveals_every_line_it_touches() {
        assert_eq!(revealed_lines(1, 7, line_of), 0..3);
    }

    #[test]
    fn a_selection_ending_at_a_line_start_does_not_reveal_that_line() {
        assert_eq!(revealed_lines(0, 3, line_of), 0..1);
    }

    #[test]
    fn a_backwards_selection_reveals_the_same_lines() {
        assert_eq!(revealed_lines(7, 1, line_of), revealed_lines(1, 7, line_of));
        assert_eq!(revealed_lines(3, 0, line_of), 0..1);
    }

    #[test]
    fn shift_follows_lines_added_above_and_inside() {
        assert_eq!(shift(4..6, 1, 2), 6..8, "two lines added above");
        assert_eq!(shift(4..6, 4, 1), 4..7, "a line added inside");
        assert_eq!(shift(4..6, 1, -2), 2..4, "two lines removed above");
        assert_eq!(shift(4..6, 7, 3), 4..6, "lines added below");
        assert_eq!(shift(4..5, 2, -5), 2..3, "the revealed line removed");
    }

    #[test]
    fn changed_is_the_symmetric_difference() {
        assert_eq!(changed(1..3, 2..4), vec![1..2, 3..4]);
        assert_eq!(changed(1..2, 5..6), vec![1..2, 5..6]);
        assert_eq!(changed(1..5, 2..3), vec![1..2, 3..5]);
        assert_eq!(changed(2..4, 2..4), Vec::<Range<usize>>::new());
        assert_eq!(changed(1..2, 2..3), vec![1..3]);
    }
}
