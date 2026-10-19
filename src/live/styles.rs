//! A source line's spans and its revealed flag → how it is laid out (phase 2 plan, Task 3 and
//! Decisions 3 and 4). Pure: the effect table is data, tested without DirectWrite.
//!
//! A rendered line hides its markup and gets real weights, styles, sizes and fonts. A revealed
//! line (the caret's) changes colour only, so typing markers on it never re-wraps it.

use super::blocks::LiveDocument;
use super::spans::{Span, SpanKind};
use super::text::LiveText;
use crate::preview::incremental::SourceText;
use std::borrow::Cow;
use std::ops::Range;

/// A colour role the painter keeps a brush for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Paint {
    Text,
    Heading,
    Emphasis,
    Marker,
    Link,
    Code,
    CodeBackground,
    Dim,
}

impl Paint {
    /// When two runs colour the same bytes, the higher rank is applied last and shows.
    const fn rank(self) -> u8 {
        match self {
            Self::Text | Self::CodeBackground => 0,
            Self::Emphasis => 1,
            Self::Marker => 2,
            Self::Heading => 3,
            Self::Code => 4,
            Self::Link => 5,
            Self::Dim => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Bold,
    Italic,
    Strike,
    Underline,
    /// The code font at the code size.
    CodeFont,
    Color(Paint),
    Background(Paint),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineStyle {
    pub revealed: bool,
    /// The line's heading level when rendered; `None` on revealed lines.
    pub heading: Option<u8>,
    /// Shown/hidden pieces covering the line, for `OffsetMap::new`.
    pub pieces: Vec<(Range<usize>, bool)>,
    /// Line-relative byte ranges and their span kinds; `Hidden` only on revealed lines.
    pub runs: Vec<(Range<usize>, SpanKind)>,
}

impl LineStyle {
    /// A line with no styling: shown in full.
    pub fn plain(len: usize) -> Self {
        Self {
            pieces: vec![(0..len, true)],
            ..Self::default()
        }
    }

    /// The colour runs, lowest rank first, so that applying them in order leaves the highest
    /// rank showing where they overlap (Dim over everything).
    pub fn colour_runs(&self) -> Vec<(Range<usize>, Paint)> {
        let mut colours = self
            .runs
            .iter()
            .flat_map(|(range, kind)| {
                effects(*kind, self.revealed)
                    .iter()
                    .filter_map(move |effect| match effect {
                        Effect::Color(paint) => Some((range.clone(), *paint)),
                        _ => None,
                    })
            })
            .collect::<Vec<_>>();
        colours.sort_by_key(|(_, paint)| paint.rank());
        colours
    }
}

/// The effects `kind` has on a rendered line, or on a revealed one (colour only).
pub fn effects(kind: SpanKind, revealed: bool) -> &'static [Effect] {
    use Effect::*;
    if revealed {
        return match kind {
            SpanKind::Hidden => &[Color(Paint::Marker)],
            SpanKind::Heading(_) => &[Color(Paint::Heading)],
            SpanKind::Bold | SpanKind::Italic => &[Color(Paint::Emphasis)],
            SpanKind::Strike | SpanKind::Dim => &[Color(Paint::Dim)],
            SpanKind::Code => &[Color(Paint::Code), Background(Paint::CodeBackground)],
            SpanKind::Link => &[Color(Paint::Link)],
        };
    }
    match kind {
        SpanKind::Hidden => &[],
        // Size and weight come from the line's heading level.
        SpanKind::Heading(_) => &[Color(Paint::Heading)],
        SpanKind::Bold => &[Bold],
        SpanKind::Italic => &[Italic],
        SpanKind::Strike => &[Strike],
        SpanKind::Code => &[
            CodeFont,
            Color(Paint::Code),
            Background(Paint::CodeBackground),
        ],
        SpanKind::Link => &[Underline, Color(Paint::Link)],
        SpanKind::Dim => &[Color(Paint::Dim)],
    }
}

/// The style of the line whose bytes (line ending excluded) are `line`, from the absolute
/// `spans` overlapping it.
pub fn line_style(spans: &[Span], line: Range<usize>, revealed: bool) -> LineStyle {
    let len = line.end - line.start;
    let relative = spans
        .iter()
        .filter(|span| span.range.end > line.start && span.range.start < line.end)
        .map(|span| {
            let start = span.range.start.max(line.start) - line.start;
            let end = span.range.end.min(line.end) - line.start;
            (start..end, span.kind)
        })
        .filter(|(range, _)| range.start < range.end)
        .collect::<Vec<_>>();
    if revealed {
        return LineStyle {
            revealed: true,
            heading: None,
            pieces: vec![(0..len, true)],
            runs: relative,
        };
    }
    let mut hidden = relative
        .iter()
        .filter(|(_, kind)| *kind == SpanKind::Hidden)
        .map(|(range, _)| range.clone())
        .collect::<Vec<_>>();
    hidden.sort_by_key(|range| range.start);
    let mut pieces = Vec::new();
    let mut at = 0;
    for range in hidden {
        if range.start > at {
            pieces.push((at..range.start, true));
        }
        if range.end > at {
            pieces.push((range.start.max(at)..range.end, false));
            at = range.end;
        }
    }
    if at < len || pieces.is_empty() {
        pieces.push((at..len, true));
    }
    // A heading line: a heading span covers the line's first shown byte.
    let first_shown = pieces
        .iter()
        .find(|(range, shown)| *shown && range.start < range.end)
        .map(|(range, _)| range.start);
    let heading = first_shown.and_then(|at| {
        relative.iter().find_map(|(range, kind)| match kind {
            SpanKind::Heading(level) if range.contains(&at) => Some(*level),
            _ => None,
        })
    });
    LineStyle {
        revealed: false,
        heading,
        pieces,
        runs: relative
            .into_iter()
            .filter(|(_, kind)| *kind != SpanKind::Hidden)
            .collect(),
    }
}

/// Lines with their styles: what `LiveLayout` lays out.
pub trait StyledLines: LiveText {
    fn line_style(&self, line: usize) -> LineStyle;
}

/// The document text, its spans and the revealed lines.
pub struct Styled<'a, T: LiveText> {
    pub text: &'a T,
    pub document: &'a LiveDocument,
    pub revealed: Range<usize>,
}

impl<T: LiveText> LiveText for Styled<'_, T> {
    fn len(&self) -> usize {
        LiveText::len(self.text)
    }

    fn line_count(&self) -> usize {
        self.text.line_count()
    }

    fn line_of(&self, position: usize) -> usize {
        LiveText::line_of(self.text, position)
    }

    fn line_start(&self, line: usize) -> usize {
        self.text.line_start(line)
    }

    fn line_end(&self, line: usize) -> usize {
        self.text.line_end(line)
    }

    fn line(&self, line: usize) -> Cow<'_, str> {
        self.text.line(line)
    }

    fn selection(&self) -> (usize, usize) {
        self.text.selection()
    }

    fn set_selection(&self, anchor: usize, caret: usize) {
        self.text.set_selection(anchor, caret);
    }

    fn word_at(&self, position: usize) -> Range<usize> {
        self.text.word_at(position)
    }
}

impl<T: LiveText + SourceText> StyledLines for Styled<'_, T> {
    fn line_style(&self, line: usize) -> LineStyle {
        let bytes = self.text.line_start(line)..self.text.line_end(line);
        let spans = self.document.spans_in(self.text, bytes.clone());
        line_style(&spans, bytes, self.revealed.contains(&line))
    }
}

/// Plain text, until the view styles its document (phase 2 plan, Task 6 replaces this).
impl StyledLines for super::text::EditorText<'_> {
    fn line_style(&self, line: usize) -> LineStyle {
        LineStyle::plain(self.line_end(line) - self.line_start(line))
    }
}

/// Plain text, for the layout tests that need no Markdown.
#[cfg(test)]
impl StyledLines for super::text::MemoryText {
    fn line_style(&self, line: usize) -> LineStyle {
        LineStyle::plain(self.line_end(line) - self.line_start(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::spans::parse_block;

    fn spans_of(text: &str) -> Vec<Span> {
        parse_block(text, &|_| None).spans
    }

    fn shown(style: &LineStyle, line: &str) -> String {
        style
            .pieces
            .iter()
            .filter(|(_, shown)| *shown)
            .map(|(range, _)| &line[range.clone()])
            .collect()
    }

    #[test]
    fn a_rendered_bold_line_hides_markers_and_bolds_the_text() {
        let line = "a **b** c";
        let style = line_style(&spans_of(line), 0..line.len(), false);
        assert_eq!(shown(&style, line), "a b c");
        assert_eq!(style.runs, vec![(4..5, SpanKind::Bold)]);
        assert_eq!(effects(SpanKind::Bold, false), &[Effect::Bold]);
        assert_eq!(style.heading, None);
    }

    #[test]
    fn a_revealed_line_has_one_shown_piece_and_marker_colours() {
        let line = "a **b** c";
        let style = line_style(&spans_of(line), 0..line.len(), true);
        assert_eq!(style.pieces, vec![(0..9, true)]);
        assert!(style.runs.contains(&(2..4, SpanKind::Hidden)));
        let colours = style.colour_runs();
        assert!(colours.contains(&(2..4, Paint::Marker)));
        assert!(colours.contains(&(4..5, Paint::Emphasis)));
    }

    #[test]
    fn a_heading_line_has_its_level_only_when_rendered() {
        let line = "## Title";
        let rendered = line_style(&spans_of(line), 0..line.len(), false);
        assert_eq!(rendered.heading, Some(2));
        assert_eq!(shown(&rendered, line), "Title");
        let revealed = line_style(&spans_of(line), 0..line.len(), true);
        assert_eq!(revealed.heading, None);
    }

    #[test]
    fn a_setext_underline_is_all_hidden() {
        let text = "Title\n=====";
        let spans = spans_of(text);
        let title = line_style(&spans, 0..5, false);
        assert_eq!(title.heading, Some(1));
        let underline = line_style(&spans, 6..11, false);
        assert_eq!(underline.heading, None);
        assert_eq!(shown(&underline, "====="), "");
    }

    #[test]
    fn spans_are_clipped_to_the_line() {
        let text = "x\n**a**";
        let style = line_style(&spans_of(text), 2..7, false);
        assert_eq!(style.runs, vec![(2..3, SpanKind::Bold)]);
        assert_eq!(shown(&style, "**a**"), "a");
    }

    #[test]
    fn a_span_crossing_lines_styles_each_line() {
        let text = "*one\ntwo*";
        let spans = spans_of(text);
        let first = line_style(&spans, 0..4, false);
        let second = line_style(&spans, 5..9, false);
        assert_eq!(first.runs, vec![(1..4, SpanKind::Italic)]);
        assert_eq!(second.runs, vec![(0..3, SpanKind::Italic)]);
        assert_eq!(shown(&first, "*one"), "one");
        assert_eq!(shown(&second, "two*"), "two");
    }

    #[test]
    fn dim_colour_wins() {
        let style = LineStyle {
            revealed: false,
            heading: None,
            pieces: vec![(0..6, true)],
            runs: vec![(0..6, SpanKind::Dim), (0..6, SpanKind::Link)],
        };
        let colours = style.colour_runs();
        assert_eq!(colours.last(), Some(&(0..6, Paint::Dim)));
    }

    #[test]
    fn effects_never_change_metrics_on_revealed_lines() {
        for kind in [
            SpanKind::Hidden,
            SpanKind::Bold,
            SpanKind::Italic,
            SpanKind::Strike,
            SpanKind::Code,
            SpanKind::Link,
            SpanKind::Heading(1),
            SpanKind::Dim,
        ] {
            for effect in effects(kind, true) {
                assert!(
                    matches!(effect, Effect::Color(_) | Effect::Background(_)),
                    "{kind:?}: {effect:?}"
                );
            }
        }
    }

    #[test]
    fn an_empty_line_has_one_empty_piece() {
        let style = line_style(&[], 3..3, false);
        assert_eq!(style.pieces, vec![(0..0, true)]);
    }
}
