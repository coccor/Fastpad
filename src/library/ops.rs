//! Every change the UI makes to a library is a `PendingOp`, applied at once to the live library
//! and kept until the next successful write. When `library.ini` changed on disk in between
//! (sync, another instance), the file is re-read and the pending operations are replayed on top,
//! so both sides' changes survive.

use super::ids::{NoteId, NotebookId, TagId};
use super::model::{Library, LibraryError, NoteRef, NotebookColor};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingOp {
    CreateNotebook {
        id: NotebookId,
        name: String,
        now: u64,
    },
    RenameNotebook {
        id: NotebookId,
        name: String,
        now: u64,
    },
    SetNotebookColor {
        id: NotebookId,
        color: Option<NotebookColor>,
        now: u64,
    },
    MoveNotebook {
        id: NotebookId,
        index: usize,
    },
    DeleteNotebook {
        id: NotebookId,
    },
    SetNoteNotebook {
        note: NoteRef,
        notebook: Option<NotebookId>,
    },
    SetFavorite {
        note: NoteRef,
        value: bool,
    },
    SetPinned {
        note: NoteRef,
        value: bool,
    },
    AddTag {
        note: NoteRef,
        tag: TagId,
        name: String,
    },
    RemoveTag {
        note: NoteRef,
        tag: TagId,
    },
    RenameTag {
        id: TagId,
        name: String,
    },
    RemoveTagEverywhere {
        id: TagId,
    },
    Relocate {
        note: NoteRef,
        path: PathBuf,
    },
    SetFingerprint {
        note: NoteRef,
        size: u64,
        hash: u64,
    },
    SetDeleted {
        note: NoteRef,
        value: bool,
    },
    Drop {
        id: NoteId,
    },
}

pub fn apply(library: &mut Library, op: &PendingOp) -> Result<(), LibraryError> {
    match op {
        PendingOp::CreateNotebook { id, name, now } => library.create_notebook(*id, name, *now),
        PendingOp::RenameNotebook { id, name, now } => library.rename_notebook(*id, name, *now),
        PendingOp::SetNotebookColor { id, color, now } => {
            library.set_notebook_color(*id, *color, *now)
        }
        PendingOp::MoveNotebook { id, index } => library.move_notebook(*id, *index),
        PendingOp::DeleteNotebook { id } => library.delete_notebook(*id).map(|_| ()),
        PendingOp::SetNoteNotebook { note, notebook } => {
            if let Some(notebook) = notebook
                && library.notebook(*notebook).is_none()
            {
                return Err(LibraryError::NotFound);
            }
            library.resolve_note(note).notebook = *notebook;
            Ok(())
        }
        PendingOp::SetFavorite { note, value } => {
            library.resolve_note(note).favorite = *value;
            Ok(())
        }
        PendingOp::SetPinned { note, value } => {
            library.resolve_note(note).pinned = *value;
            Ok(())
        }
        PendingOp::AddTag { note, tag, name } => {
            let tag = library.create_tag(*tag, name)?;
            let record = library.resolve_note(note);
            if !record.tags.contains(&tag) {
                record.tags.push(tag);
            }
            Ok(())
        }
        PendingOp::RemoveTag { note, tag } => {
            let record = library.find_note_mut(note).ok_or(LibraryError::NotFound)?;
            record.tags.retain(|existing| existing != tag);
            Ok(())
        }
        PendingOp::RenameTag { id, name } => library.rename_tag(*id, name),
        PendingOp::RemoveTagEverywhere { id } => library.remove_tag_everywhere(*id).map(|_| ()),
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
    use crate::library::ids::{NoteId, NotebookId, TagId};

    fn note(id: u128, path: &str) -> NoteRef {
        NoteRef {
            id: NoteId(id),
            path: path.into(),
        }
    }

    #[test]
    fn two_diverged_copies_merge_by_replay_without_losing_either_side() {
        // Break caught: a sync from another PC overwriting this PC's favorite, or the reverse.
        let mut base = Library::default();
        apply(
            &mut base,
            &PendingOp::CreateNotebook {
                id: NotebookId(1),
                name: "Work".into(),
                now: 1,
            },
        )
        .unwrap();

        let mut other_pc = base.clone();
        apply(
            &mut other_pc,
            &PendingOp::SetNoteNotebook {
                note: note(7, "a.md"),
                notebook: Some(NotebookId(1)),
            },
        )
        .unwrap();

        let ours = vec![
            PendingOp::SetFavorite {
                note: note(8, "b.md"),
                value: true,
            },
            PendingOp::AddTag {
                note: note(8, "b.md"),
                tag: TagId(3),
                name: "idea".into(),
            },
        ];
        let mut merged = other_pc.clone();
        assert_eq!(replay(&mut merged, &ours), 0);
        assert_eq!(
            merged.note(NoteId(7)).unwrap().notebook,
            Some(NotebookId(1))
        );
        let b = merged.note(NoteId(8)).unwrap();
        assert!(b.favorite);
        assert_eq!(b.tags, vec![TagId(3)]);
    }

    #[test]
    fn replaying_an_already_applied_log_changes_nothing() {
        let ops = vec![
            PendingOp::CreateNotebook {
                id: NotebookId(1),
                name: "Work".into(),
                now: 1,
            },
            PendingOp::SetNoteNotebook {
                note: note(7, "a.md"),
                notebook: Some(NotebookId(1)),
            },
            PendingOp::AddTag {
                note: note(7, "a.md"),
                tag: TagId(2),
                name: "todo".into(),
            },
            PendingOp::SetPinned {
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
        // Break caught: a note moved into a notebook that the other PC deleted pointing at a
        // notebook that no longer exists.
        let mut library = Library::default();
        let ops = vec![
            PendingOp::SetNoteNotebook {
                note: note(7, "a.md"),
                notebook: Some(NotebookId(9)),
            },
            PendingOp::RenameNotebook {
                id: NotebookId(9),
                name: "X".into(),
                now: 1,
            },
            PendingOp::Relocate {
                note: note(8, "gone.md"),
                path: "moved.md".into(),
            },
        ];
        assert_eq!(replay(&mut library, &ops), 3);
        assert!(library.notes.is_empty());
    }

    #[test]
    fn adding_a_tag_reuses_a_same_named_tag_created_elsewhere() {
        let mut library = Library::default();
        library.create_tag(TagId(1), "idea").unwrap();
        apply(
            &mut library,
            &PendingOp::AddTag {
                note: note(7, "a.md"),
                tag: TagId(2),
                name: "Idea".into(),
            },
        )
        .unwrap();
        assert_eq!(library.tags.len(), 1);
        assert_eq!(library.note(NoteId(7)).unwrap().tags, vec![TagId(1)]);
    }

    #[test]
    fn relocation_fingerprints_and_deletion_flags_need_an_existing_record() {
        let mut library = Library::default();
        apply(
            &mut library,
            &PendingOp::SetFavorite {
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
