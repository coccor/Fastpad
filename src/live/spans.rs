//! Block text → the inline spans the Live view renders (2026-10-18 Live editor design spec §6,
//! phase 2 plan Task 2). Pure: offsets are bytes relative to the block text, and the caller adds
//! the block's position. Spans may overlap (`Bold` inside a `Link`); `Hidden` ranges never
//! overlap any other span.
//!
//! Block constructs (lists, quotes, code blocks, tables, rules, images) give each of their lines
//! a `LineBlock` (phase 3 plan, Task 1): the containers drawn in its margin, the prefix a
//! rendered line hides and what its body is. Code blocks and HTML blocks keep the inline spans of
//! their text from being emitted; table cells keep theirs.

use crate::preview::html::Token;
use crate::preview::model::PARSE_OPTIONS;
use pulldown_cmark::{
    Alignment, BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd,
};
use std::ops::Range;
use std::rc::Rc;

/// The preview's options without front matter: a block parsed on its own would read a later
/// `---` block as metadata. `blocks.rs` finds the front matter itself.
pub const LIVE_PARSE_OPTIONS: Options =
    PARSE_OPTIONS.difference(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpanKind {
    /// Markup a rendered line does not show: emphasis markers, link syntax, heading `#`s.
    Hidden,
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    Heading(u8),
    /// Raw HTML, front matter, footnotes and math: shown as dimmed source.
    Dim,
    /// A code block line's text. Set by `styles`, never by the parser: the line's font is the
    /// code font, so the run only colours it.
    CodeBlock,
    /// Text inside a quote. Set by `styles`, never by the parser: coloured under every other run.
    Quote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    /// The whole link's source, brackets and destination included.
    pub range: Range<usize>,
    pub dest: String,
    pub wiki: bool,
}

/// A list item's marker, on the item's first line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    Bullet,
    /// `3.` or `7)`: the bytes drawn in the margin, relative to the line.
    Ordered(Range<usize>),
    /// The byte inside `[ ]` / `[x]`, relative to the line.
    Task {
        checked: bool,
        inner: usize,
    },
}

/// A block a line sits in, drawn in the line's margin when it is rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Container {
    Quote,
    /// A list item at `depth` (1 = top level); `marker` only on the item's first line.
    Item {
        depth: u8,
        ordered: bool,
        marker: Option<Marker>,
    },
}

/// An `<img>`'s `width` and `height` in pixels, when it gives them.
pub type ImageSize = (Option<u32>, Option<u32>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// What a line holds once its containers' prefixes are skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text,
    Code,
    /// A fence: an opening fence's info string, relative to the line; `None` when it has none
    /// and for a closing fence.
    Fence {
        info: Option<Range<usize>>,
    },
    Rule,
    /// Cell contents (whitespace trimmed) and the pipes, relative to the line.
    TableRow {
        header: bool,
        cells: Vec<Range<usize>>,
        pipes: Vec<usize>,
        aligns: Rc<[Align]>,
    },
    /// Shares its `aligns` with its table's rows, which is how a table's lines are found.
    TableDelimiter {
        aligns: Rc<[Align]>,
    },
    /// The line's only content is this image: a Markdown image (optionally inside one link) or
    /// an HTML `<img>`. `alt` is relative to the line; `size` is an `<img>`'s `width` and
    /// `height` in pixels, when it gives them.
    Image {
        dest: String,
        alt: Range<usize>,
        size: (Option<u32>, Option<u32>),
    },
}

/// A line's block context (phase 3 plan, Task 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBlock {
    /// The line's start, relative to the block.
    pub start: usize,
    /// Bytes a rendered line hides before its body, relative to the line.
    pub prefix: Range<usize>,
    /// Outermost first.
    pub containers: Vec<Container>,
    pub body: Body,
}

impl LineBlock {
    /// The same context for a line that starts `delta` bytes before the block's text does (a
    /// block parsed from its first marker, after the line's indent): line-relative offsets move
    /// right, and the indent joins the hidden prefix.
    pub fn shifted(mut self, delta: usize) -> Self {
        if delta == 0 {
            return self;
        }
        let shift = |range: &mut Range<usize>| *range = range.start + delta..range.end + delta;
        self.prefix = 0..self.prefix.end + delta;
        for container in &mut self.containers {
            match container {
                Container::Item {
                    marker: Some(Marker::Ordered(range)),
                    ..
                } => shift(range),
                Container::Item {
                    marker: Some(Marker::Task { inner, .. }),
                    ..
                } => *inner += delta,
                _ => {}
            }
        }
        match &mut self.body {
            Body::Fence { info: Some(range) } | Body::Image { alt: range, .. } => shift(range),
            Body::TableRow { cells, pipes, .. } => {
                cells.iter_mut().for_each(shift);
                pipes.iter_mut().for_each(|pipe| *pipe += delta);
            }
            _ => {}
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockSpans {
    /// Sorted by start.
    pub spans: Vec<Span>,
    /// For each span, the furthest end of it and every span before it: the first span that can
    /// overlap a byte is found by binary search, so styling one line of a long block does not
    /// scan the whole block's spans.
    pub reach: Vec<usize>,
    pub links: Vec<LinkSpan>,
    /// Sorted by start; a top-level line of plain text has none.
    pub lines: Vec<LineBlock>,
}

impl BlockSpans {
    /// The spans overlapping `bytes` (relative to the block), in order.
    pub fn overlapping(&self, bytes: Range<usize>) -> impl Iterator<Item = &Span> {
        let first = self.reach.partition_point(|&end| end <= bytes.start);
        self.spans[first..]
            .iter()
            .take_while(move |span| span.range.start < bytes.end)
            .filter(move |span| span.range.end > bytes.start)
    }
}

/// The spans and links of one block's `text`. `refs` resolves reference links defined elsewhere
/// in the document.
pub fn parse_block(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> BlockSpans {
    let resolve = |link: BrokenLink<'_>| {
        refs(&link.reference).map(|dest| (CowStr::from(dest), CowStr::from("")))
    };
    let mut parser = Parser::new_with_broken_link_callback(text, LIVE_PARSE_OPTIONS, Some(resolve))
        .into_offset_iter();
    let mut collector = Collector::new(text);
    // Reference definitions produce no events; a footnote definition `[^a]: …` is one.
    for (_, definition) in parser.reference_definitions().iter() {
        if text[definition.span.clone()].starts_with("[^") {
            collector.lexical.push(definition.span.clone());
        } else {
            // A link reference definition: GitHub shows nothing for it.
            collector.push(definition.span.clone(), SpanKind::Dim);
        }
    }
    for (event, range) in parser.by_ref() {
        collector.event(event, range);
    }
    collector.finish()
}

/// The lines overlapping `range`, each without its line ending.
pub(crate) fn line_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = line_start(text, range.start);
    while start <= text.len() && (start < range.end || lines.is_empty()) {
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let content_end = if end > start && text.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        };
        lines.push(start..content_end);
        if end >= text.len() {
            break;
        }
        start = end + 1;
    }
    lines
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

struct OpenLink {
    whole: Range<usize>,
    dest: String,
    wiki: bool,
    /// The first and last inner event's bytes: the shown text.
    inner: Option<Range<usize>>,
}

struct OpenImage {
    whole: Range<usize>,
    dest: String,
    /// The first and last inner event's bytes: the alt text.
    inner: Option<Range<usize>>,
}

struct ImageSeed {
    whole: Range<usize>,
    dest: String,
    alt: Range<usize>,
}

/// A container as its start event gives it; its lines' prefixes are found in `line_blocks`.
enum Seed {
    Quote,
    Item {
        depth: u8,
        ordered: bool,
        /// The marker's first byte.
        marker: usize,
        /// A task's checked state and its `[ ]` bytes.
        task: Option<(bool, Range<usize>)>,
    },
}

struct Collector<'a> {
    text: &'a str,
    spans: Vec<Span>,
    links: Vec<LinkSpan>,
    /// Footnote and math ranges: dimmed, and no other span inside them.
    lexical: Vec<Range<usize>>,
    /// Inside a code block or HTML block: inline events add no spans.
    suppress: u32,
    heading: Option<(u8, Range<usize>)>,
    open_links: Vec<OpenLink>,
    open_images: Vec<OpenImage>,
    /// Quotes and list items in the order they open: sorted by start, outer before inner.
    containers: Vec<(Range<usize>, Seed)>,
    /// Whether each open list is ordered.
    lists: Vec<bool>,
    open_items: u8,
    /// Code blocks and whether each is fenced.
    code_blocks: Vec<(Range<usize>, bool)>,
    tables: Vec<(Range<usize>, Rc<[Align]>)>,
    rules: Vec<usize>,
    images: Vec<ImageSeed>,
    /// The block holds HTML: a line may be an `<img>`.
    html: bool,
    /// Paragraphs, headings and table cells: where bare URLs and emails become links.
    prose: Vec<Range<usize>>,
    /// Code spans: a URL in them stays text.
    unlinked: Vec<Range<usize>>,
}

impl<'a> Collector<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            spans: Vec::new(),
            links: Vec::new(),
            lexical: Vec::new(),
            suppress: 0,
            heading: None,
            open_links: Vec::new(),
            open_images: Vec::new(),
            containers: Vec::new(),
            lists: Vec::new(),
            open_items: 0,
            code_blocks: Vec::new(),
            tables: Vec::new(),
            rules: Vec::new(),
            images: Vec::new(),
            html: false,
            prose: Vec::new(),
            unlinked: Vec::new(),
        }
    }

    fn push(&mut self, range: Range<usize>, kind: SpanKind) {
        if self.suppress == 0 && range.start < range.end && range.end <= self.text.len() {
            self.spans.push(Span { range, kind });
        }
    }

    fn event(&mut self, event: Event<'a>, range: Range<usize>) {
        if !matches!(
            event,
            Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link)
        ) && let Some(link) = self.open_links.last_mut()
        {
            let inner = link.inner.get_or_insert(range.clone());
            inner.start = inner.start.min(range.start);
            inner.end = inner.end.max(range.end);
        }
        if !matches!(
            event,
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image)
        ) && let Some(image) = self.open_images.last_mut()
        {
            let inner = image.inner.get_or_insert(range.clone());
            inner.start = inner.start.min(range.start);
            inner.end = inner.end.max(range.end);
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Code(_) => self.inline_code(range),
            Event::InlineHtml(_) | Event::Html(_) => {
                // An `<img>` alone on a line is an image line.
                self.html = true;
                self.push(range, SpanKind::Dim);
            }
            Event::TaskListMarker(checked) => self.task(checked, range),
            // `\` at a line's end breaks the line; spaces need no hiding.
            Event::HardBreak if self.text[range.clone()].starts_with('\\') => {
                self.push(range.start..range.start + 1, SpanKind::Hidden);
            }
            Event::Rule => self.rules.push(range.start),
            _ => {}
        }
    }

    /// A task's `[ ]` / `[x]`: recorded on its item; a checked task's text is struck and dimmed.
    fn task(&mut self, checked: bool, range: Range<usize>) {
        if let Some((_, Seed::Item { task, .. })) = self
            .containers
            .iter_mut()
            .rev()
            .find(|(_, seed)| matches!(seed, Seed::Item { .. }))
        {
            *task = Some((checked, range.clone()));
        }
        if checked {
            let line_end = line_ranges(self.text, range.end..range.end)[0].end;
            let text_start = (range.end + 1).min(line_end);
            self.push(text_start..line_end, SpanKind::Strike);
            self.push(text_start..line_end, SpanKind::Dim);
        }
    }

    fn inline_code(&mut self, range: Range<usize>) {
        let ticks = self.text[range.clone()]
            .bytes()
            .take_while(|b| *b == b'`')
            .count();
        if range.end - range.start < 2 * ticks {
            return;
        }
        self.push(range.start..range.start + ticks, SpanKind::Hidden);
        self.push(range.start + ticks..range.end - ticks, SpanKind::Code);
        self.push(range.end - ticks..range.end, SpanKind::Hidden);
        self.unlinked.push(range);
    }

    /// Hides `width`-byte markers at both ends of `range` and gives the rest `kind`.
    fn wrap(&mut self, range: &Range<usize>, width: usize, kind: SpanKind) {
        if range.end - range.start < 2 * width {
            return;
        }
        self.push(range.start..range.start + width, SpanKind::Hidden);
        self.push(range.start + width..range.end - width, kind);
        self.push(range.end - width..range.end, SpanKind::Hidden);
    }

    fn start(&mut self, tag: Tag<'a>, range: Range<usize>) {
        match tag {
            Tag::Paragraph if self.suppress == 0 => {
                self.lexical.extend(lexical_dims(self.text, range.clone()));
                self.hide_leading_space(range.clone());
                self.prose.push(range);
            }
            Tag::Heading { level, .. } => {
                if self.suppress == 0 {
                    self.lexical.extend(lexical_dims(self.text, range.clone()));
                    self.prose.push(range.clone());
                }
                self.heading = Some((level as u8, range));
            }
            Tag::TableCell if self.suppress == 0 => self.prose.push(range),
            Tag::Strong => self.wrap(&range, 2, SpanKind::Bold),
            Tag::Emphasis => self.wrap(&range, 1, SpanKind::Italic),
            Tag::Strikethrough => {
                let width = if self.text[range.clone()].starts_with("~~") {
                    2
                } else {
                    1
                };
                self.wrap(&range, width, SpanKind::Strike);
            }
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => self.open_links.push(OpenLink {
                whole: range,
                dest: dest_url.to_string(),
                wiki: matches!(link_type, LinkType::WikiLink { .. }),
                inner: None,
            }),
            Tag::Image { dest_url, .. } => self.open_images.push(OpenImage {
                whole: range,
                dest: dest_url.to_string(),
                inner: None,
            }),
            Tag::BlockQuote(_) => {
                self.alert(range.clone());
                self.containers.push((range, Seed::Quote));
            }
            Tag::List(start) => self.lists.push(start.is_some()),
            Tag::Item => {
                self.open_items = self.open_items.saturating_add(1);
                let bytes = self.text.as_bytes();
                let mut marker = range.start;
                while marker < range.end && matches!(bytes[marker], b' ' | b'\t') {
                    marker += 1;
                }
                let seed = Seed::Item {
                    depth: self.open_items,
                    ordered: self.lists.last().copied().unwrap_or(false),
                    marker,
                    task: None,
                };
                self.containers.push((range, seed));
            }
            Tag::CodeBlock(kind) => {
                self.suppress += 1;
                let fenced = matches!(kind, CodeBlockKind::Fenced(_));
                self.code_blocks.push((range, fenced));
            }
            Tag::Table(alignments) => {
                // GFM unescapes `\|` in a cell, inside code spans too.
                let escapes = self.text[range.clone()]
                    .match_indices("\\|")
                    .map(|(at, _)| range.start + at)
                    .collect::<Vec<_>>();
                for at in escapes {
                    self.push(at..at + 1, SpanKind::Hidden);
                }
                let aligns = alignments
                    .iter()
                    .map(|align| match align {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                self.tables.push((range, aligns));
            }
            Tag::HtmlBlock => {
                self.push(range, SpanKind::Dim);
                self.suppress += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                if let Some((level, whole)) = self.heading.take() {
                    self.heading_spans(level, whole);
                }
            }
            TagEnd::Link => {
                if let Some(link) = self.open_links.pop() {
                    self.link_spans(&link);
                    // The enclosing link's text includes this one.
                    if let Some(outer) = self.open_links.last_mut() {
                        let inner = outer.inner.get_or_insert(link.whole.clone());
                        inner.start = inner.start.min(link.whole.start);
                        inner.end = inner.end.max(link.whole.end);
                    }
                    if self.suppress == 0 {
                        self.links.push(LinkSpan {
                            range: link.whole,
                            dest: link.dest,
                            wiki: link.wiki,
                        });
                    }
                }
            }
            TagEnd::Image => {
                if let Some(image) = self.open_images.pop() {
                    let empty = image.whole.start + 2..image.whole.start + 2;
                    // Inside text, an image shows its alt text, dimmed (nothing is drawn for it
                    // there); an image line hides all of this and draws the picture instead.
                    if let Some(alt) = image.inner.clone() {
                        self.push(image.whole.start..alt.start, SpanKind::Hidden);
                        self.push(alt.end..image.whole.end, SpanKind::Hidden);
                        self.push(alt.clone(), SpanKind::Dim);
                        self.push(alt, SpanKind::Italic);
                    }
                    self.images.push(ImageSeed {
                        alt: image.inner.unwrap_or(empty),
                        whole: image.whole,
                        dest: image.dest,
                    });
                }
            }
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Item => self.open_items = self.open_items.saturating_sub(1),
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                self.suppress = self.suppress.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn link_spans(&mut self, link: &OpenLink) {
        // `[^label]`: a footnote reference, even when a definition makes it a link.
        if self.text[link.whole.clone()].starts_with("[^") {
            self.lexical.push(link.whole.clone());
            return;
        }
        let Some(inner) = &link.inner else {
            // `[](url)` shows as written.
            return;
        };
        self.push(link.whole.start..inner.start, SpanKind::Hidden);
        self.push(inner.clone(), SpanKind::Link);
        self.push(inner.end..link.whole.end, SpanKind::Hidden);
    }

    fn heading_spans(&mut self, level: u8, whole: Range<usize>) {
        let lines = line_ranges(self.text, whole.clone());
        let first = lines[0].clone();
        let source = &self.text[first.clone()];
        if lines.len() == 1 && source.trim_start().starts_with('#') {
            let lead = source.len() - source.trim_start().len();
            let hashes = source[lead..].bytes().take_while(|b| *b == b'#').count();
            let gap = source[lead + hashes..]
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            let content_start = first.start + lead + hashes + gap;
            let trimmed_end = first.start + source.trim_end().len();
            let body = &self.text[content_start..trimmed_end.max(content_start)];
            let closing = body.bytes().rev().take_while(|b| *b == b'#').count();
            let before_closing = &body[..body.len() - closing];
            let content_end = if closing > 0
                && (before_closing.is_empty() || before_closing.ends_with([' ', '\t']))
            {
                content_start + before_closing.trim_end().len()
            } else {
                trimmed_end.max(content_start)
            };
            self.push(first.start..content_start, SpanKind::Hidden);
            self.push(content_start..content_end, SpanKind::Heading(level));
            self.push(content_end..first.end, SpanKind::Hidden);
        } else {
            let last = lines.len() - 1;
            for (index, line) in lines.into_iter().enumerate() {
                let kind = if index == last {
                    SpanKind::Hidden
                } else {
                    SpanKind::Heading(level)
                };
                self.push(line, kind);
            }
        }
    }

    /// Each line's containers, prefix and body; lines of plain top-level text are left out.
    fn line_blocks(&self) -> Vec<LineBlock> {
        if self.containers.is_empty()
            && self.code_blocks.is_empty()
            && self.tables.is_empty()
            && self.rules.is_empty()
            && self.images.is_empty()
            && !self.html
        {
            // Plain prose (the common block, and a 10k-line paragraph): no line has a context.
            return Vec::new();
        }
        let text = self.text;
        let bytes = text.as_bytes();
        // Per list item: how far its later lines are indented to its content.
        let mut widths = vec![None; self.containers.len()];
        let mut out = Vec::new();
        for line in line_ranges(text, 0..text.len()) {
            let mut pos = line.start;
            let mut containers = Vec::new();
            // A lazy continuation line: its containers' markers are missing.
            let mut lazy = false;
            for (index, (range, seed)) in self.containers.iter().enumerate() {
                if range.start > line.end {
                    break;
                }
                if range.end <= line.start {
                    continue;
                }
                match seed {
                    Seed::Quote => {
                        containers.push(Container::Quote);
                        if lazy {
                            continue;
                        }
                        let mut probe = pos;
                        while probe < line.end && probe - pos < 3 && bytes[probe] == b' ' {
                            probe += 1;
                        }
                        if probe < line.end && bytes[probe] == b'>' {
                            pos = probe + 1;
                            if pos < line.end && matches!(bytes[pos], b' ' | b'\t') {
                                pos += 1;
                            }
                        } else {
                            lazy = true;
                        }
                    }
                    Seed::Item {
                        depth,
                        ordered,
                        marker,
                        task,
                    } => {
                        let marker_line = (line.start..line.end).contains(marker) && !lazy;
                        let mut kind = None;
                        if marker_line {
                            let item = item_prefix(text, *marker, line.end, task.as_ref());
                            widths[index] = Some(item.width_from(pos));
                            pos = item.prefix_end;
                            kind = Some(item.marker.relative_to(line.start));
                        } else if !lazy && let Some(width) = widths[index] {
                            let mut skipped = 0;
                            while skipped < width
                                && pos < line.end
                                && matches!(bytes[pos], b' ' | b'\t')
                            {
                                pos += 1;
                                skipped += 1;
                            }
                        }
                        containers.push(Container::Item {
                            depth: *depth,
                            ordered: *ordered,
                            marker: kind,
                        });
                    }
                }
            }
            let body = self.body_of(&line, &mut pos);
            if containers.is_empty() && body == Body::Text {
                continue;
            }
            out.push(LineBlock {
                start: line.start,
                prefix: 0..pos - line.start,
                containers,
                body,
            });
        }
        out
    }

    /// The body of `line`, whose containers' prefixes end at `pos`; moves `pos` past an indented
    /// code block's indent.
    fn body_of(&self, line: &Range<usize>, pos: &mut usize) -> Body {
        let text = self.text;
        let bytes = text.as_bytes();
        let relative = |range: Range<usize>| range.start - line.start..range.end - line.start;
        if self.rules.iter().any(|&at| line.contains(&at)) {
            return Body::Rule;
        }
        let overlaps = |range: &Range<usize>| range.start <= line.end && range.end > line.start;
        if let Some((range, fenced)) = self.code_blocks.iter().find(|(range, _)| overlaps(range)) {
            if !fenced {
                let mut columns = 0;
                while columns < 4 && *pos < line.end {
                    match bytes[*pos] {
                        b' ' => columns += 1,
                        b'\t' => columns = 4,
                        _ => break,
                    }
                    *pos += 1;
                }
                return Body::Code;
            }
            let fence_start = {
                let mut at = *pos;
                while at < line.end && at - *pos < 3 && bytes[at] == b' ' {
                    at += 1;
                }
                at
            };
            if line_start(text, range.start) == line.start {
                let fence = bytes[fence_start..line.end]
                    .iter()
                    .take_while(|b| matches!(b, b'`' | b'~'))
                    .count();
                let rest = fence_start + fence..line.end;
                let trimmed = text[rest.clone()].trim();
                let info = (!trimmed.is_empty()).then(|| {
                    let start = rest.start
                        + (text[rest.clone()].len() - text[rest.clone()].trim_start().len());
                    relative(start..start + trimmed.len())
                });
                return Body::Fence { info };
            }
            let is_last = line_start(text, range.end.saturating_sub(1)) == line.start;
            let rest = text[fence_start..line.end].trim_end();
            if is_last
                && rest.len() >= 3
                && (rest.bytes().all(|b| b == b'`') || rest.bytes().all(|b| b == b'~'))
            {
                return Body::Fence { info: None };
            }
            return Body::Code;
        }
        if let Some((range, aligns)) = self.tables.iter().find(|(range, _)| overlaps(range)) {
            // Only the first two rows differ: the header, then the delimiter.
            let first = line_start(text, range.start);
            let second = text[first..]
                .find('\n')
                .map_or(text.len(), |at| first + at + 1);
            let index = if line.start == first {
                0
            } else if line.start == second {
                1
            } else {
                2
            };
            if index == 1 {
                return Body::TableDelimiter {
                    aligns: Rc::clone(aligns),
                };
            }
            let (cells, pipes) = table_cells(text, *pos..line.end);
            return Body::TableRow {
                header: index == 0,
                cells: cells.into_iter().map(relative).collect(),
                pipes: pipes.into_iter().map(|pipe| pipe - line.start).collect(),
                aligns: Rc::clone(aligns),
            };
        }
        let content = &text[*pos..line.end];
        let start = *pos + (content.len() - content.trim_start().len());
        let only = start..*pos + content.trim_end().len();
        let image = self.images.iter().find(|image| {
            image.whole == only
                || (image.whole.start == only.start + 1
                    && bytes.get(image.whole.end) == Some(&b']')
                    && self.links.iter().any(|link| link.range == only))
        });
        if let Some(image) = image {
            return Body::Image {
                dest: image.dest.clone(),
                alt: relative(image.alt.clone()),
                size: (None, None),
            };
        }
        if let Some((dest, alt, size)) = html_image(text, only) {
            return Body::Image {
                dest,
                alt: relative(alt),
                size,
            };
        }
        Body::Text
    }

    /// A paragraph's lines start at their text: leading spaces (an indented continuation line)
    /// are not shown, as GitHub does not show them.
    fn hide_leading_space(&mut self, range: Range<usize>) {
        for line in line_ranges(self.text, range) {
            let spaces = self.text[line.clone()]
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            self.push(line.start..line.start + spaces, SpanKind::Hidden);
        }
    }

    /// A GitHub alert: a quote whose first line is `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`,
    /// `[!WARNING]` or `[!CAUTION]` shows the kind, bold, without its brackets.
    fn alert(&mut self, range: Range<usize>) {
        let first = line_ranges(self.text, range)[0].clone();
        let line = &self.text[first.clone()];
        let Some(open) = line.find("[!") else {
            return;
        };
        if !line[..open]
            .bytes()
            .all(|b| matches!(b, b'>' | b' ' | b'\t'))
        {
            return;
        }
        let rest = &line[open + 2..];
        let Some(close) = rest.find(']') else {
            return;
        };
        let kind = &rest[..close];
        let known = ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"]
            .iter()
            .any(|name| kind.eq_ignore_ascii_case(name));
        if !known || !rest[close + 1..].trim().is_empty() {
            return;
        }
        let kind_start = first.start + open + 2;
        self.push(first.start + open..kind_start, SpanKind::Hidden);
        self.push(kind_start..kind_start + kind.len(), SpanKind::Bold);
        self.push(
            kind_start + kind.len()..kind_start + kind.len() + 1,
            SpanKind::Hidden,
        );
    }

    /// GFM's extended autolinks: bare `www.` and `http(s)://` URLs and email addresses in prose,
    /// outside links, code spans, raw HTML, footnotes and math.
    fn autolinks(&mut self) {
        let mut taken = self
            .links
            .iter()
            .map(|link| link.range.clone())
            .chain(self.unlinked.iter().cloned())
            .chain(self.lexical.iter().cloned())
            .chain(
                self.spans
                    .iter()
                    .filter(|span| span.kind == SpanKind::Dim)
                    .map(|span| span.range.clone()),
            )
            .collect::<Vec<_>>();
        for scope in std::mem::take(&mut self.prose) {
            for (range, dest) in autolink_literals(self.text, scope) {
                if taken
                    .iter()
                    .any(|other| other.start < range.end && range.start < other.end)
                {
                    continue;
                }
                taken.push(range.clone());
                self.push(range.clone(), SpanKind::Link);
                self.links.push(LinkSpan {
                    range,
                    dest,
                    wiki: false,
                });
            }
        }
    }

    fn finish(mut self) -> BlockSpans {
        self.autolinks();
        let lines = self.line_blocks();
        let lexical = merge(self.lexical);
        let hidden = merge(
            self.spans
                .iter()
                .filter(|span| span.kind == SpanKind::Hidden)
                .map(|span| span.range.clone())
                .collect(),
        );
        // Nothing inside a footnote or math range keeps its own style; markup is shown there.
        let hidden = hidden
            .into_iter()
            .flat_map(|range| subtract(range, &lexical))
            .collect::<Vec<_>>();
        let mut spans = hidden
            .iter()
            .map(|range| Span {
                range: range.clone(),
                kind: SpanKind::Hidden,
            })
            .collect::<Vec<_>>();
        spans.extend(lexical.iter().map(|range| Span {
            range: range.clone(),
            kind: SpanKind::Dim,
        }));
        for span in self.spans {
            if span.kind == SpanKind::Hidden {
                continue;
            }
            for piece in subtract(span.range, &lexical) {
                for piece in subtract(piece, &hidden) {
                    spans.push(Span {
                        range: piece,
                        kind: span.kind,
                    });
                }
            }
        }
        spans.sort_by_key(|span| (span.range.start, span.range.end));
        let reach = spans
            .iter()
            .scan(0, |furthest, span| {
                *furthest = span.range.end.max(*furthest);
                Some(*furthest)
            })
            .collect();
        BlockSpans {
            spans,
            reach,
            links: self.links,
            lines,
        }
    }
}

/// The `<img>` tag that is all of `only` (a line's content): its `src`, the bytes of its `alt`
/// value and its pixel `width` and `height`. Percentages are left to the natural size.
fn html_image(text: &str, only: Range<usize>) -> Option<(String, Range<usize>, ImageSize)> {
    let source = &text[only.clone()];
    if !source.get(..4)?.eq_ignore_ascii_case("<img") {
        return None;
    }
    let mut tokens = crate::preview::html::tokenize(source)
        .into_iter()
        .filter(|token| !matches!(token, Token::Text(text) if text.trim().is_empty()));
    let (Some(Token::Start { name, attrs, .. }), None) = (tokens.next(), tokens.next()) else {
        return None;
    };
    if name != "img" {
        return None;
    }
    let src = crate::preview::html::attr(&attrs, "src")?.trim().to_owned();
    if src.is_empty() {
        return None;
    }
    // Where the alt value sits in the source: after `alt=`, inside its quotes if it has them.
    let alt = crate::preview::html::attr(&attrs, "alt")
        .filter(|alt| !alt.is_empty())
        .and_then(|alt| {
            let lower = source.to_ascii_lowercase();
            let key = lower.find("alt=")? + 4;
            let quoted = matches!(source.as_bytes().get(key), Some(b'"' | b'\''));
            let start = key + usize::from(quoted);
            let found = source[start..].find(alt).filter(|&at| at == 0)?;
            Some(only.start + start + found..only.start + start + found + alt.len())
        })
        .unwrap_or(only.start..only.start);
    let pixels = |name: &str| {
        let value = crate::preview::html::attr(&attrs, name)?.trim();
        value
            .strip_suffix("px")
            .unwrap_or(value)
            .parse::<u32>()
            .ok()
    };
    Some((src, alt, (pixels("width"), pixels("height"))))
}

/// GFM's extended autolinks in `range` of `text`, with the destination each opens: `www.` and
/// `http://` / `https://` URLs (a valid domain with at least one period, then a path up to the
/// next space or `<`, without trailing punctuation or an unbalanced `)`), and email addresses.
fn autolink_literals(text: &str, range: Range<usize>) -> Vec<(Range<usize>, String)> {
    let bytes = text.as_bytes();
    let end = range.end.min(text.len());
    let mut found: Vec<(Range<usize>, String)> = Vec::new();
    let mut at = range.start;
    while at < end {
        let boundary = at == range.start
            || matches!(
                bytes[at - 1],
                b' ' | b'\t' | b'\r' | b'\n' | b'*' | b'_' | b'~' | b'('
            );
        let starts = |prefix: &[u8]| {
            bytes.len() >= at + prefix.len()
                && bytes[at..at + prefix.len()].eq_ignore_ascii_case(prefix)
        };
        let scheme = if !boundary {
            None
        } else if starts(b"https://") {
            Some(8)
        } else if starts(b"http://") {
            Some(7)
        } else if starts(b"www.") {
            Some(0)
        } else {
            None
        };
        if let Some(skip) = scheme
            && let Some(length) = url_length(bytes, at + skip, end)
        {
            let url = &text[at..at + skip + length];
            let dest = if skip == 0 {
                format!("http://{url}")
            } else {
                url.to_owned()
            };
            found.push((at..at + skip + length, dest));
            at += skip + length;
            continue;
        }
        at += 1;
    }
    // Email addresses: `local@domain.tld`, outside the URLs found above.
    for (offset, _) in text[range.start..end].match_indices('@') {
        let at = range.start + offset;
        if found.iter().any(|(url, _)| url.contains(&at)) {
            continue;
        }
        let local = bytes[range.start..at]
            .iter()
            .rev()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-' | b'_'))
            .count();
        let mut domain = bytes[at + 1..end]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            .count();
        while domain > 0 && bytes[at + domain] == b'.' {
            domain -= 1;
        }
        let host = &text[at + 1..at + 1 + domain];
        if local == 0 || !host.contains('.') || host.ends_with(['-', '_']) {
            continue;
        }
        let whole = at - local..at + 1 + domain;
        found.push((whole.clone(), format!("mailto:{}", &text[whole])));
    }
    found.sort_by_key(|(range, _)| range.start);
    found
}

/// The length of a URL's domain and path starting at `start`, or `None` when it has no valid
/// domain.
fn url_length(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let domain = bytes[start..end]
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        .count();
    let host = &bytes[start..start + domain];
    let segments = host
        .split(|b| *b == b'.')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    // At least one period, and no underscore in the last two segments.
    if segments.len() < 2
        || segments[segments.len() - 2..]
            .iter()
            .any(|segment| segment.contains(&b'_'))
    {
        return None;
    }
    let mut length = domain
        + bytes[start + domain..end]
            .iter()
            .take_while(|b| !b.is_ascii_whitespace() && **b != b'<')
            .count();
    // Trailing punctuation is not part of the link, nor an unbalanced `)`.
    loop {
        let url = &bytes[start..start + length];
        let Some(&last) = url.last() else {
            break;
        };
        let punctuation = matches!(
            last,
            b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' | b'\'' | b'"'
        );
        let unbalanced = last == b')'
            && url.iter().filter(|b| **b == b')').count()
                > url.iter().filter(|b| **b == b'(').count();
        if !punctuation && !unbalanced {
            break;
        }
        length -= 1;
    }
    (length > 0).then_some(length)
}

/// A list item's marker line, measured.
struct ItemPrefix {
    /// Absolute; `Ordered` and `Task` positions are made line-relative by `relative_to`.
    marker: Marker,
    /// Where the item's content starts, before a task box.
    content: usize,
    /// The end of the prefix a rendered line hides: past the marker, its gap and a task box.
    prefix_end: usize,
}

impl ItemPrefix {
    /// How far the item's later lines are indented, measured from `pos` (the item's column).
    fn width_from(&self, pos: usize) -> usize {
        self.content.saturating_sub(pos)
    }
}

impl Marker {
    fn relative_to(self, line_start: usize) -> Self {
        match self {
            Self::Bullet => Self::Bullet,
            Self::Ordered(range) => Self::Ordered(range.start - line_start..range.end - line_start),
            Self::Task { checked, inner } => Self::Task {
                checked,
                inner: inner - line_start,
            },
        }
    }
}

/// The marker at `marker` on a line ending at `line_end`, its gap and an optional task box.
fn item_prefix(
    text: &str,
    marker: usize,
    line_end: usize,
    task: Option<&(bool, Range<usize>)>,
) -> ItemPrefix {
    let bytes = text.as_bytes();
    let (mut kind, marker_end) = if matches!(bytes.get(marker), Some(b'-' | b'*' | b'+')) {
        (Marker::Bullet, marker + 1)
    } else {
        let digits = bytes[marker..line_end]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let end = (marker + digits + 1).min(line_end);
        (Marker::Ordered(marker..end), end)
    };
    let gap = bytes[marker_end..line_end]
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    // Nothing after the marker, or more than four spaces (indented code): one column of gap.
    let content = if marker_end + gap == line_end || gap > 4 {
        marker_end + 1
    } else {
        marker_end + gap
    };
    let mut prefix_end = if marker_end + gap == line_end {
        line_end
    } else {
        content
    };
    if let Some((checked, range)) = task.filter(|(_, range)| range.start >= marker_end) {
        kind = Marker::Task {
            checked: *checked,
            inner: range.start + 1,
        };
        prefix_end = range.end;
        if prefix_end < line_end && matches!(bytes[prefix_end], b' ' | b'\t') {
            prefix_end += 1;
        }
    }
    ItemPrefix {
        marker: kind,
        content,
        prefix_end,
    }
}

/// The cells (whitespace trimmed) and pipes of the table row `line`: unescaped pipes outside
/// code spans separate cells, and a leading or trailing pipe opens or closes the row.
fn table_cells(text: &str, line: Range<usize>) -> (Vec<Range<usize>>, Vec<usize>) {
    let pipes = pipe_positions(&text[line.clone()])
        .into_iter()
        .map(|pipe| line.start + pipe)
        .collect::<Vec<_>>();
    let mut bounds = vec![line.start];
    bounds.extend(pipes.iter().flat_map(|&pipe| [pipe, pipe + 1]));
    bounds.push(line.end);
    let mut cells = bounds
        .chunks(2)
        .map(|pair| {
            let raw = &text[pair[0]..pair[1]];
            let start = pair[0] + (raw.len() - raw.trim_start().len());
            start..(pair[0] + raw.trim_end().len()).max(start)
        })
        .collect::<Vec<_>>();
    if !pipes.is_empty() && text[line.start..pipes[0]].trim().is_empty() {
        cells.remove(0);
    }
    if let Some(&last) = pipes.last()
        && cells.len() > 1
        && text[last + 1..line.end].trim().is_empty()
    {
        cells.pop();
    }
    (cells, pipes)
}

/// Byte offsets of the column separators in one table line: every unescaped pipe. As in GFM, a
/// pipe inside a code span still separates cells unless it is escaped.
fn pipe_positions(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut pipes = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'|' => {
                pipes.push(index);
                index += 1;
            }
            _ => index += 1,
        }
    }
    pipes
}

/// `ranges` sorted, with overlapping and touching ones joined.
fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.retain(|range| range.start < range.end);
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// The parts of `range` outside every one of the sorted, disjoint `holes`.
fn subtract(range: Range<usize>, holes: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut start = range.start;
    for hole in holes {
        if hole.end <= start || hole.start >= range.end {
            continue;
        }
        if hole.start > start {
            pieces.push(start..hole.start);
        }
        start = start.max(hole.end);
    }
    if start < range.end {
        pieces.push(start..range.end);
    }
    pieces
}

/// Footnotes and math in a paragraph's or heading's source `range`, which the preview's options
/// do not parse (phase 2 plan, deviation 4): `[^label]` references, `[^label]:` definitions to the
/// paragraph's end, `$$…$$` and `$…$`. Code spans and backslash escapes are skipped.
fn lexical_dims(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let end = range.end.min(text.len());
    let mut found = Vec::new();
    let mut at = range.start;
    while at < end {
        match bytes[at] {
            b'\\' => at += 2,
            b'`' => {
                let run = bytes[at..end].iter().take_while(|b| **b == b'`').count();
                // The matching closing run, else the backticks are literal.
                let close = (at + run..end).find(|&probe| {
                    bytes[probe..end].starts_with(&bytes[at..at + run])
                        && bytes.get(probe + run) != Some(&b'`')
                        && bytes[probe - 1] != b'`'
                });
                at = close.map_or(at + run, |close| close + run);
            }
            b'[' if bytes.get(at + 1) == Some(&b'^') => {
                let label = bytes[at + 2..end]
                    .iter()
                    .take_while(|b| **b != b']' && !b.is_ascii_whitespace())
                    .count();
                let close = at + 2 + label;
                if label == 0 || bytes.get(close) != Some(&b']') {
                    at += 1;
                    continue;
                }
                let line_lead = &text[line_start(text, at)..at];
                if bytes.get(close + 1) == Some(&b':')
                    && line_lead.len() <= 3
                    && line_lead.bytes().all(|b| b == b' ')
                {
                    found.push(at..trim_line_end(text, end));
                    break;
                }
                found.push(at..close + 1);
                at = close + 1;
            }
            b'$' if bytes.get(at + 1) == Some(&b'$') => match text[at + 2..end].find("$$") {
                Some(offset) => {
                    let close = at + 2 + offset + 2;
                    found.push(at..close);
                    at = close;
                }
                None => at += 2,
            },
            b'$' => match inline_math_end(bytes, at, end) {
                Some(close) => {
                    found.push(at..close);
                    at = close;
                }
                None => at += 1,
            },
            _ => at += 1,
        }
    }
    found
}

/// The end (past the closing `$`) of inline math opening at `open`: the opening `$` is not
/// followed by whitespace, and the closing one is not preceded by whitespace nor followed by a
/// digit, so `$5 and $6` is not math.
fn inline_math_end(bytes: &[u8], open: usize, end: usize) -> Option<usize> {
    let first = *bytes.get(open + 1)?;
    if open + 1 >= end || first.is_ascii_whitespace() || first == b'$' {
        return None;
    }
    let mut at = open + 1;
    while at < end {
        match bytes[at] {
            b'\\' => at += 2,
            b'$' if !bytes[at - 1].is_ascii_whitespace()
                && !bytes.get(at + 1).is_some_and(u8::is_ascii_digit) =>
            {
                return Some(at + 1);
            }
            _ => at += 1,
        }
    }
    None
}

/// `end` without trailing line endings.
fn trim_line_end(text: &str, end: usize) -> usize {
    end - (text[..end].len() - text[..end].trim_end_matches(['\r', '\n']).len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_refs(_: &str) -> Option<String> {
        None
    }

    /// One character per byte: 'H' hidden, else the kinds covering it ('b', 'i', 's', 'c', 'L',
    /// 'h' for a heading, 'd' dim), several for overlapping spans, '.' for none.
    fn kinds(text: &str) -> String {
        kinds_with(text, &no_refs)
    }

    fn kinds_with(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> String {
        let block = parse_block(text, refs);
        let mut out = vec![String::new(); text.len()];
        for span in &block.spans {
            let code = match span.kind {
                SpanKind::Hidden => 'H',
                SpanKind::Bold => 'b',
                SpanKind::Italic => 'i',
                SpanKind::Strike => 's',
                SpanKind::Code => 'c',
                SpanKind::Link => 'L',
                SpanKind::Heading(_) => 'h',
                SpanKind::Dim => 'd',
                SpanKind::CodeBlock | SpanKind::Quote => unreachable!("set by styles only"),
            };
            for slot in &mut out[span.range.clone()] {
                slot.push(code);
            }
        }
        out.into_iter()
            .map(|slot| match slot.len() {
                0 => ".".to_owned(),
                1 => slot,
                _ => format!("({slot})"),
            })
            .collect()
    }

    #[test]
    fn strong_hides_its_markers() {
        assert_eq!(kinds("a **b** c"), "..HHbHH..");
    }

    #[test]
    fn nested_emphasis_is_bold_and_italic() {
        let shown = kinds("***x***");
        assert!(
            shown.starts_with("HHH") && shown.ends_with("HHH"),
            "{shown}"
        );
        assert!(shown.contains('b') && shown.contains('i'), "{shown}");
    }

    #[test]
    fn strikethrough_hides_markers() {
        assert_eq!(kinds("~~a~~"), "HHsHH");
    }

    #[test]
    fn inline_code_hides_backticks() {
        assert_eq!(kinds("`x`"), "HcH");
        assert_eq!(kinds("``a`b``"), "HHcccHH");
    }

    #[test]
    fn inline_link_shows_only_its_text() {
        let block = parse_block("[a](u)", &no_refs);
        assert_eq!(kinds("[a](u)"), "HLHHHH");
        assert_eq!(
            block.links,
            vec![LinkSpan {
                range: 0..6,
                dest: "u".into(),
                wiki: false
            }]
        );
    }

    #[test]
    fn reference_links_resolve_through_refs() {
        let refs = |label: &str| (label == "r").then(|| "https://x".to_owned());
        assert_eq!(kinds_with("[a][r]", &refs), "HLHHHH");
        assert_eq!(parse_block("[a][r]", &refs).links[0].dest, "https://x");
    }

    #[test]
    fn autolink_hides_angle_brackets() {
        assert_eq!(kinds("<http://a>"), "HLLLLLLLLH");
    }

    #[test]
    fn atx_heading_splits_marker_and_text() {
        assert_eq!(kinds("## Hi ##"), "HHHhhHHH");
    }

    #[test]
    fn setext_heading_hides_its_underline() {
        assert_eq!(kinds("Hi\n=="), "hh.HH");
    }

    #[test]
    fn emphasis_inside_a_heading_is_heading_and_bold() {
        assert_eq!(kinds("# a **b**"), "HHhhHH(bh)HH");
    }

    #[test]
    fn inline_html_is_dim() {
        assert_eq!(kinds("a<br>"), ".dddd");
    }

    #[test]
    fn crlf_offsets_are_byte_exact() {
        assert_eq!(kinds("**a**\r\nb"), "HHbHH...");
        assert_eq!(kinds("# a\r\n# b"), "HHh..HHh");
    }

    #[test]
    fn multibyte_text_keeps_byte_offsets() {
        // 'é' is two bytes.
        assert_eq!(kinds("é **b**"), "...HHbHH");
    }

    #[test]
    fn a_setext_heading_whose_text_starts_with_a_hash_is_setext() {
        assert_eq!(kinds("#x\n=="), "hh.HH");
    }

    #[test]
    fn wikilink_shows_its_target() {
        let block = parse_block("[[Note]]", &no_refs);
        assert_eq!(kinds("[[Note]]"), "HHLLLLHH");
        assert!(block.links[0].wiki);
        assert_eq!(block.links[0].range, 0..8);
    }

    #[test]
    fn piped_wikilink_shows_its_alias() {
        assert_eq!(kinds("[[Note|Alias]]"), "HHHHHHHLLLLLHH");
    }

    #[test]
    fn wikilink_with_a_heading_shows_it() {
        assert_eq!(kinds("[[Note#Part]]"), "HHLLLLLLLLLHH");
    }

    #[test]
    fn empty_link_text_hides_nothing() {
        let block = parse_block("[](u)", &no_refs);
        assert_eq!(kinds("[](u)"), ".....");
        assert_eq!(block.links.len(), 1);
    }

    #[test]
    fn bold_inside_a_link_is_both() {
        assert_eq!(kinds("[**a**](u)"), "HHH(bL)HHHHHH");
    }

    #[test]
    fn footnote_reference_is_dim() {
        assert_eq!(kinds("a[^1] b"), ".dddd..");
    }

    #[test]
    fn footnote_definition_is_dim() {
        let text = "[^1]: the note";
        assert_eq!(kinds(text), "d".repeat(text.len()));
    }

    #[test]
    fn a_resolved_footnote_reference_is_dim_not_a_link() {
        let refs = |label: &str| (label == "^1").then(|| "note".to_owned());
        let text = "a[^1] b";
        assert_eq!(kinds_with(text, &refs), ".dddd..");
    }

    #[test]
    fn inline_math_is_dim_and_unstyled_inside() {
        assert_eq!(kinds("x $a*b*c$ y"), "..ddddddd..");
    }

    #[test]
    fn display_math_across_lines_is_dim() {
        let text = "$$\na*b*c\n$$";
        assert_eq!(kinds(text), "d".repeat(text.len()));
    }

    #[test]
    fn a_dollar_amount_is_not_math() {
        let text = "costs $5 and $6";
        assert_eq!(kinds(text), ".".repeat(text.len()));
    }

    #[test]
    fn code_blocks_get_no_inline_spans() {
        assert_eq!(kinds("```\n**a**\n```"), ".".repeat(13));
        assert_eq!(kinds("    **a**"), ".".repeat(9));
    }

    #[test]
    fn table_cells_keep_inline_spans() {
        let table = "| **a** | b |\n|---|---|\n| `c` | d |";
        assert_eq!(kinds(table), format!("..HHbHH{}HcH......", ".".repeat(19)));
    }

    fn lines(text: &str) -> Vec<LineBlock> {
        parse_block(text, &no_refs).lines
    }

    fn item(depth: u8, ordered: bool, marker: Option<Marker>) -> Container {
        Container::Item {
            depth,
            ordered,
            marker,
        }
    }

    #[test]
    fn bullet_items_have_a_marker_and_a_hidden_prefix() {
        assert_eq!(
            lines("- a\n- b"),
            vec![
                LineBlock {
                    start: 0,
                    prefix: 0..2,
                    containers: vec![item(1, false, Some(Marker::Bullet))],
                    body: Body::Text,
                },
                LineBlock {
                    start: 4,
                    prefix: 0..2,
                    containers: vec![item(1, false, Some(Marker::Bullet))],
                    body: Body::Text,
                },
            ]
        );
    }

    #[test]
    fn nested_items_carry_their_depth() {
        let found = lines("- a\n  - b\n    - c");
        assert_eq!(found[1].prefix, 0..4);
        assert_eq!(
            found[1].containers,
            vec![item(1, false, None), item(2, false, Some(Marker::Bullet))]
        );
        assert_eq!(found[2].prefix, 0..6);
        assert_eq!(found[2].containers.len(), 3);
    }

    #[test]
    fn an_items_second_paragraph_has_no_marker() {
        let found = lines("- a\n\n  more");
        let last = found.last().unwrap();
        assert_eq!(last.start, 5);
        assert_eq!(last.prefix, 0..2);
        assert_eq!(last.containers, vec![item(1, false, None)]);
    }

    #[test]
    fn ordered_markers_keep_their_number_range() {
        let found = lines("12. a\n13) b");
        assert_eq!(
            found[0].containers,
            vec![item(1, true, Some(Marker::Ordered(0..3)))]
        );
        assert_eq!(found[0].prefix, 0..4);
    }

    #[test]
    fn a_checked_task_is_struck_and_dim() {
        assert_eq!(kinds("- [x] done"), format!("......{}", "(sd)".repeat(4)));
        assert_eq!(
            lines("- [x] done")[0].containers,
            vec![item(
                1,
                false,
                Some(Marker::Task {
                    checked: true,
                    inner: 3
                })
            )]
        );
    }

    #[test]
    fn an_unchecked_task_records_its_inner_byte() {
        let found = lines("> - [ ] todo");
        assert_eq!(found[0].prefix, 0..8);
        assert_eq!(
            found[0].containers,
            vec![
                Container::Quote,
                item(
                    1,
                    false,
                    Some(Marker::Task {
                        checked: false,
                        inner: 5
                    })
                ),
            ]
        );
        assert_eq!(kinds("- [ ] todo"), ".".repeat(10));
    }

    #[test]
    fn a_lazy_continuation_keeps_the_items_containers() {
        let found = lines("> - a\nlazy");
        assert_eq!(found[1].prefix, 0..0);
        assert_eq!(
            found[1].containers,
            vec![Container::Quote, item(1, false, None)]
        );
    }

    #[test]
    fn quote_lines_carry_their_depth() {
        let found = lines("> a\n> > b");
        assert_eq!(found[0].containers, vec![Container::Quote]);
        assert_eq!(found[0].prefix, 0..2);
        assert_eq!(
            found[1].containers,
            vec![Container::Quote, Container::Quote]
        );
        assert_eq!(found[1].prefix, 0..4);
    }

    #[test]
    fn a_list_inside_a_quote_has_both_containers() {
        let found = lines("> 1. a\n>    b");
        assert_eq!(
            found[0].containers,
            vec![Container::Quote, item(1, true, Some(Marker::Ordered(2..4)))]
        );
        assert_eq!(found[1].prefix, 0..5);
    }

    #[test]
    fn fenced_code_marks_fences_and_code_lines() {
        let found = lines("```rust\nlet a;\n```");
        assert_eq!(
            found
                .iter()
                .map(|line| line.body.clone())
                .collect::<Vec<_>>(),
            vec![
                Body::Fence { info: Some(3..7) },
                Body::Code,
                Body::Fence { info: None },
            ]
        );
    }

    #[test]
    fn an_unclosed_fence_runs_to_the_block_end() {
        let found = lines("~~~\na\nb");
        assert_eq!(found.len(), 3);
        assert_eq!(found[2].body, Body::Code);
    }

    #[test]
    fn indented_code_hides_four_columns() {
        let found = lines("    code\n\tmore");
        assert_eq!(found[0].prefix, 0..4);
        assert_eq!(found[0].body, Body::Code);
        assert_eq!(found[1].prefix, 0..1);
    }

    #[test]
    fn fenced_code_inside_a_list_item_keeps_the_item_prefix() {
        let found = lines("- a\n  ```\n  x\n  ```");
        assert_eq!(found[1].prefix, 0..2);
        assert_eq!(found[1].body, Body::Fence { info: None });
        assert_eq!(found[2].body, Body::Code);
        assert_eq!(found[2].containers, vec![item(1, false, None)]);
        assert_eq!(found[3].body, Body::Fence { info: None });
    }

    #[test]
    fn table_rows_have_cells_pipes_and_aligns() {
        let found = lines("| a | bb |\n|:--|--:|\n| c | d |");
        let aligns: Rc<[Align]> = Rc::from([Align::Left, Align::Right]);
        assert_eq!(
            found[0].body,
            Body::TableRow {
                header: true,
                cells: vec![2..3, 6..8],
                pipes: vec![0, 4, 9],
                aligns: Rc::clone(&aligns),
            }
        );
        assert_eq!(
            found[1].body,
            Body::TableDelimiter {
                aligns: Rc::clone(&aligns)
            }
        );
        assert!(
            matches!(&found[2].body, Body::TableRow { header: false, cells, .. } if cells == &vec![2..3, 6..7])
        );
    }

    #[test]
    fn escaped_pipes_are_not_separators_even_in_code() {
        let found = lines("a | `x\\|y` | b\\|c\n--|--|--");
        let Body::TableRow { cells, pipes, .. } = &found[0].body else {
            panic!("{found:?}");
        };
        assert_eq!(cells, &vec![0..1, 4..10, 13..17]);
        assert_eq!(pipes, &vec![2, 11]);
    }

    #[test]
    fn a_rule_is_a_rule_and_a_setext_underline_is_not() {
        assert_eq!(lines("a\n\n---")[0].body, Body::Rule);
        assert!(lines("a\n---").is_empty());
    }

    #[test]
    fn an_image_alone_on_its_line_is_an_image_line() {
        assert_eq!(
            lines("![cat](c.png)")[0].body,
            Body::Image {
                dest: "c.png".into(),
                alt: 2..5,
                size: (None, None),
            }
        );
        assert_eq!(
            lines("[![cat](c.png)](big.png)")[0].body,
            Body::Image {
                dest: "c.png".into(),
                alt: 3..6,
                size: (None, None),
            }
        );
        assert_eq!(
            lines("- ![](c.png)")[0].body,
            Body::Image {
                dest: "c.png".into(),
                alt: 4..4,
                size: (None, None),
            }
        );
    }

    #[test]
    fn an_html_img_alone_on_its_line_is_an_image_line() {
        let text = "<img src=\"docs/hero.png\" alt=\"The hero\" width=\"900\">";
        assert_eq!(
            lines(text)[0].body,
            Body::Image {
                dest: "docs/hero.png".into(),
                alt: 30..38,
                size: (Some(900), None),
            }
        );
        assert_eq!(&text[30..38], "The hero");
        // In a paragraph too, and with its size in pixels.
        let found = lines("text\n<IMG SRC='a.svg' width=96px height=\"96\" />");
        assert_eq!(
            found.last().unwrap().body,
            Body::Image {
                dest: "a.svg".into(),
                alt: 0..0,
                size: (Some(96), Some(96)),
            }
        );
        // Not an image line: another tag, text around it, or no source.
        for text in [
            "<div><img src=\"a.png\"></div>",
            "see <img src=\"a.png\">",
            "<img alt=\"x\">",
        ] {
            assert!(
                lines(text)
                    .iter()
                    .all(|line| !matches!(line.body, Body::Image { .. })),
                "{text}"
            );
        }
    }

    #[test]
    fn extended_autolinks_link_bare_urls_and_emails() {
        let text =
            "See www.commonmark.org/help, https://example.com/a_(b)?c=1. Mail foo@bar.example.com.";
        let block = parse_block(text, &no_refs);
        let links = block
            .links
            .iter()
            .map(|link| (&text[link.range.clone()], link.dest.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            links,
            vec![
                ("www.commonmark.org/help", "http://www.commonmark.org/help"),
                (
                    "https://example.com/a_(b)?c=1",
                    "https://example.com/a_(b)?c=1"
                ),
                ("foo@bar.example.com", "mailto:foo@bar.example.com"),
            ]
        );
        assert!(kinds(text).contains("LLLL"));
    }

    #[test]
    fn autolinks_skip_code_links_and_invalid_domains() {
        for text in [
            "`https://example.com`",
            "[x](https://a.example) and [https://b.example](u)",
            "http://localhost and wwwexample.com",
            "a@b and x@y_z.com_",
            "<span>https://in.html</span>",
        ] {
            let block = parse_block(text, &no_refs);
            assert!(
                block
                    .links
                    .iter()
                    .all(|link| !text[link.range.clone()].contains("in.html")
                        && !link.dest.contains("localhost")
                        && !link.dest.starts_with("mailto")),
                "{text}: {:?}",
                block.links
            );
        }
        // Inside a link's text, the link stays the only one.
        assert_eq!(
            parse_block("[https://b.example](u)", &no_refs).links.len(),
            1
        );
    }

    #[test]
    fn a_github_alert_shows_its_kind_bold() {
        assert_eq!(kinds("> [!NOTE]\n> Info."), "..HHbbbbH........");
        assert!(!kinds("> [!NOPE]\n> x").contains('b'));
        assert!(!kinds("> see [!NOTE]").contains('b'));
    }

    #[test]
    fn a_backslash_hard_break_hides_its_backslash() {
        assert_eq!(kinds("one\\\ntwo"), "...H....");
    }

    #[test]
    fn an_escaped_pipe_in_a_cell_hides_its_backslash() {
        let table = "| `a\\|b` |\n|---|\n";
        assert!(kinds(table).starts_with("..HcHccH"), "{}", kinds(table));
    }

    #[test]
    fn an_indented_paragraph_line_hides_its_leading_spaces() {
        assert_eq!(kinds("one\n   two"), "....HHH...");
    }

    #[test]
    fn an_image_inside_text_shows_its_alt_text_dimmed() {
        assert_eq!(
            kinds("see ![cat](c.png) here"),
            format!("....HH{}HHHHHHHH.....", "(di)".repeat(3))
        );
        // A badge: an image as a link's text.
        let badge = kinds("[![cat](c.png)](u)");
        assert!(badge.starts_with("HHH"), "{badge}");
        assert!(
            badge.contains("(Ldi)") || badge.contains("(dLi)") || badge.contains("(diL)"),
            "{badge}"
        );
    }

    #[test]
    fn an_image_inside_text_is_not() {
        assert!(lines("see ![cat](c.png) here").is_empty());
    }

    #[test]
    fn overlapping_finds_long_spans_that_start_earlier() {
        let text = "<div>\nhtml\n</div>\n";
        let block = parse_block("**a** b *c*", &no_refs);
        let found = |bytes: Range<usize>| {
            block
                .overlapping(bytes)
                .map(|span| span.range.clone())
                .collect::<Vec<_>>()
        };
        // `**a** b *c*`: markers 0..2, 3..5, 8..9, 10..11; bold 2..3, italic 9..10.
        assert_eq!(found(9..10), vec![9..10]);
        assert_eq!(found(3..9), vec![3..5, 8..9]);
        assert_eq!(found(5..8), Vec::<Range<usize>>::new());
        // A dimmed HTML block spans every line: a line in its middle still finds it.
        let html = parse_block(text, &no_refs);
        assert_eq!(
            html.overlapping(7..8)
                .map(|span| span.kind)
                .collect::<Vec<_>>(),
            vec![SpanKind::Dim]
        );
    }

    #[test]
    fn crlf_prefixes_and_cells_are_byte_exact() {
        let found = lines("- a\r\n- b\r\n\r\n| x | y |\r\n|---|---|\r\n");
        assert_eq!(found[1].start, 5);
        assert_eq!(found[1].prefix, 0..2);
        let row = found
            .iter()
            .find(|line| matches!(line.body, Body::TableRow { .. }))
            .unwrap();
        assert!(
            matches!(&row.body, Body::TableRow { cells, pipes, .. } if cells == &vec![2..3, 6..7] && pipes == &vec![0, 4, 8])
        );
    }

    #[test]
    fn list_items_and_quotes_keep_inline_spans() {
        assert_eq!(kinds("- **a**"), "..HHbHH");
        assert_eq!(kinds("> *a*"), "..HiH");
    }

    #[test]
    fn hidden_ranges_never_overlap_other_spans() {
        for text in [
            "# a **b** [c](d) `e` ~~f~~ [[g|h]] <i>",
            "***x*** [**y**](z) ## t ##",
            "x $a*b$ [^1] **c**",
        ] {
            let block = parse_block(text, &no_refs);
            let hidden = block
                .spans
                .iter()
                .filter(|span| span.kind == SpanKind::Hidden)
                .collect::<Vec<_>>();
            for span in block.spans.iter().filter(|s| s.kind != SpanKind::Hidden) {
                for hide in &hidden {
                    assert!(
                        span.range.end <= hide.range.start || span.range.start >= hide.range.end,
                        "{text:?}: {span:?} overlaps {hide:?}"
                    );
                }
            }
            assert!(
                block
                    .spans
                    .windows(2)
                    .all(|w| w[0].range.start <= w[1].range.start)
            );
        }
    }

    #[test]
    fn line_ranges_returns_only_the_ranges_own_lines() {
        let text = "ab\ncd\nef\n";
        assert_eq!(line_ranges(text, 0..3), vec![0..2]);
        assert_eq!(line_ranges(text, 0..6), vec![0..2, 3..5]);
        assert_eq!(line_ranges("a\r\nb", 0..4), vec![0..1, 3..4]);
    }
}
