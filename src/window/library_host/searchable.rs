//! The notebooks Search, Quick Open and Tags cover (multi-root search spec §3): every open
//! notebook that has loaded, the label results show for each, and a result's way back to its
//! file.

use super::*;
use crate::window::main_window::push_notice;

/// Shown when a result is opened after its notebook closed.
pub(crate) const NOTEBOOK_CLOSED: &str = "That note's notebook was closed.";

/// One notebook the views search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Searchable {
    pub(crate) id: NotebookId,
    pub(crate) folder: PathBuf,
    /// What its results say: the sidebar's name, disambiguated (`roots::notebook_labels`).
    pub(crate) label: String,
}

/// The open notebooks the views leave out for now.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Skipped {
    /// Open, but their notes haven't loaded yet.
    pub(crate) loading: usize,
    /// Remembered as open, but their folder couldn't be reached.
    pub(crate) unavailable: usize,
}

/// A note in one open notebook: `path` is relative to that notebook's folder.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct NoteRef {
    pub(crate) notebook: NotebookId,
    pub(crate) path: PathBuf,
}

/// Every loaded open notebook, in sidebar order, with its label.
pub(crate) fn searchable_notebooks(hwnd: HWND) -> Vec<Searchable> {
    host(hwnd, |host| {
        let folders: Vec<PathBuf> = host
            .notebooks
            .iter()
            .map(|slot| slot.folder.clone())
            .collect();
        let labels = crate::library::roots::notebook_labels(&folders);
        host.notebooks
            .iter()
            .zip(labels)
            .filter(|(slot, _)| slot.state.is_some())
            .map(|(slot, label)| Searchable {
                id: slot.id,
                folder: slot.folder.clone(),
                label,
            })
            .collect()
    })
    .unwrap_or_default()
}

/// How many open notebooks `searchable_notebooks` leaves out, and why.
pub(crate) fn skipped_notebooks(hwnd: HWND) -> Skipped {
    host(hwnd, |host| Skipped {
        loading: host
            .notebooks
            .iter()
            .filter(|slot| slot.state.is_none())
            .count(),
        unavailable: host.unavailable.len(),
    })
    .unwrap_or_default()
}

/// What a header adds for the skipped notebooks: "" when none is, else " (1 notebook still
/// loading)", " (2 notebooks unavailable)", or both, loading first.
pub(crate) fn skipped_suffix(skipped: Skipped) -> String {
    fn count(n: usize, what: &str) -> String {
        if n == 1 {
            format!("1 notebook {what}")
        } else {
            format!("{n} notebooks {what}")
        }
    }
    let parts: Vec<String> = [
        (skipped.loading, "still loading"),
        (skipped.unavailable, "unavailable"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| count(n, what))
    .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Two or more notebooks are open (loading ones included), so results name their notebook.
pub(crate) fn several_notebooks(hwnd: HWND) -> bool {
    host(hwnd, |host| host.notebooks.len() > 1).unwrap_or(false)
}

/// The file `note` names. `None`, with a notice, when its notebook has closed since the result
/// was listed: the path is never joined to another notebook's folder.
pub(crate) fn note_ref_path(hwnd: HWND, note: &NoteRef) -> Option<PathBuf> {
    match notebook_folder(hwnd, note.notebook) {
        Some(folder) => Some(folder.join(&note.path)),
        None => {
            push_notice(hwnd, NOTEBOOK_CLOSED.to_owned());
            None
        }
    }
}
