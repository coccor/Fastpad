//! The Live document model (2026-10-18 Live editor design spec §7.1, phase 2 plan Task 2): Live's
//! own incremental block index (`index.rs`, the preview's reparsing rule without its rendering
//! model) decides which blocks an edit touched, and each block keeps its spans relative to its
//! own start, parsed only when a line in it is first styled. Untouched blocks never reparse, even
//! when an edit above them shifts their position.

use super::index::BlockIndex;
use super::spans::{BlockSpans, Body, LineBlock, LinkSpan, Span, SpanKind, parse_block};
use super::text::StrSource;
use crate::preview::incremental::{Edit, SourceText, Update};
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct LiveDocument {
    index: BlockIndex,
    /// Each block's spans, relative to its start; `None` until a line in the block is styled.
    spans: RefCell<Vec<Option<Rc<BlockSpans>>>>,
    front_matter: usize,
    /// How many blocks have been parsed for spans, for the tests that check laziness.
    parsed: Cell<usize>,
    /// How many times edits were applied or the whole document reparsed, for the tests.
    #[cfg(test)]
    updates: usize,
}

/// The end of a YAML front matter block (`---` … `---` or `...` lines at the very start),
/// excluding the closing line's terminator, or 0.
fn front_matter(source: &(impl SourceText + ?Sized)) -> usize {
    // Most documents have no front matter: decide from the first few bytes before copying more.
    let opening = source.slice(0..source.len().min(8));
    if !opening.starts_with("---") || !opening.contains('\n') {
        return 0;
    }
    let head = source.slice(0..source.len().min(64 * 1024));
    let mut lines = head.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return 0;
    };
    if first.trim_end() != "---" {
        return 0;
    }
    let mut end = first.len();
    for line in lines {
        end += line.len();
        if matches!(line.trim_end(), "---" | "...") {
            return end - (line.len() - line.trim_end_matches(['\r', '\n']).len());
        }
    }
    0
}

impl LiveDocument {
    pub fn parse(source: &str) -> Self {
        let index = BlockIndex::parse(source);
        let blocks = index.blocks.len();
        Self {
            index,
            spans: RefCell::new(vec![None; blocks]),
            front_matter: front_matter(&StrSource(source)),
            parsed: Cell::new(0),
            #[cfg(test)]
            updates: 0,
        }
    }

    /// Parses the whole document again.
    pub fn reparse(&mut self, source: &(impl SourceText + ?Sized)) {
        #[cfg(test)]
        {
            self.updates += 1;
        }
        self.index.reparse(source);
        *self.spans.borrow_mut() = vec![None; self.index.blocks.len()];
        self.front_matter = front_matter(source);
    }

    /// The end of the front matter block, or 0 when the document has none.
    pub fn front_matter_end(&self) -> usize {
        self.front_matter
    }

    /// How many times edits were applied or the whole document reparsed.
    #[cfg(test)]
    pub fn updates(&self) -> usize {
        self.updates
    }

    /// How many blocks have been parsed for spans so far.
    #[cfg(test)]
    pub fn parsed_blocks(&self) -> usize {
        self.parsed.get()
    }

    /// Applies edits already in `source`; returns the bytes whose spans may have changed.
    pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Range<usize> {
        #[cfg(test)]
        {
            self.updates += 1;
        }
        let mut changed = self.apply_blocks(source, edits);
        // A block the index kept because its range came back the same may still hold changed
        // text (a letter typed in a paragraph): forget every block the edits touched.
        for edit in edits {
            let touched = edit.position..edit.position + edit.inserted;
            let blocks = &self.index.blocks;
            let first = blocks.partition_point(|block| block.end < touched.start);
            let last = blocks.partition_point(|block| block.start <= touched.end);
            if first < last {
                for slot in &mut self.spans.borrow_mut()[first..last] {
                    *slot = None;
                }
                changed.start = changed.start.min(blocks[first].start);
                changed.end = changed.end.max(blocks[last - 1].end);
            }
        }
        let mut old_front_matter = self.front_matter;
        for edit in edits {
            if old_front_matter >= edit.position + edit.removed {
                old_front_matter = old_front_matter - edit.removed + edit.inserted;
            } else if old_front_matter > edit.position {
                old_front_matter = edit.position + edit.inserted;
            }
        }
        self.front_matter = front_matter(source);
        if self.front_matter == old_front_matter {
            return changed;
        }
        // Blocks that start inside the old or new front matter flip between shown and dimmed,
        // and may extend past it.
        let bound = old_front_matter.max(self.front_matter);
        let end = self
            .index
            .blocks
            .iter()
            .take_while(|block| block.start < bound)
            .map(|block| block.end)
            .max()
            .unwrap_or(0);
        0..changed.end.max(bound).max(end)
    }

    fn apply_blocks(
        &mut self,
        source: &(impl SourceText + ?Sized),
        edits: &[Edit],
    ) -> Range<usize> {
        match self.index.apply(source, edits) {
            Update::Unchanged => {
                let touched = edits.iter().map(|e| e.position).min().unwrap_or(0);
                touched..touched
            }
            Update::Replaced { old, new } => {
                self.spans
                    .borrow_mut()
                    .splice(old, std::iter::repeat_n(None, new.len()));
                let blocks = &self.index.blocks;
                let start = blocks.get(new.start).map_or(source.len(), |b| b.start);
                let end = new
                    .end
                    .checked_sub(1)
                    .and_then(|last| blocks.get(last))
                    .map_or(start, |b| b.end);
                let edit_start = edits.iter().map(|e| e.position).min().unwrap_or(start);
                start.min(edit_start)..end.max(edit_start)
            }
            Update::Full => {
                *self.spans.borrow_mut() = vec![None; self.index.blocks.len()];
                0..source.len()
            }
        }
    }

    /// Block `index`'s spans, parsing them on first use.
    fn block_spans(&self, source: &(impl SourceText + ?Sized), index: usize) -> Rc<BlockSpans> {
        if let Some(spans) = &self.spans.borrow()[index] {
            return Rc::clone(spans);
        }
        let refs = |label: &str| self.index.refdef_dest(label).map(str::to_owned);
        let bytes = self.index.blocks[index].clone();
        let parsed = Rc::new(parse_block(&source.slice(bytes), &refs));
        self.parsed.set(self.parsed.get() + 1);
        self.spans.borrow_mut()[index] = Some(Rc::clone(&parsed));
        parsed
    }

    /// The blocks overlapping `bytes` (an empty range finds the block holding it), skipping
    /// those that start inside the front matter: (index, absolute start).
    fn blocks_in(&self, bytes: Range<usize>) -> Vec<(usize, usize)> {
        let blocks = &self.index.blocks;
        let first = blocks.partition_point(|block| block.end <= bytes.start);
        blocks[first..]
            .iter()
            .enumerate()
            .take_while(|(_, block)| block.start < bytes.end.max(bytes.start + 1))
            .filter(|(_, block)| block.start >= self.front_matter)
            .map(|(offset, block)| (first + offset, block.start))
            .collect()
    }

    /// Absolute spans overlapping `bytes`, sorted by start; parses blocks not parsed yet.
    pub fn spans_in(&self, source: &(impl SourceText + ?Sized), bytes: Range<usize>) -> Vec<Span> {
        let mut out = Vec::new();
        if self.front_matter > 0 && bytes.start < self.front_matter {
            out.push(Span {
                range: 0..self.front_matter,
                kind: SpanKind::Dim,
            });
        }
        for (index, start) in self.blocks_in(bytes.clone()) {
            let relative = bytes.start.saturating_sub(start)..bytes.end.saturating_sub(start);
            for span in self.block_spans(source, index).overlapping(relative) {
                out.push(Span {
                    range: start + span.range.start..start + span.range.end,
                    kind: span.kind,
                });
            }
        }
        out
    }

    /// The block context of the line whose bytes (line ending excluded) are `line`, with an
    /// absolute `start` (its other offsets are relative to the line); `None` for plain prose,
    /// blank lines and the front matter.
    pub fn line_block(
        &self,
        source: &(impl SourceText + ?Sized),
        line: Range<usize>,
    ) -> Option<LineBlock> {
        let (index, start) = self
            .blocks_in(line.start..line.end.max(line.start + 1))
            .into_iter()
            .next()?;
        // A block may start after the line's indent: its first line then begins at `start`.
        let relative = line.start.saturating_sub(start);
        let spans = self.block_spans(source, index);
        let found = spans
            .lines
            .binary_search_by_key(&relative, |block| block.start)
            .ok()?;
        let block = spans.lines[found].clone();
        let delta = (start + block.start).saturating_sub(line.start);
        Some(
            LineBlock {
                start: line.start,
                ..block
            }
            .shifted(delta),
        )
    }

    /// The bytes of the top-level block holding `at` (the front matter counts as one), or `None`
    /// between blocks: what the caret reveals as source.
    pub fn block_bytes(&self, at: usize) -> Option<Range<usize>> {
        if at < self.front_matter {
            return Some(0..self.front_matter);
        }
        let blocks = &self.index.blocks;
        let found = blocks.partition_point(|block| block.end <= at);
        blocks.get(found).filter(|block| block.start <= at).cloned()
    }

    /// The absolute starts of the first and last lines of the table holding `at`.
    pub fn table_extent(
        &self,
        source: &(impl SourceText + ?Sized),
        at: usize,
    ) -> Option<(usize, usize)> {
        let (index, start) = self.blocks_in(at..at).into_iter().next()?;
        let spans = self.block_spans(source, index);
        let aligns_of = |block: &LineBlock| match &block.body {
            Body::TableRow { aligns, .. } | Body::TableDelimiter { aligns } => {
                Some(Rc::clone(aligns))
            }
            _ => None,
        };
        let relative = at.saturating_sub(start);
        let here = spans
            .lines
            .partition_point(|block| block.start <= relative)
            .checked_sub(1)?;
        let aligns = aligns_of(&spans.lines[here])?;
        let same = |block: &LineBlock| aligns_of(block).is_some_and(|a| Rc::ptr_eq(&a, &aligns));
        let first = spans.lines[..here]
            .iter()
            .rposition(|block| !same(block))
            .map_or(0, |other| other + 1);
        let last = spans.lines[here..]
            .iter()
            .position(|block| !same(block))
            .map_or(spans.lines.len(), |other| here + other)
            - 1;
        Some((
            start + spans.lines[first].start,
            start + spans.lines[last].start,
        ))
    }

    /// The link whose source covers `position`, in absolute bytes.
    pub fn link_at(
        &self,
        source: &(impl SourceText + ?Sized),
        position: usize,
    ) -> Option<LinkSpan> {
        self.blocks_in(position..position)
            .into_iter()
            .find_map(|(index, start)| {
                self.block_spans(source, index)
                    .links
                    .iter()
                    .map(|link| LinkSpan {
                        range: start + link.range.start..start + link.range.end,
                        dest: link.dest.clone(),
                        wiki: link.wiki,
                    })
                    .find(|link| link.range.contains(&position))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::spans::{Container, line_ranges};

    fn spans(doc: &LiveDocument, text: &str) -> Vec<(Range<usize>, SpanKind)> {
        doc.spans_in(&StrSource(text), 0..text.len())
            .into_iter()
            .map(|span| (span.range, span.kind))
            .collect()
    }

    /// Replaces `removed` bytes at `at` with `insert`, in `text` and `doc` the way SCN_MODIFIED
    /// reports it; returns what `apply` said changed.
    fn edit_at(
        text: &mut String,
        doc: &mut LiveDocument,
        at: usize,
        removed: usize,
        insert: &str,
    ) -> Range<usize> {
        let gone: String = text.drain(at..at + removed).collect();
        text.insert_str(at, insert);
        let edit = Edit {
            position: at,
            removed,
            inserted: insert.len(),
            lines_delta: insert.matches('\n').count() as isize
                - gone.matches('\n').count() as isize,
        };
        doc.apply(&StrSource(text), &[edit])
    }

    fn assert_matches_full_parse(text: &str, doc: &LiveDocument) {
        let full = LiveDocument::parse(text);
        assert_eq!(spans(doc, text), spans(&full, text), "text: {text:?}");
        assert_eq!(doc.front_matter_end(), full.front_matter_end(), "{text:?}");
        assert_eq!(doc.index, full.index, "block ranges, text: {text:?}");
        assert_eq!(
            line_blocks(doc, text),
            line_blocks(&full, text),
            "text: {text:?}"
        );
    }

    fn line_blocks(doc: &LiveDocument, text: &str) -> Vec<Option<LineBlock>> {
        line_ranges(text, 0..text.len())
            .into_iter()
            .map(|line| doc.line_block(&StrSource(text), line))
            .collect()
    }

    fn line_of(text: &str, needle: &str) -> Range<usize> {
        let start = text.find(needle).unwrap();
        let start = text[..start].rfind('\n').map_or(0, |at| at + 1);
        line_ranges(text, start..start)[0].clone()
    }

    #[test]
    fn line_block_is_absolute() {
        let text = "para\n\n- a\n- b\n";
        let doc = LiveDocument::parse(text);
        let block = doc
            .line_block(&StrSource(text), line_of(text, "- b"))
            .unwrap();
        assert_eq!(block.start, 10);
        assert_eq!(block.prefix, 0..2);
        assert_eq!(doc.line_block(&StrSource(text), 0..4), None);
        assert_eq!(doc.line_block(&StrSource(text), 5..5), None);
    }

    #[test]
    fn an_indented_block_hides_its_indent() {
        let text = "para\n\n  - a\n";
        let doc = LiveDocument::parse(text);
        let block = doc
            .line_block(&StrSource(text), line_of(text, "- a"))
            .unwrap();
        assert_eq!(block.start, 6);
        assert_eq!(block.prefix, 0..4);
    }

    #[test]
    fn indenting_an_item_changes_its_depth_and_reports_the_block() {
        let mut text = String::from("- a\n- b\n\nafter\n");
        let mut doc = LiveDocument::parse(&text);
        let depth = |doc: &LiveDocument, text: &str| {
            let block = doc
                .line_block(&StrSource(text), line_of(text, "b"))
                .unwrap();
            match block.containers.last() {
                Some(Container::Item { depth, .. }) => *depth,
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(depth(&doc, &text), 1);
        let changed = edit_at(&mut text, &mut doc, 4, 0, "  ");
        assert_eq!(depth(&doc, &text), 2);
        assert!(changed.start <= 4 && changed.end >= 11, "{changed:?}");
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn typing_a_pipe_row_after_a_paragraph_makes_a_table() {
        let mut text = String::from("| a | b |\n\nafter\n");
        let mut doc = LiveDocument::parse(&text);
        assert_eq!(doc.line_block(&StrSource(&text), 0..9), None);
        edit_at(&mut text, &mut doc, 10, 0, "|---|---|\n| c | d |\n");
        let row = doc.line_block(&StrSource(&text), 0..9).unwrap();
        assert!(
            matches!(row.body, Body::TableRow { header: true, .. }),
            "{row:?}"
        );
        assert_eq!(doc.table_extent(&StrSource(&text), 25), Some((0, 20)));
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn table_extent_stops_at_the_tables_edges() {
        let text = "- x\n\n| a |\n|---|\n| b |\n\n| c |\n|---|\n";
        let doc = LiveDocument::parse(text);
        let first = text.find("| a").unwrap();
        let last = text.find("| b").unwrap();
        assert_eq!(
            doc.table_extent(&StrSource(text), first + 2),
            Some((first, last))
        );
        let second = text.find("| c").unwrap();
        assert_eq!(
            doc.table_extent(&StrSource(text), second),
            Some((second, second + 6))
        );
        assert_eq!(doc.table_extent(&StrSource(text), 1), None);
    }

    #[test]
    fn front_matter_lines_have_no_block() {
        let text = "---\n- a\n---\n- b\n";
        let doc = LiveDocument::parse(text);
        assert_eq!(doc.line_block(&StrSource(text), 4..7), None);
        assert!(doc.line_block(&StrSource(text), 12..15).is_some());
    }

    #[test]
    fn spans_are_absolute() {
        let text = "para\n\n**b**\n";
        let doc = LiveDocument::parse(text);
        assert_eq!(
            spans(&doc, text),
            vec![
                (6..8, SpanKind::Hidden),
                (8..9, SpanKind::Bold),
                (9..11, SpanKind::Hidden)
            ]
        );
    }

    #[test]
    fn typing_an_open_fence_matches_a_full_parse() {
        let mut text = String::from("a\n\nb\n\n**c**\n");
        let mut doc = LiveDocument::parse(&text);
        let mut at = 3;
        for piece in ["`", "`", "`", "r", "s", "\n", "x", "\n", "```\n"] {
            edit_at(&mut text, &mut doc, at, 0, piece);
            at += piece.len();
            assert_matches_full_parse(&text, &doc);
        }
    }

    #[test]
    fn typing_a_lone_strong_marker_matches_a_full_parse() {
        let mut text = String::from("one\n\ntwo three\n");
        let mut doc = LiveDocument::parse(&text);
        for (at, piece) in [(5, "*"), (6, "*"), (11, "*"), (12, "*")] {
            edit_at(&mut text, &mut doc, at, 0, piece);
            assert_matches_full_parse(&text, &doc);
        }
        edit_at(&mut text, &mut doc, 5, 2, "");
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn reference_links_use_definitions_elsewhere_in_the_document() {
        let text = "[a][r]\n\n[r]: https://x\n";
        let doc = LiveDocument::parse(text);
        assert_eq!(doc.link_at(&StrSource(text), 1).unwrap().dest, "https://x");
    }

    #[test]
    fn apply_reports_the_changed_bytes() {
        let mut text = String::from("a\n\nb\n");
        let mut doc = LiveDocument::parse(&text);
        let changed = edit_at(&mut text, &mut doc, 3, 0, "*");
        assert!(changed.start <= 3 && changed.end >= 5, "{changed:?}");
    }

    #[test]
    fn front_matter_is_one_dim_span() {
        let text = "---\ntitle: x\n---\n\n**b**\n";
        let doc = LiveDocument::parse(text);
        let all = spans(&doc, text);
        assert_eq!(all[0], (0..16, SpanKind::Dim));
        assert!(
            all[1..].iter().all(|(range, _)| range.start >= 16),
            "{all:?}"
        );
        assert_eq!(doc.front_matter_end(), 16);
    }

    #[test]
    fn front_matter_end_with_crlf() {
        let doc = LiveDocument::parse("---\r\ntitle: x\r\n---\r\n\r\nb\r\n");
        assert_eq!(doc.front_matter_end(), 18);
    }

    #[test]
    fn a_rule_without_a_closing_fence_is_not_front_matter() {
        let doc = LiveDocument::parse("---\ntext\n");
        assert_eq!(doc.front_matter_end(), 0);
    }

    #[test]
    fn closing_the_front_matter_restyles_from_the_start() {
        let mut text = String::from("---\ntitle: x\n\nb\n");
        let mut doc = LiveDocument::parse(&text);
        assert_eq!(doc.front_matter_end(), 0);
        let at = text.len() - 3;
        let changed = edit_at(&mut text, &mut doc, at, 0, "---\n");
        assert_eq!(changed.start, 0);
        assert!(doc.front_matter_end() > 0);
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn a_later_rule_block_is_not_read_as_front_matter() {
        let text = "intro\n\n---\nkey: value\n---\n\n**b**\n";
        let doc = LiveDocument::parse(text);
        assert_eq!(doc.front_matter_end(), 0);
        assert!(
            spans(&doc, text)
                .iter()
                .all(|(_, kind)| *kind != SpanKind::Dim)
        );
    }

    #[test]
    fn spans_parse_lazily_and_once() {
        let text = "a *1*\n\nb *2*\n\nc *3*\n\nd *4*\n";
        let doc = LiveDocument::parse(text);
        assert_eq!(doc.parsed_blocks(), 0);
        doc.spans_in(&StrSource(text), 7..12);
        assert_eq!(doc.parsed_blocks(), 1);
        doc.spans_in(&StrSource(text), 7..12);
        assert_eq!(doc.parsed_blocks(), 1);
        doc.spans_in(&StrSource(text), 0..text.len());
        assert_eq!(doc.parsed_blocks(), 4);
    }

    #[test]
    fn link_at_finds_a_wikilink_and_a_link() {
        let text = "see [[Note]] and [x](y.md)\n";
        let doc = LiveDocument::parse(text);
        let wiki = doc.link_at(&StrSource(text), 6).unwrap();
        assert!(wiki.wiki);
        assert_eq!(wiki.range, 4..12);
        let link = doc.link_at(&StrSource(text), 18).unwrap();
        assert_eq!(link.dest, "y.md");
        assert!(!link.wiki);
        assert!(doc.link_at(&StrSource(text), 14).is_none());
    }

    #[test]
    fn a_multibyte_char_at_the_front_matter_window_edge_does_not_panic() {
        let text = format!("---\n{}ééé\n", "a".repeat(65531));
        let doc = LiveDocument::parse(&text);
        assert_eq!(doc.front_matter_end(), 0);
    }

    #[test]
    fn deleting_across_a_block_boundary_matches_a_full_parse() {
        let mut text = String::from("# one\n\ntwo *x*\n\n- a\n- b\n\n**c**\n");
        let mut doc = LiveDocument::parse(&text);
        edit_at(&mut text, &mut doc, 3, 12, "");
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn scripted_block_edits_match_a_full_parse() {
        const PIECES: &[&str] = &[
            "---\n",
            "...\n",
            "\n",
            "\n\n",
            "```",
            "```rs\n",
            "**",
            "*",
            "`",
            "# ",
            "> ",
            "- ",
            "1. ",
            "| a | b |\n",
            "|---|---|\n",
            "[r]: https://x\n",
            "[a][r]",
            "[l](u)",
            "[[Note|n]]",
            "~~",
            "é",
            "😀",
            "word ",
            "\t",
            "<div>",
            "    ",
            "title: x\n",
            "$x$",
            "[^1]",
            "Hi\n==\n",
            "\r\n",
            "  - ",
            "- [ ] ",
            "- [x] ",
            "> > ",
            "~~~\n",
            "***\n",
            "![a](p.png)\n",
            "| c |\n",
        ];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |bound: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % bound.max(1)
        };
        let mut text = String::from(
            "---\ntitle: Plan\n---\n\n# Head *one*\n\nPara **b** `c` [l](u) [[N]] $m$ [^1]\n\n```\ncode\n```\n\n- item ~~s~~\n  - [ ] sub\n    1. deep\n\n> quote\n> - in\nlazy\n\n| h | i |\n|:-|-:|\n| j | k |\n\n![img](p.png)\n\n~~~py\nx\n~~~\n\n    indented\n\n---\n",
        );
        let mut doc = LiveDocument::parse(&text);
        for _ in 0..500 {
            let boundaries: Vec<usize> = text
                .char_indices()
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            let at = boundaries[next(boundaries.len())];
            let later: Vec<usize> = boundaries.iter().copied().filter(|b| *b >= at).collect();
            let removed = if next(2) == 0 {
                0
            } else {
                later[next(later.len().min(8))] - at
            };
            let insert = if next(3) == 0 {
                ""
            } else {
                PIECES[next(PIECES.len())]
            };
            if removed == 0 && insert.is_empty() {
                continue;
            }
            // Style a part of the document first, so stale cached blocks would show.
            doc.spans_in(&StrSource(&text), 0..text.len().min(next(text.len() + 1)));
            edit_at(&mut text, &mut doc, at, removed, insert);
            assert_matches_full_parse(&text, &doc);
        }
    }
}
