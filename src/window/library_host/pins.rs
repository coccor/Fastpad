//! Guards for organizing the library (a readable `library.ini`, a saved file), confirmations
//! and error reports, and pinning notes.

use super::*;
use crate::library::model::LibraryError;
use crate::library::ops::PendingOp;
use crate::library::{self, LibraryState, Metadata, ids::IdSource};
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

pub(crate) fn notes_mode_notice(enabled: bool) -> &'static str {
    if enabled {
        "Notes mode is on. The open notebook is your note library."
    } else {
        "Notes mode is off. FastPad works as a plain file editor."
    }
}

pub(super) const READ_ONLY: &str = "This notebook's .fastpad\\library.ini can't be read, so pins are off until it is fixed or removed.";
pub(super) const BUSY: &str = "This notebook's .fastpad\\library.ini is in use by another program. FastPad reads it again when you come back to the window.";

/// True when organizing can proceed; otherwise explains why not.
pub(crate) fn ready_library(hwnd: HWND) -> bool {
    match host(hwnd, |host| host.state.as_ref().map(|state| state.metadata)).flatten() {
        Some(Metadata::Ready) => true,
        Some(Metadata::Unreadable) => {
            push_notice(hwnd, READ_ONLY.to_owned());
            false
        }
        Some(Metadata::Busy) => {
            push_notice(hwnd, BUSY.to_owned());
            false
        }
        None => {
            let notice = if folder(hwnd).is_some() {
                "Loading notebook…"
            } else {
                "Open a notebook first."
            };
            push_notice(hwnd, notice.to_owned());
            false
        }
    }
}

/// The active tab's file; for an untitled tab, explains that it must be saved first.
pub(crate) fn active_file(hwnd: HWND) -> Option<PathBuf> {
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    if path.is_none() {
        push_notice(hwnd, "Save this note first to organize it.".to_owned());
    }
    path
}

/// Applies one operation built from the host's ID source, then schedules the write. The model
/// validates before the operation is recorded, so a failed one leaves nothing pending.
pub(super) fn apply_op(
    hwnd: HWND,
    build: impl FnOnce(&mut LibraryState, &mut IdSource) -> Option<PendingOp>,
) -> Result<(), LibraryError> {
    let result = host(hwnd, |host| {
        let LibraryHost { state, ids, .. } = host;
        let state = state.as_mut()?;
        let op = build(state, ids)?;
        Some(state.apply(op))
    })
    .flatten()
    .unwrap_or(Ok(()));
    if result.is_ok() {
        schedule_write(hwnd);
    }
    result
}

pub(super) fn report(hwnd: HWND, result: Result<(), LibraryError>) {
    if let Err(error) = result {
        push_notice(hwnd, error.to_string());
    }
}

/// Asks `question`; false also when the window went away meanwhile.
pub(crate) fn confirmed(hwnd: HWND, question: &str, action: &str) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    crate::window::modal::confirm(hwnd, question, action) && identity.is_live_for(hwnd)
}

/// Pins or unpins `path`. Only a note inside the open notebook can be pinned: version 2 of
/// `library.ini` has no records for files outside it.
pub(crate) fn toggle_pin(hwnd: HWND, path: &Path) {
    if !ready_library(hwnd) {
        return;
    }
    if !folder(hwnd).is_some_and(|folder| library::is_inside(&folder, path)) {
        push_notice(
            hwnd,
            "Only notes in the open notebook can be pinned.".to_owned(),
        );
        return;
    }
    let mut now_on = false;
    let result = apply_op(hwnd, |state, ids| {
        now_on = !state.is_pinned(path);
        Some(PendingOp::SetPinned {
            note: state.note_ref(ids, path),
            value: now_on,
        })
    });
    if result.is_err() {
        report(hwnd, result);
        return;
    }
    push_notice(
        hwnd,
        if now_on { "Pinned." } else { "Unpinned." }.to_owned(),
    );
    crate::window::side_panel::refresh(hwnd);
}

/// The open notebook's pinned notes that are still listed, as paths relative to it; empty while
/// no notebook state is loaded.
pub(crate) fn pinned_notes(hwnd: HWND) -> Vec<PathBuf> {
    with_state(hwnd, |state| {
        library::pinned_paths(&state.library)
            .into_iter()
            .filter(|path| state.is_listed(path))
            .collect()
    })
    .unwrap_or_default()
}
