//! The find bar's matches in the rendered preview (2026-11-19 preview find highlights spec): the
//! query runs on each run's rendered text, and the current match is the one in the same block,
//! at the same position in order, as the editor's selection.

use crate::preview::model::Block;
use crate::preview::selection::RunText;
use crate::search::Matcher;
use std::ops::Range;

/// A match in the rendered text: block `block`'s `run`-th run (`selection::text_runs`), UTF-16
/// offsets `start..end` into the run's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindHit {
    pub block: usize,
    pub run: u32,
    pub start: u32,
    pub end: u32,
}

/// What the preview shades: the find bar's query, and the editor's matches of it (source byte
/// ranges, ascending) with the current one, which picks the preview's current match.
#[derive(Clone, Debug)]
pub struct PreviewFind {
    pub matcher: Matcher,
    pub editor_matches: Vec<Range<usize>>,
    pub current: Option<Range<usize>>,
}

/// The matches of `matcher` in each block's runs, in document order, at most `cap`. A match never
/// spans two runs.
pub fn find_hits<'a>(
    blocks: impl Iterator<Item = (usize, Vec<RunText<'a>>)>,
    matcher: &Matcher,
    cap: usize,
) -> Vec<FindHit> {
    let mut hits = Vec::new();
    for (block, runs) in blocks {
        for (run, text) in runs.into_iter().enumerate() {
            let text = match text {
                RunText::Rich(rich) => rich.text.as_str(),
                RunText::Code(code) => code,
            };
            let remaining = cap - hits.len();
            let found = matcher.find_up_to(text, remaining);
            // Byte offsets to UTF-16, walking forward once per run.
            let mut units = 0_u32;
            let mut byte = 0;
            let mut utf16 = |at: usize| {
                for c in text[byte..at].chars() {
                    units += c.len_utf16() as u32;
                }
                byte = at;
                units
            };
            for range in found {
                let start = utf16(range.start);
                let end = utf16(range.end);
                hits.push(FindHit {
                    block,
                    run: run as u32,
                    start,
                    end,
                });
            }
            if hits.len() >= cap {
                return hits;
            }
        }
    }
    hits
}

/// The index in `hits` of the preview's current match: in the block holding `current`, the
/// match at the same position as `current` among `editor_matches` in that block, else that
/// block's last. `None` when that block has no hits.
pub fn current_hit(
    blocks: &[Block],
    hits: &[FindHit],
    editor_matches: &[Range<usize>],
    current: &Range<usize>,
) -> Option<usize> {
    let block = blocks
        .iter()
        .position(|block| block.bytes.contains(&current.start))?;
    let bytes = &blocks[block].bytes;
    let ordinal = editor_matches
        .iter()
        .filter(|range| bytes.contains(&range.start) && range.start < current.start)
        .count();
    let first = hits.partition_point(|hit| hit.block < block);
    let count = hits[first..]
        .iter()
        .take_while(|hit| hit.block == block)
        .count();
    (count > 0).then(|| first + ordinal.min(count - 1))
}

#[cfg(test)]
mod tests;
