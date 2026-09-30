//! The per-PC local file and the library's pending writes: expanded folders and the root
//! row, rescans, rebinding moved tabs, and the debounced and before-close metadata flushes.

use super::*;
use crate::library;
use crate::window::main_window::{app_ptr, push_notice};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

#[derive(Clone, Copy)]
pub(super) struct LocalWrite {
    /// Write on this thread: at window close, a writer thread might not finish before the exit.
    pub(super) wait: bool,
    /// Write even if the conveniences did not change (the file may hold an old scan cache).
    pub(super) force: bool,
}

/// Writes the per-PC local file when its expanded folders, autosave switch or missing times changed.
/// It also carries the scan cache (about 1 MB for 10,000 notes), so it is encoded and written on
/// a one-off writer thread from a snapshot: cloning the state is cheap next to encoding it and
/// syncing the write. `local::write_in_order` keeps an older snapshot from landing last.
pub(super) fn save_local(hwnd: HWND, how: LocalWrite) {
    let job = with_state(hwnd, |state| {
        let local = library::take_local_changes(state)
            .or_else(|| how.force.then(|| state.local.clone()))?;
        Some((state.local_path.clone(), local))
    })
    .flatten();
    let Some((path, local)) = job else {
        return;
    };
    host(hwnd, |host| {
        if host.scanning {
            host.local_written_during_scan = true;
        }
    });
    let order = library::local::next_write();
    // In-process tests write inline, so what they assert is on disk when they look.
    if how.wait || cfg!(test) {
        let _ = library::local::write_in_order(&path, &local, order);
    } else {
        std::thread::spawn(move || {
            let _ = library::local::write_in_order(&path, &local, order);
        });
    }
}

/// `save_local` on a writer thread, for a change the user just made (a folder rename).
pub(crate) fn save_local_soon(hwnd: HWND) {
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: false,
        },
    );
}

/// Expands or collapses `path`, a folder relative to the open notebook, and remembers it in the
/// per-PC file. The write happens on a writer thread, and only when the set changed.
pub(crate) fn set_expanded(hwnd: HWND, path: &Path, expanded: bool) {
    let changed = with_state(hwnd, |state| {
        let was = state.local.is_expanded(path);
        state.local.set_expanded(path, expanded);
        was != expanded
    })
    .unwrap_or(false);
    if changed {
        host(hwnd, |host| {
            host.expansion_revision = host.expansion_revision.wrapping_add(1);
        });
        save_local(
            hwnd,
            LocalWrite {
                wait: false,
                force: false,
            },
        );
    }
}

/// Changes whenever a folder is expanded or collapsed.
pub(crate) fn expansion_revision(hwnd: HWND) -> u64 {
    host(hwnd, |host| host.expansion_revision).unwrap_or(0)
}

/// Whether the open notebook's root row is expanded (open editors spec §3.3). True while no
/// notebook state is loaded, so the loading and failed states show under it.
pub(crate) fn root_expanded(hwnd: HWND) -> bool {
    with_state(hwnd, |state| !state.local.root_collapsed).unwrap_or(true)
}

/// Expands or collapses the notebook's root row and remembers it in the per-PC file, the way
/// `set_expanded` does for a folder.
pub(crate) fn set_root_expanded(hwnd: HWND, expanded: bool) {
    let changed = with_state(hwnd, |state| {
        let changed = state.local.root_collapsed == expanded;
        state.local.root_collapsed = !expanded;
        changed
    })
    .unwrap_or(false);
    if changed {
        host(hwnd, |host| {
            host.expansion_revision = host.expansion_revision.wrapping_add(1);
        });
        save_local_soon(hwnd);
    }
}

/// The open notebook's expanded folders, relative to it.
#[cfg_attr(not(test), expect(dead_code, reason = "read by the window tests"))]
pub(crate) fn expanded(hwnd: HWND) -> Vec<PathBuf> {
    with_state(hwnd, |state| state.local.expanded.clone()).unwrap_or_default()
}

/// A note moved outside FastPad (or was moved/renamed by FastPad itself): an open tab for it
/// follows the file. The old path is gone, so the tab is found by its stored path, not through
/// the disk. A move leaves the content alone, so when the new file's stamp is the one the tab
/// knows, autosave carries on (or resumes, if it paused when the old file vanished); otherwise
/// the tab keeps what it knew, and its next autosave pauses on the changed file.
///
/// `Ok(true)` when a tab followed, `Ok(false)` when none had `old` open, `Err` when a tab had
/// `old` open but could not be rebound because another tab already has `new` open — the file
/// moved, but that tab is left pointing at a path that no longer exists. `move_note_to` refuses
/// upfront when the target is already open, and the renames undo themselves on `Err`
/// (`submit_rename` rebinds its own tab; `inline_name` calls this and undoes the rename), so an
/// unexpected `Err` comes from a relocation the rescan finds: a file moved outside FastPad onto a
/// path another tab already has open. The caller reports it.
pub(crate) fn rebind_open_tab(hwnd: HWND, old: &Path, new: PathBuf) -> Result<bool, ()> {
    let stamp = library::disk_stamp(&new);
    let outcome = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        let Some(id) = app.tabs.find_stored_path(old) else {
            return Ok(false);
        };
        if app.tabs.rebind_path(id, new).is_err() {
            return Err(());
        }
        if let Some(document) = app.tabs.document_mut(id)
            && document.disk_stamp.is_some()
            && document.disk_stamp == stamp
        {
            document.autosave_paused = false;
        }
        Ok(true)
    });
    match outcome {
        Some(Ok(true)) => {
            crate::window::main_window::invalidate_title_strip(hwnd);
            Ok(true)
        }
        Some(Ok(false)) => Ok(false),
        Some(Err(())) => Err(()),
        None => Ok(false),
    }
}

pub(crate) fn request_rescan(hwnd: HWND) {
    let scanning = host(hwnd, |host| {
        if host.scanning {
            host.rescan_requested = true;
        }
        host.scanning
    })
    .unwrap_or(true);
    if !scanning && folder(hwnd).is_some() {
        start_load(hwnd);
    }
}

/// `WM_ACTIVATEAPP`. Coming back after at least `RESCAN_AFTER` rescans, to catch Explorer edits.
pub(crate) fn activation_changed(hwnd: HWND, active: bool) {
    if !active {
        host(hwnd, |host| host.inactive_since = Some(Instant::now()));
        autosave_active(hwnd);
        return;
    }
    let long_enough = host(hwnd, |host| {
        host.inactive_since
            .take()
            .is_some_and(|since| since.elapsed() >= RESCAN_AFTER)
    })
    .unwrap_or(false);
    if long_enough && notes_mode(hwnd) {
        request_rescan(hwnd);
    }
}

pub(crate) fn schedule_write(hwnd: HWND) {
    unsafe {
        SetTimer(hwnd, LIBRARY_WRITE_TIMER_ID, WRITE_DELAY_MS, None);
    }
}

/// Writes pending metadata now (the debounce timer; tests).
pub(crate) fn flush_now(hwnd: HWND) {
    flush_reporting(hwnd, false);
}

/// Writes pending metadata and the local file on this thread, before the window closes. A
/// `library.ini` held open by a sync gets a few short retries, since there is no later.
pub(crate) fn flush_before_close(hwnd: HWND) {
    for _ in 0..CLOSE_BUSY_RETRIES {
        if !matches!(try_flush(hwnd, true), Ok(library::Flushed::Busy)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    flush_reporting(hwnd, true);
}

pub(super) fn flush_reporting(hwnd: HWND, wait: bool) {
    match try_flush(hwnd, wait) {
        Ok(library::Flushed::Busy) => {
            // OneDrive (or another PC's FastPad) has library.ini open: keep the operations and
            // try again shortly, without a notice for what is usually a brief sync.
            unsafe {
                SetTimer(hwnd, LIBRARY_WRITE_TIMER_ID, BUSY_RETRY_MS, None);
            }
        }
        Ok(_) => {}
        Err(error) => push_notice(
            hwnd,
            format!("FastPad could not save this notebook's pins: {error}"),
        ),
    }
}

pub(super) fn try_flush(hwnd: HWND, wait: bool) -> crate::Result<library::Flushed> {
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    let result = with_state(hwnd, library::flush).unwrap_or(Ok(library::Flushed::Nothing));
    save_local(hwnd, LocalWrite { wait, force: false });
    result
}
