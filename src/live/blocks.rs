//! The Live document model (2026-10-18 Live editor design spec §7.1, phase 2 plan Task 2): the
//! preview's incremental block list decides which blocks an edit touched, and each block keeps
//! its spans relative to its own start, parsed only when a line in it is first styled. Untouched
//! blocks never reparse, even when an edit above them shifts their position.

use super::spans::{BlockSpans, LinkSpan, Span, SpanKind, parse_block};
use super::text::StrSource;
use crate::preview::incremental::{Edit, PreviewDocument, SourceText, Update};
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct LiveDocument {
    preview: PreviewDocument,
    /// Each block's spans, relative to its start; `None` until a line in the block is styled.
    spans: RefCell<Vec<Option<Rc<BlockSpans>>>>,
    front_matter: usize,
    /// How many blocks have been parsed for spans, for the tests that check laziness.
    parsed: Cell<usize>,
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
        let preview = PreviewDocument::parse(source);
        let blocks = preview.blocks.len();
        Self {
            preview,
            spans: RefCell::new(vec![None; blocks]),
            front_matter: front_matter(&StrSource(source)),
            parsed: Cell::new(0),
        }
    }

    /// Parses the whole document again.
    pub fn reparse(&mut self, source: &(impl SourceText + ?Sized)) {
        self.preview.reparse(source);
        *self.spans.borrow_mut() = vec![None; self.preview.blocks.len()];
        self.front_matter = front_matter(source);
    }

    /// The end of the front matter block, or 0 when the document has none.
    pub fn front_matter_end(&self) -> usize {
        self.front_matter
    }

    /// How many blocks have been parsed for spans so far.
    #[cfg(test)]
    pub fn parsed_blocks(&self) -> usize {
        self.parsed.get()
    }

    /// Applies edits already in `source`; returns the bytes whose spans may have changed.
    pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Range<usize> {
        let mut changed = self.apply_blocks(source, edits);
        // A block the preview kept because it renders the same may still have shifted bytes
        // (a tab in a blank run): forget every block the edits touched.
        for edit in edits {
            let touched = edit.position..edit.position + edit.inserted;
            let blocks = &self.preview.blocks;
            let first = blocks.partition_point(|block| block.bytes.end < touched.start);
            let last = blocks.partition_point(|block| block.bytes.start <= touched.end);
            if first < last {
                for slot in &mut self.spans.borrow_mut()[first..last] {
                    *slot = None;
                }
                changed.start = changed.start.min(blocks[first].bytes.start);
                changed.end = changed.end.max(blocks[last - 1].bytes.end);
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
            .preview
            .blocks
            .iter()
            .take_while(|block| block.bytes.start < bound)
            .map(|block| block.bytes.end)
            .max()
            .unwrap_or(0);
        0..changed.end.max(bound).max(end)
    }

    fn apply_blocks(
        &mut self,
        source: &(impl SourceText + ?Sized),
        edits: &[Edit],
    ) -> Range<usize> {
        match self.preview.apply(source, edits) {
            Update::Unchanged => {
                let touched = edits.iter().map(|e| e.position).min().unwrap_or(0);
                touched..touched
            }
            Update::Replaced { old, new } => {
                self.spans
                    .borrow_mut()
                    .splice(old, std::iter::repeat_n(None, new.len()));
                let blocks = &self.preview.blocks;
                let start = blocks
                    .get(new.start)
                    .map_or(source.len(), |b| b.bytes.start);
                let end = new
                    .end
                    .checked_sub(1)
                    .and_then(|last| blocks.get(last))
                    .map_or(start, |b| b.bytes.end);
                let edit_start = edits.iter().map(|e| e.position).min().unwrap_or(start);
                start.min(edit_start)..end.max(edit_start)
            }
            Update::Full => {
                *self.spans.borrow_mut() = vec![None; self.preview.blocks.len()];
                0..source.len()
            }
        }
    }

    /// Block `index`'s spans, parsing them on first use.
    fn block_spans(&self, source: &(impl SourceText + ?Sized), index: usize) -> Rc<BlockSpans> {
        if let Some(spans) = &self.spans.borrow()[index] {
            return Rc::clone(spans);
        }
        let refs = |label: &str| self.preview.refdef_dest(label).map(str::to_owned);
        let bytes = self.preview.blocks[index].bytes.clone();
        let parsed = Rc::new(parse_block(&source.slice(bytes), &refs));
        self.parsed.set(self.parsed.get() + 1);
        self.spans.borrow_mut()[index] = Some(Rc::clone(&parsed));
        parsed
    }

    /// The blocks overlapping `bytes` (an empty range finds the block holding it), skipping
    /// those that start inside the front matter: (index, absolute start).
    fn blocks_in(&self, bytes: Range<usize>) -> Vec<(usize, usize)> {
        let blocks = &self.preview.blocks;
        let first = blocks.partition_point(|block| block.bytes.end <= bytes.start);
        blocks[first..]
            .iter()
            .enumerate()
            .take_while(|(_, block)| block.bytes.start < bytes.end.max(bytes.start + 1))
            .filter(|(_, block)| block.bytes.start >= self.front_matter)
            .map(|(offset, block)| (first + offset, block.bytes.start))
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
            for span in &self.block_spans(source, index).spans {
                let range = start + span.range.start..start + span.range.end;
                if range.end > bytes.start && range.start < bytes.end {
                    out.push(Span {
                        range,
                        kind: span.kind,
                    });
                }
            }
        }
        out
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
    fn scripted_edits_match_a_full_parse() {
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
        ];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |bound: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % bound.max(1)
        };
        let mut text = String::from(
            "---\ntitle: Plan\n---\n\n# Head *one*\n\nPara **b** `c` [l](u) [[N]] $m$ [^1]\n\n```\ncode\n```\n\n- item ~~s~~\n",
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
