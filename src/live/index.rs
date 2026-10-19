//! Live's block index: the byte ranges of the document's top-level blocks and its reference
//! definitions, kept in step with edits by reparsing only the blocks around them. It follows the
//! preview's incremental rule (`preview/incremental.rs`): a reparsed slice is accepted only when
//! the unchanged sentinel blocks at both ends parse back to the same ranges, which proves the
//! parser state at the slice's edges did not change; otherwise the slice widens and finally the
//! whole document is parsed. Unlike the preview's document it builds no rendering model, so a
//! keystroke in a long paragraph costs one parser pass over it and nothing more.

use super::spans::LIVE_PARSE_OPTIONS;
use crate::preview::incremental::{Edit, SourceText, Update};
use crate::preview::model::{PARSE_OPTIONS, normalize_label};
use pulldown_cmark::{Event, Parser};
use std::ops::Range;

/// Unchanged blocks reparsed on each side of an edit, to prove the slice's edges.
const SENTINELS: usize = 2;
const MAX_WIDENINGS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Definition {
    /// The label, normalized as CommonMark matches labels.
    key: String,
    dest: String,
    span: Range<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockIndex {
    /// Each top-level block's bytes, in order.
    pub blocks: Vec<Range<usize>>,
    definitions: Vec<Definition>,
    /// The text starts with a `---` line: front matter may end anywhere below, so every edit
    /// parses the whole document.
    front_matter: bool,
}

/// Whether `text` starts with a `---` line (after a byte order mark).
fn may_start_front_matter(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.strip_prefix("---")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('\n') || rest.starts_with("\r\n"))
}

/// The top-level blocks of `text` (which starts at document byte `base`). Only a slice at the
/// document's start may hold front matter: elsewhere a `---` block is a rule or a heading.
fn parse_blocks(text: &str, base: usize) -> (Vec<Range<usize>>, Vec<Definition>) {
    let options = if base == 0 {
        PARSE_OPTIONS
    } else {
        LIVE_PARSE_OPTIONS
    };
    let mut parser = Parser::new_ext(text, options).into_offset_iter();
    let mut blocks = Vec::new();
    let mut depth = 0_usize;
    let mut open: Option<Range<usize>> = None;
    for (event, range) in parser.by_ref() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    open = Some(range);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some(start) = open.take()
                {
                    blocks.push(base + start.start..base + start.end.max(range.end));
                }
            }
            // A top-level leaf: a thematic break.
            _ if depth == 0 => blocks.push(base + range.start..base + range.end),
            _ => {}
        }
    }
    let definitions = parser
        .reference_definitions()
        .iter()
        .map(|(label, definition)| Definition {
            key: normalize_label(label),
            dest: definition.dest.to_string(),
            span: base + definition.span.start..base + definition.span.end,
        })
        .collect();
    (blocks, definitions)
}

impl BlockIndex {
    pub fn parse(source: &str) -> Self {
        let (blocks, definitions) = parse_blocks(source, 0);
        Self {
            blocks,
            definitions,
            front_matter: may_start_front_matter(source),
        }
    }

    pub fn reparse(&mut self, source: &(impl SourceText + ?Sized)) -> Update {
        *self = Self::parse(&source.slice(0..source.len()));
        Update::Full
    }

    /// The destination of the reference definition whose label matches `label`.
    pub fn refdef_dest(&self, label: &str) -> Option<&str> {
        let key = normalize_label(label);
        self.definitions
            .iter()
            .find(|definition| definition.key == key)
            .map(|definition| definition.dest.as_str())
    }

    /// Applies `edits`, already in `source`, reparsing as little as proves correct.
    pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Update {
        match self.try_apply(source, edits) {
            Some(update) => update,
            None => self.reparse(source),
        }
    }

    fn try_apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Option<Update> {
        if edits.is_empty() {
            return Some(Update::Unchanged);
        }
        if self.front_matter || may_start_front_matter(&source.head(8)) {
            return None;
        }
        let dirty = self.shift_for_edits(edits)?;
        let count = self.blocks.len();
        let first = self.blocks.partition_point(|block| block.end < dirty.start);
        let last = self
            .blocks
            .partition_point(|block| block.start <= dirty.end);
        let mut low = first.saturating_sub(SENTINELS);
        let mut high = (last.max(first) + SENTINELS).min(count);
        for _ in 0..=MAX_WIDENINGS {
            if low == 0 && high == count {
                // The whole document: a full parse says the same, with less bookkeeping.
                return None;
            }
            let start = if low == 0 { 0 } else { self.blocks[low].start };
            let end = if high == count {
                source.len()
            } else {
                self.blocks[high - 1].end
            };
            let text = source.slice(start..end);
            // A definition in the slice changes references anywhere.
            if text.contains("]:") {
                return None;
            }
            let (mut parsed, _) = parse_blocks(&text, start);
            let start_ok = low == 0 || parsed.first() == Some(&self.blocks[low]);
            let end_ok = high == count || parsed.last() == Some(&self.blocks[high - 1]);
            if start_ok && end_ok {
                let previous = &self.blocks[low..high];
                let same_start = previous
                    .iter()
                    .zip(&parsed)
                    .take_while(|(old, new)| old == new)
                    .count();
                let same_end = previous[same_start..]
                    .iter()
                    .rev()
                    .zip(parsed[same_start..].iter().rev())
                    .take_while(|(old, new)| old == new)
                    .count();
                let changed = same_start..parsed.len() - same_end;
                let old = low + same_start..high - same_end;
                let new = low + same_start..low + changed.end;
                self.blocks.splice(old.clone(), parsed.drain(changed));
                return Some(Update::Replaced { old, new });
            }
            if !start_ok {
                low = low.saturating_sub(1);
            }
            if !end_ok {
                high = (high + 1).min(count);
            }
        }
        None
    }

    /// Moves block and definition ranges through `edits` in order and returns the edited bytes
    /// in final coordinates, or `None` when an edit touches a reference definition.
    fn shift_for_edits(&mut self, edits: &[Edit]) -> Option<Range<usize>> {
        let mut dirty: Option<Range<usize>> = None;
        for edit in edits {
            let removed_end = edit.position + edit.removed;
            if self.definitions.iter().any(|definition| {
                definition.span.start <= removed_end && edit.position <= definition.span.end
            }) {
                return None;
            }
            let first = self
                .blocks
                .partition_point(|block| block.end < edit.position);
            for block in &mut self.blocks[first..] {
                *block = shift_range(block.clone(), edit);
            }
            for definition in &mut self.definitions {
                definition.span = shift_range(definition.span.clone(), edit);
            }
            let touched = edit.position..edit.position + edit.inserted;
            dirty = Some(match dirty {
                None => touched,
                Some(previous) => {
                    let previous = shift_range(previous, edit);
                    previous.start.min(touched.start)..previous.end.max(touched.end)
                }
            });
        }
        dirty
    }
}

/// `position` after `edit`, as the preview's index moves it: a position inside the removed bytes
/// goes past the inserted text.
fn shift_position(position: usize, edit: &Edit, is_end: bool) -> usize {
    let removed_end = edit.position + edit.removed;
    if position < edit.position || (position == edit.position && !is_end) {
        position
    } else if position >= removed_end {
        position - edit.removed + edit.inserted
    } else {
        edit.position + edit.inserted
    }
}

fn shift_range(range: Range<usize>, edit: &Edit) -> Range<usize> {
    let start = shift_position(range.start, edit, false);
    start..shift_position(range.end, edit, true).max(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::text::StrSource;

    fn edit(text: &mut String, at: usize, removed: usize, insert: &str) -> Edit {
        let gone: String = text.drain(at..at + removed).collect();
        text.insert_str(at, insert);
        Edit {
            position: at,
            removed,
            inserted: insert.len(),
            lines_delta: insert.matches('\n').count() as isize
                - gone.matches('\n').count() as isize,
        }
    }

    #[test]
    fn blocks_are_top_level_ranges() {
        let text = "# A\n\npara\n\n- x\n  - y\n\n---\n\n```\nc\n```\n";
        let index = BlockIndex::parse(text);
        let shown = index
            .blocks
            .iter()
            .map(|range| text[range.clone()].trim_end())
            .collect::<Vec<_>>();
        assert_eq!(shown, ["# A", "para", "- x\n  - y", "---", "```\nc\n```"]);
    }

    #[test]
    fn a_rule_later_in_the_document_is_not_front_matter() {
        let mut text = String::from("para\n\nmore\n\n---\nkey: v\n---\n\nend\n");
        let mut index = BlockIndex::parse(&text);
        let typed = edit(&mut text, 0, 0, "x");
        index.apply(&StrSource(&text), &[typed]);
        assert_eq!(index, BlockIndex::parse(&text));
    }

    #[test]
    fn an_edit_in_one_paragraph_replaces_only_its_block() {
        let mut text = String::from("one\n\ntwo\n\nthree\n\nfour\n\nfive\n\nsix\n");
        let mut index = BlockIndex::parse(&text);
        let at = text.find("four").unwrap() + 2;
        let typed = edit(&mut text, at, 0, "x");
        // A letter typed in a paragraph keeps every range: no block is replaced (the document
        // forgets the touched block's spans itself), and nothing was parsed whole.
        let update = index.apply(&StrSource(&text), &[typed]);
        assert!(
            matches!(&update, Update::Replaced { old, new } if old.is_empty() && new.is_empty()),
            "{update:?}"
        );
        assert_eq!(index, BlockIndex::parse(&text));
        // A blank line typed in it splits it into two blocks.
        let at = text.find("foxur").unwrap() + 2;
        let split = edit(&mut text, at, 0, "\n\n");
        assert_eq!(
            index.apply(&StrSource(&text), &[split]),
            Update::Replaced {
                old: 3..4,
                new: 3..5
            }
        );
        assert_eq!(index, BlockIndex::parse(&text));
    }

    #[test]
    fn references_resolve_and_editing_one_parses_everything() {
        let mut text = String::from("[a][r]\n\n[r]: https://x\n");
        let mut index = BlockIndex::parse(&text);
        assert_eq!(index.refdef_dest("R"), Some("https://x"));
        let at = text.find("x\n").unwrap();
        let typed = edit(&mut text, at, 1, "y");
        assert_eq!(index.apply(&StrSource(&text), &[typed]), Update::Full);
        assert_eq!(index.refdef_dest("r"), Some("https://y"));
    }
}
