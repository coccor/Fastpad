use super::*;
use crate::preview::model::parse_document;
use crate::preview::selection::text_runs;
use crate::search::MatchOptions;

fn hits_in(source: &str, query: &str, cap: usize, open: bool) -> Vec<FindHit> {
    let (blocks, _) = parse_document(source);
    let matcher = Matcher::new(query, MatchOptions::default()).unwrap();
    let runs = blocks.iter().enumerate().map(|(index, block)| {
        let runs = text_runs(&block.kind, &mut |_, _| open);
        (index, runs.into_iter().map(|run| run.text).collect())
    });
    find_hits(runs, &matcher, cap)
}

fn hit(block: usize, run: u32, start: u32, end: u32) -> FindHit {
    FindHit {
        block,
        run,
        start,
        end,
    }
}

#[test]
fn hits_are_found_in_rendered_text_not_markup() {
    // Break caught: matching the source instead of the rendered text, so `**` finds the bold
    // markers the preview never draws, or the hit sits two characters late.
    assert_eq!(hits_in("**bold** text\n", "bold", 10, true), [hit(0, 0, 0, 4)]);
    assert!(hits_in("**bold** text\n", "**", 10, true).is_empty());
}

#[test]
fn offsets_are_utf16() {
    // Break caught: byte offsets reaching DirectWrite, which counts UTF-16 units, so text after
    // an accent or an emoji is shaded a few characters late.
    assert_eq!(hits_in("é😀 x\n", "x", 10, true), [hit(0, 0, 4, 5)]);
}

#[test]
fn hits_stop_at_the_cap() {
    // Break caught: an unbounded scan of a document full of matches.
    assert_eq!(hits_in("a a a\n", "a", 2, true).len(), 2);
}

#[test]
fn a_closed_details_hides_its_hits() {
    // Break caught: shading text inside a collapsed section, which has no layout to draw on.
    let source = "<details><summary>s</summary>\n\nx\n</details>\n";
    assert!(hits_in(source, "x", 10, false).is_empty());
    assert_eq!(hits_in(source, "x", 10, true).len(), 1);
}

const ORDINAL: &str = "**a** a a\n\na\n";
const EDITOR: [std::ops::Range<usize>; 4] = [2..3, 6..7, 8..9, 11..12];

#[test]
fn the_current_hit_is_the_same_ordinal_in_its_block() {
    // Break caught: the current shade landing on the first rendered `a` of the block (or of
    // the document) instead of the one the editor's selection is.
    let (blocks, _) = parse_document(ORDINAL);
    let hits = hits_in(ORDINAL, "a", 10, true);
    assert_eq!(hits.len(), 4);
    assert_eq!(current_hit(&blocks, &hits, &EDITOR, &(6..7)), Some(1));
    assert_eq!(current_hit(&blocks, &hits, &EDITOR, &(11..12)), Some(3));
}

#[test]
fn too_few_hits_fall_back_to_the_blocks_last() {
    // Break caught: indexing past the block's hits into the next block's, or no current shade
    // at all when the preview found fewer matches than the editor.
    let (blocks, _) = parse_document(ORDINAL);
    let mut hits = hits_in(ORDINAL, "a", 10, true);
    hits.remove(2);
    assert_eq!(current_hit(&blocks, &hits, &EDITOR, &(8..9)), Some(1));
}

#[test]
fn no_hit_in_the_block_means_no_current() {
    // Break caught: falling back to a hit in another block.
    let source = "**a**\n\nb\n";
    let (blocks, _) = parse_document(source);
    let hits = hits_in(source, "a", 10, true);
    assert_eq!(current_hit(&blocks, &hits, &[2..3, 7..8], &(7..8)), None);
}
