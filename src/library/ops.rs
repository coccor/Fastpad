//! Every change the UI makes to a library is a `PendingOp`, applied at once to the live library
//! and kept until the next successful write. When `library.ini` changed on disk in between
//! (sync, another instance), the file is re-read and the pending operations are replayed on top,
//! so both sides' changes survive.

use super::ids::NoteId;
use super::model::{Library, LibraryError, NoteRef};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingOp {
    SetPinned { note: NoteRef, value: bool },
    Relocate { note: NoteRef, path: PathBuf },
    SetFingerprint { note: NoteRef, size: u64, hash: u64 },
    SetDeleted { note: NoteRef, value: bool },
    Drop { id: NoteId },
}

pub fn apply(library: &mut Library, op: &PendingOp) -> Result<(), LibraryError> {
    match op {
        PendingOp::SetPinned { note, value } => {
            library.resolve_note(note).pinned = *value;
            Ok(())
        }
        PendingOp::Relocate { note, path } => {
            library
                .find_note_mut(note)
                .ok_or(LibraryError::NotFound)?
                .path = path.clone();
            Ok(())
        }
        PendingOp::SetFingerprint { note, size, hash } => {
            let record = library.find_note_mut(note).ok_or(LibraryError::NotFound)?;
            record.size = *size;
            record.hash = *hash;
            Ok(())
        }
        PendingOp::SetDeleted { note, value } => {
            library
                .find_note_mut(note)
                .ok_or(LibraryError::NotFound)?
                .deleted = *value;
            Ok(())
        }
        PendingOp::Drop { id } => {
            library.notes.retain(|note| note.id != *id);
            Ok(())
        }
    }
}

/// Applies every operation in order. Operations that no longer apply are dropped and counted.
pub fn replay(library: &mut Library, ops: &[PendingOp]) -> usize {
    ops.iter().filter(|op| apply(library, op).is_err()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::NoteId;

    fn note(id: u128, path: &str) -> NoteRef {
        NoteRef {
            id: NoteId(id),
            path: path.into(),
        }
    }

    #[test]
    fn two_diverged_copies_merge_by_replay_without_losing_either_side() {
        // Break caught: a sync from another PC overwriting this PC's pin, or the reverse.
        let mut other_pc = Library::default();
        apply(
            &mut other_pc,
            &PendingOp::SetPinned {
                note: note(7, "a.md"),
                value: true,
            },
        )
        .unwrap();
        let ours = vec![PendingOp::SetPinned {
            note: note(8, "b.md"),
            value: true,
        }];
        let mut merged = other_pc.clone();
        assert_eq!(replay(&mut merged, &ours), 0);
        assert!(merged.note(NoteId(7)).unwrap().pinned);
        assert!(merged.note(NoteId(8)).unwrap().pinned);
    }

    #[test]
    fn replaying_an_already_applied_log_changes_nothing() {
        let ops = vec![
            PendingOp::SetPinned {
                note: note(7, "a.md"),
                value: true,
            },
            PendingOp::SetFingerprint {
                note: note(7, "a.md"),
                size: 3,
                hash: 9,
            },
            PendingOp::SetDeleted {
                note: note(7, "a.md"),
                value: true,
            },
        ];
        let mut once = Library::default();
        replay(&mut once, &ops);
        let mut twice = once.clone();
        assert_eq!(replay(&mut twice, &ops), 0);
        assert_eq!(once, twice);
    }

    #[test]
    fn operations_whose_target_was_removed_are_dropped() {
        // Break caught: a relocation or fingerprint for a record the other PC purged creating a
        // bare record that points at nothing.
        let mut library = Library::default();
        let ops = vec![
            PendingOp::Relocate {
                note: note(8, "gone.md"),
                path: "moved.md".into(),
            },
            PendingOp::SetFingerprint {
                note: note(8, "gone.md"),
                size: 1,
                hash: 2,
            },
            PendingOp::SetDeleted {
                note: note(8, "gone.md"),
                value: true,
            },
        ];
        assert_eq!(replay(&mut library, &ops), 3);
        assert!(library.notes.is_empty());
    }

    #[test]
    fn relocation_fingerprints_and_deletion_flags_need_an_existing_record() {
        let mut library = Library::default();
        apply(
            &mut library,
            &PendingOp::SetPinned {
                note: note(7, "a.md"),
                value: true,
            },
        )
        .unwrap();
        apply(
            &mut library,
            &PendingOp::Relocate {
                note: note(7, "a.md"),
                path: "b.md".into(),
            },
        )
        .unwrap();
        apply(
            &mut library,
            &PendingOp::SetFingerprint {
                note: note(7, "b.md"),
                size: 3,
                hash: 9,
            },
        )
        .unwrap();
        apply(
            &mut library,
            &PendingOp::SetDeleted {
                note: note(7, "b.md"),
                value: true,
            },
        )
        .unwrap();
        let record = library.note(NoteId(7)).unwrap();
        assert_eq!(record.path, std::path::PathBuf::from("b.md"));
        assert_eq!((record.size, record.hash, record.deleted), (3, 9, true));
        apply(&mut library, &PendingOp::Drop { id: NoteId(7) }).unwrap();
        assert!(library.notes.is_empty());
    }
}
