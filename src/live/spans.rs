//! Block text → the styled spans and painted decorations Live Markdown shows for it (live mode
//! spec §6). Pure: offsets are bytes relative to the block text, and the caller adds the block's
//! position. Spans are flattened so markup (hidden, blanked, marker) wins over the content kind
//! underneath, which is what keeps a quote's `>` blank inside a code block.

use crate::preview::model::PARSE_OPTIONS;
use pulldown_cmark::{BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpanKind {
    Text,
    Bold,
    Italic,
    BoldItalic,
    InlineCode,
    CodeBlock,
    Link,
    Quote,
    Dim,
    Marker,
    Hide,
    Blank,
    TableCell,
    TableHeader,
    TableBlank,
    HeadingMarker(u8),
    HeadingText(u8),
}

impl SpanKind {
    /// Markup outranks the content it sits in when spans overlap.
    const fn priority(self) -> u8 {
        match self {
            Self::Hide | Self::Blank | Self::TableBlank | Self::Marker | Self::HeadingMarker(_) => {
                2
            }
            _ => 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decoration {
    Heading {
        at: usize,
        level: u8,
        text: String,
    },
    Bullet {
        at: usize,
        depth: u8,
    },
    Checkbox {
        range: Range<usize>,
        checked: bool,
    },
    QuoteBar {
        at: usize,
        depth: u8,
    },
    Rule {
        at: usize,
    },
    Fence {
        at: usize,
        language: String,
    },
    TableRow {
        at: usize,
        pipes: Vec<usize>,
        header: bool,
    },
    TableDelimiter {
        at: usize,
        pipes: Vec<usize>,
    },
    Image {
        at: usize,
        dest: String,
        alt: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkSpan {
    pub range: Range<usize>,
    pub dest: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockSpans {
    pub spans: Vec<Span>,
    pub decorations: Vec<Decoration>,
    pub links: Vec<LinkSpan>,
    pub strikes: Vec<Range<usize>>,
}

pub fn parse_block(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> BlockSpans {
    let resolve = |link: BrokenLink<'_>| {
        refs(&link.reference).map(|dest| (CowStr::from(dest), CowStr::from("")))
    };
    let parser = Parser::new_with_broken_link_callback(text, PARSE_OPTIONS, Some(resolve))
        .into_offset_iter();
    let mut collector = Collector::new(text);
    for (event, range) in parser {
        collector.event(event, range);
    }
    collector.finish()
}

/// Byte offsets of the column separators in one table line: unescaped pipes outside code spans.
pub(crate) fn pipe_positions(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut pipes = Vec::new();
    let mut index = 0;
    let mut code_run: Option<usize> = None;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if code_run.is_none() => index += 2,
            b'`' => {
                let run = bytes[index..].iter().take_while(|b| **b == b'`').count();
                code_run = match code_run {
                    None => Some(run),
                    Some(open) if open == run => None,
                    other => other,
                };
                index += run;
            }
            b'|' if code_run.is_none() => {
                pipes.push(index);
                index += 1;
            }
            _ => index += 1,
        }
    }
    pipes
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
    autolink: bool,
    text_end: usize,
}

struct OpenImage {
    whole: Range<usize>,
    dest: String,
    alt: String,
    alt_end: usize,
}

struct OpenItem {
    marker: usize,
    checked: bool,
}

struct Collector<'a> {
    text: &'a str,
    raw: Vec<Span>,
    out: BlockSpans,
    strong: u32,
    emphasis: u32,
    quote: u32,
    checked: u32,
    list_depth: u8,
    /// Inside a heading, code block, table or HTML block: inline events add no spans.
    suppress: u32,
    heading: Option<(u8, Range<usize>, String)>,
    links: Vec<OpenLink>,
    image: Option<OpenImage>,
    items: Vec<OpenItem>,
}

impl<'a> Collector<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            raw: Vec::new(),
            out: BlockSpans::default(),
            strong: 0,
            emphasis: 0,
            quote: 0,
            checked: 0,
            list_depth: 0,
            suppress: 0,
            heading: None,
            links: Vec::new(),
            image: None,
            items: Vec::new(),
        }
    }

    fn push(&mut self, range: Range<usize>, kind: SpanKind) {
        if range.start < range.end && range.end <= self.text.len() {
            self.raw.push(Span { range, kind });
        }
    }

    fn line_start(&self, at: usize) -> usize {
        line_start(self.text, at)
    }

    fn inline_kind(&self) -> SpanKind {
        if self.checked > 0 {
            return SpanKind::Dim;
        }
        if !self.links.is_empty() {
            return SpanKind::Link;
        }
        match (self.strong > 0, self.emphasis > 0) {
            (true, true) => SpanKind::BoldItalic,
            (true, false) => SpanKind::Bold,
            (false, true) => SpanKind::Italic,
            (false, false) if self.quote > 0 => SpanKind::Quote,
            (false, false) => SpanKind::Text,
        }
    }

    fn event(&mut self, event: Event<'a>, range: Range<usize>) {
        if !matches!(
            event,
            Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link)
        ) && let Some(link) = self.links.last_mut()
        {
            link.text_end = link.text_end.max(range.end);
        }
        if !matches!(
            event,
            Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image)
        ) && let Some(image) = &mut self.image
        {
            image.alt_end = image.alt_end.max(range.end);
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag, range),
            Event::Text(text) => self.text(&text, range),
            Event::Code(_) => self.inline_code(range),
            Event::InlineHtml(_) | Event::Html(_) if self.suppress == 0 => {
                self.push(range, SpanKind::Dim)
            }
            Event::TaskListMarker(checked) => self.task(checked, range),
            Event::Rule => {
                let at = self.line_start(range.start);
                let line = line_ranges(self.text, range).remove(0);
                self.push(line, SpanKind::Blank);
                self.out.decorations.push(Decoration::Rule { at });
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str, range: Range<usize>) {
        if let Some((_, _, heading)) = &mut self.heading {
            heading.push_str(text);
            return;
        }
        if let Some(image) = &mut self.image {
            image.alt.push_str(text);
            self.push(range, SpanKind::Dim);
            return;
        }
        if self.suppress > 0 {
            return;
        }
        let kind = self.inline_kind();
        if kind != SpanKind::Text {
            self.push(range, kind);
        }
    }

    fn inline_code(&mut self, range: Range<usize>) {
        if let Some((_, _, heading)) = &mut self.heading {
            heading.push_str(self.text[range].trim_matches('`'));
            return;
        }
        if self.suppress > 0 {
            return;
        }
        let ticks = self.text[range.clone()]
            .bytes()
            .take_while(|b| *b == b'`')
            .count();
        self.push(range.start..range.start + ticks, SpanKind::Hide);
        self.push(range.start + ticks..range.end - ticks, SpanKind::InlineCode);
        self.push(range.end - ticks..range.end, SpanKind::Hide);
    }

    fn wrap(&mut self, range: &Range<usize>, width: usize) {
        if self.suppress > 0 || range.end - range.start < 2 * width {
            return;
        }
        self.push(range.start..range.start + width, SpanKind::Hide);
        self.push(range.end - width..range.end, SpanKind::Hide);
    }

    fn start(&mut self, tag: Tag<'a>, range: Range<usize>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.suppress += 1;
                self.heading = Some((level as u8, range, String::new()));
            }
            Tag::Strong => {
                self.wrap(&range, 2);
                self.strong += 1;
            }
            Tag::Emphasis => {
                self.wrap(&range, 1);
                self.emphasis += 1;
            }
            Tag::Strikethrough => {
                let width = if self.text[range.clone()].starts_with("~~") {
                    2
                } else {
                    1
                };
                self.wrap(&range, width);
                if self.suppress == 0 {
                    self.out
                        .strikes
                        .push(range.start + width..range.end - width);
                }
            }
            Tag::Link {
                link_type,
                dest_url,
                ..
            } => {
                let autolink = matches!(link_type, LinkType::Autolink | LinkType::Email);
                if self.suppress == 0 {
                    self.push(range.start..range.start + 1, SpanKind::Hide);
                }
                self.links.push(OpenLink {
                    whole: range.clone(),
                    dest: dest_url.to_string(),
                    autolink,
                    text_end: range.start + 1,
                });
            }
            Tag::Image { dest_url, .. } => {
                if self.suppress == 0 {
                    self.push(range.start..range.start + 2, SpanKind::Hide);
                }
                self.image = Some(OpenImage {
                    whole: range.clone(),
                    dest: dest_url.to_string(),
                    alt: String::new(),
                    alt_end: range.start + 2,
                });
            }
            Tag::List(_) => self.list_depth += 1,
            Tag::Item => self.item(range),
            Tag::BlockQuote(_) => {
                if self.quote == 0 {
                    self.quote_markers(range);
                }
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.suppress += 1;
                self.code_block(kind, range);
            }
            Tag::Table(_) => {
                self.suppress += 1;
                self.table(range);
            }
            Tag::HtmlBlock => {
                self.push(range, SpanKind::Dim);
                self.suppress += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd, range: Range<usize>) {
        match tag {
            TagEnd::Heading(_) => {
                self.suppress -= 1;
                if let Some((level, whole, text)) = self.heading.take() {
                    self.heading_spans(level, whole, text);
                }
            }
            TagEnd::Strong => self.strong -= 1,
            TagEnd::Emphasis => self.emphasis -= 1,
            TagEnd::Link => {
                if let Some(link) = self.links.pop() {
                    if self.suppress == 0 {
                        let tail = if link.autolink {
                            link.whole.end - 1
                        } else {
                            link.text_end
                        };
                        self.push(tail..link.whole.end, SpanKind::Hide);
                        if link.autolink {
                            self.push(link.whole.start + 1..link.whole.end - 1, SpanKind::Link);
                        }
                    }
                    self.out.links.push(LinkSpan {
                        range: link.whole,
                        dest: link.dest,
                    });
                }
            }
            TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    if self.suppress == 0 {
                        self.push(image.alt_end..image.whole.end, SpanKind::Hide);
                    }
                    let at = self.line_start(image.whole.start);
                    self.out.decorations.push(Decoration::Image {
                        at,
                        dest: image.dest,
                        alt: image.alt,
                    });
                }
            }
            TagEnd::List(_) => self.list_depth -= 1,
            TagEnd::Item => {
                if self.items.pop().is_some_and(|item| item.checked) {
                    self.checked -= 1;
                }
            }
            TagEnd::BlockQuote(_) => self.quote -= 1,
            TagEnd::CodeBlock | TagEnd::Table | TagEnd::HtmlBlock => self.suppress -= 1,
            _ => {}
        }
        let _ = range;
    }

    fn item(&mut self, range: Range<usize>) {
        let bytes = self.text.as_bytes();
        let mut marker = range.start;
        while marker < range.end && matches!(bytes[marker], b' ' | b'\t') {
            marker += 1;
        }
        if marker < range.end && matches!(bytes[marker], b'-' | b'*' | b'+') {
            self.push(marker..marker + 1, SpanKind::Blank);
            self.out.decorations.push(Decoration::Bullet {
                at: marker,
                depth: self.list_depth,
            });
        } else {
            let digits = bytes[marker..range.end]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            self.push(marker..marker + digits + 1, SpanKind::Marker);
        }
        self.items.push(OpenItem {
            marker,
            checked: false,
        });
    }

    fn task(&mut self, checked: bool, range: Range<usize>) {
        self.push(range.clone(), SpanKind::Blank);
        if let Some(item) = self.items.last_mut() {
            let marker = item.marker;
            self.out
                .decorations
                .retain(|d| !matches!(d, Decoration::Bullet { at, .. } if *at == marker));
            if checked {
                item.checked = true;
                self.checked += 1;
                let line_end = line_ranges(self.text, range.end..range.end)[0].end;
                let text_start = (range.end + 1).min(line_end);
                if text_start < line_end {
                    self.out.strikes.push(text_start..line_end);
                }
            }
        }
        self.out
            .decorations
            .push(Decoration::Checkbox { range, checked });
    }

    fn quote_markers(&mut self, range: Range<usize>) {
        let mut previous_depth = 1;
        let quote_start = range.start;
        for (index, line) in line_ranges(self.text, range).into_iter().enumerate() {
            let bytes = self.text.as_bytes();
            let mut at = if index == 0 { quote_start } else { line.start };
            let mut depth = 0u8;
            loop {
                let mut probe = at;
                while probe < line.end && probe - at < 3 && bytes[probe] == b' ' {
                    probe += 1;
                }
                if probe < line.end && bytes[probe] == b'>' {
                    self.push(probe..probe + 1, SpanKind::Blank);
                    depth += 1;
                    at = probe + 1;
                    if at < line.end && bytes[at] == b' ' {
                        at += 1;
                    }
                } else {
                    break;
                }
            }
            // A lazy continuation line has no `>` but still belongs to the quote.
            let depth = if depth == 0 { previous_depth } else { depth };
            previous_depth = depth;
            self.out.decorations.push(Decoration::QuoteBar {
                at: line.start,
                depth,
            });
        }
    }

    fn code_block(&mut self, kind: CodeBlockKind<'a>, range: Range<usize>) {
        let lines = line_ranges(self.text, range);
        let CodeBlockKind::Fenced(language) = kind else {
            for line in lines {
                self.push(line, SpanKind::CodeBlock);
            }
            return;
        };
        let content_start = |line: &Range<usize>| {
            let text = &self.text[line.clone()];
            // Skip a container's `>` prefix: the code starts at the first fence or code byte.
            let skip = text
                .find(|c: char| c != ' ' && c != '>')
                .unwrap_or(text.len());
            line.start + skip
        };
        let is_fence = |line: &Range<usize>, start: usize| {
            let rest = self.text[start..line.end].trim_end();
            rest.len() >= 3 && (rest.bytes().all(|b| b == b'`') || rest.bytes().all(|b| b == b'~'))
        };
        let last = lines.len() - 1;
        for (index, line) in lines.iter().enumerate() {
            let start = content_start(line);
            if index == 0 {
                self.push(start..line.end, SpanKind::Blank);
                self.out.decorations.push(Decoration::Fence {
                    at: line.start,
                    language: language.to_string(),
                });
            } else if index == last && is_fence(line, start) {
                self.push(start..line.end, SpanKind::Blank);
            } else {
                self.push(line.clone(), SpanKind::CodeBlock);
            }
        }
    }

    fn table(&mut self, range: Range<usize>) {
        for (index, line) in line_ranges(self.text, range).into_iter().enumerate() {
            if index == 1 {
                self.push(line.clone(), SpanKind::TableBlank);
                let pipes = pipe_positions(&self.text[line.clone()])
                    .into_iter()
                    .map(|pipe| line.start + pipe)
                    .collect();
                self.out.decorations.push(Decoration::TableDelimiter {
                    at: line.start,
                    pipes,
                });
                continue;
            }
            let kind = if index == 0 {
                SpanKind::TableHeader
            } else {
                SpanKind::TableCell
            };
            self.push(line.clone(), kind);
            let pipes: Vec<usize> = pipe_positions(&self.text[line.clone()])
                .into_iter()
                .map(|pipe| line.start + pipe)
                .collect();
            for pipe in &pipes {
                self.push(*pipe..*pipe + 1, SpanKind::TableBlank);
            }
            self.out.decorations.push(Decoration::TableRow {
                at: line.start,
                pipes,
                header: index == 0,
            });
        }
    }

    fn heading_spans(&mut self, level: u8, whole: Range<usize>, text: String) {
        let at = self.line_start(whole.start);
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
            self.push(first.start..content_start, SpanKind::HeadingMarker(level));
            self.push(content_start..content_end, SpanKind::HeadingText(level));
            self.push(content_end..first.end, SpanKind::HeadingMarker(level));
        } else {
            let last = lines.len() - 1;
            for (index, line) in lines.into_iter().enumerate() {
                let kind = if index == last {
                    SpanKind::HeadingMarker(level)
                } else {
                    SpanKind::HeadingText(level)
                };
                self.push(line, kind);
            }
        }
        self.out
            .decorations
            .push(Decoration::Heading { at, level, text });
    }

    fn finish(mut self) -> BlockSpans {
        // Flatten: per byte, the highest priority wins; equal priority, the later push wins.
        let mut slots: Vec<Option<SpanKind>> = vec![None; self.text.len()];
        for span in &self.raw {
            for slot in &mut slots[span.range.clone()] {
                if slot.is_none_or(|current| current.priority() <= span.kind.priority()) {
                    *slot = Some(span.kind);
                }
            }
        }
        let mut index = 0;
        while index < slots.len() {
            let Some(kind) = slots[index] else {
                index += 1;
                continue;
            };
            let start = index;
            while index < slots.len() && slots[index] == Some(kind) {
                index += 1;
            }
            if kind != SpanKind::Text {
                self.out.spans.push(Span {
                    range: start..index,
                    kind,
                });
            }
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_refs(_: &str) -> Option<String> {
        None
    }

    /// One character per byte: the span kind covering it ('.' for Text).
    fn kinds(text: &str) -> String {
        kinds_with(text, &no_refs)
    }

    fn kinds_with(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> String {
        let block = parse_block(text, refs);
        let mut out = vec!['.'; text.len()];
        for span in &block.spans {
            let code = match span.kind {
                SpanKind::Text => '.',
                SpanKind::Bold => 'b',
                SpanKind::Italic => 'i',
                SpanKind::BoldItalic => 'z',
                SpanKind::InlineCode => 'c',
                SpanKind::CodeBlock => 'C',
                SpanKind::Link => 'L',
                SpanKind::Quote => 'q',
                SpanKind::Dim => 'd',
                SpanKind::Marker => 'm',
                SpanKind::Hide => 'H',
                SpanKind::Blank => 'B',
                SpanKind::TableCell => 'x',
                SpanKind::TableHeader => 'X',
                SpanKind::TableBlank => '|',
                SpanKind::HeadingMarker(_) => '#',
                SpanKind::HeadingText(_) => 'h',
            };
            for slot in &mut out[span.range.clone()] {
                *slot = code;
            }
        }
        out.into_iter().collect()
    }

    #[test]
    fn strong_hides_its_markers() {
        assert_eq!(kinds("a **b** c"), "..HHbHH..");
    }

    #[test]
    fn nested_emphasis_is_bold_italic() {
        assert_eq!(kinds("***x***"), "HHHzHHH");
    }

    #[test]
    fn strikethrough_hides_markers_and_records_the_strike() {
        let block = parse_block("~~a~~", &no_refs);
        assert_eq!(kinds("~~a~~"), "HH.HH");
        assert_eq!(block.strikes, vec![2..3]);
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
        assert_eq!(block.links.len(), 1);
        assert_eq!(block.links[0].range, 0..6);
        assert_eq!(block.links[0].dest, "u");
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
        let block = parse_block("## Hi ##", &no_refs);
        assert_eq!(kinds("## Hi ##"), "###hh###");
        assert!(matches!(
            &block.decorations[..],
            [Decoration::Heading { at: 0, level: 2, text }] if text == "Hi"
        ));
    }

    #[test]
    fn setext_heading_hides_its_underline() {
        assert_eq!(kinds("Hi\n=="), "hh.##");
    }

    #[test]
    fn emphasis_inside_a_heading_is_heading_text() {
        assert_eq!(kinds("# a *b*"), "##hhhhh");
    }

    #[test]
    fn bullets_are_blanked_and_decorated() {
        let block = parse_block("- a\n- b", &no_refs);
        assert_eq!(kinds("- a\n- b"), "B...B..");
        let bullets: Vec<_> = block
            .decorations
            .iter()
            .filter_map(|d| match d {
                Decoration::Bullet { at, depth } => Some((*at, *depth)),
                _ => None,
            })
            .collect();
        assert_eq!(bullets, [(0, 1), (4, 1)]);
    }

    #[test]
    fn nested_bullets_carry_their_depth() {
        let block = parse_block("- a\n  - b", &no_refs);
        assert!(
            block
                .decorations
                .contains(&Decoration::Bullet { at: 6, depth: 2 })
        );
    }

    #[test]
    fn ordered_numbers_stay_visible_as_markers() {
        assert_eq!(kinds("1. a"), "mm..");
    }

    #[test]
    fn a_checked_task_is_a_checkbox_with_dim_struck_text() {
        let block = parse_block("- [x] done", &no_refs);
        assert_eq!(kinds("- [x] done"), "B.BBB.dddd");
        assert!(block.decorations.contains(&Decoration::Checkbox {
            range: 2..5,
            checked: true
        }));
        assert!(
            !block
                .decorations
                .iter()
                .any(|d| matches!(d, Decoration::Bullet { .. }))
        );
        assert_eq!(block.strikes, vec![6..10]);
    }

    #[test]
    fn quote_markers_are_blanked_with_a_bar_per_line() {
        let block = parse_block("> a\n> b", &no_refs);
        assert_eq!(kinds("> a\n> b"), "B.q.B.q");
        assert!(
            block
                .decorations
                .contains(&Decoration::QuoteBar { at: 0, depth: 1 })
        );
        assert!(
            block
                .decorations
                .contains(&Decoration::QuoteBar { at: 4, depth: 1 })
        );
    }

    #[test]
    fn nested_quote_depth_counts_markers() {
        let block = parse_block("> > a", &no_refs);
        assert!(
            block
                .decorations
                .contains(&Decoration::QuoteBar { at: 0, depth: 2 })
        );
        assert_eq!(kinds("> > a"), "B.B.q");
    }

    #[test]
    fn fenced_code_blanks_fences_and_records_the_language() {
        let text = "```rs\nx\n```";
        let block = parse_block(text, &no_refs);
        assert_eq!(kinds(text), "BBBBB.C.BBB");
        assert!(block.decorations.contains(&Decoration::Fence {
            at: 0,
            language: "rs".into()
        }));
    }

    #[test]
    fn an_unclosed_fence_styles_the_rest_as_code() {
        assert_eq!(kinds("```\nx"), "BBB.C");
    }

    #[test]
    fn fenced_code_inside_a_quote_keeps_the_quote_marker_blank() {
        // The space after a `>` inside the code takes the code style; only the `>` stays blank.
        assert_eq!(kinds("> ```\n> x\n> ```"), "B.BBB.BCC.B.BBB");
    }

    #[test]
    fn indented_code_is_code() {
        assert_eq!(kinds("    x"), "CCCCC");
    }

    #[test]
    fn table_pipes_and_delimiter_are_blank() {
        let text = "|a|b|\n|-|-|\n|c|d|";
        let block = parse_block(text, &no_refs);
        assert_eq!(kinds(text), "|X|X|.|||||.|x|x|");
        assert!(block.decorations.contains(&Decoration::TableRow {
            at: 0,
            pipes: vec![0, 2, 4],
            header: true
        }));
        assert!(block.decorations.contains(&Decoration::TableDelimiter {
            at: 6,
            pipes: vec![6, 8, 10]
        }));
    }

    #[test]
    fn escaped_and_code_pipes_are_cell_text() {
        assert_eq!(pipe_positions(r"|a\|b|"), vec![0, 5]);
        assert_eq!(pipe_positions("|`a|b`|"), vec![0, 6]);
    }

    #[test]
    fn thematic_break_is_blank_with_a_rule() {
        let block = parse_block("---", &no_refs);
        assert_eq!(kinds("---"), "BBB");
        assert_eq!(block.decorations, vec![Decoration::Rule { at: 0 }]);
    }

    #[test]
    fn image_shows_dim_alt_text_and_records_the_image() {
        let text = "![a](p.png)";
        let block = parse_block(text, &no_refs);
        assert_eq!(kinds(text), "HHdHHHHHHHH");
        assert!(block.decorations.contains(&Decoration::Image {
            at: 0,
            dest: "p.png".into(),
            alt: "a".into()
        }));
    }

    #[test]
    fn inline_html_is_dim() {
        assert_eq!(kinds("a<br>"), ".dddd");
    }

    #[test]
    fn crlf_offsets_are_byte_exact() {
        assert_eq!(kinds("**a**\r\nb"), "HHbHH...");
        assert_eq!(kinds("# a\r\n# b"), "##h..##h");
    }

    fn decoration_lines(block: &BlockSpans) -> Vec<usize> {
        block
            .decorations
            .iter()
            .map(|d| match d {
                Decoration::Heading { at, .. }
                | Decoration::Bullet { at, .. }
                | Decoration::QuoteBar { at, .. }
                | Decoration::Rule { at }
                | Decoration::Fence { at, .. }
                | Decoration::TableRow { at, .. }
                | Decoration::TableDelimiter { at, .. }
                | Decoration::Image { at, .. } => *at,
                Decoration::Checkbox { range, .. } => range.start,
            })
            .collect()
    }

    #[test]
    fn line_ranges_returns_only_the_ranges_own_lines() {
        let text = "ab\ncd\nef\n";
        assert_eq!(line_ranges(text, 0..3), vec![0..2]);
        assert_eq!(line_ranges(text, 0..6), vec![0..2, 3..5]);
        assert_eq!(line_ranges(text, 3..8), vec![3..5, 6..8]);
        assert_eq!(line_ranges(text, 4..4), vec![3..5]);
        assert_eq!(line_ranges("a\r\nb", 0..4), vec![0..1, 3..4]);
    }

    #[test]
    fn a_closed_fence_with_a_trailing_newline_ignores_later_lines() {
        let text = "```\nx\n```\n";
        assert_eq!(kinds(text), "BBB.C.BBB.");
        assert_eq!(decoration_lines(&parse_block(text, &no_refs)), vec![0]);
    }

    #[test]
    fn a_quote_followed_by_a_line_decorates_only_its_own_lines() {
        let text = "> a\n\nb\n";
        assert_eq!(kinds(text), "B.q....");
        assert_eq!(decoration_lines(&parse_block(text, &no_refs)), vec![0]);
        assert_eq!(
            decoration_lines(&parse_block("> a\nb\n", &no_refs)),
            vec![0, 4]
        );
    }

    #[test]
    fn a_setext_heading_with_a_trailing_newline_ignores_later_lines() {
        let text = "Hi\n==\nnext\n";
        assert_eq!(kinds(text), "hh.##......");
        assert_eq!(decoration_lines(&parse_block(text, &no_refs)), vec![0]);
    }

    #[test]
    fn a_quote_inside_a_list_item_blanks_its_first_marker() {
        assert_eq!(kinds("- > a"), "B.B.q");
        assert_eq!(kinds("- > a\n  > b"), "B.B.q...B.q");
        assert_eq!(kinds("1. > a"), "mm.B.q");
    }

    #[test]
    fn a_setext_heading_whose_text_starts_with_a_hash_is_setext() {
        assert_eq!(kinds("#x\n=="), "hh.##");
    }

    #[test]
    fn multibyte_text_keeps_byte_offsets() {
        // 'é' is two bytes.
        assert_eq!(kinds("é **b**"), "...HHbHH");
    }
}
