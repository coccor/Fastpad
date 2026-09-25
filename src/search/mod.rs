//! Matching note text for Search: the matcher and the snippet shown under each result. Pure.

pub mod matcher;
pub mod snippet;

pub use matcher::{MatchOptions, Matcher, PatternError, SearchOption, escape};
pub use snippet::{AFTER_CHARS, BEFORE_CHARS, Snippet, cut, first_snippet};
