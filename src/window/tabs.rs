use crate::document::{CloseCancelled, CloseDecision, Document, DocumentId};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

/// Title-strip state shared with the accessibility provider: the selected tab, how far the tab
/// strip is scrolled and where it starts (right of the sidebar), so both painting and
/// accessibility locate the same tab rectangles. The provider may run on another thread and never
/// reads the App.
#[derive(Clone, Debug)]
pub(crate) struct TabSelection {
    active: Arc<AtomicUsize>,
    scroll: Arc<AtomicI32>,
    strip_left: Arc<AtomicI32>,
}

impl TabSelection {
    pub(crate) fn new(active: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(active)),
            scroll: Arc::new(AtomicI32::new(0)),
            strip_left: Arc::new(AtomicI32::new(0)),
        }
    }

    /// Where the tabs start: the sidebar's right edge, 0 without a sidebar.
    pub(crate) fn strip_left(&self) -> i32 {
        self.strip_left.load(Ordering::Acquire)
    }

    pub(crate) fn set_strip_left(&self, left: i32) {
        self.strip_left.store(left.max(0), Ordering::Release);
    }

    pub(crate) fn scroll_offset(&self) -> i32 {
        self.scroll.load(Ordering::Acquire)
    }

    pub(crate) fn set_scroll_offset(&self, offset: i32) -> bool {
        self.scroll.swap(offset.max(0), Ordering::AcqRel) != offset.max(0)
    }

    pub(crate) fn active_index(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn select(&self, index: usize, tab_count: usize) -> bool {
        if index >= tab_count {
            return false;
        }
        self.active.store(index, Ordering::Release);
        true
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TabView {
    state: Arc<RwLock<TabViewState>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TabViewSnapshot {
    pub(crate) revision: u64,
    pub(crate) tabs: Vec<TabViewTab>,
    pub(crate) preview_buttons: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TabViewTab {
    pub(crate) id: DocumentId,
    pub(crate) title: String,
    /// Painted in italics.
    pub(crate) preview: bool,
}

#[derive(Debug)]
struct TabViewState {
    revision: u64,
    tabs: Vec<TabViewTab>,
    preview_buttons: bool,
}

impl TabView {
    fn new(documents: &[Document]) -> Self {
        Self {
            state: Arc::new(RwLock::new(TabViewState {
                revision: 0,
                tabs: view_tabs(documents),
                preview_buttons: false,
            })),
        }
    }

    pub(crate) fn snapshot(&self) -> TabViewSnapshot {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        TabViewSnapshot {
            revision: state.revision,
            tabs: state.tabs.clone(),
            preview_buttons: state.preview_buttons,
        }
    }

    fn update(&self, documents: &[Document]) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.revision = state.revision.saturating_add(1);
        state.tabs = view_tabs(documents);
    }

    pub(crate) fn set_preview_buttons(&self, visible: bool) {
        self.state
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .preview_buttons = visible;
    }
}

fn view_tabs(documents: &[Document]) -> Vec<TabViewTab> {
    documents
        .iter()
        .map(|document| TabViewTab {
            id: document.id,
            title: document.title(),
            preview: document.preview,
        })
        .collect()
}

#[derive(Debug)]
pub struct Tabs {
    documents: Vec<Document>,
    /// Every tab, the most recently activated first; the active tab is always first
    /// (quick-open spec §3.2). Kept in memory only, never saved.
    recent: Vec<DocumentId>,
    selection: TabSelection,
    view: TabView,
}

impl Tabs {
    pub fn new() -> Self {
        let documents = Vec::new();
        Self {
            view: TabView::new(&documents),
            recent: Vec::new(),
            documents,
            selection: TabSelection::new(0),
        }
    }

    pub fn with_document(document: Document) -> Self {
        let documents = vec![document];
        Self {
            view: TabView::new(&documents),
            recent: documents.iter().map(|document| document.id).collect(),
            documents,
            selection: TabSelection::new(0),
        }
    }

    pub fn from_documents(
        documents: impl IntoIterator<Item = Document>,
    ) -> Result<Self, DuplicateDocumentPath> {
        let documents = documents.into_iter().collect::<Vec<_>>();
        validate_unique_paths(&documents)?;
        Ok(Self {
            view: TabView::new(&documents),
            recent: documents.iter().map(|document| document.id).collect(),
            documents,
            selection: TabSelection::new(0),
        })
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.selection.active_index()
    }

    pub(crate) fn scroll_offset(&self) -> i32 {
        self.selection.scroll_offset()
    }

    /// Returns whether the offset changed.
    pub(crate) fn set_scroll_offset(&self, offset: i32) -> bool {
        self.selection.set_scroll_offset(offset)
    }

    /// Publishes the sidebar's right edge for the accessibility provider (`TabSelection`).
    pub(crate) fn set_strip_left(&self, left: i32) {
        self.selection.set_strip_left(left);
    }

    pub(crate) fn selection(&self) -> TabSelection {
        self.selection.clone()
    }

    pub(crate) fn view(&self) -> TabView {
        self.view.clone()
    }

    /// Refreshes the retained tab-view snapshot from the current documents, without otherwise
    /// changing anything (e.g. after a document's title-affecting field changes in place).
    pub(crate) fn refresh_view(&self) {
        self.view.update(&self.documents);
    }

    pub(crate) fn set_preview_buttons(&self, visible: bool) {
        self.view.set_preview_buttons(visible);
    }

    /// The selected document, or `None` once every tab has been closed.
    pub fn active(&self) -> Option<&Document> {
        self.documents.get(self.active_index())
    }

    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.documents.iter().find(|document| document.id == id)
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.iter_mut().find(|document| document.id == id)
    }

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

    /// The tab whose stored path names `path`, compared as absolute paths ignoring case, without
    /// touching the disk: unlike `find_path`, it finds a tab whose file no longer exists (renamed
    /// or moved outside FastPad).
    pub(crate) fn find_stored_path(&self, path: &Path) -> Option<DocumentId> {
        let key = lexical_key(path);
        self.documents
            .iter()
            .find(|document| {
                document
                    .path
                    .as_deref()
                    .is_some_and(|p| lexical_key(p) == key)
            })
            .map(|document| document.id)
    }

    pub(crate) fn replace_active_untitled(&mut self, document: Document) -> Option<Document> {
        let index = self.active_index();
        let active = self.documents.get_mut(index)?;
        let old = std::mem::replace(active, document);
        let id = active.id;
        self.recent.retain(|recent| *recent != old.id);
        self.touch(id);
        self.view.update(&self.documents);
        Some(old)
    }

    #[cfg(test)]
    pub fn ids(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.documents.iter().map(|document| document.id)
    }

    pub fn titles(&self) -> impl Iterator<Item = String> + '_ {
        self.documents.iter().map(Document::title)
    }

    pub fn activate(&mut self, id: DocumentId) -> Result<(), UnknownDocument> {
        let index = self
            .documents
            .iter()
            .position(|document| document.id == id)
            .ok_or(UnknownDocument(id))?;
        self.selection.select(index, self.documents.len());
        self.touch(id);
        Ok(())
    }

    /// Moves `id` to the front of the activation order.
    fn touch(&mut self, id: DocumentId) {
        self.recent.retain(|recent| *recent != id);
        self.recent.insert(0, id);
    }

    /// The tabs, the most recently activated first; the active tab leads (spec §3.2).
    pub(crate) fn activation_order(&self) -> &[DocumentId] {
        &self.recent
    }

    /// Restarts the activation order from the strip, the active tab first: how restored tabs
    /// enter it once a session restore has reopened them all (spec §3.2).
    pub(crate) fn reset_activation_order(&mut self) {
        let active = self.active().map(|document| document.id);
        self.recent = active
            .into_iter()
            .chain(
                self.documents
                    .iter()
                    .map(|document| document.id)
                    .filter(|id| Some(*id) != active),
            )
            .collect();
    }

    pub fn activate_index(&mut self, index: usize) -> Result<(), UnknownDocument> {
        let id = self
            .documents
            .get(index)
            .map(|document| document.id)
            .ok_or(UnknownDocument(DocumentId(u64::MAX)))?;
        self.activate(id)
    }

    pub fn push(&mut self, document: Document) -> Result<(), DuplicateDocumentPath> {
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
        let id = document.id;
        self.documents.push(document);
        let index = self.documents.len() - 1;
        self.selection.select(index, self.documents.len());
        self.touch(id);
        self.view.update(&self.documents);
        Ok(())
    }

    pub fn close_active(&mut self, decision: CloseDecision) -> Result<Document, CloseCancelled> {
        if decision == CloseDecision::Cancel || self.documents.is_empty() {
            return Err(CloseCancelled);
        }
        let index = self.active_index();
        let closed = self.documents.remove(index);
        self.select_after_removal(index, closed.id);
        Ok(closed)
    }

    /// Keeps the successor of a removed tab selected (or its predecessor at the end of the strip).
    /// `closed` leaves the activation order and the tab now selected takes its front.
    fn select_after_removal(&mut self, removed: usize, closed: DocumentId) {
        let active = removed.min(self.documents.len().saturating_sub(1));
        self.selection.active.store(active, Ordering::Release);
        self.recent.retain(|recent| *recent != closed);
        if let Some(id) = self.documents.get(active).map(|document| document.id) {
            self.touch(id);
        }
        self.view.update(&self.documents);
    }

    pub fn active_close_review(&self) -> Option<CloseReview> {
        let document = self.documents.get(self.active_index())?;
        Some(CloseReview {
            id: document.id,
            generation: document.generation,
        })
    }

    pub fn close_reviewed(
        &mut self,
        review: CloseReview,
        decision: CloseDecision,
    ) -> Result<Document, CloseReviewError> {
        if decision == CloseDecision::Cancel {
            return Err(CloseReviewError::Cancelled);
        }
        let Some(index) = self
            .documents
            .iter()
            .position(|document| document.id == review.id)
        else {
            return Err(CloseReviewError::Stale);
        };
        if index != self.active_index() || self.documents[index].generation != review.generation {
            return Err(CloseReviewError::Stale);
        }
        // A Save answer must never close a document whose save did not actually happen.
        if decision == CloseDecision::Save && self.documents[index].dirty {
            return Err(CloseReviewError::Unsaved);
        }
        let closed = self.documents.remove(index);
        self.select_after_removal(index, closed.id);
        Ok(closed)
    }

    /// Closes the clean tab `review` names without activating it (quick-open spec §5): the
    /// active tab stays active and keeps its place in the activation order. `Stale` when that
    /// tab has gone, is the active one or changed since `review`; `Unsaved` when it has unsaved
    /// edits, which only the usual reviewed close may discard.
    pub(crate) fn close_clean_background(
        &mut self,
        review: CloseReview,
    ) -> Result<Document, CloseReviewError> {
        let Some(index) = self
            .documents
            .iter()
            .position(|document| document.id == review.id)
        else {
            return Err(CloseReviewError::Stale);
        };
        let active = self.active_index();
        if index == active || self.documents[index].generation != review.generation {
            return Err(CloseReviewError::Stale);
        }
        if self.documents[index].dirty {
            return Err(CloseReviewError::Unsaved);
        }
        let closed = self.documents.remove(index);
        if index < active {
            self.selection.active.store(active - 1, Ordering::Release);
        }
        self.recent.retain(|recent| *recent != closed.id);
        self.view.update(&self.documents);
        Ok(closed)
    }

    pub fn next_dirty_review(&self, reviewed: &[CloseReviewKey]) -> Option<CloseReview> {
        self.documents
            .iter()
            .find(|document| {
                document.dirty
                    && !reviewed.contains(&CloseReviewKey {
                        id: document.id,
                        generation: document.generation,
                    })
            })
            .map(|document| CloseReview {
                id: document.id,
                generation: document.generation,
            })
    }

    pub fn dirty_review_is_current(&self, review: CloseReview) -> bool {
        self.document(review.id)
            .is_some_and(|document| document.dirty && document.generation == review.generation)
    }

    pub fn set_active_dirty(&mut self, dirty: bool) -> bool {
        let active = self.active_index();
        let Some(document) = self.documents.get_mut(active) else {
            return false;
        };
        // Undo can reach Scintilla's empty save point; a recovered tab stays dirty until saved.
        if document.dirty == dirty || (!dirty && document.recovery_origin.is_some()) {
            return false;
        }
        document.dirty = dirty;
        document.generation = document.generation.saturating_add(1);
        self.view.update(&self.documents);
        true
    }

    /// Records the active document's language after an explicit `LanguageX` command or automatic
    /// detection, independent of applying the actual lexer to the live editor. Returns `false`
    /// (no-op) when the active document already has this language.
    pub(crate) fn set_active_language(&mut self, language: crate::document::Language) -> bool {
        let active = self.active_index();
        let Some(document) = self.documents.get_mut(active) else {
            return false;
        };
        if document.language == language {
            return false;
        }
        document.language = language;
        true
    }

    pub(crate) fn documents(&self) -> impl Iterator<Item = &Document> + '_ {
        self.documents.iter()
    }

    pub(crate) fn record_recovery_generation(&mut self, id: DocumentId, generation: u64) {
        if let Some(document) = self.documents.iter_mut().find(|document| document.id == id) {
            document.recovery_generation = Some(generation);
        }
    }

    pub(crate) fn take_active_recovery_origin(
        &mut self,
    ) -> Option<crate::document::RecoveryOrigin> {
        let active = self.active_index();
        self.documents.get_mut(active)?.recovery_origin.take()
    }

    /// A text change in the active tab. The first one makes a preview tab normal, so replacing
    /// the preview can never drop an edit; returns whether that happened.
    pub(crate) fn note_active_text_change(&mut self) -> bool {
        let active = self.active_index();
        let Some(document) = self.documents.get_mut(active) else {
            return false;
        };
        document.generation = document.generation.saturating_add(1);
        if !document.preview {
            return false;
        }
        document.preview = false;
        self.view.update(&self.documents);
        true
    }

    /// A background tab's text was changed in the editor while it was swapped in with
    /// notifications suppressed (a Search replace, note-search spec §12a). Records what
    /// `SCN_SAVEPOINTLEFT` and `SCN_MODIFIED` would have for the active tab: dirty, a new
    /// generation (so recovery snapshots it), and a preview kept. Returns whether the strip
    /// changed.
    pub(crate) fn note_background_edit(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.documents.iter_mut().find(|document| document.id == id) else {
            return false;
        };
        document.generation = document.generation.saturating_add(1);
        let changed = !document.dirty || document.preview;
        document.dirty = true;
        document.preview = false;
        if changed {
            self.view.update(&self.documents);
        }
        changed
    }

    /// The preview tab, if one is open. There is at most one.
    pub(crate) fn preview_id(&self) -> Option<DocumentId> {
        self.documents
            .iter()
            .find(|document| document.preview)
            .map(|document| document.id)
    }

    /// Puts `document` where the preview tab is, selects it and returns the document it replaced.
    /// With no preview tab, `document` is added like any new tab and `None` is returned. The same
    /// happens when the preview somehow has unsaved edits: it is kept as a normal tab. The caller
    /// has already checked that `document`'s file is not open in another tab.
    pub(crate) fn replace_preview(&mut self, document: Document) -> Option<Document> {
        match self.documents.iter().position(|existing| existing.preview) {
            Some(index) if !self.documents[index].dirty => {
                let id = document.id;
                let old = std::mem::replace(&mut self.documents[index], document);
                self.selection.select(index, self.documents.len());
                self.recent.retain(|recent| *recent != old.id);
                self.touch(id);
                self.view.update(&self.documents);
                Some(old)
            }
            edited => {
                if let Some(index) = edited {
                    self.documents[index].preview = false;
                }
                if self.push(document).is_err() {
                    self.view.update(&self.documents);
                }
                None
            }
        }
    }

    /// Makes `id` a normal tab; returns whether it was the preview.
    pub(crate) fn promote(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.document_mut(id) else {
            return false;
        };
        if !document.preview {
            return false;
        }
        document.preview = false;
        self.view.update(&self.documents);
        true
    }

    pub fn clear_for_shutdown(&mut self) {
        self.documents.clear();
        self.recent.clear();
        self.selection.active.store(0, Ordering::Release);
        self.view.update(&self.documents);
    }

    pub(crate) fn active_handle(&self) -> Option<&crate::editor::EditorDocument> {
        self.active().map(|document| &document.handle)
    }

    /// Rejects `path` if it canonicalizes to the same file another open tab (any document other
    /// than the one at `exclude`) already owns, preserving Task 9's
    /// one-native-document-per-canonical-path invariant. A `path` that does not yet exist on disk
    /// (the common brand-new Save As destination) cannot collide with any already-open document,
    /// so it is returned unchanged without a canonicalization check. Shared by `set_active_path`
    /// and `rebind_path`.
    fn reject_path_collision(
        &self,
        exclude: usize,
        path: PathBuf,
    ) -> Result<PathBuf, DuplicateDocumentPath> {
        if let Ok(candidate) = canonical_key(&path) {
            let collides = self.documents.iter().enumerate().any(|(index, existing)| {
                index != exclude
                    && existing
                        .path
                        .as_deref()
                        .and_then(|existing_path| canonical_key(existing_path).ok())
                        .is_some_and(|existing_candidate| existing_candidate == candidate)
            });
            if collides {
                return Err(DuplicateDocumentPath(candidate));
            }
        }
        Ok(path)
    }

    /// Renames the active document's path, e.g. after a successful Save As write. Rejects the
    /// rename if `path` canonicalizes to the same file another open tab already owns; see
    /// `reject_path_collision`.
    pub(crate) fn set_active_path(&mut self, path: PathBuf) -> Result<(), DuplicateDocumentPath> {
        let active = self.active_index();
        if active >= self.documents.len() {
            return Err(DuplicateDocumentPath(path));
        }
        let path = self.reject_path_collision(active, path)?;
        self.documents[active].path = Some(path);
        self.view.update(&self.documents);
        Ok(())
    }

    /// Rebinds `id`'s path, e.g. after a note is renamed on disk or moved between folders.
    /// Rejects the path if it canonicalizes to the same file another open tab already owns; see
    /// `reject_path_collision`. Updates the tab view so the tab strip and accessibility see the
    /// new title.
    pub fn rebind_path(
        &mut self,
        id: DocumentId,
        path: PathBuf,
    ) -> Result<(), DuplicateDocumentPath> {
        let Some(index) = self.documents.iter().position(|document| document.id == id) else {
            return Err(DuplicateDocumentPath(path));
        };
        let path = self.reject_path_collision(index, path)?;
        self.documents[index].path = Some(path);
        self.view.update(&self.documents);
        Ok(())
    }

    /// Reverts the active document's path back to a value it previously, legitimately held (or
    /// `None`, if it had not yet claimed a path), e.g. after a Save As write that renamed the
    /// path via `set_active_path` but then failed before anything was actually written to the
    /// new location. No collision check is needed: `original` was already valid for this
    /// document, so restoring it cannot newly collide with any other tab.
    pub(crate) fn revert_active_path(&mut self, original: Option<PathBuf>) {
        let active = self.active_index();
        if let Some(document) = self.documents.get_mut(active) {
            document.path = original;
            self.view.update(&self.documents);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CloseReview {
    pub id: DocumentId,
    pub generation: u64,
}

impl CloseReview {
    pub fn key(self) -> CloseReviewKey {
        CloseReviewKey {
            id: self.id,
            generation: self.generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CloseReviewKey {
    id: DocumentId,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseReviewError {
    Cancelled,
    Stale,
    Unsaved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownDocument(pub DocumentId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateDocumentPath(pub PathBuf);

fn canonical_key(path: &Path) -> Result<PathBuf, DuplicateDocumentPath> {
    let canonical = path
        .canonicalize()
        .map_err(|_| DuplicateDocumentPath(path.to_path_buf()))?;
    #[cfg(windows)]
    {
        Ok(PathBuf::from(canonical.to_string_lossy().to_lowercase()))
    }
    #[cfg(not(windows))]
    {
        Ok(canonical)
    }
}

fn lexical_key(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .to_lowercase()
}

fn validate_unique_paths(documents: &[Document]) -> Result<(), DuplicateDocumentPath> {
    let mut paths = Vec::new();
    for document in documents {
        let Some(path) = document.path.as_deref() else {
            continue;
        };
        let canonical = canonical_key(path)?;
        if paths.contains(&canonical) {
            return Err(DuplicateDocumentPath(canonical));
        }
        paths.push(canonical);
    }
    Ok(())
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Tabs {
    fn drop(&mut self) {
        // Normal shutdown drains before HWND destruction; emergency teardown must still retire
        // metadata so retained accessibility providers cannot target a recycled window.
        self.clear_for_shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::Tabs;
    use super::{CloseReview, CloseReviewError};
    use crate::document::{Document, DocumentId};
    use std::fs;

    use crate::document::CloseDecision;

    fn order(tabs: &Tabs) -> Vec<u64> {
        tabs.activation_order().iter().map(|id| id.0).collect()
    }

    fn document(id: u64) -> Document {
        Document::test_fixture(DocumentId(id), false)
    }

    #[test]
    fn a_clean_background_tab_closes_where_it_is_and_the_active_tab_stays() {
        // Break caught: a middle-click on another tab switching to it first (the editor flashes),
        // closing the active tab instead, the active index left pointing one tab too far right,
        // or a dirty tab closed without its prompt.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        let review = |tabs: &Tabs, id: u64| CloseReview {
            id: DocumentId(id),
            generation: tabs.document(DocumentId(id)).unwrap().generation,
        };
        let first = review(&tabs, 1);
        let closed = tabs.close_clean_background(first).unwrap();
        assert_eq!(closed.id, DocumentId(1));
        assert_eq!(tabs.active().unwrap().id, DocumentId(3));
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(order(&tabs), [3, 2]);
        assert_eq!(tabs.view().snapshot().tabs.len(), 2);

        let active = tabs.active_close_review().unwrap();
        assert_eq!(
            tabs.close_clean_background(active),
            Err(CloseReviewError::Stale)
        );
        tabs.document_mut(DocumentId(2)).unwrap().dirty = true;
        let dirty = review(&tabs, 2);
        assert_eq!(
            tabs.close_clean_background(dirty),
            Err(CloseReviewError::Unsaved)
        );
        let stale = CloseReview {
            generation: dirty.generation + 1,
            ..dirty
        };
        assert_eq!(
            tabs.close_clean_background(stale),
            Err(CloseReviewError::Stale)
        );
        assert_eq!(tabs.len(), 2);
    }

    /// `push` canonicalizes paths through the disk, so these pure tests use untitled documents.
    fn preview(id: u64) -> Document {
        let mut document = document(id);
        document.preview = true;
        document
    }

    #[test]
    fn a_background_edit_marks_only_that_tab_dirty_and_keeps_its_preview() {
        // Break caught: a Search replace into a background tab leaving it clean (closing it
        // would drop the replacement without asking), marking the active tab instead, or leaving
        // a preview that the next click replaces with its edits in it.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(preview(2)).unwrap();
        tabs.activate(DocumentId(1)).unwrap();
        let before = tabs.document(DocumentId(2)).unwrap().generation;
        assert!(tabs.note_background_edit(DocumentId(2)));
        let edited = tabs.document(DocumentId(2)).unwrap();
        assert!(edited.dirty && !edited.preview);
        assert_eq!(edited.generation, before + 1);
        assert!(!tabs.active().unwrap().dirty);
        assert!(!tabs.note_background_edit(DocumentId(2)), "already dirty");
        assert!(!tabs.note_background_edit(DocumentId(9)), "no such tab");
    }

    #[test]
    fn replacing_the_preview_keeps_its_place_and_selects_it() {
        // Break caught: a second preview appended at the end (the strip grows with every click),
        // or replaced in place but left unselected.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(preview(2)).unwrap();
        tabs.push(document(3)).unwrap();
        assert_eq!(tabs.preview_id(), Some(DocumentId(2)));

        let old = tabs.replace_preview(preview(4)).unwrap();
        assert_eq!(old.id, DocumentId(2));
        assert_eq!(
            tabs.documents()
                .map(|document| document.id)
                .collect::<Vec<_>>(),
            [DocumentId(1), DocumentId(4), DocumentId(3)]
        );
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.preview_id(), Some(DocumentId(4)));
        assert!(tabs.view().snapshot().tabs[1].preview);
    }

    #[test]
    fn a_dirty_preview_is_kept_as_a_normal_tab_and_the_new_one_is_added() {
        // Break caught: a preview whose promotion was missed being replaced with its edits in it.
        let mut tabs = Tabs::with_document(document(1));
        let mut edited = preview(2);
        edited.dirty = true;
        tabs.push(edited).unwrap();

        assert!(tabs.replace_preview(preview(3)).is_none());
        assert_eq!(tabs.len(), 3);
        assert!(!tabs.document(DocumentId(2)).unwrap().preview);
        assert_eq!(tabs.preview_id(), Some(DocumentId(3)));
        assert_eq!(tabs.active_index(), 2);
    }

    #[test]
    fn with_no_preview_replace_preview_adds_a_tab() {
        // Break caught: the first preview of a session silently dropped because there was nothing
        // to replace.
        let mut tabs = Tabs::with_document(document(1));
        assert!(tabs.replace_preview(preview(2)).is_none());
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
    }

    #[test]
    fn the_first_edit_promotes_the_active_preview_once() {
        // Break caught: typing into a preview leaving it a preview, so the next click replaces it.
        let mut tabs = Tabs::with_document(preview(1));
        assert!(tabs.note_active_text_change());
        assert!(!tabs.note_active_text_change());
        assert_eq!(tabs.preview_id(), None);
        assert!(!tabs.promote(DocumentId(1)));
        let mut tabs = Tabs::with_document(preview(1));
        assert!(tabs.promote(DocumentId(1)));
        assert!(!tabs.view().snapshot().tabs[0].preview);
    }

    #[test]
    fn recovered_documents_stay_dirty_until_their_origin_is_cleared() {
        // Break caught: undoing a recovered tab to Scintilla's save point marks it clean, so
        // closing skips the prompt and deletes the only copy of its text.
        let mut recovered = Document::test_fixture(DocumentId(1), true);
        recovered.recovery_origin = Some(crate::document::RecoveryOrigin {
            snapshot_path: std::path::PathBuf::from("a.fps"),
            original_path: None,
            from_session: false,
        });
        let mut tabs = Tabs::with_document(recovered);

        assert!(!tabs.set_active_dirty(false));
        assert!(tabs.active().unwrap().dirty);
        assert!(tabs.take_active_recovery_origin().is_some());
        assert!(tabs.set_active_dirty(false));
    }

    #[test]
    fn native_document_tab_is_selected() {
        let tabs = Tabs::with_document(Document::test_fixture(DocumentId(1), false));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.active_index(), 0);
        assert_eq!(tabs.titles().collect::<Vec<_>>(), ["Untitled"]);
    }

    #[test]
    fn set_active_language_updates_only_the_active_document() {
        // Break caught: explicit language selection or detection-on-open applying to the wrong
        // tab, or leaving Document::language stale after a real lexer switch.
        use crate::document::Language;
        let mut tabs = Tabs::from_documents([document(1), document(2)]).unwrap();
        tabs.activate(DocumentId(2)).unwrap();

        assert!(tabs.set_active_language(Language::Json));
        assert!(!tabs.set_active_language(Language::Json));
        assert_eq!(
            tabs.document(DocumentId(1)).unwrap().language,
            Language::PlainText
        );
        assert_eq!(
            tabs.document(DocumentId(2)).unwrap().language,
            Language::Json
        );
    }

    #[test]
    fn shared_selection_updates_the_tab_model_and_rejects_out_of_range_indices() {
        // Break caught: accessibility can keep a provider-private selection that title painting
        // cannot observe, or accept a button/out-of-range index as a tab.
        let tabs = Tabs::from_documents([
            Document::test_fixture(DocumentId(1), false),
            Document::test_fixture(DocumentId(2), false),
        ])
        .unwrap();
        let selection = tabs.selection();

        assert!(selection.select(1, tabs.len()));
        assert_eq!(tabs.active_index(), 1);
        assert!(!selection.select(2, tabs.len()));
        assert_eq!(tabs.active_index(), 1);
    }

    #[test]
    fn renaming_the_active_document_updates_its_path_and_the_tab_view() {
        // Break caught: Save As writing the path field directly instead of going through a
        // validated method can desync the tab view from the document model.
        let root = std::env::temp_dir().join(format!(
            "fastpad-task11-rename-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("saved-as.txt");
        fs::write(&target, b"saved").unwrap();
        let mut tabs = Tabs::with_document(document(1));
        let view = tabs.view();
        let before = view.snapshot().revision;

        tabs.set_active_path(target.clone()).unwrap();

        assert_eq!(
            tabs.active().unwrap().path.as_deref(),
            Some(target.as_path())
        );
        assert!(view.snapshot().revision > before);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renaming_to_a_path_that_does_not_yet_exist_succeeds() {
        // Break caught: reusing canonical_key's existence requirement verbatim can reject every
        // brand-new Save As destination, since a not-yet-written file cannot be canonicalized.
        let root = std::env::temp_dir().join(format!(
            "fastpad-task11-rename-new-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("not-written-yet.txt");
        let mut tabs = Tabs::with_document(document(1));

        assert!(tabs.set_active_path(target.clone()).is_ok());
        assert_eq!(
            tabs.active().unwrap().path.as_deref(),
            Some(target.as_path())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renaming_the_active_document_onto_another_open_tabs_canonical_path_is_rejected() {
        // Break caught: Save As can create two tabs that own the same canonical path, breaking
        // Task 9's one-native-document-per-path invariant.
        let root = std::env::temp_dir().join(format!(
            "fastpad-task11-rename-collision-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let other_path = root.join("other.txt");
        fs::write(&other_path, b"other").unwrap();
        let mut other = document(2);
        other.path = Some(other_path.clone());
        let mut tabs = Tabs::from_documents([document(1), other]).unwrap();
        tabs.activate(DocumentId(1)).unwrap();

        let alternate = root.join(".").join("other.txt");
        assert!(tabs.set_active_path(alternate).is_err());
        assert_eq!(tabs.active().unwrap().path, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rebinding_a_path_rejects_one_another_tab_has() {
        // Break caught: rebinding a document's path (e.g. after a note rename) can create two
        // tabs that own the same canonical path, breaking Task 9's invariant, or can fail to
        // update the tab view so the title strip shows a stale name.
        let root = std::env::temp_dir().join(format!(
            "fastpad-task13-rebind-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path_a = root.join("a.md");
        let path_b = root.join("b.md");
        let path_c = root.join("c.md");
        fs::write(&path_a, b"a").unwrap();
        fs::write(&path_b, b"b").unwrap();
        fs::write(&path_c, b"c").unwrap();

        let mut first = document(1);
        first.path = Some(path_a);
        let mut second = document(2);
        second.path = Some(path_b.clone());
        let mut tabs = Tabs::from_documents([first, second]).unwrap();

        assert!(tabs.rebind_path(DocumentId(1), path_b).is_err());
        tabs.rebind_path(DocumentId(1), path_c.clone()).unwrap();
        assert_eq!(
            tabs.document(DocumentId(1)).unwrap().path.as_deref(),
            Some(path_c.as_path())
        );
        tabs.document_mut(DocumentId(2)).unwrap().autosave_paused = true;
        assert!(tabs.document(DocumentId(2)).unwrap().autosave_paused);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn activating_or_opening_a_tab_moves_it_to_the_front_of_the_activation_order() {
        // Break caught: Ctrl+P listing tabs in strip order, so Ctrl+P then Enter doesn't go back
        // to the previous note, or a new tab missing from the list.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        assert_eq!(order(&tabs), [3, 2, 1]);
        tabs.activate(DocumentId(1)).unwrap();
        assert_eq!(order(&tabs), [1, 3, 2]);
        tabs.activate_index(2).unwrap();
        assert_eq!(order(&tabs), [3, 1, 2]);
        tabs.activate(DocumentId(3)).unwrap();
        assert_eq!(order(&tabs), [3, 1, 2], "the active tab stays first");
    }

    #[test]
    fn a_closed_tab_leaves_the_order_and_the_tab_taking_its_place_comes_first() {
        // Break caught: Ctrl+P offering a closed tab's dead document, or leaving the tab now on
        // screen second, so Ctrl+P then Enter re-selects the tab already shown.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        tabs.activate(DocumentId(2)).unwrap();
        assert_eq!(order(&tabs), [2, 3, 1]);
        tabs.close_active(CloseDecision::Discard).unwrap();
        assert_eq!(tabs.active().unwrap().id, DocumentId(3));
        assert_eq!(order(&tabs), [3, 1]);
        let review = tabs.active_close_review().unwrap();
        tabs.close_reviewed(review, CloseDecision::Discard).unwrap();
        assert_eq!(order(&tabs), [1]);
    }

    #[test]
    fn a_tab_replaced_in_place_takes_the_front_and_the_old_document_leaves_the_order() {
        // Break caught: a replaced preview (or reused untitled tab) still listed by Ctrl+P under
        // its old document, or the note now in it missing.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(preview(2)).unwrap();
        tabs.push(document(3)).unwrap();
        tabs.replace_preview(preview(4)).unwrap();
        assert_eq!(order(&tabs), [4, 3, 1]);
        tabs.activate(DocumentId(1)).unwrap();
        tabs.replace_active_untitled(document(5)).unwrap();
        assert_eq!(order(&tabs), [5, 4, 3]);
        tabs.clear_for_shutdown();
        assert!(order(&tabs).is_empty());
    }

    #[test]
    fn restored_tabs_restart_the_order_from_the_strip_with_the_active_tab_first() {
        // Break caught: after a session restore, the tabs listed last-restored first (each one
        // entered at the front as it opened).
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        tabs.push(document(4)).unwrap();
        tabs.activate(DocumentId(3)).unwrap();
        tabs.reset_activation_order();
        assert_eq!(order(&tabs), [3, 1, 2, 4]);
        let from = Tabs::from_documents([document(5), document(6)]).unwrap();
        assert_eq!(order(&from), [5, 6]);
    }
}
