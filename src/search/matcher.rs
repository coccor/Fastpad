//! Text matching for the note search: a plain phrase or a regex, with match case and whole word,
//! one line at a time. Pure: no Win32 and no disk.

use regex::{Regex, RegexBuilder};
use std::fmt;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatchOptions {
    /// Match case. Off, both sides are folded one character at a time.
    pub case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchOption {
    Case,
    WholeWord,
    Regex,
}

impl SearchOption {
    /// In the order the toggles are drawn.
    pub const ALL: [SearchOption; 3] = [Self::Case, Self::WholeWord, Self::Regex];
}

impl MatchOptions {
    pub fn get(self, option: SearchOption) -> bool {
        match option {
            SearchOption::Case => self.case,
            SearchOption::WholeWord => self.whole_word,
            SearchOption::Regex => self.regex,
        }
    }

    pub fn toggled(mut self, option: SearchOption) -> MatchOptions {
        match option {
            SearchOption::Case => self.case = !self.case,
            SearchOption::WholeWord => self.whole_word = !self.whole_word,
            SearchOption::Regex => self.regex = !self.regex,
        }
        self
    }
}

/// Why a query can't run, worded for the line under the search box.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternError {
    pub message: String,
}

const EMPTY_MATCH: &str = "The pattern matches empty text.";
const TOO_LARGE: &str = "The pattern is too large.";

impl PatternError {
    fn empty_match() -> PatternError {
        PatternError {
            message: EMPTY_MATCH.to_owned(),
        }
    }

    /// The description from the crate's message. A syntax error reads "regex parse error:", the
    /// pattern with a caret under the error, then "error: <description>" on the last line; only
    /// that description is kept, with a capital first letter ("Unclosed group").
    fn from_regex(error: &regex::Error) -> PatternError {
        let message = match error {
            regex::Error::CompiledTooBig(_) => TOO_LARGE.to_owned(),
            other => {
                let text = other.to_string();
                let last = text
                    .lines()
                    .map(str::trim)
                    .rfind(|line| !line.is_empty())
                    .unwrap_or_default();
                capitalized(last.strip_prefix("error:").map_or(last, str::trim))
            }
        };
        PatternError { message }
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PatternError {}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The one character outside ASCII whose single-character lowercase is ASCII (U+212A → `k`).
/// A test proves no other character does.
const KELVIN_SIGN: char = '\u{212A}';

#[derive(Clone, Debug)]
pub struct Matcher {
    query: String,
    options: MatchOptions,
    engine: Engine,
    /// Whether the text is matched one line at a time. Off, the whole text is one haystack: for a
    /// regex that names `\n`, and for a plain phrase without `\r` or `\n`, which can't span lines
    /// or take in a line's `\r` anyway, so one search over the whole text gives the same matches.
    per_line: bool,
}

#[derive(Clone, Debug)]
enum Engine {
    /// A literal phrase. `direct` runs on the text as it is: always with match case, and without
    /// it for an ASCII phrase on a text without the Kelvin sign, where folding changes nothing but
    /// ASCII letters (it is then ASCII case-insensitive). `folded` runs on the folded text; it is
    /// set only without match case.
    Plain {
        direct: Option<Regex>,
        folded: Option<Regex>,
    },
    Regex(Regex),
}

impl Matcher {
    pub fn new(query: &str, options: MatchOptions) -> Result<Matcher, PatternError> {
        let (engine, per_line) = if options.regex {
            regex_engine(query, options)?
        } else {
            plain_engine(query, options)?
        };
        Ok(Matcher {
            query: query.to_owned(),
            options,
            engine,
            per_line,
        })
    }

    pub fn options(&self) -> MatchOptions {
        self.options
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The byte range of the first match in `text`.
    pub fn first_in(&self, text: &str) -> Option<Range<usize>> {
        let mut first = None;
        self.each_match(text, &mut |range| {
            first = Some(range);
            false
        });
        first
    }

    /// Every match in `text`, in order, none overlapping.
    pub fn find_iter(&self, text: &str) -> Vec<Range<usize>> {
        let mut all = Vec::new();
        self.each_match(text, &mut |range| {
            all.push(range);
            true
        });
        all
    }

    /// Calls `emit` with each match until it returns false.
    fn each_match(&self, text: &str, emit: &mut dyn FnMut(Range<usize>) -> bool) {
        if !self.per_line {
            self.segment_matches(text, 0, emit);
            return;
        }
        let mut start = 0;
        for line in text.split('\n') {
            let body = line.strip_suffix('\r').unwrap_or(line);
            if !self.segment_matches(body, start, emit) {
                return;
            }
            start += line.len() + 1;
        }
    }

    /// The matches in `segment`, which starts at byte `base` of the text. Returns false once
    /// `emit` asks to stop.
    fn segment_matches(
        &self,
        segment: &str,
        base: usize,
        emit: &mut dyn FnMut(Range<usize>) -> bool,
    ) -> bool {
        let mut shifted = |range: Range<usize>| emit(range.start + base..range.end + base);
        match &self.engine {
            Engine::Regex(regex) => {
                for found in regex.find_iter(segment) {
                    // A pattern that passed the empty check can still match empty text at a
                    // position (`\b`); those are never matches.
                    if !found.is_empty() && !shifted(found.range()) {
                        return false;
                    }
                }
                true
            }
            Engine::Plain { direct, folded } => {
                let whole_word = self.options.whole_word;
                if let Some(direct) = direct
                    && (self.options.case || !segment.contains(KELVIN_SIGN))
                {
                    let mut walker = Walker::new(segment, true);
                    return literal_matches(
                        direct,
                        segment,
                        segment,
                        whole_word,
                        &mut walker,
                        &mut shifted,
                    );
                }
                let Some(folded) = folded else {
                    return true;
                };
                let (folded_text, same_lengths) = fold(segment);
                let mut walker = Walker::new(segment, same_lengths);
                literal_matches(
                    folded,
                    &folded_text,
                    segment,
                    whole_word,
                    &mut walker,
                    &mut shifted,
                )
            }
        }
    }
}

/// `regex::escape`, for text that must match as it is in regex mode.
pub fn escape(text: &str) -> String {
    regex::escape(text)
}

fn plain_engine(query: &str, options: MatchOptions) -> Result<(Engine, bool), PatternError> {
    if query.is_empty() {
        return Err(PatternError::empty_match());
    }
    let per_line = query.contains(['\r', '\n']);
    let engine = if options.case {
        Engine::Plain {
            direct: Some(literal(query, false)?),
            folded: None,
        }
    } else {
        let direct = if query.is_ascii() {
            Some(literal(query, true)?)
        } else {
            None
        };
        Engine::Plain {
            direct,
            folded: Some(literal(&fold(query).0, false)?),
        }
    };
    Ok((engine, per_line))
}

fn regex_engine(query: &str, options: MatchOptions) -> Result<(Engine, bool), PatternError> {
    // Matching is per line unless the pattern names a newline, in which case the whole text is
    // matched in one pass and `multi_line` keeps `^`/`$` anchored to each line inside it rather
    // than only to the start/end of the whole text.
    let per_line = !(query.contains('\n') || query.contains(r"\n"));
    let multi_line = !per_line;
    // The pattern is checked alone first: wrapped for whole word, `a)|(b` would compile, and
    // `a*` would no longer match "".
    let inner = build(query, !options.case, multi_line)?;
    if inner.is_match("") {
        return Err(PatternError::empty_match());
    }
    let regex = if options.whole_word {
        build(&format!(r"\b(?:{query})\b"), !options.case, multi_line)?
    } else {
        inner
    };
    Ok((Engine::Regex(regex), per_line))
}

fn build(pattern: &str, case_insensitive: bool, multi_line: bool) -> Result<Regex, PatternError> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .multi_line(multi_line)
        .build()
        .map_err(|error| PatternError::from_regex(&error))
}

/// A regex matching `text` literally. `ascii_case_insensitive` folds ASCII letters only: the
/// crate's Unicode folding would also match `ſ` for `s`, which `to_lowercase` folding doesn't.
fn literal(text: &str, ascii_case_insensitive: bool) -> Result<Regex, PatternError> {
    RegexBuilder::new(&regex::escape(text))
        .case_insensitive(ascii_case_insensitive)
        .unicode(!ascii_case_insensitive)
        .build()
        .map_err(|error| PatternError::from_regex(&error))
}

/// `c` lowercased when that gives one character, else `c` itself.
fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// `text` folded one character for one, and whether every character kept its UTF-8 length (so
/// byte offsets in the folded text are offsets in `text`).
fn fold(text: &str) -> (String, bool) {
    let mut folded = String::with_capacity(text.len());
    let mut same_lengths = true;
    for c in text.chars() {
        let lower = fold_char(c);
        same_lengths &= lower.len_utf8() == c.len_utf8();
        folded.push(lower);
    }
    (folded, same_lengths)
}

/// Maps byte offsets in the folded text back to `original`, walking both forward from the last
/// offset it was moved to. Folding is one character for one, so every folded character boundary
/// is an original one.
struct Walker<'a> {
    original: &'a str,
    identity: bool,
    folded_at: usize,
    original_at: usize,
}

impl<'a> Walker<'a> {
    fn new(original: &'a str, identity: bool) -> Self {
        Walker {
            original,
            identity,
            folded_at: 0,
            original_at: 0,
        }
    }

    fn walk(&self, target: usize) -> (usize, usize) {
        debug_assert!(target >= self.folded_at);
        let (mut folded_at, mut original_at) = (self.folded_at, self.original_at);
        for c in self.original[original_at..].chars() {
            if folded_at >= target {
                break;
            }
            folded_at += fold_char(c).len_utf8();
            original_at += c.len_utf8();
        }
        debug_assert_eq!(folded_at, target);
        (folded_at, original_at)
    }

    /// The original offset of `target`, which becomes the new starting point.
    fn seek(&mut self, target: usize) -> usize {
        if self.identity {
            return target;
        }
        (self.folded_at, self.original_at) = self.walk(target);
        self.original_at
    }

    /// The original offset of `target`, leaving the starting point where it is.
    fn peek(&self, target: usize) -> usize {
        if self.identity {
            target
        } else {
            self.walk(target).1
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether neither the character before `range` nor the one after it is a word character.
fn at_word_boundaries(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let after = text[range.end..].chars().next();
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

/// The literal's matches in `haystack` (the text, or its folding), mapped to `original`. With
/// whole word, a match next to a word character is passed over and the search goes on from the
/// character after its start, so "foobar foo" still finds the second "foo".
fn literal_matches(
    regex: &Regex,
    haystack: &str,
    original: &str,
    whole_word: bool,
    walker: &mut Walker<'_>,
    emit: &mut dyn FnMut(Range<usize>) -> bool,
) -> bool {
    let mut from = 0;
    while let Some(found) = regex.find_at(haystack, from) {
        let start = walker.seek(found.start());
        let end = walker.peek(found.end());
        if !whole_word || at_word_boundaries(original, &(start..end)) {
            if !emit(start..end) {
                return false;
            }
            walker.seek(found.end());
            from = found.end();
        } else {
            from = found.start()
                + haystack[found.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
        }
    }
    true
}

#[cfg(test)]
// Expected matches are byte ranges; a one-range list is not a mistaken `(a..b).collect()`.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn options(case: bool, whole_word: bool, regex: bool) -> MatchOptions {
        MatchOptions {
            case,
            whole_word,
            regex,
        }
    }

    fn matcher(query: &str, options: MatchOptions) -> Matcher {
        Matcher::new(query, options).unwrap()
    }

    fn error(query: &str, options: MatchOptions) -> String {
        Matcher::new(query, options).unwrap_err().message
    }

    #[test]
    fn every_combination_of_the_three_options_matches_as_described() {
        // Break caught: one option silently ignored in one mode, e.g. match case dropped in regex
        // mode or whole word only applied to plain text.
        let text = "Foo foo_bar food (foo) FOO.";
        let any_case = vec![0..3, 4..7, 12..15, 18..21, 23..26];
        let exact_case = vec![4..7, 12..15, 18..21];
        let whole_words = vec![0..3, 18..21, 23..26];
        let exact_whole_word = vec![18..21];
        for regex in [false, true] {
            let cases = [
                (options(false, false, regex), &any_case),
                (options(true, false, regex), &exact_case),
                (options(false, true, regex), &whole_words),
                (options(true, true, regex), &exact_whole_word),
            ];
            for (options, expected) in cases {
                let found = matcher("foo", options);
                assert_eq!(&found.find_iter(text), expected, "{options:?}");
                assert_eq!(
                    found.first_in(text),
                    expected.first().cloned(),
                    "{options:?}"
                );
            }
        }
        let pattern = matcher(r"fo+d?", options(false, false, true));
        assert_eq!(pattern.first_in("xx FOOOD"), Some(3..8));
        let pattern = matcher(r"fo+d?", options(true, false, true));
        assert_eq!(pattern.first_in("xx FOOOD"), None);
    }

    #[test]
    fn folding_matches_letters_with_a_single_character_lowercase() {
        // Break caught: case-insensitive search that only folds ASCII, so "école" misses "ÉCOLE".
        let any_case = MatchOptions::default();
        let text = "ÉCOLE école ЖУК жук";
        assert_eq!(matcher("école", any_case).find_iter(text), [0..6, 7..13]);
        assert_eq!(matcher("жук", any_case).find_iter(text), [14..20, 21..27]);
        assert_eq!(
            matcher("école", options(true, false, false)).find_iter(text),
            [7..13]
        );
        // `ß` has no one-character lowercase other than itself; it is not "ss".
        assert_eq!(matcher("ss", any_case).first_in("Straße"), None);
        assert_eq!(matcher("STRASSE", any_case).first_in("Straße"), None);
        assert_eq!(matcher("ẞ", any_case).first_in("Straße"), Some(4..6));
    }

    #[test]
    fn folding_maps_offsets_back_to_the_original_text() {
        // Break caught: a highlight shifted by the bytes folding added or removed. The Kelvin sign
        // (3 bytes) folds to `k` (1 byte) and the Ohm sign (3 bytes) to `ω` (2 bytes), so every
        // offset after them differs between the folded and the original text.
        let any_case = MatchOptions::default();
        let text = "\u{212A}\u{212A}\u{212A} \u{2126}hm KELVIN ohm";
        let kkk = matcher("kkk", any_case).first_in(text).unwrap();
        assert_eq!(&text[kkk], "\u{212A}\u{212A}\u{212A}");
        let ohm = matcher("ωHM", any_case).first_in(text).unwrap();
        assert_eq!(&text[ohm], "\u{2126}hm");
        let kelvin = matcher("kelvin", any_case).first_in(text).unwrap();
        assert_eq!(&text[kelvin], "KELVIN");
        let every_k = matcher("k", any_case).find_iter(text);
        assert_eq!(every_k, [0..3, 3..6, 6..9, 16..17]);
        // Whole word walks the same mapping: the first hit is followed by `_`, the second isn't.
        let text = "\u{212A}elvin_x \u{212A}ELVIN.";
        let found = matcher("kelvin", options(false, true, false)).find_iter(text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].clone()], "\u{212A}ELVIN");
        // Start and end of the text.
        let text = "Élan … ÉLAN";
        assert_eq!(
            matcher("élan", any_case).find_iter(text),
            [0..5, text.len() - 5..text.len()]
        );
    }

    #[test]
    fn only_the_kelvin_sign_folds_into_ascii() {
        // Break caught: a future Unicode version adding a character whose lowercase is ASCII, which
        // the ASCII fast path would then miss.
        let folding_into_ascii: Vec<char> = (0..=char::MAX as u32)
            .filter_map(char::from_u32)
            .filter(|c| !c.is_ascii() && fold_char(*c).is_ascii())
            .collect();
        assert_eq!(folding_into_ascii, [KELVIN_SIGN]);
    }

    #[test]
    fn whole_word_skips_a_partial_hit_and_finds_the_next() {
        // Break caught: whole word giving up after the first partial hit, so "foobar foo" never
        // finds the "foo" at its end.
        for regex in [false, true] {
            for case in [false, true] {
                let found = matcher("foo", options(case, true, regex));
                assert_eq!(found.first_in("foobar foo"), Some(7..10), "regex {regex}");
                assert_eq!(found.find_iter("foofoo foo xfoo foo"), [7..10, 16..19]);
            }
        }
        let found = matcher("école", options(false, true, false));
        assert_eq!(found.first_in("ÉCOLEs École"), Some(8..14));
    }

    #[test]
    fn whole_word_looks_at_underscores_digits_letters_and_punctuation_on_both_sides() {
        // Break caught: `_` or a digit counted as a word break, so whole word "foo" matched
        // "foo_bar" or "foo2".
        for regex in [false, true] {
            let found = matcher("foo", options(false, true, regex));
            for text in ["foo", "foo.", "(foo)", "a foo, b", "-foo-", "\"foo\""] {
                assert!(found.first_in(text).is_some(), "{text:?}, regex {regex}");
            }
            for text in [
                "foo_bar", "bar_foo", "foo2", "2foo", "éfoo", "fooé", "_foo_",
            ] {
                assert_eq!(found.first_in(text), None, "{text:?}, regex {regex}");
            }
        }
    }

    #[test]
    fn a_regex_error_shows_the_crates_description_with_a_capital() {
        // Break caught: the multi-line "regex parse error:" block with its caret shown under the
        // search box instead of a short message.
        let regex = options(false, false, true);
        assert_eq!(error("(abc", regex), "Unclosed group");
        assert_eq!(error("a)", regex), "Unopened group");
        assert_eq!(error("[a", regex), "Unclosed character class");
        assert_eq!(error("*a", regex), "Repetition operator missing expression");
        assert_eq!(error(r"\p{Nope}", regex), "Unicode property not found");
        assert_eq!(
            error("(a\nb", regex),
            "Unclosed group",
            "a pattern with a newline"
        );
        assert_eq!(
            error("a)|(b", options(false, true, true)),
            "Unopened group",
            "the pattern is checked before it is wrapped for whole word"
        );
        assert_eq!(error(r"(?:\w{100}){200}", regex), TOO_LARGE);
        let shown = Matcher::new("(abc", regex).unwrap_err();
        assert_eq!(shown.to_string(), "Unclosed group");
    }

    #[test]
    fn a_pattern_that_matches_empty_text_is_rejected() {
        // Break caught: `a*` accepted, giving an empty "match" in every note.
        for query in ["", "a*", "(?:)", "x?", "^", "$", "a|"] {
            assert_eq!(
                error(query, options(false, false, true)),
                EMPTY_MATCH,
                "{query:?}"
            );
        }
        assert_eq!(
            error("a*", options(false, true, true)),
            EMPTY_MATCH,
            "whole word does not hide an empty match"
        );
        assert_eq!(error("", MatchOptions::default()), EMPTY_MATCH);
        // `\b` matches no "" but does match empty text between characters: it compiles, and its
        // empty matches are never reported.
        let boundary = matcher(r"\b", options(false, false, true));
        assert_eq!(boundary.first_in("foo bar"), None);
        assert!(boundary.find_iter("foo bar").is_empty());
    }

    #[test]
    fn matches_never_span_lines_or_take_in_a_lines_carriage_return() {
        // Break caught: `foo\s+bar` matching across a line break, or `\s+$` highlighting the `\r`
        // of a CRLF line.
        let regex = options(false, false, true);
        assert_eq!(matcher(r"foo\s*bar", regex).first_in("foo\r\nbar"), None);
        assert_eq!(matcher(r"o$", regex).first_in("foo\r\nbar"), Some(2..3));
        assert_eq!(matcher(r"\s+$", regex).first_in("foo  \r\nx"), Some(3..5));
        assert_eq!(matcher(r"^bar", regex).first_in("foo\r\nbar"), Some(5..8));
        assert_eq!(matcher(r".$", regex).first_in("ab\r\ncd\r"), Some(1..2));
        assert_eq!(matcher(r"\r", regex).first_in("a\r\nb"), None);
        assert_eq!(matcher(r"\r", regex).first_in("a\rb"), Some(1..2));
        let plain = MatchOptions::default();
        assert_eq!(matcher("foo\r", plain).first_in("foo\r\n"), None);
        assert_eq!(matcher("foo\r", plain).first_in("foo\rx"), Some(0..4));
        assert_eq!(matcher("o\nb", plain).first_in("foo\nbar"), None);
        assert_eq!(
            matcher("bar", plain).find_iter("bar\r\nbar\nBAR"),
            [0..3, 5..8, 9..12]
        );
    }

    #[test]
    fn a_regex_that_names_a_newline_is_matched_against_the_whole_text() {
        // Break caught: `foo\nbar` never matching because every line was searched on its own.
        let regex = options(false, false, true);
        let text = "x foo\nbar y";
        assert_eq!(matcher(r"foo\nbar", regex).first_in(text), Some(2..9));
        assert_eq!(matcher("foo\nbar", regex).first_in(text), Some(2..9));
        assert_eq!(
            matcher(r"foo\r\nbar", regex).first_in("foo\r\nbar"),
            Some(0..8)
        );
        assert_eq!(
            matcher(r"o\nb", regex).find_iter("oo\nbb o\nb"),
            [1..4, 6..9]
        );
    }

    #[test]
    fn whole_text_mode_gives_caret_and_dollar_line_meaning() {
        // Break caught: the whole text is matched in one pass once the pattern names `\n`, so
        // without `multi_line`, `^`/`$` would only anchor to the start/end of the whole text
        // instead of every line inside it.
        let regex = options(false, false, true);
        assert_eq!(matcher(r"o\n^bar", regex).first_in("foo\nbar"), Some(2..7));
        assert_eq!(
            matcher(r"foo$\nbar", regex).first_in("foo\nbar"),
            Some(0..7)
        );
    }

    #[test]
    fn the_query_is_matched_as_typed_including_its_spaces() {
        let plain = MatchOptions::default();
        assert_eq!(matcher(" foo", plain).first_in("foo  foo"), Some(4..8));
        assert_eq!(matcher(" foo", plain).first_in("foo"), None);
        let found = matcher("a.b", plain);
        assert_eq!(
            found.first_in("axb a.b"),
            Some(4..7),
            "plain text is not a pattern"
        );
        assert_eq!(found.query(), "a.b");
        assert_eq!(found.options(), plain);
    }

    #[test]
    fn escape_makes_text_match_literally_in_regex_mode() {
        assert_eq!(escape("a.b*(c)"), r"a\.b\*\(c\)");
        let found = matcher(&escape("a.b"), options(false, false, true));
        assert_eq!(found.first_in("axb a.b"), Some(4..7));
    }

    #[test]
    fn options_toggle_one_at_a_time() {
        let none = MatchOptions::default();
        assert_eq!(none, options(false, false, false));
        assert_eq!(
            SearchOption::ALL,
            [
                SearchOption::Case,
                SearchOption::WholeWord,
                SearchOption::Regex
            ]
        );
        for option in SearchOption::ALL {
            let on = none.toggled(option);
            assert!(on.get(option));
            let others = SearchOption::ALL.iter().filter(|other| **other != option);
            for other in others {
                assert!(!on.get(*other), "{option:?} also set {other:?}");
            }
            assert_eq!(on.toggled(option), none);
        }
    }

    #[test]
    fn a_matcher_can_be_shared_with_the_search_thread() {
        fn shareable<T: Send + Sync + Clone>() {}
        shareable::<Matcher>();
    }

    #[test]
    fn forty_megabytes_of_text_are_searched_quickly() {
        // Break caught: per-line folding or allocation on the common path (an ASCII query without
        // match case), which would put a 10,000-note search past its 400 ms budget.
        let note = "Plain text with a café, some numbers 12345 and words.\r\n".repeat(75);
        let found = matcher("invoice march", MatchOptions::default());
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(found.first_in(&note), None);
        }
        let elapsed = started.elapsed();
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < std::time::Duration::from_millis(100),
                "{elapsed:?}"
            );
        }
    }
}
