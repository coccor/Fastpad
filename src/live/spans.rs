//! Block text → the inline spans the Live view renders (2026-10-18 Live editor design spec §6,
//! phase 2 plan Task 2). Pure: offsets are bytes relative to the block text, and the caller adds
//! the block's position. Spans may overlap (`Bold` inside a `Link`); `Hidden` ranges never
//! overlap any other span.
//!
//! Block constructs (lists, quotes, code blocks, tables, rules, images) emit nothing of their own
//! in phase 2 and show as their source text; code blocks, tables and HTML blocks also keep the
//! inline spans of their text from being emitted.

use crate::preview::model::PARSE_OPTIONS;
use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use std::ops::Range;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockSpans {
    /// Sorted by start.
    pub spans: Vec<Span>,
    pub links: Vec<LinkSpan>,
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

struct Collector<'a> {
    text: &'a str,
    spans: Vec<Span>,
    links: Vec<LinkSpan>,
    /// Footnote and math ranges: dimmed, and no other span inside them.
    lexical: Vec<Range<usize>>,
    /// Inside a code block, table or HTML block: inline events add no spans.
    suppress: u32,
    heading: Option<(u8, Range<usize>)>,
    open_links: Vec<OpenLink>,
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
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Code(_) => self.inline_code(range),
            Event::InlineHtml(_) | Event::Html(_) => self.push(range, SpanKind::Dim),
            _ => {}
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
                self.lexical.extend(lexical_dims(self.text, range));
            }
            Tag::Heading { level, .. } => {
                if self.suppress == 0 {
                    self.lexical.extend(lexical_dims(self.text, range.clone()));
                }
                self.heading = Some((level as u8, range));
            }
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
            Tag::CodeBlock(_) | Tag::Table(_) => self.suppress += 1,
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
            TagEnd::CodeBlock | TagEnd::Table | TagEnd::HtmlBlock => {
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

    fn finish(self) -> BlockSpans {
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
        BlockSpans {
            spans,
            links: self.links,
        }
    }
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
    fn code_blocks_and_tables_get_no_inline_spans() {
        assert_eq!(kinds("```\n**a**\n```"), ".".repeat(13));
        let table = "| **a** | b |\n|---|---|\n| `c` | d |";
        assert_eq!(kinds(table), ".".repeat(table.len()));
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
