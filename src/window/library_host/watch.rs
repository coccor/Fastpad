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

/// Starts watching the default notebook (a shim until every caller names its notebook).
#[allow(
    dead_code,
    reason = "single-notebook shim; loads start the watcher of their own slot"
)]
pub(super) fn ensure_watcher(hwnd: HWND) {
    if let Some(id) = default_notebook(hwnd) {
        ensure_watcher_of(hwnd, id);
    }
}

/// Starts watching the notebook `id`, once its state has loaded, unless notes mode is off, one
/// is already running, or watching it failed before.
pub(super) fn ensure_watcher_of(hwnd: HWND, id: NotebookId) {
    if !notes_mode(hwnd) {
        return;
    }
    let start = host(hwnd, |host| {
        let slot = host.slot_mut(id)?;
        if slot.watcher.is_some() || slot.watch_failed {
            return None;
        }
        let generation = host.fresh_generation();
        let slot = host.slot_mut(id)?;
        slot.watch_generation = generation;
        Some((slot.folder.clone(), generation))
    })
    .flatten();
    let Some((folder, generation)) = start else {
        return;
    };
    match Watcher::start(
        folder,
        hwnd as isize,
        crate::window::WM_FASTPAD_FILES_CHANGED,
        generation,
    ) {
        Ok(watcher) => {
            slot(hwnd, id, |slot| slot.watcher = Some(watcher));
        }
        // A folder Windows can't watch (some network shares): rescans on activation stand in.
        Err(_) => {
            slot(hwnd, id, |slot| slot.watch_failed = true);
        }
    }
}

/// Stops every notebook's watcher: notes mode is going off, or the window is going away.
/// FastPad's own writes and the changed-notes filter are forgotten with them.
pub(crate) fn stop_watcher(hwnd: HWND) {
    host(hwnd, |host| {
        host.own_writes.clear();
        // The next notebook opens showing all its notes.
        host.changed_only = false;
        host.shown_templates = None;
    });
    for (id, _) in notebooks(hwnd) {
        stop_watcher_of(hwnd, id);
    }
}

/// Forgets what the window-wide state holds for the closed notebook `folder`: FastPad's own
/// writes under it, and the template list when it was for a folder in it. `last` means no
/// notebook remains, so the changed-notes filter resets too.
pub(super) fn forget_closed_notebook(hwnd: HWND, folder: &Path, last: bool) {
    host(hwnd, |host| {
        let prefix = own_write_key(folder);
        host.own_writes.retain(|key, _| {
            !(key == &prefix
                || key
                    .strip_prefix(&prefix)
                    .is_some_and(|rest| rest.starts_with(['\\', '/'])))
        });
        let templates_inside = match &host.shown_templates {
            Some(((id, _), _)) => host.notebooks.iter().any(|slot| {
                slot.id == *id && crate::library::model::same_path(&slot.folder, folder)
            }),
            None => false,
        };
        if templates_inside || last {
            host.shown_templates = None;
        }
        if last {
            host.changed_only = false;
        }
    });
}

/// Stops the notebook `id`'s watcher and waits for its thread. The notebook gets a fresh chance
/// to be watched.
pub(crate) fn stop_watcher_of(hwnd: HWND, id: NotebookId) {
    let watcher = host(hwnd, |host| {
        let generation = host.fresh_generation();
        let slot = host.slot_mut(id)?;
        slot.watch_generation = generation;
        slot.watch_failed = false;
        slot.watcher.take()
    })
    .flatten();
    drop(watcher);
    // The index and the conventions belong to the notebook that is going too.
    reset_index_of(hwnd, id);
    forget_notebook_settings_of(hwnd, id);
}

/// Whether a watcher is watching the default notebook right now.
#[cfg(test)]
pub(crate) fn watching(hwnd: HWND) -> bool {
    default_notebook(hwnd).is_some_and(|id| watching_of(hwnd, id))
}

/// Whether a watcher is watching the notebook `id` right now.
pub(crate) fn watching_of(hwnd: HWND, id: NotebookId) -> bool {
    slot(hwnd, id, |slot| slot.watcher.is_some()).unwrap_or(false)
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
    // A batch from a watcher since stopped names no slot.
    let Some(id) = host(hwnd, |host| {
        host.slot_by_watch_generation(changes.generation)
    })
    .flatten() else {
        return true;
    };
    if changes.failed {
        let watcher = slot(hwnd, id, |slot| {
            slot.watch_failed = true;
            slot.watcher.take()
        })
        .flatten();
        drop(watcher);
        return true;
    }
    if changes.overflow {
        // Windows dropped changes: everything is checked again.
        request_rescan_of(hwnd, id);
        check_all_tabs(hwnd);
        return true;
    }
    let paths = without_own_writes(hwnd, changes.paths);
    if paths.is_empty() {
        return true;
    }
    check_paths(hwnd, &paths);
    mark_outside_changes_of(hwnd, id, &paths);
    if paths.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(library::notebook_ini::NOTEBOOK_FILE))
    }) {
        reload_notebook_settings_of(hwnd, id);
    }
    if changes.names_changed {
        // The rescan's result asks for the index itself.
        request_rescan_of(hwnd, id);
    } else if paths.iter().any(|path| library::index::is_indexed(path)) {
        request_index_of(hwnd, id);
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
fn mark_outside_changes_of(hwnd: HWND, id: NotebookId, paths: &[PathBuf]) {
    let visible = visible_paths(hwnd);
    let marked = with_notebook(hwnd, id, |state| {
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
        save_local_of(
            hwnd,
            id,
            LocalWrite {
                wait: false,
                force: false,
            },
        );
        crate::window::side_panel::refresh(hwnd);
    }
}

/// The user has seen the note at `path`: it opened in a tab, or a tab showing it came to the
/// front. Its "changed outside FastPad" mark goes.
pub(crate) fn note_seen(hwnd: HWND, path: &Path) {
    let Some(id) = notebook_of(hwnd, path) else {
        return;
    };
    if with_notebook(hwnd, id, |state| state.clear_changed(path)).unwrap_or(false) {
        save_local_soon_of(hwnd, id);
        crate::window::side_panel::refresh(hwnd);
    }
}

/// Clears the marks of the notes the tabs show right now, after a rescan found them changed.
pub(super) fn clear_visible_marks_of(hwnd: HWND, id: NotebookId) {
    let cleared = visible_paths(hwnd)
        .iter()
        .filter(|path| with_notebook(hwnd, id, |state| state.clear_changed(path)).unwrap_or(false))
        .count();
    if cleared > 0 {
        save_local_of(
            hwnd,
            id,
            LocalWrite {
                wait: false,
                force: false,
            },
        );
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
    // One filter for the whole window (multi-root notebooks spec §3.1): it says so only when no
    // open notebook has a changed note.
    let none_changed = notebooks(hwnd).into_iter().all(|(id, _)| {
        with_notebook(hwnd, id, |state| state.local.changed.is_empty()).unwrap_or(true)
    });
    if on && none_changed {
        push_notice(
            hwnd,
            "No notes changed outside FastPad since you last opened them.".to_owned(),
        );
    }
    crate::window::side_panel::refresh(hwnd);
}

/// Notes: Clear changed marks, in every open notebook: the changed-notes filter they feed is
/// one for the whole window.
pub(crate) fn clear_all_marks(hwnd: HWND) {
    let mut cleared = false;
    for (id, _) in notebooks(hwnd) {
        if with_notebook(hwnd, id, LibraryState::clear_all_changed).unwrap_or(false) {
            save_local_soon_of(hwnd, id);
            cleared = true;
        }
    }
    if cleared {
        crate::window::side_panel::refresh(hwnd);
    }
}
