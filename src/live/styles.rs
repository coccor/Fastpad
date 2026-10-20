//! A source line's spans, its block context and its revealed flag → how it is laid out (phase 2
//! plan, Task 3 and Decisions 3 and 4; phase 3 plan, Task 3). Pure: the effect table is data,
//! tested without DirectWrite.
//!
//! A rendered line hides its markup and gets real weights, styles, sizes and fonts; its
//! containers (quotes, list items) are drawn in its margin and their markers hidden. A revealed
//! line (in the caret's block) shows all its markup, in the marker colour, and keeps the rest of
//! its formatting: a heading keeps its size, bold stays bold, code keeps its font (phase 3
//! review). Only the margin and the block decorations give way to the source.

use super::blocks::LiveDocument;
use super::offsets::Show;
use super::spans::{Align, Body, Container, LineBlock, Span, SpanKind};
use super::text::LiveText;
use crate::preview::incremental::SourceText;
use std::borrow::Cow;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
    /// Rules and table borders.
    Rule,
    QuoteBar,
    /// Text inside a quote.
    Quote,
}

impl Paint {
    /// When two runs colour the same bytes, the higher rank is applied last and shows.
    const fn rank(self) -> u8 {
        match self {
            Self::Text | Self::CodeBackground | Self::Rule | Self::QuoteBar => 0,
            Self::Quote => 1,
            Self::Emphasis => 2,
            Self::Marker => 3,
            Self::Heading => 4,
            Self::Code => 5,
            Self::Link => 6,
            Self::Dim => 7,
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

/// How a line's block context shows: its margin and its body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockStyle {
    /// Containers drawn in the margin, outermost first; empty on revealed lines.
    pub containers: Vec<Container>,
    pub body: BodyStyle,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BodyStyle {
    #[default]
    Text,
    /// The code font and a background across the text width; `fence` for a fence line, whose
    /// shown `info` (a language) is trailing-aligned.
    Code {
        fence: bool,
        info: bool,
    },
    Rule,
    TableRow {
        header: bool,
        cells: usize,
        aligns: Rc<[Align]>,
    },
    TableDelimiter,
    /// A line holding only an image; `alt` is relative to the line, for the fallback, and
    /// `size` an `<img>`'s pixel width and height, when it gives them.
    Image {
        dest: String,
        alt: Range<usize>,
        size: (Option<u32>, Option<u32>),
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineStyle {
    pub revealed: bool,
    /// The line's heading level when rendered; `None` on revealed lines.
    pub heading: Option<u8>,
    /// Pieces covering the line, for `OffsetMap::new`.
    pub pieces: Vec<(Range<usize>, Show)>,
    /// Line-relative byte ranges and their span kinds; `Hidden` only on revealed lines.
    pub runs: Vec<(Range<usize>, SpanKind)>,
    pub block: BlockStyle,
}

impl LineStyle {
    /// A line with no styling: shown in full.
    pub fn plain(len: usize) -> Self {
        Self {
            pieces: vec![(0..len, Show::Text)],
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

    /// An image line whose image cannot be shown (remote, missing, not decoded yet): its alt
    /// text, dimmed and italic, and nothing else. Any other line is returned unchanged.
    pub fn image_fallback(&self) -> Self {
        let BodyStyle::Image { alt, .. } = &self.block.body else {
            return self.clone();
        };
        let len = self.pieces.last().map_or(0, |(range, _)| range.end);
        let runs = if alt.is_empty() {
            Vec::new()
        } else {
            vec![
                (alt.clone(), SpanKind::Dim),
                (alt.clone(), SpanKind::Italic),
            ]
        };
        Self {
            revealed: false,
            heading: None,
            pieces: pieces_of(
                len,
                vec![(0..alt.start, Show::Hidden), (alt.end..len, Show::Hidden)],
            ),
            runs,
            block: BlockStyle {
                containers: self.block.containers.clone(),
                body: BodyStyle::Text,
            },
        }
    }
}

/// The effects `kind` has on a rendered line, or on a revealed one: the same, except that the
/// markup a rendered line hides shows in the marker colour.
pub fn effects(kind: SpanKind, revealed: bool) -> &'static [Effect] {
    use Effect::*;
    if revealed && kind == SpanKind::Hidden {
        return &[Color(Paint::Marker)];
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
        // The line's body sets the code font and its background.
        SpanKind::CodeBlock => &[Color(Paint::Code)],
        SpanKind::Quote => &[Color(Paint::Quote)],
    }
}

/// The style of the line whose bytes (line ending excluded) are `line`, from the absolute
/// `spans` overlapping it and its block context.
pub fn line_style(
    spans: &[Span],
    block: Option<&LineBlock>,
    line: Range<usize>,
    revealed: bool,
) -> LineStyle {
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
    let prefix = block.map_or(0..0, |block| block.prefix.clone());
    let content = prefix.end.min(len)..len;
    let body = block.map(|block| &block.body);
    if revealed {
        return revealed_style(relative, body, prefix, content);
    }
    let mut marks = relative
        .iter()
        .filter(|(_, kind)| *kind == SpanKind::Hidden)
        .map(|(range, _)| (range.clone(), Show::Hidden))
        .collect::<Vec<_>>();
    marks.push((prefix, Show::Hidden));
    let mut runs = relative
        .into_iter()
        .filter(|(_, kind)| *kind != SpanKind::Hidden)
        .collect::<Vec<_>>();
    let body = match body {
        None | Some(Body::Text) => BodyStyle::Text,
        Some(Body::Code) => {
            runs = vec![(content.clone(), SpanKind::CodeBlock)];
            BodyStyle::Code {
                fence: false,
                info: false,
            }
        }
        Some(Body::Fence { info }) => {
            runs.clear();
            match info {
                Some(info) => {
                    marks.push((content.start..info.start, Show::Hidden));
                    marks.push((info.end..len, Show::Hidden));
                    runs.push((info.clone(), SpanKind::Dim));
                }
                None => marks.push((content.clone(), Show::Hidden)),
            }
            BodyStyle::Code {
                fence: true,
                info: info.is_some(),
            }
        }
        Some(Body::Rule) => {
            runs.clear();
            marks.push((0..len, Show::Hidden));
            BodyStyle::Rule
        }
        Some(Body::TableDelimiter { .. }) => {
            runs.clear();
            marks.push((0..len, Show::Hidden));
            BodyStyle::TableDelimiter
        }
        Some(Body::TableRow {
            header,
            cells,
            pipes,
            aligns,
        }) => {
            marks.extend(table_marks(cells, pipes, content.clone()));
            if *header {
                runs.extend(
                    cells
                        .iter()
                        .filter(|cell| !cell.is_empty())
                        .map(|cell| (cell.clone(), SpanKind::Bold)),
                );
            }
            BodyStyle::TableRow {
                header: *header,
                cells: cells.len(),
                aligns: Rc::clone(aligns),
            }
        }
        Some(Body::Image { dest, alt, size }) => {
            runs.clear();
            marks.push((0..len, Show::Hidden));
            BodyStyle::Image {
                dest: dest.clone(),
                alt: alt.clone(),
                size: *size,
            }
        }
    };
    let containers = block.map_or_else(Vec::new, |block| block.containers.clone());
    if body == BodyStyle::Text && !content.is_empty() && containers.contains(&Container::Quote) {
        runs.push((content, SpanKind::Quote));
    }
    let pieces = pieces_of(len, marks);
    // A heading line: a heading span covers the line's first shown byte.
    let first_shown = pieces
        .iter()
        .find(|(range, show)| *show == Show::Text && range.start < range.end)
        .map(|(range, _)| range.start);
    let heading = first_shown.and_then(|at| {
        runs.iter().find_map(|(range, kind)| match kind {
            SpanKind::Heading(level) if range.contains(&at) => Some(*level),
            _ => None,
        })
    });
    LineStyle {
        revealed: false,
        heading,
        pieces,
        runs,
        block: BlockStyle { containers, body },
    }
}

/// A revealed line: its source in full, formatted as when rendered, its markup and prefix in the
/// marker colour. A code block's lines keep their code body.
fn revealed_style(
    mut runs: Vec<(Range<usize>, SpanKind)>,
    body: Option<&Body>,
    prefix: Range<usize>,
    content: Range<usize>,
) -> LineStyle {
    let len = content.end;
    if !prefix.is_empty() {
        runs.push((prefix, SpanKind::Hidden));
    }
    let body = match body {
        Some(Body::Code) => {
            if !content.is_empty() {
                runs.push((content, SpanKind::CodeBlock));
            }
            BodyStyle::Code {
                fence: false,
                info: false,
            }
        }
        Some(Body::Fence { .. }) => {
            if !content.is_empty() {
                runs.push((content, SpanKind::Hidden));
            }
            BodyStyle::Code {
                fence: true,
                info: false,
            }
        }
        _ => BodyStyle::Text,
    };
    // A heading keeps its size while it is edited.
    let heading = runs.iter().find_map(|(_, kind)| match kind {
        SpanKind::Heading(level) => Some(*level),
        _ => None,
    });
    LineStyle {
        revealed: true,
        heading,
        pieces: vec![(0..len, Show::Text)],
        runs,
        block: BlockStyle {
            containers: Vec::new(),
            body,
        },
    }
}

/// A rendered table row's marks: pipes become objects, everything else outside the cells is
/// hidden, and a row without an outer pipe gets an empty object in its place.
fn table_marks(
    cells: &[Range<usize>],
    pipes: &[usize],
    content: Range<usize>,
) -> Vec<(Range<usize>, Show)> {
    let mut items = cells
        .iter()
        .map(|cell| (cell.clone(), Show::Text))
        .chain(pipes.iter().map(|&pipe| (pipe..pipe + 1, Show::Object)))
        .collect::<Vec<_>>();
    items.sort_by_key(|(range, _)| range.start);
    let mut marks = Vec::new();
    if items.first().is_none_or(|(_, show)| *show != Show::Object) {
        let at = items
            .first()
            .map_or(content.start, |(range, _)| range.start);
        marks.push((content.start..at, Show::Hidden));
        marks.push((at..at, Show::Object));
    }
    let mut at = content.start;
    for (range, show) in &items {
        marks.push((at..range.start, Show::Hidden));
        if *show == Show::Object {
            marks.push((range.clone(), Show::Object));
        }
        at = range.end;
    }
    marks.push((at..content.end, Show::Hidden));
    if items.last().is_none_or(|(_, show)| *show != Show::Object) {
        marks.push((content.end..content.end, Show::Object));
    }
    marks
}

/// Pieces covering `0..len` from the hidden and object `marks` (in any order; hidden ones may
/// overlap): the bytes no mark covers are shown, adjacent hidden ranges join, and an object
/// inside a hidden range is dropped.
fn pieces_of(len: usize, mut marks: Vec<(Range<usize>, Show)>) -> Vec<(Range<usize>, Show)> {
    marks.retain(|(range, show)| *show == Show::Object || range.start < range.end);
    marks.sort_by_key(|(range, _)| (range.start, range.end));
    let mut pieces: Vec<(Range<usize>, Show)> = Vec::new();
    let mut at = 0;
    for (range, show) in marks {
        if show == Show::Object {
            if range.start < at {
                continue;
            }
            if range.start > at {
                pieces.push((at..range.start, Show::Text));
            }
            at = range.end;
            pieces.push((range, Show::Object));
            continue;
        }
        if range.end <= at {
            continue;
        }
        let start = range.start.max(at);
        if start > at {
            pieces.push((at..start, Show::Text));
        }
        match pieces.last_mut() {
            Some((last, Show::Hidden)) if last.end == start => last.end = range.end,
            _ => pieces.push((start..range.end, Show::Hidden)),
        }
        at = range.end;
    }
    if at < len || pieces.is_empty() {
        pieces.push((at..len, Show::Text));
    }
    pieces
}

/// What the view knows about the picture of an image line (phase 3 plan, Decision 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageInfo {
    /// Decoded: its file and natural size in pixels.
    Ready { path: PathBuf, size: (u32, u32) },
    /// A local file not decoded yet.
    Pending(PathBuf),
    /// Remote, missing, failed, or not a local path: the line shows its alt text.
    Unavailable,
}

/// An image resolver that knows no images: every image line shows its alt text.
pub fn no_images(_: &str) -> ImageInfo {
    ImageInfo::Unavailable
}

/// What `dest` (an image line's destination) shows as, for a document in `document_dir`.
/// Remote, `#`, UNC and device destinations are `Unavailable` without asking `size` or `failed`
/// anything, so they never reach the disk or the network; a local path is `Ready` once `size`
/// knows it, `Unavailable` once `failed` says so, and `Pending` until then.
pub fn resolve_image(
    dest: &str,
    document_dir: Option<&Path>,
    size: impl Fn(&Path) -> Option<(u32, u32)>,
    failed: impl Fn(&Path) -> bool,
) -> ImageInfo {
    let Some(path) = crate::preview::links::resolve_image_path(dest, document_dir) else {
        return ImageInfo::Unavailable;
    };
    match size(&path) {
        Some(size) if size.0 > 0 && size.1 > 0 => ImageInfo::Ready { path, size },
        _ if failed(&path) => ImageInfo::Unavailable,
        _ => ImageInfo::Pending(path),
    }
}

/// Lines with their styles: what `LiveLayout` lays out.
pub trait StyledLines: LiveText {
    fn line_style(&self, line: usize) -> LineStyle;

    /// The lines of the table holding `line`, when `line` is a table row or delimiter.
    fn table_lines(&self, _line: usize) -> Option<Range<usize>> {
        None
    }

    /// `line`'s style as if it were not revealed: what a table measures its rows by, the
    /// revealed row included.
    fn rendered_style(&self, line: usize) -> LineStyle {
        self.line_style(line)
    }

    /// What an image line whose destination is `dest` shows.
    fn image(&self, _dest: &str) -> ImageInfo {
        ImageInfo::Unavailable
    }
}

/// The document text, its spans, the revealed lines and the images it can show.
pub struct Styled<'a, T: LiveText> {
    pub text: &'a T,
    pub document: &'a LiveDocument,
    pub revealed: Range<usize>,
    pub images: &'a dyn Fn(&str) -> ImageInfo,
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
        let block = self.document.line_block(self.text, bytes.clone());
        line_style(&spans, block.as_ref(), bytes, self.revealed.contains(&line))
    }

    fn rendered_style(&self, line: usize) -> LineStyle {
        let bytes = self.text.line_start(line)..self.text.line_end(line);
        let spans = self.document.spans_in(self.text, bytes.clone());
        let block = self.document.line_block(self.text, bytes.clone());
        line_style(&spans, block.as_ref(), bytes, false)
    }

    fn table_lines(&self, line: usize) -> Option<Range<usize>> {
        // A byte of the line itself: an indented table's block starts after the line's start.
        let start = self.text.line_start(line);
        let at = self.text.line_end(line).saturating_sub(1).max(start);
        let (first, last) = self.document.table_extent(self.text, at)?;
        Some(LiveText::line_of(self.text, first)..LiveText::line_of(self.text, last) + 1)
    }

    fn image(&self, dest: &str) -> ImageInfo {
        (self.images)(dest)
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
    use crate::live::spans::{Marker, line_ranges, parse_block};
    use crate::live::text::StrSource;

    /// Line `index` of `text` styled as the view styles it, and the line's text.
    fn style_of(text: &str, index: usize, revealed: bool) -> (LineStyle, String) {
        let document = LiveDocument::parse(text);
        let source = StrSource(text);
        let bytes = line_ranges(text, 0..text.len())[index].clone();
        let spans = document.spans_in(&source, bytes.clone());
        let block = document.line_block(&source, bytes.clone());
        (
            line_style(&spans, block.as_ref(), bytes.clone(), revealed),
            text[bytes].to_owned(),
        )
    }

    /// The layout text, with each object shown as `|`.
    fn drawn(style: &LineStyle, line: &str) -> String {
        style
            .pieces
            .iter()
            .map(|(range, show)| match show {
                Show::Text => &line[range.clone()],
                Show::Hidden => "",
                Show::Object => "|",
            })
            .collect()
    }

    #[test]
    fn a_rendered_item_hides_its_prefix_and_keeps_its_containers() {
        let (style, line) = style_of("- **a** b", 0, false);
        assert_eq!(drawn(&style, &line), "a b");
        assert_eq!(
            style.block.containers,
            vec![Container::Item {
                depth: 1,
                ordered: false,
                marker: Some(Marker::Bullet)
            }]
        );
        assert_eq!(style.block.body, BodyStyle::Text);
    }

    #[test]
    fn a_revealed_item_shows_its_prefix_without_containers() {
        let (style, line) = style_of("- **a** b", 0, true);
        assert_eq!(drawn(&style, &line), "- **a** b");
        assert!(style.block.containers.is_empty());
        assert!(style.colour_runs().contains(&(0..2, Paint::Marker)));
    }

    #[test]
    fn code_lines_use_the_code_body_in_both_states() {
        let text = "```\nx = *1*\n```";
        for revealed in [false, true] {
            let (style, line) = style_of(text, 1, revealed);
            assert_eq!(drawn(&style, &line), "x = *1*");
            assert_eq!(
                style.block.body,
                BodyStyle::Code {
                    fence: false,
                    info: false
                }
            );
            assert_eq!(style.colour_runs(), vec![(0..7, Paint::Code)]);
        }
    }

    #[test]
    fn a_fence_hides_its_markers_and_dims_its_info() {
        let text = "```rust\nx\n```";
        let (open, line) = style_of(text, 0, false);
        assert_eq!(drawn(&open, &line), "rust");
        assert_eq!(open.colour_runs(), vec![(3..7, Paint::Dim)]);
        assert_eq!(
            open.block.body,
            BodyStyle::Code {
                fence: true,
                info: true
            }
        );
        let (close, line) = style_of(text, 2, false);
        assert_eq!(drawn(&close, &line), "");
        let (revealed, line) = style_of(text, 0, true);
        assert_eq!(drawn(&revealed, &line), "```rust");
        assert!(matches!(
            revealed.block.body,
            BodyStyle::Code { fence: true, .. }
        ));
    }

    #[test]
    fn table_pipes_become_objects_and_cells_are_trimmed() {
        let text = "| a | **b** |\n|---|---|\n| c | d |";
        let (header, line) = style_of(text, 0, false);
        assert_eq!(drawn(&header, &line), "|a|b|");
        assert!(matches!(
            header.block.body,
            BodyStyle::TableRow {
                header: true,
                cells: 2,
                ..
            }
        ));
        assert!(header.runs.contains(&(2..3, SpanKind::Bold)));
        let (delimiter, line) = style_of(text, 1, false);
        assert_eq!(drawn(&delimiter, &line), "");
        assert_eq!(delimiter.block.body, BodyStyle::TableDelimiter);
        let (row, line) = style_of(text, 2, false);
        assert_eq!(drawn(&row, &line), "|c|d|");
        assert!(row.runs.is_empty());
        let (revealed, line) = style_of(text, 2, true);
        assert_eq!(drawn(&revealed, &line), line);
    }

    #[test]
    fn a_row_without_outer_pipes_gets_empty_objects() {
        let (style, line) = style_of("a | b\n--|--", 0, false);
        assert_eq!(drawn(&style, &line), "|a|b|");
        assert_eq!(style.pieces.first(), Some(&(0..0, Show::Object)));
        assert_eq!(style.pieces.last(), Some(&(5..5, Show::Object)));
    }

    #[test]
    fn a_rule_hides_everything() {
        let (style, line) = style_of("a\n\n***", 2, false);
        assert_eq!(drawn(&style, &line), "");
        assert_eq!(style.block.body, BodyStyle::Rule);
    }

    #[test]
    fn an_image_line_hides_everything_and_falls_back_to_dim_alt() {
        let (style, line) = style_of("![cat](c.png)", 0, false);
        assert_eq!(drawn(&style, &line), "");
        assert_eq!(
            style.block.body,
            BodyStyle::Image {
                dest: "c.png".into(),
                alt: 2..5,
                size: (None, None),
            }
        );
        let fallback = style.image_fallback();
        assert_eq!(drawn(&fallback, &line), "cat");
        assert_eq!(
            fallback.runs,
            vec![(2..5, SpanKind::Dim), (2..5, SpanKind::Italic)]
        );
        assert_eq!(fallback.block.body, BodyStyle::Text);
    }

    #[test]
    fn quote_text_is_coloured_below_links() {
        let (style, line) = style_of("> see [l](u)", 0, false);
        assert_eq!(drawn(&style, &line), "see l");
        let colours = style.colour_runs();
        let quote = colours.iter().position(|(_, paint)| *paint == Paint::Quote);
        let link = colours.iter().position(|(_, paint)| *paint == Paint::Link);
        assert!(quote < link, "{colours:?}");
    }

    #[test]
    fn remote_and_unc_images_show_alt_text() {
        let asked = std::cell::Cell::new(0);
        let size = |_: &Path| {
            asked.set(asked.get() + 1);
            None
        };
        let failed = |_: &Path| {
            asked.set(asked.get() + 1);
            false
        };
        let dir = Some(Path::new(r"C:\notes"));
        for dest in [
            "https://example.com/a.png",
            "//host/a.png",
            r"\\host\share\a.png",
            "%5C%5Chost%5Cshare%5Ca.png",
            "#top",
            "",
        ] {
            assert_eq!(
                resolve_image(dest, dir, size, failed),
                ImageInfo::Unavailable,
                "{dest}"
            );
        }
        assert_eq!(asked.get(), 0);
    }

    #[test]
    fn local_images_are_pending_until_decoded() {
        let dir = Some(Path::new(r"C:\notes"));
        let path = PathBuf::from(r"C:\notes\img\a.png");
        assert_eq!(
            resolve_image("img/a.png", dir, |_| None, |_| false),
            ImageInfo::Pending(path.clone())
        );
        assert_eq!(
            resolve_image("img/a.png", dir, |_| Some((4, 3)), |_| false),
            ImageInfo::Ready { path, size: (4, 3) }
        );
        assert_eq!(
            resolve_image("img/a.png", dir, |_| None, |_| true),
            ImageInfo::Unavailable
        );
    }

    #[test]
    fn a_checked_task_hides_its_box_and_is_struck() {
        let (style, line) = style_of("- [x] done", 0, false);
        assert_eq!(drawn(&style, &line), "done");
        assert!(style.colour_runs().contains(&(6..10, Paint::Dim)));
    }

    fn spans_of(text: &str) -> Vec<Span> {
        parse_block(text, &|_| None).spans
    }

    fn shown(style: &LineStyle, line: &str) -> String {
        style
            .pieces
            .iter()
            .filter(|(_, show)| *show == Show::Text)
            .map(|(range, _)| &line[range.clone()])
            .collect()
    }

    #[test]
    fn a_rendered_bold_line_hides_markers_and_bolds_the_text() {
        let line = "a **b** c";
        let style = line_style(&spans_of(line), None, 0..line.len(), false);
        assert_eq!(shown(&style, line), "a b c");
        assert_eq!(style.runs, vec![(4..5, SpanKind::Bold)]);
        assert_eq!(effects(SpanKind::Bold, false), &[Effect::Bold]);
        assert_eq!(style.heading, None);
    }

    #[test]
    fn a_revealed_line_has_one_shown_piece_and_marker_colours() {
        let line = "a **b** c";
        let style = line_style(&spans_of(line), None, 0..line.len(), true);
        assert_eq!(style.pieces, vec![(0..9, Show::Text)]);
        assert!(style.runs.contains(&(2..4, SpanKind::Hidden)));
        let colours = style.colour_runs();
        assert!(colours.contains(&(2..4, Paint::Marker)));
        // Bold stays bold while it is edited.
        assert!(style.runs.contains(&(4..5, SpanKind::Bold)));
        assert_eq!(effects(SpanKind::Bold, true), &[Effect::Bold]);
    }

    #[test]
    fn a_heading_keeps_its_level_when_revealed() {
        let line = "## Title";
        let rendered = line_style(&spans_of(line), None, 0..line.len(), false);
        assert_eq!(rendered.heading, Some(2));
        assert_eq!(shown(&rendered, line), "Title");
        let revealed = line_style(&spans_of(line), None, 0..line.len(), true);
        assert_eq!(revealed.heading, Some(2));
        assert_eq!(shown(&revealed, line), "## Title");
    }

    #[test]
    fn a_setext_underline_is_all_hidden() {
        let text = "Title\n=====";
        let spans = spans_of(text);
        let title = line_style(&spans, None, 0..5, false);
        assert_eq!(title.heading, Some(1));
        let underline = line_style(&spans, None, 6..11, false);
        assert_eq!(underline.heading, None);
        assert_eq!(shown(&underline, "====="), "");
    }

    #[test]
    fn spans_are_clipped_to_the_line() {
        let text = "x\n**a**";
        let style = line_style(&spans_of(text), None, 2..7, false);
        assert_eq!(style.runs, vec![(2..3, SpanKind::Bold)]);
        assert_eq!(shown(&style, "**a**"), "a");
    }

    #[test]
    fn a_span_crossing_lines_styles_each_line() {
        let text = "*one\ntwo*";
        let spans = spans_of(text);
        let first = line_style(&spans, None, 0..4, false);
        let second = line_style(&spans, None, 5..9, false);
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
            pieces: vec![(0..6, Show::Text)],
            runs: vec![(0..6, SpanKind::Dim), (0..6, SpanKind::Link)],
            ..LineStyle::default()
        };
        let colours = style.colour_runs();
        assert_eq!(colours.last(), Some(&(0..6, Paint::Dim)));
    }

    #[test]
    fn revealed_lines_keep_their_formatting_and_colour_their_markup() {
        for kind in [
            SpanKind::Hidden,
            SpanKind::Bold,
            SpanKind::Italic,
            SpanKind::Strike,
            SpanKind::Code,
            SpanKind::Link,
            SpanKind::Heading(1),
            SpanKind::Dim,
            SpanKind::CodeBlock,
            SpanKind::Quote,
        ] {
            if kind == SpanKind::Hidden {
                assert_eq!(effects(kind, true), &[Effect::Color(Paint::Marker)]);
                assert!(effects(kind, false).is_empty());
            } else {
                assert_eq!(effects(kind, true), effects(kind, false), "{kind:?}");
            }
        }
    }

    #[test]
    fn an_empty_line_has_one_empty_piece() {
        let style = line_style(&[], None, 3..3, false);
        assert_eq!(style.pieces, vec![(0..0, Show::Text)]);
    }
}
