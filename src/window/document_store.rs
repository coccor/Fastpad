//! Every open document, once (split editors spec §3.1). Tabs are views onto these; a document
//! leaves the store when its last view closes.

use crate::document::{Document, DocumentId};
use crate::window::tabs::{DuplicateDocumentPath, canonical_key, lexical_key};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct DocumentStore {
    documents: Vec<Document>,
}

impl DocumentStore {
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.documents.len()
    }

    pub(crate) fn get(&self, id: DocumentId) -> Option<&Document> {
        self.documents.iter().find(|document| document.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.iter_mut().find(|document| document.id == id)
    }

    /// Refuses a document whose id is already here, or whose file another document already has.
    pub(crate) fn insert(&mut self, document: Document) -> Result<(), DuplicateDocumentPath> {
        if self.get(document.id).is_some() {
            return Err(DuplicateDocumentPath(
                document.path.clone().unwrap_or_default(),
            ));
        }
        if let Some(path) = document.path.as_deref() {
            let candidate = canonical_key(path)?;
            if self.documents.iter().any(|existing| {
                existing
                    .path
                    .as_deref()
                    .and_then(|path| canonical_key(path).ok())
                    .is_some_and(|path| path == candidate)
            }) {
                return Err(DuplicateDocumentPath(candidate));
            }
        }
        self.documents.push(document);
        Ok(())
    }

    /// Adds `document` without the path check, for strips whose paths were already validated
    /// together (`Tabs::from_documents`) or never are (`Tabs::with_document`).
    pub(crate) fn insert_unchecked(&mut self, document: Document) {
        self.documents.push(document);
    }

    /// Puts `document` where `old` was; returns `old`.
    pub(crate) fn replace(&mut self, old: DocumentId, document: Document) -> Option<Document> {
        let slot = self
            .documents
            .iter_mut()
            .find(|existing| existing.id == old)?;
        Some(std::mem::replace(slot, document))
    }

    pub(crate) fn remove(&mut self, id: DocumentId) -> Option<Document> {
        let index = self
            .documents
            .iter()
            .position(|document| document.id == id)?;
        Some(self.documents.remove(index))
    }

    pub(crate) fn clear(&mut self) {
        self.documents.clear();
    }

    /// The document whose file is `path`, compared on disk.
    pub(crate) fn find_path(&self, path: &Path) -> Option<DocumentId> {
        let key = canonical_key(path).ok()?;
        self.documents
            .iter()
            .find(|document| {
                document
                    .path
                    .as_deref()
                    .and_then(|path| canonical_key(path).ok())
                    .is_some_and(|path| path == key)
            })
            .map(|document| document.id)
    }

    /// The document whose stored path names `path`, compared lexically without touching the disk.
    pub(crate) fn find_stored_path(&self, path: &Path) -> Option<DocumentId> {
        let key = lexical_key(path);
        self.documents
            .iter()
            .find(|document| {
                document
                    .path
                    .as_deref()
                    .is_some_and(|path| lexical_key(path) == key)
            })
            .map(|document| document.id)
    }

    /// Refuses `path` when a document other than `exclude` already has that file. A path that
    /// does not exist yet (a brand-new Save As target) cannot collide and is returned unchanged.
    pub(crate) fn reject_path_collision(
        &self,
        exclude: DocumentId,
        path: PathBuf,
    ) -> Result<PathBuf, DuplicateDocumentPath> {
        if let Ok(candidate) = canonical_key(&path) {
            let collides = self.documents.iter().any(|existing| {
                existing.id != exclude
                    && existing
                        .path
                        .as_deref()
                        .and_then(|path| canonical_key(path).ok())
                        .is_some_and(|path| path == candidate)
            });
            if collides {
                return Err(DuplicateDocumentPath(candidate));
            }
        }
        Ok(path)
    }

    /// A text change in `id`: a new generation, so recovery snapshots it. The reporting-editor
    /// rule (spec §3.4) makes sure each change arrives here once.
    pub(crate) fn note_text_change(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.get_mut(id) else {
            return false;
        };
        if document.is_image() {
            return false;
        }
        document.generation = document.generation.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentId};

    fn document(id: u64) -> Document {
        Document::test_fixture(DocumentId(id), false)
    }

    #[test]
    fn documents_are_kept_once_and_found_by_id() {
        // Break caught: a second view of a document duplicating it, so its two copies drift apart.
        let mut store = DocumentStore::default();
        store.insert(document(1)).unwrap();
        store.insert(document(2)).unwrap();
        assert!(store.insert(document(1)).is_err());
        assert_eq!(store.len(), 2);
        assert!(store.get(DocumentId(2)).is_some());
        assert_eq!(
            store.remove(DocumentId(1)).map(|document| document.id),
            Some(DocumentId(1))
        );
        assert!(store.get(DocumentId(1)).is_none());
    }

    #[test]
    fn a_path_already_open_is_refused() {
        // Break caught: the same file open in two documents, so saving one overwrites the other.
        let dir = std::env::temp_dir().join(format!("fastpad-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "a").unwrap();
        let mut store = DocumentStore::default();
        let mut first = document(1);
        first.path = Some(path.clone());
        store.insert(first).unwrap();
        let mut second = document(2);
        second.path = Some(dir.join(".").join("a.txt"));
        assert!(store.insert(second).is_err());
        assert_eq!(store.find_path(&path), Some(DocumentId(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_text_change_bumps_the_generation_once() {
        // Break caught: a document shown by two editors counting one edit twice.
        let mut store = DocumentStore::default();
        store.insert(document(1)).unwrap();
        let before = store.get(DocumentId(1)).unwrap().generation;
        assert!(store.note_text_change(DocumentId(1)));
        assert_eq!(store.get(DocumentId(1)).unwrap().generation, before + 1);
        assert!(!store.note_text_change(DocumentId(99)));
    }
}
