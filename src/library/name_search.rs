//! Note-name search for the Search view: every note whose name contains the query, ignoring
//! case. Names that start with it come first, then the rest, each group in the tree's natural
//! name order. Pure: no Win32 and no disk.

use super::tree::natural_cmp;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameMatch {
    /// As given: relative to the notebook.
    pub path: PathBuf,
    /// The file name without its extension, as the tree shows it.
    pub name: String,
    /// The folder the note is in, relative to the notebook and joined with `\`; `""` at the root.
    pub folder: String,
}

struct Candidate<'a> {
    /// Whether the name only contains the query, rather than starting with it.
    inside: bool,
    name: String,
    folder: String,
    path: &'a Path,
}

/// Prefix matches first, then by name, then by folder (the root first), then by exact path.
fn rank(a: &Candidate<'_>, b: &Candidate<'_>) -> Ordering {
    a.inside
        .cmp(&b.inside)
        .then_with(|| natural_cmp(&a.name, &b.name))
        .then_with(|| natural_cmp(&a.folder, &b.folder))
        .then_with(|| a.path.cmp(b.path))
}

/// The notes whose name contains `query` (trimmed, ignoring case), best first, at most `limit`.
pub fn search<'a, P>(
    notes: impl IntoIterator<Item = &'a P>,
    query: &str,
    limit: usize,
) -> Vec<NameMatch>
where
    P: AsRef<Path> + ?Sized + 'a,
{
    let query = query.trim().to_lowercase();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut found: Vec<Candidate<'_>> = Vec::new();
    for path in notes {
        let path = path.as_ref();
        let Some(stem) = path.file_stem() else {
            continue;
        };
        let name = stem.to_string_lossy();
        let Some(at) = name.to_lowercase().find(&query) else {
            continue;
        };
        found.push(Candidate {
            inside: at != 0,
            name: name.into_owned(),
            folder: folder_of(path),
            path,
        });
    }
    // Only the best `limit` need a full sort: a one-letter query can match every note.
    if found.len() > limit {
        found.select_nth_unstable_by(limit, rank);
        found.truncate(limit);
    }
    found.sort_by(rank);
    found
        .into_iter()
        .map(|candidate| NameMatch {
            path: candidate.path.to_path_buf(),
            name: candidate.name,
            folder: candidate.folder,
        })
        .collect()
}

fn folder_of(path: &Path) -> String {
    path.parent()
        .map(|parent| {
            parent
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("\\")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn names(matches: &[NameMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.name.as_str()).collect()
    }

    #[test]
    fn names_that_start_with_the_query_come_first_then_the_rest_in_natural_order() {
        // Break caught: "Meeting plan" listed above "Plan 2" when the user typed "plan", or
        // "Plan 10" above "Plan 2".
        let notes = paths(&[
            "Plan 10.md",
            "plan 2.md",
            "Meeting plan.md",
            "Replanning.md",
            r"work\Plans.txt",
            "Other.md",
        ]);
        assert_eq!(
            names(&search(&notes, "plan", 50)),
            ["plan 2", "Plan 10", "Plans", "Meeting plan", "Replanning"]
        );
    }

    #[test]
    fn matching_ignores_case_and_surrounding_spaces_but_not_the_extension() {
        let notes = paths(&["Über uns.md", "notes.md", "Readme"]);
        assert_eq!(names(&search(&notes, "  ÜBER ", 50)), ["Über uns"]);
        assert_eq!(names(&search(&notes, "README", 50)), ["Readme"]);
        assert!(
            search(&notes, "md", 50).is_empty(),
            "the extension is not searched"
        );
    }

    #[test]
    fn each_match_names_its_folder_relative_to_the_notebook() {
        // Break caught: results that lose the tree's context showing no folder, a leading
        // separator, or a mix of `/` and `\`.
        let notes = paths(&[r"a\b\x.md", "x.md", "a/x.txt"]);
        let found = search(&notes, "x", 50);
        let folders: Vec<(&str, &str)> = found
            .iter()
            .map(|found| (found.folder.as_str(), found.name.as_str()))
            .collect();
        assert_eq!(folders, [("", "x"), ("a", "x"), (r"a\b", "x")]);
        assert_eq!(found[2].path, PathBuf::from(r"a\b\x.md"));
    }

    #[test]
    fn an_empty_query_or_no_match_finds_nothing() {
        let notes = paths(&["a.md"]);
        assert!(search(&notes, "", 50).is_empty());
        assert!(search(&notes, "   ", 50).is_empty());
        assert!(search(&notes, "zzz", 50).is_empty());
        assert!(search(&notes, "a", 0).is_empty());
    }

    #[test]
    fn the_limit_keeps_the_best_matches() {
        let many: Vec<PathBuf> = (0..300)
            .map(|index| PathBuf::from(format!("n{index}.md")))
            .collect();
        assert_eq!(
            names(&search(&many, "n", 5)),
            ["n0", "n1", "n2", "n3", "n4"]
        );
        let notes = paths(&["xa.md", "zz.md", "a1.md"]);
        assert_eq!(
            names(&search(&notes, "a", 1)),
            ["a1"],
            "a prefix match beats a substring under the limit"
        );
    }

    #[test]
    fn ten_thousand_names_search_quickly() {
        // Break caught: a keystroke that sorts or allocates per note so heavily that typing in
        // the search box lags on a big notebook.
        let notes: Vec<PathBuf> = (0..10_000)
            .map(|index| PathBuf::from(format!(r"folder {}\Note {index}.md", index / 100)))
            .collect();
        let started = std::time::Instant::now();
        let found = search(&notes, "note", 200);
        let elapsed = started.elapsed();
        assert_eq!(found.len(), 200);
        assert_eq!(found[0].name, "Note 0");
        assert_eq!(found[1].name, "Note 1");
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < std::time::Duration::from_millis(20),
                "{elapsed:?}"
            );
        }
    }
}
