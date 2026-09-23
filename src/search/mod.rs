//! Matching note text for Search: the matcher. Pure.

pub mod matcher;

pub use matcher::{MatchOptions, Matcher, PatternError, SearchOption, escape};
