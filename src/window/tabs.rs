use crate::document::{CloseCancelled, CloseDecision, Document, DocumentId};
use crate::editor::ViewState;
use crate::window::document_store::DocumentStore;
use crate::window::split_tree::GroupId;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

/// Tab-strip state shared with the accessibility provider: the selected tab and how far the strip
/// is scrolled, so both painting and accessibility locate the same tab rectangles. The provider
/// may run on another thread and never reads the App.
#[derive(Clone, Debug)]
pub(crate) struct TabSelection {
    active: Arc<AtomicUsize>,
    scroll: Arc<AtomicI32>,
}

impl TabSelection {
    pub(crate) fn new(active: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(active)),
            scroll: Arc::new(AtomicI32::new(0)),
        }
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
    fn new(tabs: Vec<TabViewTab>) -> Self {
        Self {
            state: Arc::new(RwLock::new(TabViewState {
                revision: 0,
                tabs,
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

    fn update(&self, tabs: Vec<TabViewTab>) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.revision = state.revision.saturating_add(1);
        state.tabs = tabs;
    }

    pub(crate) fn set_preview_buttons(&self, visible: bool) {
        self.state
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .preview_buttons = visible;
    }
}

fn view_tabs<'a>(documents: impl Iterator<Item = &'a Document>) -> Vec<TabViewTab> {
    documents
        .map(|document| TabViewTab {
            id: document.id,
            title: document.title(),
            preview: document.preview,
        })
        .collect()
}

/// One tab: a view onto a document in the store (split editors spec §3.2).
#[derive(Debug)]
pub(crate) struct EditorTab {
    pub(crate) document: DocumentId,
    /// Where this view was when it last stopped being shown; while it is shown, the live editor
    /// is the truth.
    pub(crate) view_state: ViewState,
}

/// One group's tabs: views onto documents in the store, in strip order (split editors spec
/// §3.2). A group never has two views of the same document.
#[derive(Debug)]
pub(crate) struct GroupTabs {
    pub(crate) id: GroupId,
    tabs: Vec<EditorTab>,
    selection: TabSelection,
    view: TabView,
}

// The group API is wired into the window in the next commits.
#[allow(dead_code)]
impl GroupTabs {
    fn new(id: GroupId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            selection: TabSelection::new(0),
            view: TabView::new(Vec::new()),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.tabs.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub(crate) fn active_index(&self) -> usize {
        self.selection.active_index()
    }

    /// The document of the selected tab, or `None` in an empty group.
    pub(crate) fn active_document(&self) -> Option<DocumentId> {
        self.tabs.get(self.active_index()).map(|tab| tab.document)
    }

    /// The documents of this group's tabs, in strip order.
    pub(crate) fn document_ids(&self) -> Vec<DocumentId> {
        self.tabs.iter().map(|tab| tab.document).collect()
    }

    pub(crate) fn contains(&self, id: DocumentId) -> bool {
        self.position(id).is_some()
    }

    fn position(&self, id: DocumentId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.document == id)
    }

    pub(crate) fn selection(&self) -> TabSelection {
        self.selection.clone()
    }

    pub(crate) fn view(&self) -> TabView {
        self.view.clone()
    }

    pub(crate) fn scroll_offset(&self) -> i32 {
        self.selection.scroll_offset()
    }

    /// Where this group's view of `id` was when it was last left; the start of the document for
    /// a view never left.
    pub(crate) fn view_state(&self, id: DocumentId) -> ViewState {
        self.tabs
            .iter()
            .find(|tab| tab.document == id)
            .map(|tab| tab.view_state)
            .unwrap_or_default()
    }

    fn select(&self, index: usize) -> bool {
        self.selection.select(index, self.tabs.len())
    }

    /// Keeps the successor of a removed tab selected (or its predecessor at the end of the strip).
    fn select_after_removal(&self, removed: usize) {
        let active = removed.min(self.tabs.len().saturating_sub(1));
        self.selection.active.store(active, Ordering::Release);
    }
}

/// The open documents and the groups of tabs showing them. The documents live in a
/// [`DocumentStore`]; each tab is an [`EditorTab`] view onto one, in one group's strip (split
/// editors spec §3). Methods that don't name a group act on the active group.
// The group API is wired into the window in the next commits.
#[allow(dead_code)]
#[derive(Debug)]
pub struct Tabs {
    store: DocumentStore,
    groups: Vec<GroupTabs>,
    /// Index into `groups` of the active group.
    active: usize,
    next_group: u32,
    /// Every view, the most recently activated first, across groups; the active view is always
    /// first (spec §3.3, quick-open spec §3.2). Kept in memory only, never saved.
    recent: Vec<(GroupId, DocumentId)>,
}

// The group API is wired into the window in the next commits.
#[allow(dead_code)]
impl Tabs {
    pub fn new() -> Self {
        Self {
            store: DocumentStore::default(),
            groups: vec![GroupTabs::new(GroupId(1))],
            active: 0,
            next_group: 2,
            recent: Vec::new(),
        }
    }

    pub fn with_document(document: Document) -> Self {
        let mut tabs = Self::new();
        tabs.append_unchecked(document);
        tabs.refresh_view();
        tabs
    }

    pub fn from_documents(
        documents: impl IntoIterator<Item = Document>,
    ) -> Result<Self, DuplicateDocumentPath> {
        let documents = documents.into_iter().collect::<Vec<_>>();
        validate_unique_paths(&documents)?;
        let mut tabs = Self::new();
        for document in documents {
            tabs.append_unchecked(document);
        }
        tabs.refresh_view();
        Ok(tabs)
    }

    fn current(&self) -> &GroupTabs {
        &self.groups[self.active]
    }

    fn current_mut(&mut self) -> &mut GroupTabs {
        &mut self.groups[self.active]
    }

    fn group_index(&self, id: GroupId) -> Option<usize> {
        self.groups.iter().position(|group| group.id == id)
    }

    pub(crate) fn active_group(&self) -> GroupId {
        self.current().id
    }

    /// Makes `id` the active group; returns whether it changed.
    pub(crate) fn set_active_group(&mut self, id: GroupId) -> bool {
        let Some(index) = self.group_index(id) else {
            return false;
        };
        if index == self.active {
            return false;
        }
        self.active = index;
        if let Some(document) = self.current().active_document() {
            self.touch(document);
        }
        true
    }

    /// A new, empty group after the others; the active group stays.
    pub(crate) fn add_group(&mut self) -> GroupId {
        let id = GroupId(self.next_group);
        self.next_group += 1;
        self.groups.push(GroupTabs::new(id));
        id
    }

    /// Removes an empty group. The last group stays, and a group with tabs can't be removed.
    pub(crate) fn remove_group(&mut self, id: GroupId) -> bool {
        let Some(index) = self.group_index(id) else {
            return false;
        };
        if self.groups.len() == 1 || !self.groups[index].is_empty() {
            return false;
        }
        self.groups.remove(index);
        if index < self.active {
            self.active -= 1;
        } else if index == self.active {
            self.active = index.min(self.groups.len() - 1);
        }
        true
    }

    /// The groups in creation order.
    pub(crate) fn group_ids(&self) -> Vec<GroupId> {
        self.groups.iter().map(|group| group.id).collect()
    }

    pub(crate) fn group(&self, id: GroupId) -> Option<&GroupTabs> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// `id`'s documents in strip order.
    pub(crate) fn group_documents(&self, id: GroupId) -> Vec<&Document> {
        self.group(id)
            .map(|group| {
                group
                    .tabs
                    .iter()
                    .filter_map(|tab| self.store.get(tab.document))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The groups with a view of `id`, in creation order.
    pub(crate) fn views_of(&self, id: DocumentId) -> Vec<GroupId> {
        self.groups
            .iter()
            .filter(|group| group.contains(id))
            .map(|group| group.id)
            .collect()
    }

    /// Shows the open document `id` in `group` too, selected there. A document with two or more
    /// views is never a preview (plan amendment 3). False for an unknown group or document.
    pub(crate) fn add_view(&mut self, group: GroupId, id: DocumentId, state: ViewState) -> bool {
        let Some(index) = self.group_index(group) else {
            return false;
        };
        if self.store.get(id).is_none() {
            return false;
        }
        let target = &mut self.groups[index];
        let position = match target.position(id) {
            Some(position) => position,
            None => {
                target.tabs.push(EditorTab {
                    document: id,
                    view_state: state,
                });
                target.tabs.len() - 1
            }
        };
        target.select(position);
        self.touch_in(group, id);
        if self.views_of(id).len() > 1
            && let Some(document) = self.store.get_mut(id)
        {
            document.preview = false;
        }
        self.refresh_views();
        true
    }

    /// Moves the view of `id` from one group to another, keeping its view state. When `to`
    /// already shows `id`, its view is selected and the moved one is dropped.
    pub(crate) fn move_view(&mut self, from: GroupId, id: DocumentId, to: GroupId) -> bool {
        if from == to || self.group_index(to).is_none() {
            return false;
        }
        let Some(source) = self.group_index(from) else {
            return false;
        };
        let Some(position) = self.groups[source].position(id) else {
            return false;
        };
        let tab = self.groups[source].tabs.remove(position);
        let selected = self.groups[source].active_index();
        if position < selected {
            self.groups[source]
                .selection
                .active
                .store(selected - 1, Ordering::Release);
        } else {
            self.groups[source].select_after_removal(position);
        }
        self.recent.retain(|recent| *recent != (from, id));
        self.add_view(to, id, tab.view_state)
    }

    /// Selects `id` in `group` and makes it the most recent view; false when `group` has no view
    /// of it.
    pub(crate) fn activate_in(&mut self, group: GroupId, id: DocumentId) -> bool {
        let Some(index) = self.group_index(group) else {
            return false;
        };
        let Some(position) = self.groups[index].position(id) else {
            return false;
        };
        self.groups[index].select(position);
        self.touch_in(group, id);
        true
    }

    pub(crate) fn view_state_in(&self, group: GroupId, id: DocumentId) -> ViewState {
        self.group(group)
            .map(|group| group.view_state(id))
            .unwrap_or_default()
    }

    pub(crate) fn set_view_state_in(&mut self, group: GroupId, id: DocumentId, state: ViewState) {
        let Some(index) = self.group_index(group) else {
            return;
        };
        if let Some(tab) = self.groups[index]
            .tabs
            .iter_mut()
            .find(|tab| tab.document == id)
        {
            tab.view_state = state;
        }
    }

    /// Adds a tab for `document` at the end of the strip without the path check or selecting it.
    fn append_unchecked(&mut self, document: Document) {
        let id = document.id;
        let group = self.active_group();
        self.store.insert_unchecked(document);
        self.current_mut().tabs.push(EditorTab {
            document: id,
            view_state: ViewState::default(),
        });
        self.recent.push((group, id));
    }

    /// The active group's documents in strip order.
    fn strip(&self) -> impl Iterator<Item = &Document> + '_ {
        self.current()
            .tabs
            .iter()
            .filter_map(|tab| self.store.get(tab.document))
    }

    fn document_at(&self, index: usize) -> Option<&Document> {
        self.store.get(self.current().tabs.get(index)?.document)
    }

    fn document_at_mut(&mut self, index: usize) -> Option<&mut Document> {
        let id = self.current().tabs.get(index)?.document;
        self.store.get_mut(id)
    }

    fn position(&self, id: DocumentId) -> Option<usize> {
        self.current().position(id)
    }

    /// Removes the active group's tab at `index`. Returns its document's id, and the document
    /// itself when that was its last view: a document leaves the store with its last view.
    fn remove_tab(&mut self, index: usize) -> (DocumentId, Option<Document>) {
        let tab = self.current_mut().tabs.remove(index);
        if !self.views_of(tab.document).is_empty() {
            return (tab.document, None);
        }
        (tab.document, self.store.remove(tab.document))
    }

    pub fn len(&self) -> usize {
        self.current().len()
    }

    pub fn is_empty(&self) -> bool {
        self.current().is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.current().active_index()
    }

    pub(crate) fn scroll_offset(&self) -> i32 {
        self.current().scroll_offset()
    }

    /// Returns whether the offset changed.
    pub(crate) fn set_scroll_offset(&self, offset: i32) -> bool {
        self.current().selection.set_scroll_offset(offset)
    }

    pub(crate) fn selection(&self) -> TabSelection {
        self.current().selection()
    }

    pub(crate) fn view(&self) -> TabView {
        self.current().view()
    }

    /// Refreshes the retained tab-view snapshots from the current documents, without otherwise
    /// changing anything (e.g. after a document's title-affecting field changes in place).
    pub(crate) fn refresh_view(&self) {
        self.refresh_views();
    }

    /// Refreshes every group's tab-view snapshot whose tabs changed: a document's title shows
    /// in each group that has a view of it.
    pub(crate) fn refresh_views(&self) {
        for group in &self.groups {
            let tabs = view_tabs(
                group
                    .tabs
                    .iter()
                    .filter_map(|tab| self.store.get(tab.document)),
            );
            if group.view.snapshot().tabs != tabs {
                group.view.update(tabs);
            }
        }
    }

    pub(crate) fn set_preview_buttons(&self, visible: bool) {
        self.current().view.set_preview_buttons(visible);
    }

    /// The selected document, or `None` once every tab has been closed.
    pub fn active(&self) -> Option<&Document> {
        self.document_at(self.active_index())
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut Document> {
        self.document_at_mut(self.active_index())
    }

    pub fn document(&self, id: DocumentId) -> Option<&Document> {
        self.store.get(id)
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.store.get_mut(id)
    }

    pub(crate) fn find_path(&self, path: &Path) -> Option<DocumentId> {
        self.store.find_path(path)
    }

    /// The tab whose stored path names `path`, compared as absolute paths ignoring case, without
    /// touching the disk: unlike `find_path`, it finds a tab whose file no longer exists (renamed
    /// or moved outside FastPad).
    pub(crate) fn find_stored_path(&self, path: &Path) -> Option<DocumentId> {
        self.store.find_stored_path(path)
    }

    /// Where the active group's tab showing `id` was when it was last left; the start of the
    /// document for a tab never left.
    pub(crate) fn view_state(&self, id: DocumentId) -> ViewState {
        self.current().view_state(id)
    }

    pub(crate) fn set_view_state(&mut self, id: DocumentId, state: ViewState) {
        let group = self.active_group();
        self.set_view_state_in(group, id, state);
    }

    /// Puts `document` in place of the active tab's untitled document. `None` when there is no
    /// active tab, or when its document is also shown in another group, which would keep it.
    pub(crate) fn replace_active_untitled(&mut self, document: Document) -> Option<Document> {
        let index = self.active_index();
        let old_id = self.current().tabs.get(index)?.document;
        if self.views_of(old_id).len() > 1 {
            return None;
        }
        let id = document.id;
        let old = self.store.replace(old_id, document)?;
        let group = self.active_group();
        let tab = &mut self.current_mut().tabs[index];
        tab.document = id;
        tab.view_state = ViewState::default();
        self.recent.retain(|recent| *recent != (group, old.id));
        self.touch(id);
        self.refresh_views();
        Some(old)
    }

    #[cfg(test)]
    pub fn ids(&self) -> impl Iterator<Item = DocumentId> + '_ {
        self.current().tabs.iter().map(|tab| tab.document)
    }

    pub fn titles(&self) -> impl Iterator<Item = String> + '_ {
        self.strip().map(Document::title)
    }

    pub fn activate(&mut self, id: DocumentId) -> Result<(), UnknownDocument> {
        let index = self.position(id).ok_or(UnknownDocument(id))?;
        self.current().select(index);
        self.touch(id);
        Ok(())
    }

    /// Moves the active group's view of `id` to the front of the activation order.
    fn touch(&mut self, id: DocumentId) {
        let group = self.active_group();
        self.touch_in(group, id);
    }

    fn touch_in(&mut self, group: GroupId, id: DocumentId) {
        self.recent.retain(|recent| *recent != (group, id));
        self.recent.insert(0, (group, id));
    }

    /// Every view, the most recently activated first; the active view leads (spec §3.2).
    pub(crate) fn activation_order(&self) -> &[(GroupId, DocumentId)] {
        &self.recent
    }

    /// Restarts the activation order from the strips, the active view first, then every group in
    /// creation order: how restored tabs enter it once a session restore has reopened them all
    /// (spec §3.2).
    pub(crate) fn reset_activation_order(&mut self) {
        let group = self.active_group();
        let active = self
            .current()
            .active_document()
            .map(|document| (group, document));
        let rest = self
            .groups
            .iter()
            .flat_map(|group| group.tabs.iter().map(|tab| (group.id, tab.document)))
            .filter(|view| Some(*view) != active)
            .collect::<Vec<_>>();
        self.recent = active.into_iter().chain(rest).collect();
    }

    pub fn activate_index(&mut self, index: usize) -> Result<(), UnknownDocument> {
        let id = self
            .current()
            .tabs
            .get(index)
            .map(|tab| tab.document)
            .ok_or(UnknownDocument(DocumentId(u64::MAX)))?;
        self.activate(id)
    }

    pub fn push(&mut self, document: Document) -> Result<(), DuplicateDocumentPath> {
        let id = document.id;
        self.store.insert(document)?;
        let group = self.current_mut();
        group.tabs.push(EditorTab {
            document: id,
            view_state: ViewState::default(),
        });
        group.select(group.tabs.len() - 1);
        self.touch(id);
        self.refresh_views();
        Ok(())
    }

    /// Closes the active tab. `Ok(None)` when its document is still shown in another group and
    /// so stays open.
    pub fn close_active(
        &mut self,
        decision: CloseDecision,
    ) -> Result<Option<Document>, CloseCancelled> {
        if decision == CloseDecision::Cancel || self.is_empty() {
            return Err(CloseCancelled);
        }
        let index = self.active_index();
        let (closed, document) = self.remove_tab(index);
        self.select_after_removal(index, closed);
        Ok(document)
    }

    /// Keeps the successor of a removed tab selected (or its predecessor at the end of the strip).
    /// `closed`'s view leaves the activation order and the tab now selected takes its front.
    fn select_after_removal(&mut self, removed: usize, closed: DocumentId) {
        let group = self.active_group();
        self.current().select_after_removal(removed);
        self.recent.retain(|recent| *recent != (group, closed));
        if let Some(id) = self.current().active_document() {
            self.touch(id);
        }
        self.refresh_views();
    }

    pub fn active_close_review(&self) -> Option<CloseReview> {
        let document = self.active()?;
        Some(CloseReview {
            id: document.id,
            generation: document.generation,
        })
    }

    /// Closes the active tab once its review is settled. `Ok(None)` when its document is still
    /// shown in another group and so stays open.
    pub fn close_reviewed(
        &mut self,
        review: CloseReview,
        decision: CloseDecision,
    ) -> Result<Option<Document>, CloseReviewError> {
        if decision == CloseDecision::Cancel {
            return Err(CloseReviewError::Cancelled);
        }
        let Some(index) = self.position(review.id) else {
            return Err(CloseReviewError::Stale);
        };
        let Some(document) = self.document_at(index) else {
            return Err(CloseReviewError::Stale);
        };
        if index != self.active_index() || document.generation != review.generation {
            return Err(CloseReviewError::Stale);
        }
        // A Save answer must never close a document whose save did not actually happen.
        if decision == CloseDecision::Save && document.dirty {
            return Err(CloseReviewError::Unsaved);
        }
        let (closed, document) = self.remove_tab(index);
        self.select_after_removal(index, closed);
        Ok(document)
    }

    /// Closes the clean tab `review` names without activating it (quick-open spec §5): the
    /// active tab stays active and keeps its place in the activation order. `Stale` when that
    /// tab has gone, is the active one or changed since `review`; `Unsaved` when it has unsaved
    /// edits, which only the usual reviewed close may discard. `Ok(None)` when its document is
    /// still shown in another group.
    pub(crate) fn close_clean_background(
        &mut self,
        review: CloseReview,
    ) -> Result<Option<Document>, CloseReviewError> {
        let Some(index) = self.position(review.id) else {
            return Err(CloseReviewError::Stale);
        };
        let Some(document) = self.document_at(index) else {
            return Err(CloseReviewError::Stale);
        };
        let active = self.active_index();
        if index == active || document.generation != review.generation {
            return Err(CloseReviewError::Stale);
        }
        if document.dirty {
            return Err(CloseReviewError::Unsaved);
        }
        let group = self.active_group();
        let (closed, document) = self.remove_tab(index);
        if index < active {
            self.current()
                .selection
                .active
                .store(active - 1, Ordering::Release);
        }
        self.recent.retain(|recent| *recent != (group, closed));
        self.refresh_views();
        Ok(document)
    }

    /// The first dirty document not yet `reviewed`, across every group (the exit review).
    pub fn next_dirty_review(&self, reviewed: &[CloseReviewKey]) -> Option<CloseReview> {
        self.documents()
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
        let Some(id) = self.active().map(|document| document.id) else {
            return false;
        };
        self.set_dirty(id, dirty)
    }

    /// Records `id`'s dirty state; returns whether it changed.
    pub(crate) fn set_dirty(&mut self, id: DocumentId, dirty: bool) -> bool {
        let Some(document) = self.store.get_mut(id) else {
            return false;
        };
        if document.is_image() {
            return false;
        }
        // Undo can reach Scintilla's empty save point; a recovered tab stays dirty until saved.
        if document.dirty == dirty || (!dirty && document.recovery_origin.is_some()) {
            return false;
        }
        document.dirty = dirty;
        document.generation = document.generation.saturating_add(1);
        self.refresh_views();
        true
    }

    /// Records the active document's language after an explicit `LanguageX` command or automatic
    /// detection, independent of applying the actual lexer to the live editor. Returns `false`
    /// (no-op) when the active document already has this language.
    pub(crate) fn set_active_language(&mut self, language: crate::document::Language) -> bool {
        let Some(document) = self.active_mut() else {
            return false;
        };
        if document.language == language {
            return false;
        }
        document.language = language;
        true
    }

    /// Every open document once: the groups in creation order, each in strip order.
    pub(crate) fn documents(&self) -> impl Iterator<Item = &Document> + '_ {
        let mut seen = Vec::new();
        self.groups
            .iter()
            .flat_map(|group| group.tabs.iter().map(|tab| tab.document))
            .filter(move |id| {
                if seen.contains(id) {
                    return false;
                }
                seen.push(*id);
                true
            })
            .filter_map(|id| self.store.get(id))
    }

    pub(crate) fn record_recovery_generation(&mut self, id: DocumentId, generation: u64) {
        if let Some(document) = self.store.get_mut(id) {
            document.recovery_generation = Some(generation);
        }
    }

    pub(crate) fn take_active_recovery_origin(
        &mut self,
    ) -> Option<crate::document::RecoveryOrigin> {
        self.active_mut()?.recovery_origin.take()
    }

    /// A text change in the active tab; see `note_text_change`.
    pub(crate) fn note_active_text_change(&mut self) -> bool {
        let Some(id) = self.active().map(|document| document.id) else {
            return false;
        };
        self.note_text_change(id)
    }

    /// A text change in `id`, recorded once however many views show it. The first one makes a
    /// preview tab normal, so replacing the preview can never drop an edit; returns whether that
    /// happened.
    pub(crate) fn note_text_change(&mut self, id: DocumentId) -> bool {
        if !self.store.note_text_change(id) {
            return false;
        }
        let Some(document) = self.store.get_mut(id) else {
            return false;
        };
        if !document.preview {
            return false;
        }
        document.preview = false;
        self.refresh_views();
        true
    }

    /// A background tab's text was changed through the document host, whose notifications reach
    /// no window (a Search replace, note-search spec §12a). Records what `SCN_SAVEPOINTLEFT` and
    /// `SCN_MODIFIED` would have for the active tab: dirty, a new generation (so recovery
    /// snapshots it), and a preview kept. Returns whether the strip changed.
    pub(crate) fn note_background_edit(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.store.get_mut(id) else {
            return false;
        };
        document.generation = document.generation.saturating_add(1);
        let changed = !document.dirty || document.preview;
        document.dirty = true;
        document.preview = false;
        if changed {
            self.refresh_views();
        }
        changed
    }

    /// The active group's preview tab, if it has one. There is at most one per group.
    pub(crate) fn preview_id(&self) -> Option<DocumentId> {
        self.strip()
            .find(|document| document.preview)
            .map(|document| document.id)
    }

    /// Puts `document` where the active group's preview tab is, selects it and returns the
    /// document it replaced. With no preview tab, `document` is added like any new tab and `None`
    /// is returned. The same happens when the preview somehow has unsaved edits, or is also shown
    /// in another group: it is kept as a normal tab. The caller has already checked that
    /// `document`'s file is not open in another tab.
    pub(crate) fn replace_preview(&mut self, document: Document) -> Option<Document> {
        let preview = self.current().tabs.iter().position(|tab| {
            self.store
                .get(tab.document)
                .is_some_and(|existing| existing.preview)
        });
        let replaceable = preview.filter(|index| {
            let id = self.current().tabs[*index].document;
            self.views_of(id).len() == 1
                && self.store.get(id).is_some_and(|existing| !existing.dirty)
        });
        match replaceable {
            Some(index) => {
                let id = document.id;
                let group = self.active_group();
                let old_id = self.current().tabs[index].document;
                let old = self.store.replace(old_id, document)?;
                let current = self.current_mut();
                let tab = &mut current.tabs[index];
                tab.document = id;
                tab.view_state = ViewState::default();
                current.select(index);
                self.recent.retain(|recent| *recent != (group, old.id));
                self.touch(id);
                self.refresh_views();
                Some(old)
            }
            None => {
                if let Some(existing) = preview.and_then(|index| self.document_at_mut(index)) {
                    existing.preview = false;
                }
                if self.push(document).is_err() {
                    self.refresh_views();
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
        self.refresh_views();
        true
    }

    /// Closes every tab and leaves one empty group.
    pub fn clear_for_shutdown(&mut self) {
        self.groups.truncate(1);
        let group = &mut self.groups[0];
        group.tabs.clear();
        group.selection.active.store(0, Ordering::Release);
        self.active = 0;
        self.store.clear();
        self.recent.clear();
        self.refresh_views();
    }

    pub(crate) fn active_handle(&self) -> Option<&crate::editor::EditorDocument> {
        self.active().and_then(Document::text_handle)
    }

    /// Renames the active document's path, e.g. after a successful Save As write. Rejects the
    /// rename if `path` canonicalizes to the same file another open tab already owns; see
    /// `DocumentStore::reject_path_collision`.
    pub(crate) fn set_active_path(&mut self, path: PathBuf) -> Result<(), DuplicateDocumentPath> {
        let Some(id) = self.active().map(|document| document.id) else {
            return Err(DuplicateDocumentPath(path));
        };
        let path = self.store.reject_path_collision(id, path)?;
        if let Some(document) = self.store.get_mut(id) {
            document.path = Some(path);
        }
        self.refresh_views();
        Ok(())
    }

    /// Rebinds `id`'s path, e.g. after a note is renamed on disk or moved between folders.
    /// Rejects the path if it canonicalizes to the same file another open tab already owns; see
    /// `DocumentStore::reject_path_collision`. Updates the tab views so the tab strips and
    /// accessibility see the new title.
    pub fn rebind_path(
        &mut self,
        id: DocumentId,
        path: PathBuf,
    ) -> Result<(), DuplicateDocumentPath> {
        if self.store.get(id).is_none() {
            return Err(DuplicateDocumentPath(path));
        }
        let path = self.store.reject_path_collision(id, path)?;
        if let Some(document) = self.store.get_mut(id) {
            document.path = Some(path);
        }
        self.refresh_views();
        Ok(())
    }

    /// Reverts the active document's path back to a value it previously, legitimately held (or
    /// `None`, if it had not yet claimed a path), e.g. after a Save As write that renamed the
    /// path via `set_active_path` but then failed before anything was actually written to the
    /// new location. No collision check is needed: `original` was already valid for this
    /// document, so restoring it cannot newly collide with any other tab.
    pub(crate) fn revert_active_path(&mut self, original: Option<PathBuf>) {
        if let Some(document) = self.active_mut() {
            document.path = original;
            self.refresh_views();
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

pub(crate) fn canonical_key(path: &Path) -> Result<PathBuf, DuplicateDocumentPath> {
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

pub(crate) fn lexical_key(path: &Path) -> String {
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
    use crate::editor::ViewState;
    use std::fs;

    use crate::document::CloseDecision;

    fn order(tabs: &Tabs) -> Vec<u64> {
        tabs.activation_order().iter().map(|(_, id)| id.0).collect()
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
        let closed = tabs.close_clean_background(first).unwrap().unwrap();
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

    #[test]
    fn each_tab_keeps_its_own_view_state() {
        // Break caught: switching tabs forgetting where you were, or one tab's caret landing in
        // another.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        let state = crate::editor::ViewState {
            caret: 7,
            anchor: 3,
            first_line: 2,
            x_offset: 0,
        };
        tabs.set_view_state(DocumentId(1), state);
        assert_eq!(tabs.view_state(DocumentId(1)), state);
        assert_eq!(
            tabs.view_state(DocumentId(2)),
            crate::editor::ViewState::default()
        );
    }

    #[test]
    fn closing_the_shown_tab_keeps_the_next_tabs_view_state() {
        // Break caught: the tab activated by a close inheriting the closed tab's caret.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        let second = crate::editor::ViewState {
            caret: 9,
            anchor: 9,
            first_line: 4,
            x_offset: 0,
        };
        tabs.set_view_state(DocumentId(2), second);
        tabs.activate(DocumentId(1)).unwrap();
        tabs.close_active(CloseDecision::Discard).unwrap();
        assert_eq!(
            tabs.active().map(|document| document.id),
            Some(DocumentId(2))
        );
        assert_eq!(tabs.view_state(DocumentId(2)), second);
    }

    #[test]
    fn a_document_can_have_a_view_in_each_group_and_leaves_with_its_last_view() {
        // Break caught: a second group's tab removing the document from the store when the first
        // group's tab closes, or a close leaving an orphan in the store.
        let mut tabs = Tabs::new();
        tabs.push(document(1)).unwrap();
        let first = tabs.active_group();
        let second = tabs.add_group();
        assert!(tabs.add_view(second, DocumentId(1), ViewState::default()));
        assert_eq!(tabs.views_of(DocumentId(1)), vec![first, second]);
        let kept = tabs.close_active(CloseDecision::Discard).unwrap();
        assert!(kept.is_none(), "not the last view: the document stays");
        assert_eq!(tabs.views_of(DocumentId(1)), vec![second]);
        assert!(tabs.document(DocumentId(1)).is_some());
        tabs.set_active_group(second);
        let closed = tabs.close_active(CloseDecision::Discard).unwrap().unwrap();
        assert_eq!(closed.id, DocumentId(1));
        assert!(tabs.document(DocumentId(1)).is_none());
    }

    #[test]
    fn the_facade_methods_act_on_the_active_group() {
        // Break caught: `active()` or `len()` reading the first group after the user moved to
        // another one, so commands act on the wrong tab.
        let mut tabs = Tabs::new();
        tabs.push(document(1)).unwrap();
        let second = tabs.add_group();
        assert!(tabs.set_active_group(second));
        assert!(tabs.is_empty());
        assert!(tabs.active().is_none());
        tabs.push(document(2)).unwrap();
        assert_eq!(
            tabs.active().map(|document| document.id),
            Some(DocumentId(2))
        );
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.documents().count(), 2, "documents() spans every group");
        assert_eq!(
            tabs.activation_order(),
            &[
                (second, DocumentId(2)),
                (tabs.group_ids()[0], DocumentId(1))
            ]
        );
    }

    #[test]
    fn a_second_view_promotes_a_preview_and_a_preview_is_replaced_only_in_its_group() {
        // Break caught: an italic tab whose document is also open elsewhere being replaced by the
        // next tree click, closing a view the user did not click away from.
        let mut tabs = Tabs::new();
        let mut preview = document(1);
        preview.preview = true;
        tabs.push(preview).unwrap();
        let first = tabs.active_group();
        let second = tabs.add_group();
        tabs.set_active_group(second);
        let mut other = document(2);
        other.preview = true;
        assert!(
            tabs.replace_preview(other).is_none(),
            "group 2 had no preview"
        );
        assert_eq!(tabs.group(first).unwrap().len(), 1);
        assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
        assert!(tabs.add_view(first, DocumentId(2), ViewState::default()));
        assert!(!tabs.document(DocumentId(2)).unwrap().preview);
    }

    #[test]
    fn moving_a_view_keeps_its_state_and_the_document() {
        // Break caught: Move to Next Group dropping the caret, or removing the document between
        // the removal and the insertion.
        let mut tabs = Tabs::new();
        tabs.push(document(1)).unwrap();
        let first = tabs.active_group();
        let second = tabs.add_group();
        let state = ViewState {
            caret: 7,
            anchor: 3,
            first_line: 2,
            x_offset: 0,
        };
        tabs.set_view_state_in(first, DocumentId(1), state);
        assert!(tabs.move_view(first, DocumentId(1), second));
        assert!(tabs.group(first).unwrap().is_empty());
        assert_eq!(tabs.view_state_in(second, DocumentId(1)), state);
        assert!(tabs.document(DocumentId(1)).is_some());
        assert!(tabs.remove_group(first));
        assert!(!tabs.remove_group(second), "the last group stays");
    }

    #[test]
    fn a_text_change_is_recorded_once_on_the_document() {
        // Break caught: a document-level change applied per view, bumping the generation twice
        // for one keystroke.
        let mut tabs = Tabs::new();
        tabs.push(document(1)).unwrap();
        let second = tabs.add_group();
        tabs.add_view(second, DocumentId(1), ViewState::default());
        let before = tabs.document(DocumentId(1)).unwrap().generation;
        tabs.note_text_change(DocumentId(1));
        assert_eq!(tabs.document(DocumentId(1)).unwrap().generation, before + 1);
        assert!(tabs.set_dirty(DocumentId(1), true));
        assert!(!tabs.set_dirty(DocumentId(1), true));
    }
}
