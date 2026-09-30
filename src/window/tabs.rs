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
        self.add_view_at(group, id, state, None)
    }

    /// `add_view` with a new view going in at strip `index` (clamped; `None`: the end). A group
    /// that already shows `id` selects that view where it is. A preview arriving where another
    /// preview already is becomes a normal tab: one preview per group (PR 3 amendment 3).
    pub(crate) fn add_view_at(
        &mut self,
        group: GroupId,
        id: DocumentId,
        state: ViewState,
        index: Option<usize>,
    ) -> bool {
        let Some(group_index) = self.group_index(group) else {
            return false;
        };
        if self.store.get(id).is_none() {
            return false;
        }
        let target = &mut self.groups[group_index];
        let position = match target.position(id) {
            Some(position) => position,
            None => {
                let at = index.unwrap_or(target.tabs.len()).min(target.tabs.len());
                target.tabs.insert(
                    at,
                    EditorTab {
                        document: id,
                        view_state: state,
                    },
                );
                at
            }
        };
        target.select(position);
        let other_preview = self.groups[group_index].tabs.iter().any(|tab| {
            tab.document != id
                && self
                    .store
                    .get(tab.document)
                    .is_some_and(|document| document.preview)
        });
        self.touch_in(group, id);
        if (self.views_of(id).len() > 1 || other_preview)
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
        self.move_view_at(from, id, to, None)
    }

    /// `move_view` into strip position `index` of `to` (`None`: the end).
    pub(crate) fn move_view_at(
        &mut self,
        from: GroupId,
        id: DocumentId,
        to: GroupId,
        index: Option<usize>,
    ) -> bool {
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
        self.add_view_at(to, id, tab.view_state, index)
    }

    /// Moves `group`'s tab at `from` to `to` in its strip. The document that was active stays
    /// selected. False for an unknown group or an index past the end.
    pub(crate) fn reorder(&mut self, group: GroupId, from: usize, to: usize) -> bool {
        let Some(index) = self.group_index(group) else {
            return false;
        };
        let target = &mut self.groups[index];
        if from >= target.tabs.len() || to >= target.tabs.len() {
            return false;
        }
        let active = target.active_document();
        let tab = target.tabs.remove(from);
        target.tabs.insert(to, tab);
        if let Some(position) = active.and_then(|active| target.position(active)) {
            target.select(position);
        }
        self.refresh_views();
        true
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

    /// Whether `group`'s strip offers the preview buttons, for its accessibility provider.
    pub(crate) fn set_preview_buttons_in(&self, group: GroupId, visible: bool) {
        if let Some(group) = self.group(group) {
            group.view.set_preview_buttons(visible);
        }
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
mod tests;
