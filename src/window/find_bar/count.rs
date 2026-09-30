//! The find bar's match counter: how many matches the query has in the document, and which one
//! the selection is. Counting is bounded (`COUNT_CAP` matches, `COUNT_MAX_BYTES` of text) and runs
//! off the typing path, from a debounce timer (`main_window::find`).

use super::{regex_matcher, search_flags};
use crate::editor::Editor;
use crate::search::MatchOptions;
use std::ops::Range;

/// Counting stops here, and the bar shows "1000+".
pub(crate) const COUNT_CAP: usize = 1000;

/// Documents longer than this are not counted: even a bounded scan of a miss reads all the text.
pub(crate) const COUNT_MAX_BYTES: usize = 32 * 1024 * 1024;

/// What the counter shows for a query that has been counted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MatchCount {
    /// The matches found, at most `COUNT_CAP`.
    pub(crate) total: usize,
    /// There are more matches than `total`.
    pub(crate) capped: bool,
    /// The 1-based position of the match the selection is, when it is one of the counted.
    pub(crate) index: Option<usize>,
}

impl MatchCount {
    /// "3 of 12", "12 results" when the selection isn't a match, "No results", "1000+ results".
    pub(crate) fn label(&self) -> String {
        if self.total == 0 {
            return "No results".to_owned();
        }
        let total = if self.capped {
            format!("{}+", self.total)
        } else {
            self.total.to_string()
        };
        match self.index {
            Some(index) => format!("{index} of {total}"),
            None if self.total == 1 && !self.capped => "1 result".to_owned(),
            None => format!("{total} results"),
        }
    }
}

/// Counts `matches` (ascending, non-overlapping) up to `cap`, and finds `selection` among them.
/// The iterator is read at most `cap + 1` items, so an unbounded source stays bounded.
pub(super) fn tally(
    matches: impl Iterator<Item = Range<usize>>,
    selection: &Range<usize>,
    cap: usize,
) -> MatchCount {
    let mut total = 0;
    let mut index = None;
    for range in matches.take(cap + 1) {
        if total < cap && range == *selection {
            index = Some(total + 1);
        }
        total += 1;
    }
    let capped = total > cap;
    MatchCount {
        total: total.min(cap),
        capped,
        index,
    }
}

/// The count of `query` under `options` in `editor`'s document, with `selection` as the current
/// match. `None` shows nothing: an empty query, or a document too large to count. A pattern that
/// doesn't compile has no matches, as the bar's navigation treats it.
pub(crate) fn count_matches(
    editor: &Editor,
    query: &str,
    options: MatchOptions,
    selection: &Range<usize>,
) -> Option<MatchCount> {
    if query.is_empty() {
        return None;
    }
    let doc_len = editor.length().ok()?;
    if doc_len > COUNT_MAX_BYTES {
        return None;
    }
    if options.regex {
        let Some(matcher) = regex_matcher(query, options) else {
            return Some(tally(std::iter::empty(), selection, COUNT_CAP));
        };
        return editor
            .with_document_text(|text| {
                let found = matcher.find_up_to(text, COUNT_CAP + 1);
                tally(found.into_iter(), selection, COUNT_CAP)
            })
            .ok();
    }
    // Plain mode counts with the search navigation uses, one match after another.
    let flags = search_flags(options);
    let mut from = 0;
    let found = std::iter::from_fn(|| {
        if from >= doc_len {
            return None;
        }
        let found = editor
            .search_in_target(query, from..doc_len, flags)
            .ok()
            .flatten()?;
        // A match that doesn't advance would repeat forever.
        (found.end > from).then(|| {
            from = found.end;
            found
        })
    });
    Some(tally(found, selection, COUNT_CAP))
}
