//! Input on the Keyboard Shortcuts page: its clicks and effects, the row context menu,
//! record-keys search, and the search field's keys, characters, focus and cue.

use super::*;

/// A click released on the shortcuts page.
pub(super) fn page_click(hwnd: HWND, hit: crate::window::shortcuts_page::PageHit) {
    use crate::window::shortcuts_model::ShortcutsEffect;
    use crate::window::shortcuts_page::PageHit;
    let now = unsafe { GetMessageTime() } as u32;
    let double_click_time = unsafe { GetDoubleClickTime() };
    let effect = state(hwnd).map(|dialog| {
        let model = &mut dialog.shortcuts;
        match hit {
            PageHit::RecordToggle => model.toggle_record_keys(),
            PageHit::Row(index) => {
                let double = dialog.last_row_click.is_some_and(|(time, row)| {
                    row == index && now.wrapping_sub(time) <= double_click_time
                });
                dialog.last_row_click = (!double).then_some((now, index));
                model.select(index);
                if double {
                    model.start_change()
                } else {
                    ShortcutsEffect::Repaint
                }
            }
            PageHit::Pencil(index) => {
                model.select(index);
                model.start_change()
            }
            PageHit::ConflictLink => model.follow_conflicts(),
            PageHit::RecordBox => ShortcutsEffect::None,
            PageHit::OutsideRecordBox => model.cancel_recording(),
        }
    });
    if let Some(effect) = effect {
        run_shortcuts(hwnd, effect);
    }
    // The toggle, and the conflict link (which turns record-keys on), hand the field the keys.
    let record_keys = state(hwnd).is_some_and(|dialog| dialog.shortcuts.record_keys);
    if hit == PageHit::RecordToggle || (hit == PageHit::ConflictLink && record_keys) {
        run_shortcuts(hwnd, ShortcutsEffect::FocusSearch);
    }
}

/// Carries out a shortcuts page effect. As in `run`, no `Dialog` borrow may be alive: applying
/// keys runs main window code.
pub(crate) fn run_shortcuts(hwnd: HWND, effect: crate::window::shortcuts_model::ShortcutsEffect) {
    use crate::window::shortcuts_model::ShortcutsEffect;
    match effect {
        ShortcutsEffect::None => {}
        ShortcutsEffect::Repaint => invalidate(hwnd),
        ShortcutsEffect::SetKeys(command, keys) => {
            crate::window::main_window::set_command_keys(owner(hwnd), command, keys);
            after_keymap_change(hwnd);
        }
        ShortcutsEffect::Reset(command) => {
            crate::window::main_window::reset_command_keys(owner(hwnd), command);
            after_keymap_change(hwnd);
        }
        ShortcutsEffect::CopyId(id) => {
            if let Err(error) = crate::platform::clipboard::set_text(hwnd, id) {
                crate::window::main_window::push_notice(
                    owner(hwnd),
                    format!("FastPad could not copy to the clipboard: {error}"),
                );
            }
        }
        ShortcutsEffect::FocusSearch | ShortcutsEffect::FocusTable => {
            let focus = if effect == ShortcutsEffect::FocusSearch {
                Focus::Search
            } else {
                Focus::Table
            };
            let repaint = state(hwnd).map(|dialog| dialog.model.set_focus(focus, &dialog.view));
            if let Some(repaint) = repaint {
                run(hwnd, repaint);
            }
        }
        ShortcutsEffect::SetSearchText(text) => {
            let search = state(hwnd).and_then(|dialog| dialog.search);
            if let Some(search) = search {
                unsafe {
                    SetWindowTextW(search, wide_null(&text).as_ptr());
                    // The cue follows the mode even when the text stays empty.
                    InvalidateRect(search, std::ptr::null(), 1);
                }
            }
            invalidate(hwnd);
        }
        ShortcutsEffect::Close => close(hwnd),
    }
}

pub(super) const CHANGE: usize = 1;
pub(super) const ADD: usize = 2;
pub(super) const REMOVE: usize = 3;
pub(super) const RESET: usize = 4;
pub(super) const COPY_ID: usize = 5;

/// The selected row's menu (keyboard shortcuts spec §6.4), at client point `at`.
pub(super) fn context_menu(hwnd: HWND, at: POINT) {
    let Some((user, has_key)) = state(hwnd).and_then(|dialog| {
        let shortcuts = &dialog.shortcuts;
        shortcuts
            .selected_row()
            .map(|row| (shortcuts.can_reset(), row.stroke.is_some()))
    }) else {
        return;
    };
    let mut items = vec![
        ("Change keybinding\tEnter".to_owned(), CHANGE),
        ("Add keybinding\tCtrl+Enter".to_owned(), ADD),
    ];
    if has_key {
        items.push(("Remove keybinding\tDelete".to_owned(), REMOVE));
    }
    if user {
        items.push(("Reset keybinding".to_owned(), RESET));
    }
    items.push((String::new(), 0));
    items.push(("Copy command ID\tCtrl+C".to_owned(), COPY_ID));
    let choice = crate::window::menus::track_choice(owner(hwnd), hwnd, &items, at);
    let effect = state(hwnd).map(|dialog| {
        let model = &mut dialog.shortcuts;
        match choice {
            Some(CHANGE) => model.start_change(),
            Some(ADD) => model.start_add(),
            Some(REMOVE) => model.remove(),
            Some(RESET) => model.reset(),
            Some(COPY_ID) => model.copy_id(),
            _ => crate::window::shortcuts_model::ShortcutsEffect::None,
        }
    });
    if let Some(effect) = effect {
        run_shortcuts(hwnd, effect);
    }
}

/// Shift+F10 or the context-menu key (`WM_CONTEXTMENU` with no point): the selected row's
/// menu, under the row, when the table has the focus and no recording box is open.
pub(super) fn keyboard_context_menu(hwnd: HWND) {
    let at = state(hwnd).and_then(|dialog| {
        if dialog.model.page != Page::Shortcuts
            || dialog.model.focus != Focus::Table
            || dialog.shortcuts.recording.is_some()
        {
            return None;
        }
        let selected = dialog.shortcuts.selected;
        dialog.shortcuts.selected_row()?;
        // Scrolled away, the row comes back into view first.
        dialog.shortcuts.select(selected);
        let rect = dialog.page_layout.row_rect(selected - dialog.shortcuts.top);
        Some(POINT {
            x: rect.left + (rect.bottom - rect.top) / 2,
            y: rect.bottom,
        })
    });
    if let Some(at) = at {
        invalidate(hwnd);
        context_menu(hwnd, at);
    }
}

/// Alt+K from outside the search field: toggles record-keys search, and when that turns it on
/// the field takes the focus so the next stroke is recorded there.
pub(super) fn toggle_record_keys(hwnd: HWND) {
    use crate::window::shortcuts_model::ShortcutsEffect;
    let Some(effect) = state(hwnd).map(|dialog| dialog.shortcuts.toggle_record_keys()) else {
        return;
    };
    run_shortcuts(hwnd, effect);
    if state(hwnd).is_some_and(|dialog| dialog.shortcuts.record_keys) {
        run_shortcuts(hwnd, ShortcutsEffect::FocusSearch);
    }
}

/// Re-reads the keymap after a change applied, keeping the selection.
pub(super) fn after_keymap_change(hwnd: HWND) {
    let keymap = crate::window::main_window::keymap(owner(hwnd));
    let lines = crate::window::main_window::key_line_commands(owner(hwnd));
    if let Some(dialog) = state(hwnd) {
        dialog.shortcuts.refresh(keymap, lines);
    }
    if unsafe { GetFocus() } != hwnd {
        unsafe { SetFocus(hwnd) };
    }
    invalidate(hwnd);
}

/// The stroke `virtual_key` makes with the modifiers held now.
pub(super) fn current_stroke(virtual_key: u16) -> Option<KeyStroke> {
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    KeyStroke::from_key(virtual_key, held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU))
}

/// A key on the shortcuts page: the recording box takes every key, Alt+K toggles record-keys
/// search, and the focused table takes its keys. False when the key is not the page's: Shift+F10
/// then reaches DefWindowProc, which sends `WM_CONTEXTMENU` for the row menu.
pub(super) fn shortcuts_key(hwnd: HWND, virtual_key: u16) -> bool {
    let stroke = current_stroke(virtual_key);
    let idle = state(hwnd).is_some_and(|dialog| {
        dialog.model.page == Page::Shortcuts && dialog.shortcuts.recording.is_none()
    });
    if idle && stroke == Some(KeyStroke::new(false, false, true, u16::from(b'K'))) {
        toggle_record_keys(hwnd);
        return true;
    }
    let routed = state(hwnd).and_then(|dialog| {
        if dialog.model.page != Page::Shortcuts {
            return None;
        }
        if dialog.shortcuts.recording.is_some() {
            // Every key goes to the box; a modifier alone records nothing.
            return Some(stroke.map_or(
                crate::window::shortcuts_model::ShortcutsEffect::None,
                |stroke| dialog.shortcuts.record_key(stroke),
            ));
        }
        let stroke = stroke?;
        if dialog.model.focus != Focus::Table {
            return None;
        }
        let effect = dialog.shortcuts.table_key(stroke);
        (effect != crate::window::shortcuts_model::ShortcutsEffect::None).then_some(effect)
    });
    let Some(effect) = routed else {
        return false;
    };
    run_shortcuts(hwnd, effect);
    true
}

/// A key pressed in the search field; true when the dialog took it (keyboard shortcuts spec
/// §6.2).
pub(crate) fn search_key(dialog: HWND, virtual_key: u16) -> bool {
    use crate::window::shortcuts_model::ShortcutsEffect;
    let stroke = current_stroke(virtual_key);
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(record_keys) = state(dialog).map(|d| d.shortcuts.record_keys) else {
        return false;
    };
    let plain = |key: u16| stroke == Some(KeyStroke::new(false, false, false, key));
    let effect = if virtual_key == VK_TAB && !held(VK_CONTROL) && !held(VK_MENU) {
        let effect = state(dialog).map(|d| {
            d.model.key(
                Key::Tab {
                    back: held(VK_SHIFT),
                },
                &d.view,
            )
        });
        if let Some(effect) = effect {
            run(dialog, effect);
        }
        return true;
    } else if held(VK_CONTROL) && matches!(virtual_key, VK_PRIOR | VK_NEXT) {
        let effect = state(dialog).map(|d| {
            d.model.key(
                Key::NextPage {
                    back: virtual_key == VK_PRIOR,
                },
                &d.view,
            )
        });
        if let Some(effect) = effect {
            run(dialog, effect);
        }
        return true;
    } else if stroke == Some(KeyStroke::new(false, false, true, u16::from(b'K')))
        || (record_keys && plain(VK_ESCAPE))
    {
        state(dialog).map(|d| d.shortcuts.toggle_record_keys())
    } else if record_keys {
        // Every stroke becomes the filter; a modifier alone waits for its key.
        match stroke {
            Some(stroke) => state(dialog).map(|d| d.shortcuts.record_search_key(stroke)),
            None => return true,
        }
    } else if plain(VK_DOWN) || plain(VK_RETURN) {
        // Into the table on its first row, whatever was selected before the filter changed.
        state(dialog).map(|d| {
            d.shortcuts.select(0);
            ShortcutsEffect::FocusTable
        })
    } else if plain(VK_ESCAPE) {
        Some(ShortcutsEffect::Close)
    } else {
        return false;
    };
    if let Some(effect) = effect {
        run_shortcuts(dialog, effect);
    }
    true
}

/// Whether the field must not see a character: every one in record-keys mode (so a recorded
/// `S` never brings its `s` along), the Tab, Enter and Escape characters it would beep at, and
/// Alt+letters.
pub(crate) fn search_swallows_char(dialog: HWND, c: u32, sys: bool) -> bool {
    sys || matches!(c, 0x09 | 0x0d | 0x1b) || state(dialog).is_some_and(|d| d.shortcuts.record_keys)
}

/// The field took the focus (a click on it): the model follows. The field is outside the
/// recording box, so taking the focus while the box is open (it never asks for it) cancels
/// the box, as any click outside it does.
pub(crate) fn search_focused(dialog: HWND) {
    if let Some(d) = state(dialog) {
        d.shortcuts.cancel_recording();
        d.model.focus = Focus::Search;
    }
    invalidate(dialog);
}

/// The empty field's cue, in the muted colour.
pub(crate) fn paint_search_cue(dialog: HWND, edit: HWND) {
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, EndPaint, FillRect, PAINTSTRUCT, SetBkMode, TRANSPARENT,
    };
    let Some((brush, color, font, cue)) = state(dialog).map(|d| {
        let cue = if d.shortcuts.record_keys {
            "Press keys to search"
        } else {
            "Type to search in keybindings"
        };
        (d.search_brush, d.colors.muted_foreground, d.body_font, cue)
    }) else {
        return;
    };
    let mut paint = PAINTSTRUCT::default();
    unsafe {
        let dc = BeginPaint(edit, &mut paint);
        let mut client = RECT::default();
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(edit, &mut client);
        FillRect(dc, &client, brush);
        let previous = SelectObject(dc, font as _);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let mut text = wide_null(cue);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut client,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        EndPaint(edit, &paint);
    }
}

/// `hwnd`'s text.
pub(super) fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}
