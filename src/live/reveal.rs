//! Which lines show source (live mode spec §5): every line holding a caret or touching a
//! selection. A selection that ends at the very start of a line does not touch that line.

use std::collections::BTreeSet;
use std::ops::Range;

pub fn revealed_lines(
    selections: &[Range<usize>],
    line_of: impl Fn(usize) -> usize,
) -> BTreeSet<usize> {
    let mut lines = BTreeSet::new();
    for selection in selections {
        let (start, end) = (
            selection.start.min(selection.end),
            selection.start.max(selection.end),
        );
        let first = line_of(start);
        let last = if end > start && line_of(end) > first && line_of(end - 1) < line_of(end) {
            line_of(end) - 1
        } else {
            line_of(end)
        };
        lines.extend(first..=last.max(first));
    }
    lines
}

pub fn changed_lines(old: &BTreeSet<usize>, new: &BTreeSet<usize>) -> Vec<usize> {
    old.symmetric_difference(new).copied().collect()
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    // Lines of "aa\nbb\ncc\n": 0..3, 3..6, 6..9.
    fn line_of(at: usize) -> usize {
        at / 3
    }

    #[test]
    fn a_caret_reveals_its_line() {
        assert_eq!(revealed_lines(&[4..4], line_of), BTreeSet::from([1]));
    }

    #[test]
    fn a_selection_reveals_every_line_it_touches() {
        assert_eq!(revealed_lines(&[1..7], line_of), BTreeSet::from([0, 1, 2]));
    }

    #[test]
    fn a_selection_ending_at_a_line_start_does_not_reveal_that_line() {
        assert_eq!(revealed_lines(&[0..3], line_of), BTreeSet::from([0]));
    }

    #[test]
    fn multiple_carets_reveal_each_line() {
        assert_eq!(
            revealed_lines(&[0..0, 7..7], line_of),
            BTreeSet::from([0, 2])
        );
    }

    #[test]
    fn changed_lines_is_the_symmetric_difference() {
        let old = BTreeSet::from([1, 2]);
        let new = BTreeSet::from([2, 3]);
        assert_eq!(changed_lines(&old, &new), vec![1, 3]);
    }
}
