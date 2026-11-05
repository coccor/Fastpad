//! The open notebooks (multi-root notebooks spec §3.1): one `NotebookSlot` per notebook, in
//! sidebar order, each holding everything that is per-notebook, and the lookups that route a
//! worker's result to the slot it was for.

use super::*;
use crate::library::index::NoteIndex;
use crate::library::notebook_ini::NotebookSettings;
use crate::library::watch::Watcher;
use std::sync::Arc;

/// Names one open notebook for the session. Handed out when the notebook opens and never reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub(crate) struct NotebookId(pub(crate) u32);

/// One open notebook and its bookkeeping.
#[derive(Debug)]
pub(crate) struct NotebookSlot {
    pub(crate) id: NotebookId,
    /// The notebook's folder. Set as soon as it is opened (or, at startup, assumed), before its
    /// state has loaded.
    pub(crate) folder: PathBuf,
    pub(crate) state: Option<LibraryState>,
    /// The load in flight, or the last one: a result whose generation is not a slot's is dropped.
    pub(crate) generation: u64,
    pub(crate) scanning: bool,
    pub(crate) rescan_requested: bool,
    /// The UI thread wrote the local file while a scan ran, possibly with the old scan cache, so
    /// the merge that follows writes it again.
    pub(super) local_written_during_scan: bool,
    /// The last load of the notebook failed, so the sidebar offers Retry instead of saying
    /// "Loading…" forever. Starting a load clears it.
    pub(crate) load_failed: bool,
    /// The folder the first-load notices (truncated, unreadable) were last given for.
    pub(super) notified: Option<PathBuf>,
    /// Watches the notebook for outside changes (second brain phase 1 spec §5.1). `None` before
    /// its first scan, with notes mode off, or after watching failed.
    pub(crate) watcher: Option<Watcher>,
    /// Renewed whenever a watcher starts or stops: a batch from an older one is dropped.
    pub(crate) watch_generation: u64,
    /// Watching this notebook failed: FastPad rescans on activation instead.
    pub(crate) watch_failed: bool,
    /// The note index (second brain phase 1 spec §6): replaced whole when a build lands, so a
    /// worker can read the previous one while the UI keeps using it.
    pub(crate) index: Arc<NoteIndex>,
    /// Renewed whenever the index is reset: a build for an older index is dropped.
    pub(crate) index_generation: u64,
    /// The index has loaded (its first build landed).
    pub(crate) index_loaded: bool,
    pub(crate) index_running: bool,
    /// A request arrived while a build ran: one more runs after it.
    pub(crate) index_again: bool,
    /// Notes the scans found renamed outside FastPad (relative old and new paths) whose links
    /// were not updated yet (second brain phase 1 spec §7.6), and whether a notice said so.
    pub(crate) renamed_outside: Vec<(PathBuf, PathBuf)>,
    pub(crate) renamed_outside_told: bool,
    /// The notebook's `notebook.ini` (second brain phase 1 spec §9.1), and its text as last read,
    /// so a reload reports bad lines only when the file changed.
    pub(crate) notebook: NotebookSettings,
    pub(crate) notebook_text: Option<String>,
}

impl LibraryHost {
    /// The next value of the window-wide generation counter. Every load, watch and index
    /// generation comes from here, so a worker's generation names exactly one slot.
    pub(crate) fn fresh_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }

    /// Appends a slot for `folder`, nothing loaded yet. It becomes the default notebook when
    /// there is none.
    pub(super) fn add_notebook(&mut self, folder: PathBuf) -> NotebookId {
        self.next_id = self.next_id.wrapping_add(1);
        let id = NotebookId(self.next_id);
        let slot = NotebookSlot {
            id,
            folder,
            state: None,
            generation: self.fresh_generation(),
            scanning: false,
            rescan_requested: false,
            local_written_during_scan: false,
            load_failed: false,
            notified: None,
            watcher: None,
            watch_generation: self.fresh_generation(),
            watch_failed: false,
            index: Arc::default(),
            index_generation: self.fresh_generation(),
            index_loaded: false,
            index_running: false,
            index_again: false,
            renamed_outside: Vec::new(),
            renamed_outside_told: false,
            notebook: NotebookSettings::default(),
            notebook_text: None,
        };
        self.notebooks.push(slot);
        if self.default_notebook.is_none() {
            self.default_notebook = Some(id);
        }
        id
    }

    /// Removes the slot `id`. When it was the default, the first remaining slot is, or none.
    pub(super) fn remove_notebook(&mut self, id: NotebookId) -> Option<NotebookSlot> {
        let index = self.notebooks.iter().position(|slot| slot.id == id)?;
        let slot = self.notebooks.remove(index);
        if self.default_notebook == Some(id) {
            self.default_notebook = self.notebooks.first().map(|slot| slot.id);
        }
        Some(slot)
    }

    pub(super) fn slot_mut(&mut self, id: NotebookId) -> Option<&mut NotebookSlot> {
        self.notebooks.iter_mut().find(|slot| slot.id == id)
    }

    pub(super) fn slot_by_load_generation(&self, generation: u64) -> Option<NotebookId> {
        self.notebooks
            .iter()
            .find(|slot| slot.generation == generation)
            .map(|slot| slot.id)
    }

    pub(super) fn slot_by_watch_generation(&self, generation: u64) -> Option<NotebookId> {
        self.notebooks
            .iter()
            .find(|slot| slot.watch_generation == generation)
            .map(|slot| slot.id)
    }

    pub(super) fn slot_by_index_generation(&self, generation: u64) -> Option<NotebookId> {
        self.notebooks
            .iter()
            .find(|slot| slot.index_generation == generation)
            .map(|slot| slot.id)
    }

    /// The default notebook's slot.
    #[cfg(test)]
    pub(crate) fn default_slot(&mut self) -> Option<&mut NotebookSlot> {
        let id = self.default_notebook?;
        self.slot_mut(id)
    }

    /// The default notebook's loaded state.
    #[cfg(test)]
    pub(crate) fn default_state(&mut self) -> Option<&mut LibraryState> {
        self.default_slot()?.state.as_mut()
    }

    /// Whether a load is running for the default notebook, or the startup load that has no
    /// notebook to belong to yet.
    #[cfg(test)]
    pub(crate) fn scanning(&mut self) -> bool {
        self.startup_load.is_some() || self.default_slot().is_some_and(|slot| slot.scanning)
    }

    /// Makes `folder` the default notebook's folder, opening a slot for it if none is open, as
    /// the startup step does before the worker answers.
    #[cfg(test)]
    pub(crate) fn assume_folder(&mut self, folder: PathBuf) -> NotebookId {
        match self.default_slot() {
            Some(slot) => {
                slot.folder = folder;
                slot.id
            }
            None => self.add_notebook(folder),
        }
    }
}

/// Runs `f` on the slot `id`, if it is still open.
pub(super) fn slot<R>(
    hwnd: HWND,
    id: NotebookId,
    f: impl FnOnce(&mut NotebookSlot) -> R,
) -> Option<R> {
    host(hwnd, |host| host.slot_mut(id).map(f)).flatten()
}

/// Runs `f` on the default notebook's slot (a shim until every caller names its notebook).
pub(super) fn default_slot<R>(hwnd: HWND, f: impl FnOnce(&mut NotebookSlot) -> R) -> Option<R> {
    slot(hwnd, default_notebook(hwnd)?, f)
}

/// The open notebooks and their folders, in sidebar order.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the multi-root tree lists them")
)]
pub(crate) fn notebooks(hwnd: HWND) -> Vec<(NotebookId, PathBuf)> {
    host(hwnd, |host| {
        host.notebooks
            .iter()
            .map(|slot| (slot.id, slot.folder.clone()))
            .collect()
    })
    .unwrap_or_default()
}

/// The default notebook: where capture and the single-notebook commands go.
pub(crate) fn default_notebook(hwnd: HWND) -> Option<NotebookId> {
    host(hwnd, |host| host.default_notebook).flatten()
}

/// The folder of the open notebook `id`.
pub(crate) fn notebook_folder(hwnd: HWND, id: NotebookId) -> Option<PathBuf> {
    slot(hwnd, id, |slot| slot.folder.clone())
}

/// Runs `f` on the loaded state of the notebook `id`. Nothing happens when that notebook is
/// closed or has not loaded yet.
pub(crate) fn with_notebook<R>(
    hwnd: HWND,
    id: NotebookId,
    f: impl FnOnce(&mut LibraryState) -> R,
) -> Option<R> {
    slot(hwnd, id, |slot| slot.state.as_mut().map(f)).flatten()
}

/// The load, watch and index generations of the notebook `id`.
#[cfg(test)]
pub(crate) fn generations_of(hwnd: HWND, id: NotebookId) -> (u64, u64, u64) {
    slot(hwnd, id, |slot| {
        (
            slot.generation,
            slot.watch_generation,
            slot.index_generation,
        )
    })
    .unwrap_or_default()
}
