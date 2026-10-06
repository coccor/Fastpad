//! Spans plus the reveal set → Scintilla styling runs (live mode spec §5, §7). Pure, so the
//! window layer only sends the runs.

use super::spans::SpanKind;
use super::styles::style_for;
use std::ops::Range;

pub fn style_runs(
    spans: &[(Range<usize>, SpanKind)],
    range: Range<usize>,
    revealed: &[Range<usize>],
) -> Vec<(usize, u8)> {
    let mut runs: Vec<(usize, u8)> = Vec::new();
    let mut push = |length: usize, style: u8| {
        if length == 0 {
            return;
        }
        match runs.last_mut() {
            Some((last_length, last_style)) if *last_style == style => *last_length += length,
            _ => runs.push((length, style)),
        }
    };
    // Cut points: span edges and revealed-line edges inside `range`.
    let mut cuts = vec![range.start, range.end];
    for (span, _) in spans {
        cuts.extend([span.start, span.end]);
    }
    for line in revealed {
        cuts.extend([line.start, line.end]);
    }
    cuts.retain(|cut| (range.start..=range.end).contains(cut));
    cuts.sort_unstable();
    cuts.dedup();
    let mut span_index = 0;
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        while span_index < spans.len() && spans[span_index].0.end <= start {
            span_index += 1;
        }
        let kind = spans
            .get(span_index)
            .filter(|(span, _)| span.start <= start)
            .map_or(SpanKind::Text, |(_, kind)| *kind);
        let is_revealed = revealed.iter().any(|line| line.start <= start && start < line.end);
        push(end - start, style_for(kind, is_revealed));
    }
    runs
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    use crate::live::spans::SpanKind;
    use crate::live::styles::{BOLD, HIDDEN, MARKER, TEXT};

    #[test]
    fn gaps_are_text_and_runs_cover_the_range() {
        // "a **b** c": Hide 2..4, Bold 4..5, Hide 5..7.
        let spans = [(2..4, SpanKind::Hide), (4..5, SpanKind::Bold), (5..7, SpanKind::Hide)];
        assert_eq!(
            style_runs(&spans, 0..9, &[]),
            vec![(2, TEXT), (2, HIDDEN), (1, BOLD), (2, HIDDEN), (2, TEXT)]
        );
    }

    #[test]
    fn a_revealed_line_uses_source_styles_and_splits_spans() {
        // Line 0 is 0..4 ("**b\n"), line 1 is 4..7: a span crossing the boundary splits.
        let spans = [(0..2, SpanKind::Hide), (2..6, SpanKind::Bold)];
        assert_eq!(
            style_runs(&spans, 0..7, &[0..4]),
            vec![(2, MARKER), (4, BOLD), (1, TEXT)]
        );
    }

    #[test]
    fn a_range_starting_inside_a_span_is_clipped() {
        let spans = [(0..4, SpanKind::Hide)];
        assert_eq!(style_runs(&spans, 2..6, &[]), vec![(2, HIDDEN), (2, TEXT)]);
    }
}
