//! The line shown under a Search result: its first match with a little context either side, cut
//! on character boundaries so painting never measures a long line. Pure.

use super::matcher::Matcher;
use std::ops::Range;

/// At most this many characters are kept before the match.
pub const BEFORE_CHARS: usize = 40;
/// At most this many characters are kept from the start of the match: the match and what follows.
pub const AFTER_CHARS: usize = 80;
const ELLIPSIS: char = '…';

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snippet {
    pub text: String,
    /// The match, as a byte range within `text`, on character boundaries.
    pub highlight: Range<usize>,
}

/// `line` cut around `hit`, a byte range within it: leading white space removed (never past the
/// hit), at most `BEFORE_CHARS` characters before the hit and `AFTER_CHARS` from its start, with
/// `…` at each cut. A hit longer than `AFTER_CHARS` characters is cut itself and ends with `…`.
/// Costs a walk of those few characters, whatever the line's length.
pub fn cut(line: &str, hit: Range<usize>) -> Snippet {
    let end = floor_boundary(line, hit.end.min(line.len()));
    let start = floor_boundary(line, hit.start.min(end));
    let lead = line.len() - line.trim_start().len();
    let before = &line[lead.min(start)..start];
    let keep_from = before
        .char_indices()
        .rev()
        .nth(BEFORE_CHARS - 1)
        .map_or(0, |(at, _)| at);
    let mut text = String::new();
    if keep_from > 0 {
        text.push(ELLIPSIS);
    }
    text.push_str(&before[keep_from..]);
    let highlight_start = text.len();
    let rest = &line[start..];
    let limit = rest
        .char_indices()
        .nth(AFTER_CHARS)
        .map_or(rest.len(), |(at, _)| at);
    let hit_len = end - start;
    let highlight = if hit_len > limit {
        text.push_str(&rest[..limit]);
        let highlight = highlight_start..text.len();
        text.push(ELLIPSIS);
        highlight
    } else {
        text.push_str(&rest[..hit_len]);
        let highlight = highlight_start..text.len();
        text.push_str(&rest[hit_len..limit]);
        if limit < rest.len() {
            text.push(ELLIPSIS);
        }
        highlight
    };
    Snippet { text, highlight }
}

/// The snippet for `text`'s first match: the line the match starts on, without its `\r`. A match
/// that runs over more lines (a regex naming `\n`) is highlighted to the end of that first line;
/// one that starts with line breaks starts on the line after them.
pub fn first_snippet(text: &str, matcher: &Matcher) -> Option<Snippet> {
    let hit = matcher.first_in(text)?;
    let matched = &text[hit.clone()];
    let breaks = matched.len() - matched.trim_start_matches(['\r', '\n']).len();
    let start = if breaks < matched.len() {
        hit.start + breaks
    } else {
        hit.start
    };
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    let line = &text[line_start..line_end];
    let line = line.strip_suffix('\r').unwrap_or(line);
    let local_start = (start - line_start).min(line.len());
    let local_end = (hit.end.min(line_start + line.len()) - line_start).max(local_start);
    Some(cut(line, local_start..local_end))
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::MatchOptions;

    fn cut_at(line: &str, needle: &str) -> Snippet {
        let start = line.find(needle).unwrap();
        cut(line, start..start + needle.len())
    }

    fn highlighted(snippet: &Snippet) -> &str {
        &snippet.text[snippet.highlight.clone()]
    }

    #[test]
    fn a_short_line_is_kept_whole() {
        let snippet = cut_at("paid the invoice march 3", "invoice");
        assert_eq!(snippet.text, "paid the invoice march 3");
        assert_eq!(snippet.highlight, 9..16);
        assert_eq!(highlighted(&snippet), "invoice");
    }

    #[test]
    fn leading_white_space_is_removed_but_never_past_the_hit() {
        // Break caught: an indented line showing as a blank-looking row, or a search for " foo"
        // losing the space it matched.
        let snippet = cut_at("\t   - send invoice", "invoice");
        assert_eq!(snippet.text, "- send invoice");
        assert_eq!(highlighted(&snippet), "invoice");
        let snippet = cut("    foo", 2..7);
        assert_eq!(snippet.text, "  foo");
        assert_eq!(snippet.highlight, 0..5);
    }

    #[test]
    fn a_long_line_is_cut_before_and_after_the_hit() {
        let before = "b".repeat(100);
        let after = "a".repeat(200);
        let line = format!("{before}HIT{after}");
        let snippet = cut_at(&line, "HIT");
        let expected = format!("…{}HIT{}…", "b".repeat(40), "a".repeat(77));
        assert_eq!(snippet.text, expected);
        assert_eq!(highlighted(&snippet), "HIT");
        assert_eq!(snippet.highlight.start, '…'.len_utf8() + 40);
    }

    #[test]
    fn exactly_the_limits_are_kept_without_an_ellipsis() {
        // Break caught: an off-by-one that adds `…` though nothing was cut, or drops a character.
        let line = format!("{}HIT{}", "b".repeat(40), "a".repeat(77));
        assert_eq!(cut_at(&line, "HIT").text, line);
        let line = format!("{}HIT{}", "b".repeat(41), "a".repeat(78));
        let snippet = cut_at(&line, "HIT");
        assert!(snippet.text.starts_with('…') && snippet.text.ends_with('…'));
        assert_eq!(snippet.text.chars().count(), 1 + 40 + 80 + 1);
    }

    #[test]
    fn a_hit_at_the_very_start_or_end_of_the_line_cuts_cleanly() {
        let snippet = cut_at("invoice sent", "invoice");
        assert_eq!(
            (snippet.text.as_str(), snippet.highlight.clone()),
            ("invoice sent", 0..7)
        );
        let snippet = cut_at("sent the invoice", "invoice");
        assert_eq!(snippet.highlight, 9..16);
        assert_eq!(snippet.highlight.end, snippet.text.len());
        let line = format!("{}invoice", "x".repeat(60));
        let snippet = cut_at(&line, "invoice");
        assert_eq!(snippet.text, format!("…{}invoice", "x".repeat(40)));
        assert_eq!(snippet.highlight.end, snippet.text.len());
    }

    #[test]
    fn a_hit_longer_than_the_after_limit_is_cut_itself() {
        let line = format!("ab {} cd", "h".repeat(100));
        let start = 3;
        let snippet = cut(&line, start..start + 100);
        assert_eq!(snippet.text, format!("ab {}…", "h".repeat(80)));
        assert_eq!(highlighted(&snippet), "h".repeat(80));
    }

    #[test]
    fn a_cut_never_splits_a_multibyte_character() {
        // Break caught: counting bytes instead of characters, which panics slicing inside `é` or
        // an emoji, or shows 40 bytes (20 characters) of Cyrillic context.
        let line = format!("{}Жук{}", "é😀".repeat(30), "ж".repeat(100));
        let snippet = cut_at(&line, "Жук");
        assert_eq!(highlighted(&snippet), "Жук");
        let before: String = snippet.text[..snippet.highlight.start].chars().collect();
        assert_eq!(before.chars().count(), 1 + 40);
        assert!(before.starts_with('…'));
        let after = &snippet.text[snippet.highlight.end..];
        assert_eq!(after.chars().count(), 77 + 1);
        assert!(after.ends_with('…'));
        // A range that is not on character boundaries is moved back onto them, never panicking.
        let snippet = cut("éé", 1..3);
        assert_eq!(snippet.text, "éé");
        assert_eq!(highlighted(&snippet), "é");
    }

    #[test]
    fn the_first_snippet_is_the_line_of_the_first_match_without_its_carriage_return() {
        let matcher = Matcher::new("invoice", MatchOptions::default()).unwrap();
        let text = "Notes\r\n\r\n    paid the INVOICE\r\nlater invoice\r\n";
        let snippet = first_snippet(text, &matcher).unwrap();
        assert_eq!(snippet.text, "paid the INVOICE");
        assert_eq!(highlighted(&snippet), "INVOICE");
        assert_eq!(first_snippet("nothing here", &matcher), None);
        let at_end = first_snippet("x\ninvoice", &matcher).unwrap();
        assert_eq!(at_end.text, "invoice");
    }

    #[test]
    fn a_folded_match_highlights_the_original_characters() {
        let matcher = Matcher::new("école", MatchOptions::default()).unwrap();
        let snippet = first_snippet("\u{212A} at the ÉCOLE today", &matcher).unwrap();
        assert_eq!(highlighted(&snippet), "ÉCOLE");
    }

    #[test]
    fn a_match_over_several_lines_is_highlighted_to_the_end_of_its_first_line() {
        let regex = MatchOptions {
            regex: true,
            ..MatchOptions::default()
        };
        let matcher = Matcher::new(r"march\r?\n\d+", regex).unwrap();
        let snippet = first_snippet("intro\r\npaid in march\r\n12 days", &matcher).unwrap();
        assert_eq!(snippet.text, "paid in march");
        assert_eq!(highlighted(&snippet), "march");
        // A match that starts with the line break starts on the next line.
        let matcher = Matcher::new(r"\n\d+", regex).unwrap();
        let snippet = first_snippet("paid\n12 days", &matcher).unwrap();
        assert_eq!(snippet.text, "12 days");
        assert_eq!(highlighted(&snippet), "12");
        // A match of line breaks alone shows the line before them, highlighting nothing.
        let matcher = Matcher::new(r"\n\n", regex).unwrap();
        let snippet = first_snippet("a\n\nb", &matcher).unwrap();
        assert_eq!(snippet.text, "a");
        assert_eq!(snippet.highlight, 1..1);
    }
}
