//! What a command palette picker row does once chosen.

use super::*;
use crate::library;
use crate::window::command_palette::{PickerChoice, PickerKind};
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use windows_sys::Win32::Foundation::HWND;

#[cfg(test)]
thread_local! {
    static LAST_PICK: std::cell::RefCell<Option<(PickerKind, PickerChoice)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn take_last_pick() -> Option<(PickerKind, PickerChoice)> {
    LAST_PICK.with(|last| last.borrow_mut().take())
}

/// A picker row was chosen. Every kind resolves the row against what that picker showed.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    match (kind, choice) {
        (PickerKind::RecentFolder, PickerChoice::Item(index)) => {
            let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
            if let Some(folder) = shown.unwrap_or_default().get(index) {
                open_listed_notebook(hwnd, folder);
            }
        }
        (PickerKind::MoveToNotebook, PickerChoice::Item(index)) => {
            let Some((note, destinations)) = host(hwnd, |host| host.shown_move.take()).flatten()
            else {
                return;
            };
            let destination = match destinations.get(index) {
                Some(folder) => folder.clone(),
                // The row after the notebooks is "Browse…".
                None if index == destinations.len() => {
                    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
                        return;
                    };
                    let choice = crate::window::modal::choose_folder(hwnd);
                    if !identity.is_live_for(hwnd) {
                        return;
                    }
                    match choice {
                        Ok(Some(folder)) => library::normalize_folder(&folder),
                        Ok(None) => return,
                        Err(error) => {
                            push_notice(
                                hwnd,
                                format!("FastPad could not open the folder picker: {error}"),
                            );
                            return;
                        }
                    }
                }
                None => return,
            };
            move_note_to(hwnd, &note, &destination);
        }
        (PickerKind::QuickOpen, PickerChoice::Note { path, line }) => {
            crate::window::main_window::open_quick_open_choice(hwnd, &path, line);
        }
        (PickerKind::QuickOpen, PickerChoice::View { path, group }) => {
            // The tab listed, in the group listed: never a new view (split editors spec §7).
            let id = folder(hwnd).and_then(|folder| {
                let app = unsafe { app_ptr(hwnd) }?;
                unsafe { app.as_ref() }
                    .tabs
                    .find_stored_path(&folder.join(&path))
            });
            match id {
                Some(id) if crate::window::main_window::focus_view(hwnd, group, id) => {
                    crate::window::main_window::focus_content(hwnd);
                }
                _ => crate::window::main_window::open_quick_open_choice(hwnd, &path, None),
            }
        }
        (PickerKind::QuickOpen, PickerChoice::GoToLine(line)) => {
            crate::window::main_window::go_to_line(hwnd, line);
            // A note pick focuses the editor (`open_note(.., true)`); a `:n` pick moves the
            // caret the same way, so it must land keyboard focus there too, even when the pick
            // was made with the sidebar focused.
            crate::window::main_window::focus_content(hwnd);
        }
        _ => {}
    }
}
