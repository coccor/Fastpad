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

/// The open notebook's index as it stands (empty until the first build lands).
pub(crate) fn note_index(hwnd: HWND) -> Arc<NoteIndex> {
    host(hwnd, |host| Arc::clone(&host.index)).unwrap_or_default()
}

/// Whether the open notebook's first index build has landed.
pub(crate) fn index_loaded(hwnd: HWND) -> bool {
    host(hwnd, |host| host.index_loaded).unwrap_or(false)
}

/// Builds the index from the current note list, reading only the notes whose size or write time
/// changed. A request while a build runs makes one more run after it.
pub(crate) fn request_index(hwnd: HWND) {
    let Some(job) = with_state(hwnd, |state| {
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
    let Some((generation, previous)) = host(hwnd, |host| {
        if host.index_running {
            host.index_again = true;
            return None;
        }
        host.index_running = true;
        Some((host.index_generation, Arc::clone(&host.index)))
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

/// `WM_FASTPAD_INDEX_READY`: a build landed. It replaces the index when it is for the open
/// notebook, and runs once more if a request arrived meanwhile.
pub(crate) fn index_ready(hwnd: HWND, lparam: isize) {
    if lparam == 0 {
        return;
    }
    let ready = unsafe { Box::from_raw(lparam as *mut IndexReady) };
    let open = folder(hwnd);
    let again = host(hwnd, |host| {
        if host.index_generation != ready.generation {
            // A reset while it ran also cleared `index_running`: its own build is the one now.
            return false;
        }
        host.index_running = false;
        if open
            .as_deref()
            .is_some_and(|open| library::model::same_path(open, &ready.folder))
        {
            host.index = Arc::new(ready.index);
            host.index_loaded = true;
        }
        std::mem::take(&mut host.index_again)
    })
    .unwrap_or(false);
    outside_renames_check(hwnd);
    crate::window::side_panel::index_changed(hwnd);
    if again {
        request_index(hwnd);
    }
}

/// Forgets the index: the notebook closed or switched, or notes mode went off. A build still
/// running is dropped when it lands.
pub(crate) fn reset_index(hwnd: HWND) {
    host(hwnd, |host| {
        host.index = Arc::default();
        host.index_generation += 1;
        host.index_loaded = false;
        host.index_running = false;
        host.index_again = false;
        host.renamed_outside.clear();
        host.renamed_outside_told = false;
    });
}
