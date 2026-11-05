//! Pure helpers for several open notebooks: nesting checks, which root owns a path, and the
//! startup notice for roots that could not be opened. Touches no disk.

use super::{is_inside, normalize_folder};
use crate::library::model::same_path;
use std::path::{Path, PathBuf};

/// How a candidate notebook folder overlaps the notebooks already open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Nesting {
    /// The candidate is inside `open[i]`.
    Inside(usize),
    /// The candidate contains `open[i]`.
    Contains(usize),
}

/// Whether `candidate` is nested with an open notebook. `None` when it is unrelated to all of
/// them, and also when it is the same folder as one: callers check `same_path` first, and that
/// case means "reveal", not "refuse".
pub fn nesting(open: &[PathBuf], candidate: &Path) -> Option<Nesting> {
    let candidate = normalize_folder(candidate);
    if open
        .iter()
        .any(|root| same_path(&normalize_folder(root), &candidate))
    {
        return None;
    }
    for (index, root) in open.iter().enumerate() {
        let root = normalize_folder(root);
        if is_inside(&root, &candidate) {
            return Some(Nesting::Inside(index));
        }
        if is_inside(&candidate, &root) {
            return Some(Nesting::Contains(index));
        }
    }
    None
}

/// The index of the open notebook that contains `path`, if any. At most one can, because nested
/// notebooks are refused.
pub fn owner(open: &[PathBuf], path: &Path) -> Option<usize> {
    open.iter().position(|root| is_inside(root, path))
}

/// The startup notice for notebooks that could not be opened, or `None` when none is missing:
/// "X is not available.", "X and Y are not available.", "X, Y and Z are not available.".
pub fn missing_notice(missing: &[PathBuf]) -> Option<String> {
    let names: Vec<String> = missing
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    match names.as_slice() {
        [] => None,
        [one] => Some(format!("{one} is not available.")),
        [rest @ .., last] => Some(format!("{} and {last} are not available.", rest.join(", "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn nesting_finds_a_folder_inside_or_around_an_open_one() {
        // Break caught: a notebook opened inside (or around) another, so one note path has two
        // owners; or a name that merely shares a prefix being refused as nested.
        let open = [PathBuf::from(r"D:\Notes")];
        assert_eq!(
            nesting(&open, &PathBuf::from(r"D:\Notes\Work")),
            Some(Nesting::Inside(0))
        );
        assert_eq!(
            nesting(&open, &PathBuf::from(r"D:\")),
            Some(Nesting::Contains(0))
        );
        assert_eq!(nesting(&open, &PathBuf::from(r"D:\NotesOld")), None);
        assert_eq!(nesting(&open, &PathBuf::from(r"d:\notes\")), None);
    }

    #[test]
    fn owner_picks_the_root_that_contains_the_path() {
        // Break caught: a note being filed under the wrong notebook, or a loose file claimed
        // by one.
        let open = [PathBuf::from(r"D:\A"), PathBuf::from(r"E:\B")];
        assert_eq!(owner(&open, &PathBuf::from(r"E:\B\x\n.md")), Some(1));
        assert_eq!(owner(&open, &PathBuf::from(r"F:\n.md")), None);
    }

    #[test]
    fn missing_notice_lists_every_missing_root() {
        // Break caught: a startup notice naming only the first missing notebook, or showing
        // when nothing is missing.
        assert_eq!(missing_notice(&[]), None);
        assert_eq!(
            missing_notice(&[PathBuf::from(r"D:\Notes")]).as_deref(),
            Some("D:\\Notes is not available.")
        );
        assert_eq!(
            missing_notice(&[PathBuf::from(r"D:\A"), PathBuf::from(r"D:\B")]).as_deref(),
            Some("D:\\A and D:\\B are not available.")
        );
        assert_eq!(
            missing_notice(&[
                PathBuf::from(r"D:\A"),
                PathBuf::from(r"D:\B"),
                PathBuf::from(r"D:\C")
            ])
            .as_deref(),
            Some("D:\\A, D:\\B and D:\\C are not available.")
        );
    }
}
