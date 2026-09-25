//! Note records, the only metadata a notebook keeps: a pin and a deleted flag per note, attached
//! to its file by path, stable ID and content fingerprint.

use super::ids::NoteId;
use std::path::{Path, PathBuf};

/// One note's metadata. `path` is relative to the notebook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub pinned: bool,
    /// Sent to the Recycle Bin by FastPad: kept, hidden, until the purge or a restore.
    pub deleted: bool,
    pub size: u64,
    pub hash: u64,
    pub path: PathBuf,
}

impl NoteRecord {
    pub fn new(id: NoteId, path: PathBuf) -> Self {
        Self {
            id,
            pinned: false,
            deleted: false,
            size: 0,
            hash: 0,
            path,
        }
    }

    /// Whether the record is worth keeping. Only a pin is; the deleted flag alone is not.
    pub fn has_metadata(&self) -> bool {
        self.pinned
    }
}

/// Names a note for an operation: by stable ID first, then by path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRef {
    pub id: NoteId,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryError {
    NotFound,
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("That item no longer exists."),
        }
    }
}

/// NTFS compares names ignoring case, so the library does too.
pub fn same_path(left: &Path, right: &Path) -> bool {
    super::path_key(left) == super::path_key(right)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Library {
    pub notes: Vec<NoteRecord>,
}

impl Library {
    pub fn note(&self, id: NoteId) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| note.id == id)
    }

    pub fn note_by_path(&self, path: &Path) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| same_path(&note.path, path))
    }

    /// An existing record, by ID and then by path. Never creates one.
    pub fn find_note_mut(&mut self, target: &NoteRef) -> Option<&mut NoteRecord> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| {
                self.notes
                    .iter()
                    .position(|note| same_path(&note.path, &target.path))
            })?;
        self.notes.get_mut(index)
    }

    /// The record for `target`, created with its ID and path when none exists.
    pub fn resolve_note(&mut self, target: &NoteRef) -> &mut NoteRecord {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| {
                self.notes
                    .iter()
                    .position(|note| same_path(&note.path, &target.path))
            });
        let index = index.unwrap_or_else(|| {
            self.notes
                .push(NoteRecord::new(target.id, target.path.clone()));
            self.notes.len() - 1
        });
        &mut self.notes[index]
    }

    /// Drops records that carry no pin. Run before every write.
    pub fn prune(&mut self) {
        self.notes.retain(NoteRecord::has_metadata);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_finds_by_id_then_by_path_ignoring_case_then_creates() {
        let mut library = Library::default();
        library.resolve_note(&NoteRef {
            id: NoteId(1),
            path: "Plan.md".into(),
        });
        let by_path = library.resolve_note(&NoteRef {
            id: NoteId(2),
            path: "plan.MD".into(),
        });
        assert_eq!(by_path.id, NoteId(1));
        let by_id = library.resolve_note(&NoteRef {
            id: NoteId(1),
            path: "other.md".into(),
        });
        assert_eq!(by_id.path, PathBuf::from("Plan.md"));
        library.resolve_note(&NoteRef {
            id: NoteId(3),
            path: "new.md".into(),
        });
        assert_eq!(library.notes.len(), 2);
    }

    #[test]
    fn prune_keeps_pinned_records_and_drops_the_rest() {
        // Break caught: an unpinned note's record staying in library.ini forever, or a pinned
        // note that was deleted losing its pin before the purge decides.
        let mut library = Library::default();
        for (id, path, pinned, deleted) in [
            (1, "a.md", true, false),
            (2, "b.md", false, false),
            (3, "c.md", true, true),
            (4, "d.md", false, true),
        ] {
            let record = library.resolve_note(&NoteRef {
                id: NoteId(id),
                path: path.into(),
            });
            record.pinned = pinned;
            record.deleted = deleted;
        }
        library.prune();
        let kept: Vec<_> = library.notes.iter().map(|note| note.id).collect();
        assert_eq!(kept, [NoteId(1), NoteId(3)]);
    }

    #[test]
    fn paths_compare_ignoring_case_like_ntfs() {
        assert!(same_path(
            Path::new(r"Sub\Plan.md"),
            Path::new(r"sub\plan.MD")
        ));
        assert!(!same_path(Path::new("a.md"), Path::new("b.md")));
    }
}
