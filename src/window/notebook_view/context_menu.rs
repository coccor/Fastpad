//! The Notebook view's context menus: for a row, from the mouse or the keyboard.

use super::*;
use crate::library::tree::RowKind;
use crate::window::commands::CommandId;
use crate::window::design::metrics::scale;
use crate::window::main_window::OpenMode;
use crate::window::menus::MenuEntry;
use crate::window::panel_cursor::Cursor;
use crate::window::side_panel::point_of;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;

impl NotebookView {
    /// Under row `index`, in main-window client coordinates, for a menu opened from the
    /// keyboard.
    pub(super) fn row_menu_point(&self, index: usize) -> POINT {
        let list = self.list_rect(self.client());
        let rect = self.row_rect(list, index).unwrap_or(list);
        self.to_main(POINT {
            x: rect.left + scale(24, self.dpi()),
            y: rect.bottom,
        })
    }
}

/// Row `index`'s context menu (spec §6.6), at `at` (main-window client coordinates) or under
/// the row when opened from the keyboard. The chosen entry acts on that row, not the active
/// tab. "Open in new tab" is `CommandId::Open` and "New note here" is `CommandId::NoteNew` here.
pub(crate) fn open_context_menu(hwnd: HWND, index: usize, at: Option<POINT>) {
    let Some((target, point)) = with_view(hwnd, |view| {
        if view.mode != Mode::Tree {
            return None;
        }
        view.select(index);
        Some((
            view.target(index),
            at.unwrap_or_else(|| view.row_menu_point(index)),
        ))
    })
    .flatten() else {
        return;
    };
    let Target::Row(row) = target else {
        return;
    };
    let Some(root) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    match &row.kind {
        RowKind::Note(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::local("Open in new tab", CommandId::Open),
                MenuEntry::command(
                    if row.pinned { "Unpin" } else { "Pin" },
                    CommandId::NoteTogglePin,
                ),
                MenuEntry::Separator,
                MenuEntry::command("Move to notebook...", CommandId::NoteMoveToNotebook),
                MenuEntry::local("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::local("Delete...\tDel", CommandId::NoteDelete),
            ];
            match crate::window::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::Open) => {
                    if let Err(error) = crate::window::main_window::open_note(
                        hwnd,
                        &path,
                        OpenMode::Permanent,
                        true,
                    ) {
                        crate::window::main_window::report_open_failure(hwnd, &path, &error);
                    }
                }
                Some(CommandId::NoteTogglePin) => {
                    crate::window::library_host::toggle_pin(hwnd, &path)
                }
                Some(CommandId::NoteMoveToNotebook) => {
                    crate::window::library_host::move_to_notebook(hwnd, &path);
                }
                Some(CommandId::NoteRename) => crate::window::inline_name::rename(hwnd, &row.kind),
                Some(CommandId::NoteRevealInExplorer) => {
                    crate::window::library_host::reveal(hwnd, &path)
                }
                Some(CommandId::NoteDelete) if crate::window::library_host::ready_library(hwnd) => {
                    crate::window::library_host::delete_file(hwnd, &path);
                }
                _ => {}
            }
        }
        RowKind::Folder(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("New note here", CommandId::NoteNew),
                MenuEntry::command("New folder here", CommandId::NoteNewFolder),
                MenuEntry::Separator,
                MenuEntry::local("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::local("Delete...\tDel", CommandId::NoteDelete),
            ];
            match crate::window::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::NoteNew) => {
                    crate::window::inline_name::new_note(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteNewFolder) => {
                    crate::window::inline_name::new_folder(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteRename) => crate::window::inline_name::rename(hwnd, &row.kind),
                Some(CommandId::NoteRevealInExplorer) => {
                    crate::window::library_host::reveal(hwnd, &path)
                }
                Some(CommandId::NoteDelete) if crate::window::library_host::ready_library(hwnd) => {
                    crate::window::library_host::delete_folder(hwnd, relative);
                }
                _ => {}
            }
        }
        RowKind::Draft => {}
    }
}

/// `WM_CONTEXTMENU`: from a right-click (screen coordinates) or from Shift+F10 or the
/// context-menu key (`lparam` of -1, for the selected row).
pub(super) fn context_menu(hwnd: HWND, lparam: LPARAM) {
    let keyboard = lparam as u32 == u32::MAX;
    let target = with_view(hwnd, |view| {
        if keyboard {
            // A row's menu is for the tree's row, not a header or a tab row.
            if view.cursor != Cursor::Tree {
                return None;
            }
            return view.list.selected.map(|index| (index, None));
        }
        let (x, y) = point_of(lparam);
        let mut client = POINT { x, y };
        unsafe {
            ScreenToClient(view.panel, &mut client);
        }
        match view.hit_test(client.x, client.y) {
            Hit::Row { index, .. } => Some((index, Some(view.to_main(client)))),
            _ => None,
        }
    })
    .flatten();
    if let Some((index, at)) = target {
        open_context_menu(hwnd, index, at);
    }
}
