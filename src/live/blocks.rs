//! The Live document model (live mode spec §7): the preview's incremental block list decides
//! which blocks an edit touched, and each block keeps its spans relative to its own start, so
//! untouched blocks never reparse even when an edit above them shifts their position.

use super::spans::{BlockSpans, SpanKind, parse_block};
use crate::preview::incremental::{Edit, PreviewDocument, SourceText, Update};
use std::ops::Range;

#[derive(Debug, Default)]
pub struct LiveDocument {
    preview: PreviewDocument,
    spans: Vec<BlockSpans>,
    front_matter: usize,
}

/// The end of a YAML front matter block (`---` ... `---` or `...` lines at the very start),
/// excluding the closing line's terminator, or 0.
fn front_matter(source: &(impl SourceText + ?Sized)) -> usize {
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
        let mut document = Self {
            preview,
            spans: Vec::new(),
            front_matter: front_matter(source),
        };
        document.spans = document.parse_range(source, 0..document.preview.blocks.len());
        document
    }

    /// The end of the front matter block, or 0 when the document has none.
    pub fn front_matter_end(&self) -> usize {
        self.front_matter
    }

    fn parse_range(
        &self,
        source: &(impl SourceText + ?Sized),
        blocks: Range<usize>,
    ) -> Vec<BlockSpans> {
        let refs = |label: &str| self.preview.refdef_dest(label).map(str::to_owned);
        self.preview.blocks[blocks]
            .iter()
            .map(|block| parse_block(&source.slice(block.bytes.clone()), &refs))
            .collect()
    }

    /// Applies edits (already in `source`); returns the byte range whose styling changed.
    pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Range<usize> {
        let changed = self.apply_blocks(source, edits);
        let old_front_matter = self.front_matter;
        self.front_matter = front_matter(source);
        if self.front_matter == old_front_matter {
            return changed;
        }
        0..changed.end.max(old_front_matter).max(self.front_matter)
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
                let fresh = self.parse_range(source, new.clone());
                self.spans.splice(old, fresh);
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
                self.spans = self.parse_range(source, 0..self.preview.blocks.len());
                0..source.len()
            }
        }
    }

    /// Blocks overlapping `bytes`: (absolute block range, spans relative to its start). Blocks
    /// that start inside the front matter are skipped.
    pub fn blocks_in(
        &self,
        bytes: Range<usize>,
    ) -> impl Iterator<Item = (Range<usize>, &BlockSpans)> {
        let first = self
            .preview
            .blocks
            .partition_point(|block| block.bytes.end <= bytes.start);
        let front_matter = self.front_matter;
        self.preview.blocks[first..]
            .iter()
            .zip(&self.spans[first..])
            .take_while(move |(block, _)| block.bytes.start < bytes.end.max(bytes.start + 1))
            .filter(move |(block, _)| block.bytes.start >= front_matter)
            .map(|(block, spans)| (block.bytes.clone(), spans))
    }

    /// Absolute spans overlapping `bytes`, sorted.
    pub fn spans_in(&self, bytes: Range<usize>) -> Vec<(Range<usize>, SpanKind)> {
        let mut out = Vec::new();
        if self.front_matter > 0 && bytes.start < self.front_matter {
            out.push((0..self.front_matter, SpanKind::Dim));
        }
        for (block, spans) in self.blocks_in(bytes.clone()) {
            for span in &spans.spans {
                let range = block.start + span.range.start..block.start + span.range.end;
                if range.end > bytes.start && range.start < bytes.end {
                    out.push((range, span.kind));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::incremental::Edit;

    /// Applies `insert` at `at` to `text` and to `doc` the way SCN_MODIFIED reports it.
    fn type_at(text: &mut String, doc: &mut LiveDocument, at: usize, insert: &str) {
        text.insert_str(at, insert);
        let edit = Edit {
            position: at,
            removed: 0,
            inserted: insert.len(),
            lines_delta: insert.matches('\n').count() as isize,
        };
        doc.apply(text.as_str(), &[edit]);
    }

    fn delete_at(text: &mut String, doc: &mut LiveDocument, at: usize, length: usize) {
        let removed: String = text.drain(at..at + length).collect();
        let edit = Edit {
            position: at,
            removed: length,
            inserted: 0,
            lines_delta: -(removed.matches('\n').count() as isize),
        };
        doc.apply(text.as_str(), &[edit]);
    }

    fn assert_matches_full_parse(text: &str, doc: &LiveDocument) {
        let full = LiveDocument::parse(text);
        assert_eq!(
            doc.spans_in(0..text.len()),
            full.spans_in(0..text.len()),
            "text: {text:?}"
        );
    }

    #[test]
    fn spans_are_absolute() {
        let doc = LiveDocument::parse("para\n\n**b**\n");
        assert_eq!(
            doc.spans_in(0..12),
            vec![
                (6..8, SpanKind::Hide),
                (8..9, SpanKind::Bold),
                (9..11, SpanKind::Hide)
            ]
        );
    }

    #[test]
    fn typing_an_open_fence_matches_a_full_parse() {
        let mut text = String::from("a\n\nb\n\n**c**\n");
        let mut doc = LiveDocument::parse(&text);
        let mut at = 3;
        for piece in ["`", "`", "`", "r", "s", "\n", "x", "\n"] {
            type_at(&mut text, &mut doc, at, piece);
            at += piece.len();
            assert_matches_full_parse(&text, &doc);
        }
        type_at(&mut text, &mut doc, at, "```\n");
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn typing_a_lone_strong_marker_matches_a_full_parse() {
        let mut text = String::from("one\n\ntwo three\n");
        let mut doc = LiveDocument::parse(&text);
        for (at, piece) in [(5, "*"), (6, "*"), (11, "*"), (12, "*")] {
            type_at(&mut text, &mut doc, at, piece);
            assert_matches_full_parse(&text, &doc);
        }
        delete_at(&mut text, &mut doc, 5, 2);
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn reference_links_use_definitions_elsewhere_in_the_document() {
        let doc = LiveDocument::parse("[a][r]\n\n[r]: https://x\n");
        let (_, block) = doc.blocks_in(0..1).next().unwrap();
        assert_eq!(block.links[0].dest, "https://x");
    }

    #[test]
    fn apply_reports_the_changed_bytes() {
        let mut text = String::from("a\n\nb\n");
        let mut doc = LiveDocument::parse(&text);
        text.insert(3, '*');
        let changed = doc.apply(
            text.as_str(),
            &[Edit {
                position: 3,
                removed: 0,
                inserted: 1,
                lines_delta: 0,
            }],
        );
        assert!(changed.start <= 3 && changed.end >= 5, "{changed:?}");
    }

    #[test]
    fn front_matter_is_one_dim_span() {
        let text = "---\ntitle: x\n---\n\n**b**\n";
        let doc = LiveDocument::parse(text);
        let spans = doc.spans_in(0..text.len());
        assert_eq!(spans[0], (0..16, SpanKind::Dim));
        assert!(
            spans[1..].iter().all(|(range, _)| range.start >= 16),
            "{spans:?}"
        );
        assert_eq!(doc.front_matter_end(), 16);
        assert!(
            doc.blocks_in(0..text.len())
                .all(|(block, _)| block.start >= 16)
        );
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
        text.insert_str(at, "---\n");
        let edit = Edit {
            position: at,
            removed: 0,
            inserted: 4,
            lines_delta: 1,
        };
        let changed = doc.apply(text.as_str(), &[edit]);
        assert_eq!(changed.start, 0);
        assert!(doc.front_matter_end() > 0);
        assert_matches_full_parse(&text, &doc);
    }
}
