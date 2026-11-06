//! The note index on the window side (second brain phase 1 spec §6.4): one build at a time on
//! a worker, started whenever the note list changed, and swapped in whole when it lands.

use super::*;
use crate::library::index::{self, NoteIndex};
use std::sync::Arc;

/// A finished build, posted back from the worker.
struct IndexReady {
    generation: u64,
    folder: PathBuf,
    index: NoteIndex,
}

/// The default notebook's index as it stands (empty until the first build lands), for tests that
/// open one notebook.
#[cfg(test)]
pub(crate) fn default_note_index(hwnd: HWND) -> Arc<NoteIndex> {
    default_notebook(hwnd)
        .map(|id| note_index_of(hwnd, id))
        .unwrap_or_default()
}

/// The notebook `id`'s index as it stands (empty until the first build lands).
pub(crate) fn note_index_of(hwnd: HWND, id: NotebookId) -> Arc<NoteIndex> {
    slot(hwnd, id, |slot| Arc::clone(&slot.index)).unwrap_or_default()
}

/// The index of the notebook that owns `path` (a note, or a notebook folder), or an empty index
/// when no open notebook does.
pub(crate) fn note_index_for(hwnd: HWND, path: &std::path::Path) -> Arc<NoteIndex> {
    notebook_of(hwnd, path)
        .map(|id| note_index_of(hwnd, id))
        .unwrap_or_default()
}

/// Whether the default notebook's first index build has landed.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn index_loaded(hwnd: HWND) -> bool {
    default_notebook(hwnd).is_some_and(|id| index_loaded_of(hwnd, id))
}

/// Whether the notebook `id`'s first index build has landed.
pub(crate) fn index_loaded_of(hwnd: HWND, id: NotebookId) -> bool {
    slot(hwnd, id, |slot| slot.index_loaded).unwrap_or(false)
}

/// Builds the default notebook's index.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn request_index(hwnd: HWND) {
    if let Some(id) = default_notebook(hwnd) {
        request_index_of(hwnd, id);
    }
}

/// Builds the notebook `id`'s index from its current note list, reading only the notes whose
/// size or write time changed. A request while a build runs makes one more run after it.
pub(crate) fn request_index_of(hwnd: HWND, id: NotebookId) {
    let Some(job) = with_notebook(hwnd, id, |state| {
        let listed = state
            .notes
            .iter()
            .filter(|note| index::is_indexed(&note.path))
            .cloned()
            .collect::<Vec<_>>();
        (
            state.folder.clone(),
            index::cache_file(&state.local_path),
            listed,
        )
    }) else {
        return;
    };
    let Some((generation, previous)) = slot(hwnd, id, |slot| {
        if slot.index_running {
            slot.index_again = true;
            return None;
        }
        slot.index_running = true;
        Some((slot.index_generation, Arc::clone(&slot.index)))
    })
    .flatten() else {
        return;
    };
    let (folder, cache, listed) = job;
    let target = hwnd as isize;
    std::thread::spawn(move || {
        // The first build of a notebook starts from what the last run wrote down.
        let cached;
        let previous: &NoteIndex = if previous.notes.is_empty() {
            cached = std::fs::read_to_string(&cache)
                .ok()
                .and_then(|text| NoteIndex::parse(&text))
                .unwrap_or_default();
            &cached
        } else {
            &previous
        };
        let (index, changed) = NoteIndex::build(&folder, &listed, previous);
        if changed {
            let _ = std::fs::create_dir_all(cache.parent().unwrap_or(&folder))
                .map_err(crate::FastPadError::Io)
                .and_then(|()| crate::file::saver::save_atomic(&cache, index.encode().as_bytes()));
        }
        let payload = Box::into_raw(Box::new(IndexReady {
            generation,
            folder,
            index,
        }));
        if unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_INDEX_READY,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// An empty finished build for the index generation `generation`, as a worker would post it.
#[cfg(test)]
pub(crate) fn test_index_ready_payload(generation: u64, folder: PathBuf) -> isize {
    Box::into_raw(Box::new(IndexReady {
        generation,
        folder,
        index: NoteIndex::default(),
    })) as isize
}

/// `WM_FASTPAD_INDEX_READY`: a build landed. It replaces the index when it is for the open
/// notebook, and runs once more if a request arrived meanwhile.
pub(crate) fn index_ready(hwnd: HWND, lparam: isize) {
    if lparam == 0 {
        return;
    }
    let ready = unsafe { Box::from_raw(lparam as *mut IndexReady) };
    // A build for an index since reset (its notebook closed or switched) names no slot. A reset
    // while it ran also cleared `index_running`: its own build is the one now.
    let Some(id) = host(hwnd, |host| host.slot_by_index_generation(ready.generation)).flatten()
    else {
        return;
    };
    let again = slot(hwnd, id, |slot| {
        slot.index_running = false;
        if library::model::same_path(&slot.folder, &ready.folder) {
            slot.index = Arc::new(ready.index);
            slot.index_loaded = true;
        }
        std::mem::take(&mut slot.index_again)
    })
    .unwrap_or(false);
    outside_renames_check_of(hwnd, id);
    crate::window::side_panel::index_changed(hwnd);
    if again {
        request_index_of(hwnd, id);
    }
}

/// Forgets the notebook `id`'s index: it closed or switched, or notes mode went off. A build
/// still running is dropped when it lands.
pub(crate) fn reset_index_of(hwnd: HWND, id: NotebookId) {
    host(hwnd, |host| {
        let generation = host.fresh_generation();
        let Some(slot) = host.slot_mut(id) else {
            return;
        };
        slot.index = Arc::default();
        slot.index_generation = generation;
        slot.index_loaded = false;
        slot.index_running = false;
        slot.index_again = false;
        slot.renamed_outside.clear();
        slot.renamed_outside_told = false;
    });
}
