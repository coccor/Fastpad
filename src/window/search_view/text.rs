//! The Search view's words: the box's placeholder, the notices, the summary and status lines,
//! and the skipped-notes tooltip.

use super::*;
use crate::library::text_search::{self, Progress, SkipReason};
use crate::window::library_host;
use crate::window::notebook_view::LOAD_FAILED;
use std::path::Path;

/// The box's placeholder: "Search text in <notebook>", with the notebook's display name.
pub(crate) fn placeholder(notebook: Option<&Path>) -> String {
    notebook
        .map(|notebook| format!("Search text in {}", library_host::notebook_name(notebook)))
        .unwrap_or_else(|| "Search text".to_owned())
}

/// The search's progress, as the summary and status lines read it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchState {
    /// Nothing typed, or only white space.
    #[default]
    Idle,
    /// One character: text search starts at two.
    TooShort,
    Running(Progress),
    Done {
        progress: Progress,
        capped: bool,
    },
    /// The pattern can't run. The previous results stay.
    PatternError(String),
}

/// The line shown instead of the results, if any. `failed` is a notebook whose load failed.
pub(crate) fn notice_text(notebook_open: bool, loaded: bool, failed: bool) -> Option<&'static str> {
    if !notebook_open {
        Some(NO_NOTEBOOK)
    } else if failed {
        Some(LOAD_FAILED)
    } else if !loaded {
        Some(LOADING)
    } else {
        None
    }
}

/// The summary line under the box and whether it is an error: "N notes" while results arrive
/// and when the search is done, "No notes match." for a finished search without one.
pub(crate) fn summary_text(state: &SearchState, results: usize) -> Option<(String, bool)> {
    match state {
        SearchState::Idle => None,
        SearchState::TooShort => Some((TOO_SHORT.to_owned(), false)),
        SearchState::PatternError(message) => Some((message.clone(), true)),
        SearchState::Running(_) => (results > 0).then(|| (note_count(results, false), false)),
        SearchState::Done { capped, .. } => {
            let text = if results == 0 {
                NO_MATCH.to_owned()
            } else {
                note_count(results, *capped)
            };
            Some((text, false))
        }
    }
}

/// The status line at the bottom: progress while the search runs, what it skipped once done.
/// `None` hides it.
pub(crate) fn status_text(state: &SearchState) -> Option<String> {
    match state {
        SearchState::Running(progress) => Some(format!(
            "Searching\u{2026} {} of {}",
            thousands(progress.visited),
            thousands(progress.total)
        )),
        SearchState::Done { progress, .. } => match progress.skipped_total() {
            0 => None,
            1 => Some("1 note wasn't searched".to_owned()),
            skipped => Some(format!("{} notes weren't searched", thousands(skipped))),
        },
        _ => None,
    }
}

pub(super) fn note_count(count: usize, capped: bool) -> String {
    if capped {
        format!("{}+ notes", thousands(text_search::RESULT_CAP))
    } else if count == 1 {
        "1 note".to_owned()
    } else {
        format!("{} notes", thousands(count))
    }
}

/// `value` with a comma between each group of three digits.
pub(crate) fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// The status line's tooltip: one line per reason that skipped a note.
pub(crate) fn skipped_tooltip(progress: &Progress) -> String {
    SkipReason::ALL
        .into_iter()
        .filter_map(|reason| {
            let count = progress.skipped[reason.index()];
            let why = match reason {
                SkipReason::OnlineOnly => "online only",
                SkipReason::TooLarge => "larger than 4 MB",
                SkipReason::Unreadable => "couldn't be read",
                SkipReason::NotText => "not text",
            };
            (count > 0).then(|| format!("{} {why}", thousands(count)))
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}
