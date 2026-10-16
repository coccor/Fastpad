//! The open notebook's watcher, on the window side (second brain phase 1 spec §5.1): starting
//! and stopping it with the notebook, and acting on its batches of outside changes.

use super::*;
use crate::library::watch::{FileChanges, Watcher};
use crate::window::main_window::push_notice;
use std::path::Path;

/// How long after FastPad's own write finished a watcher report of that file is still its own.
/// The watcher posts 250 ms after the last change; this leaves room for a busy message queue.
const OWN_WRITE_GRACE: Duration = Duration::from_secs(2);
/// The longest a write that never reports back (cancelled, or its window busy) keeps a file's
/// changes counted as FastPad's own.
const OWN_WRITE_LIMIT: Duration = Duration::from_secs(60);

/// Starts watching the open notebook, once its state has loaded, unless notes mode is off, one
/// is already running, or watching it failed before.
pub(super) fn ensure_watcher(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let Some(folder) = folder(hwnd) else {
        return;
    };
    let start = host(hwnd, |host| {
        if host.watcher.is_some() || host.watch_failed {
            return None;
        }
        host.watch_generation += 1;
        Some(host.watch_generation)
    })
    .flatten();
    let Some(generation) = start else {
        return;
    };
    match Watcher::start(
        folder,
        hwnd as isize,
        crate::window::WM_FASTPAD_FILES_CHANGED,
        generation,
    ) {
        Ok(watcher) => {
            host(hwnd, |host| host.watcher = Some(watcher));
        }
        // A folder Windows can't watch (some network shares): rescans on activation stand in.
        Err(_) => {
            host(hwnd, |host| host.watch_failed = true);
        }
    }
}

/// Stops the watcher and waits for its thread: the notebook is closing or switching, notes mode
/// is going off, or the window is going away. The next notebook gets a fresh chance to be
/// watched.
pub(crate) fn stop_watcher(hwnd: HWND) {
    let watcher = host(hwnd, |host| {
        host.watch_generation += 1;
        host.watch_failed = false;
        host.own_writes.clear();
        // The next notebook opens showing all its notes.
        host.changed_only = false;
        host.watcher.take()
    })
    .flatten();
    drop(watcher);
}

/// Whether a watcher is watching the open notebook right now.
pub(crate) fn watching(hwnd: HWND) -> bool {
    host(hwnd, |host| host.watcher.is_some()).unwrap_or(false)
}

fn own_write_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// FastPad is about to write `paths` (files, or folders it fills) on a worker: the watcher's
/// reports of them, and of anything inside them, are its own.
pub(crate) fn begin_own_writes<'a>(hwnd: HWND, paths: impl IntoIterator<Item = &'a Path>) {
    let until = Instant::now() + OWN_WRITE_LIMIT;
    host(hwnd, |host| {
        for path in paths {
            host.own_writes.insert(own_write_key(path), Some(until));
        }
    });
}

/// FastPad's worker finished writing `paths`. A report that arrives a little later is still its
/// own.
pub(crate) fn end_own_writes<'a>(hwnd: HWND, paths: impl IntoIterator<Item = &'a Path>) {
    let until = Instant::now() + OWN_WRITE_GRACE;
    host(hwnd, |host| {
        for path in paths {
            host.own_writes.insert(own_write_key(path), Some(until));
        }
    });
}

/// Drops from `paths` the files FastPad itself is writing or has just written, and forgets the
/// writes whose grace has passed.
fn without_own_writes(hwnd: HWND, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let now = Instant::now();
    host(hwnd, |host| {
        host.own_writes
            .retain(|_, until| until.is_none_or(|until| until > now));
        paths
            .into_iter()
            .filter(|path| {
                let key = own_write_key(path);
                !host.own_writes.keys().any(|own| {
                    key == *own
                        || key
                            .strip_prefix(own.as_str())
                            .is_some_and(|rest| rest.starts_with('\\'))
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// `WM_FASTPAD_FILES_CHANGED`: a batch from the watcher. Returns whether `lparam` held one.
pub(crate) fn files_changed(hwnd: HWND, lparam: isize) -> bool {
    if lparam == 0 {
        return false;
    }
    let changes = unsafe { Box::from_raw(lparam as *mut FileChanges) };
    let current = host(hwnd, |host| host.watch_generation == changes.generation).unwrap_or(false);
    if !current {
        return true;
    }
    if changes.failed {
        let watcher = host(hwnd, |host| {
            host.watch_failed = true;
            host.watcher.take()
        })
        .flatten();
        drop(watcher);
        return true;
    }
    if changes.overflow {
        // Windows dropped changes: everything is checked again.
        request_rescan(hwnd);
        check_all_tabs(hwnd);
        return true;
    }
    let paths = without_own_writes(hwnd, changes.paths);
    if paths.is_empty() {
        return true;
    }
    check_paths(hwnd, &paths);
    mark_outside_changes(hwnd, &paths);
    if changes.names_changed {
        request_rescan(hwnd);
    }
    true
}

/// The files open in a tab some group is showing right now: the user sees them change, so they
/// are never marked.
fn visible_paths(hwnd: HWND) -> Vec<PathBuf> {
    unsafe { crate::window::main_window::app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            tabs.group_ids()
                .into_iter()
                .filter_map(|group| tabs.group(group)?.active_document())
                .filter_map(|id| tabs.document(id)?.path.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Marks the notes among `paths` whose size or write time changed, unless a tab shows them
/// (second brain phase 1 spec §5.5), and redraws the tree when a mark is new.
fn mark_outside_changes(hwnd: HWND, paths: &[PathBuf]) {
    let visible = visible_paths(hwnd);
    let marked = with_state(hwnd, |state| {
        let mut marked = false;
        for path in paths {
            let changed = state.take_outside_stamp(path) == Some(true);
            let seen = visible
                .iter()
                .any(|open| library::model::same_path(open, path));
            if changed
                && !seen
                && let Some(relative) = path.strip_prefix(&state.folder).ok()
            {
                marked |= state.mark_changed(relative);
            }
        }
        marked
    })
    .unwrap_or(false);
    if marked {
        save_local_soon(hwnd);
        crate::window::side_panel::refresh(hwnd);
    }
}

/// The user has seen the note at `path`: it opened in a tab, or a tab showing it came to the
/// front. Its "changed outside FastPad" mark goes.
pub(crate) fn note_seen(hwnd: HWND, path: &Path) {
    if with_state(hwnd, |state| state.clear_changed(path)).unwrap_or(false) {
        save_local_soon(hwnd);
        crate::window::side_panel::refresh(hwnd);
    }
}

/// Clears the marks of the notes the tabs show right now, after a rescan found them changed.
pub(super) fn clear_visible_marks(hwnd: HWND) {
    let cleared = visible_paths(hwnd)
        .iter()
        .filter(|path| with_state(hwnd, |state| state.clear_changed(path)).unwrap_or(false))
        .count();
    if cleared > 0 {
        save_local_soon(hwnd);
    }
}

/// Whether the Notebook view shows only notes changed outside FastPad.
pub(crate) fn changed_only(hwnd: HWND) -> bool {
    host(hwnd, |host| host.changed_only).unwrap_or(false)
}

/// Notes: Show changed notes: the Notebook view shows only the marked notes and the folders
/// they are in, or everything again.
pub(crate) fn toggle_changed_only(hwnd: HWND) {
    let Some(on) = host(hwnd, |host| {
        host.changed_only = !host.changed_only;
        host.changed_only
    }) else {
        return;
    };
    if on && with_state(hwnd, |state| state.local.changed.is_empty()).unwrap_or(true) {
        push_notice(
            hwnd,
            "No notes changed outside FastPad since you last opened them.".to_owned(),
        );
    }
    crate::window::side_panel::refresh(hwnd);
}

/// Notes: Clear changed marks.
pub(crate) fn clear_all_marks(hwnd: HWND) {
    if with_state(hwnd, LibraryState::clear_all_changed).unwrap_or(false) {
        save_local_soon(hwnd);
        crate::window::side_panel::refresh(hwnd);
    }
}
