//! The command palette host: opening it in command or picker mode, quick open, filtering and
//! running its selection, and the About, Settings and Keyboard Shortcuts launchers.

use super::*;

/// Overlays the palette at the top of the editor area, even with no tab open (New and Open stay
/// available then), below the tab strip and a visible find bar or name box so all stay usable.
pub(super) fn layout_command_palette(hwnd: HWND) {
    if !with_command_palette(hwnd, CommandPalette::is_visible).unwrap_or(false) {
        return;
    }
    let top = title_layout(hwnd).height + menu_band_height(hwnd) + bar_band_height(hwnd);
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let font = title_chrome(hwnd).1.text();
    let left = crate::window::side_panel::left_edge(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
    {
        palette.measure(width, dpi, font);
    }
    with_command_palette(hwnd, |palette| {
        palette.apply_layout(left, width, top, dpi, font)
    });
}

/// Runs `action` on the palette through a shared borrow only, so re-entrant window-procedure
/// calls its Win32 messages trigger can borrow the App again.
pub(super) fn with_command_palette<R>(
    hwnd: HWND,
    action: impl FnOnce(&CommandPalette) -> R,
) -> Option<R> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.command_palette.as_ref().map(action)
}

/// Records where focus should return, and the sidebar's focused note if the panel had the
/// keyboard focus, before the command palette or a picker takes it for its query field (spec
/// §6.3). `close_command_palette` restores the focus; `run_command_palette_selection` takes the
/// note for the command it runs.
pub(super) fn capture_palette_focus(hwnd: HWND) {
    let panel = crate::window::side_panel::windows(hwnd).map(|(_, panel)| panel);
    let focused_panel = panel.filter(|&panel| unsafe { GetFocus() } == panel);
    let note = focused_panel.and_then(|_| crate::window::notebook_view::focused_note(hwnd));
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.palette_note_target = note;
        app.palette_focus_return = focused_panel.unwrap_or(std::ptr::null_mut());
    }
}

/// Takes the note `capture_palette_focus` recorded, if any. Consumed at most once per palette
/// visit: by the command it runs, or discarded when the palette closes without running one.
fn take_palette_note_target(hwnd: HWND) -> Option<std::path::PathBuf> {
    unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.palette_note_target.take())
}

/// The theme's link colour, as the Markdown preview draws links.
pub(crate) fn link_color(hwnd: HWND) -> u32 {
    let theme = effective_theme(hwnd);
    let high_contrast = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .theme
            .is_some_and(|system| system.high_contrast)
    });
    crate::preview::colors::preview_colors(theme, high_contrast).link
}

/// Help → About FastPad, in the current theme's colors and the Markdown preview's link color.
pub(super) fn show_about(hwnd: HWND) {
    crate::window::about::show(hwnd, current_palette(hwnd), link_color(hwnd));
}

/// The Settings dialog: File → Settings…, Ctrl+, and the activity bar's gear (settings dialog
/// spec §4.3).
pub(crate) fn show_settings(hwnd: HWND) {
    show_settings_page(hwnd, crate::window::settings_model::Page::General);
}

/// Preferences: Open Keyboard Shortcuts (keyboard shortcuts spec section 2).
pub(crate) fn show_keyboard_shortcuts(hwnd: HWND) {
    show_settings_page(hwnd, crate::window::settings_model::Page::Shortcuts);
}

fn show_settings_page(hwnd: HWND, page: crate::window::settings_model::Page) {
    let outcome =
        crate::window::settings_dialog::show(hwnd, current_palette(hwnd), link_color(hwnd), page);
    if outcome == crate::window::settings_dialog::Outcome::EditIni {
        edit_settings_file(hwnd);
    }
}

pub(super) const EDIT_INI_NOTICE: &str =
    "Changes saved in fastpad.ini apply the next time FastPad starts.";

/// Preferences: Edit fastpad.ini. Creates the file (empty) when it doesn't exist yet, then opens
/// it in a tab through the normal open path (settings dialog spec §3.6).
pub(crate) fn edit_settings_file(hwnd: HWND) {
    let result = settings_file_for_editing().and_then(|path| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        open_path(hwnd, &path)
    });
    match result {
        // Settings are read once, at startup: say so rather than leave a saved edit looking
        // ignored.
        Ok(()) => push_notice(hwnd, EDIT_INI_NOTICE.to_owned()),
        Err(error) => push_notice(hwnd, format!("FastPad could not open fastpad.ini: {error}")),
    }
}

#[cfg(not(test))]
fn settings_file_for_editing() -> Result<std::path::PathBuf> {
    crate::config::persisted::settings_file_path()
}

/// Tests open only the file they chose with `save_settings_to`.
#[cfg(test)]
fn settings_file_for_editing() -> Result<std::path::PathBuf> {
    TEST_SETTINGS_PATH
        .with(|path| path.borrow().clone())
        .ok_or(crate::FastPadError::Invariant(
            "a test opened fastpad.ini without save_settings_to",
        ))
}

pub(crate) fn open_command_palette(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    capture_palette_focus(hwnd);
    let colors = title_chrome(hwnd).0;
    let newly_shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.command_palette.is_none() {
            app.command_palette = CommandPalette::create(hwnd).ok();
        }
        let palette = app.command_palette.as_mut()?;
        let newly_shown = palette.mark_shown(colors);
        // Reopening the palette normally always shows commands, even right after a picker.
        palette.set_picker(None);
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    if newly_shown {
        // Clearing the field sends EN_CHANGE, which lists every available command.
        with_command_palette(hwnd, CommandPalette::clear_query);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// Opens the palette in picker mode: it lists `picker`'s items instead of commands, and the
/// choice made on Enter goes to `library_host::picked` instead of running a command.
pub(crate) fn open_picker(hwnd: HWND, picker: command_palette::Picker) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    capture_palette_focus(hwnd);
    let colors = title_chrome(hwnd).0;
    let newly_shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.command_palette.is_none() {
            app.command_palette = CommandPalette::create(hwnd).ok();
        }
        let palette = app.command_palette.as_mut()?;
        let newly_shown = palette.mark_shown(colors);
        palette.set_picker(Some(picker));
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    if newly_shown {
        with_command_palette(hwnd, CommandPalette::clear_query);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// Ctrl+P, the palette row and File → Go to note… (quick-open spec §3.1): the palette in the
/// `QuickOpen` picker with an empty query. While that picker already shows, nothing changes.
pub(crate) fn open_quick_open(hwnd: HWND) {
    let (visible, showing) = with_command_palette(hwnd, |palette| {
        let quick_open = palette
            .picker()
            .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen);
        (palette.is_visible(), palette.is_visible() && quick_open)
    })
    .unwrap_or((false, false));
    if showing {
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // Still made on first use (spec §3.6), and with nothing borrowed: it creates windows.
    let missing = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.command_palette.is_none());
    if missing {
        let Ok(created) = CommandPalette::create(hwnd) else {
            return;
        };
        if !identity.is_live_for(hwnd) {
            return;
        }
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            if app.command_palette.is_none() {
                app.command_palette = Some(created);
            }
        }
    }
    // Switching from the open command list keeps the focus the palette first took from.
    if !visible {
        capture_palette_focus(hwnd);
    }
    let colors = title_chrome(hwnd).0;
    let shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let palette = unsafe { app.as_mut() }.command_palette.as_mut()?;
        palette.mark_shown(colors);
        palette.set_picker(Some(command_palette::Picker {
            kind: command_palette::PickerKind::QuickOpen,
            items: Vec::new(),
            create: None,
        }));
        Some(())
    });
    if shown.is_none() {
        return;
    }
    // Always from an empty query. Clearing sends EN_CHANGE, which lists the rows.
    with_command_palette(hwnd, CommandPalette::clear_query);
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// The quick-open rows for `query` and the row to select (spec §3.1–3.4). Only reads what is
/// in memory: the tabs and `LibraryState.notes`.
pub(super) fn quick_open_rows(
    hwnd: HWND,
    query: &str,
) -> (Vec<command_palette::PickerRow>, Option<usize>) {
    use crate::library::quick_open::{self, QuickMatch};
    use command_palette::PickerRow;
    let (text, line) = quick_open::split_line(query);
    let typed = !text.trim().is_empty();
    // `:<n>` alone needs no notebook: it moves the current tab's caret.
    if let Some(line) = line.filter(|_| !typed) {
        return (vec![PickerRow::GoToLine(line)], Some(0));
    }
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return (vec![PickerRow::Notice(command_palette::NO_NOTEBOOK)], None);
    };
    if typed {
        let rows = crate::window::library_host::with_state(hwnd, |state| {
            quick_open::search(
                state.notes.iter().map(|note| &note.path),
                text,
                command_palette::QUICK_OPEN_ROWS,
            )
        })
        .unwrap_or_default()
        .into_iter()
        .map(|found| PickerRow::Note { found, line })
        .collect::<Vec<_>>();
        let selected = (!rows.is_empty()).then_some(0);
        return (rows, selected);
    }
    // Nothing typed: the notes open in tabs, the most recently used first (spec §3.2), across
    // groups (split editors spec §7). Tabs outside the notebook are left out here, untitled ones
    // by having no path.
    let order = group_order(hwnd);
    let number = |group: GroupId| {
        (order.len() > 1)
            .then(|| {
                order
                    .iter()
                    .position(|id| *id == group)
                    .map(|index| index + 1)
            })
            .flatten()
    };
    let open = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let active = tabs
                .active()
                .map(|document| (tabs.active_group(), document.id));
            tabs.activation_order()
                .iter()
                .filter_map(|&(group, id)| {
                    let path = tabs.document(id)?.path.as_deref()?;
                    let relative = crate::library::record_path(&folder, path);
                    (!relative.is_absolute()).then(|| {
                        (
                            crate::library::path_key(&relative),
                            Some((group, id)) == active,
                            group,
                        )
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if open.is_empty() {
        return (Vec::new(), None);
    }
    // Then only the notes the library lists, spelled as it spells them.
    let (rows, first_active) = crate::window::library_host::with_state(hwnd, |state| {
        let wanted = open
            .iter()
            .map(|(key, ..)| key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut listed = std::collections::HashMap::new();
        for note in &state.notes {
            let key = crate::library::path_key(&note.path);
            if wanted.contains(key.as_str()) {
                listed.insert(key, note.path.clone());
            }
        }
        let mut rows = Vec::new();
        let mut first_active = false;
        for (key, active, group) in &open {
            let Some(path) = listed.get(key) else {
                continue;
            };
            let Some(found) = QuickMatch::plain(path) else {
                continue;
            };
            if rows.is_empty() {
                first_active = *active;
            }
            rows.push(PickerRow::View {
                found,
                group: *group,
                number: number(*group),
            });
        }
        (rows, first_active)
    })
    .unwrap_or_default();
    // The current note leads, so the selection starts on the one before it: Ctrl+P then Enter
    // goes back to the previous note.
    let selected = match rows.len() {
        0 => None,
        1 => Some(0),
        _ if first_active => Some(1),
        _ => Some(0),
    };
    (rows, selected)
}

/// Shows `id`'s view in `group`, making that group active; never adds a view. False when `group`
/// has no view of `id`. The focus stays where it is.
pub(crate) fn focus_view(hwnd: HWND, group: GroupId, id: DocumentId) -> bool {
    let revision = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = unsafe { app.as_ref() }.tabs.group(group)?;
        tabs.contains(id).then(|| tabs.view().snapshot().revision)
    });
    let Some(revision) = revision else {
        return false;
    };
    activate_group(hwnd, group);
    let shown = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(group)
            .is_some_and(|tabs| tabs.active_document() == Some(id))
    });
    shown || activate_document_in(hwnd, group, id, revision)
}

/// Opens a quick-open pick (spec §3.5). `relative` is resolved again against the notebook's
/// notes, since it may have left the library since the list was shown; then it opens as a
/// normal tab (an open one is switched to) with the focus in the editor, and `line` applies.
pub(crate) fn open_quick_open_choice(hwnd: HWND, relative: &std::path::Path, line: Option<u32>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    let key = crate::library::path_key(relative);
    let listed = crate::window::library_host::with_state(hwnd, |state| {
        state
            .notes
            .iter()
            .any(|note| crate::library::path_key(&note.path) == key)
    })
    .unwrap_or(false);
    if !listed {
        report_open_failure(
            hwnd,
            &path,
            &crate::FastPadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the note is no longer in the notebook",
            )),
        );
        return;
    }
    if let Err(error) = open_note(hwnd, &path, OpenMode::Permanent, true) {
        report_open_failure(hwnd, &path, &error);
        return;
    }
    if let Some(line) = line
        && identity.is_live_for(hwnd)
    {
        go_to_line(hwnd, line);
    }
}

/// Moves the active tab's caret to the start of 1-based `line`, the last line when past the end,
/// and scrolls it into view (spec §3.4). Does nothing with no tab open.
pub(crate) fn go_to_line(hwnd: HWND, line: u32) {
    if tab_count(hwnd) == 0 {
        return;
    }
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let _ = editor.go_to_line(line.saturating_sub(1) as usize);
}

/// Hides the palette. `restore_focus` returns the focus to the editor (or the frame with no tab);
/// it is false when the focus already moved somewhere else.
pub(crate) fn close_command_palette(hwnd: HWND, restore_focus: bool) {
    let was_visible = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .command_palette
            .as_mut()
            .is_some_and(CommandPalette::mark_hidden)
    });
    if !was_visible {
        return;
    }
    with_command_palette(hwnd, CommandPalette::hide_controls);
    // Repaint what the palette covered now: a command run right after this (Find) can move the
    // editor before its queued paint, leaving the palette's pixels in the unpainted gaps.
    if let Some(editor_hwnd) = unsafe { editor_hwnd(hwnd) } {
        unsafe {
            windows_sys::Win32::Graphics::Gdi::UpdateWindow(editor_hwnd);
        }
    }
    // A leftover note (the palette closed without running the command that would consume it)
    // must not leak into some later, unrelated command.
    let _ = take_palette_note_target(hwnd);
    let panel_return = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let panel = std::mem::replace(&mut app.palette_focus_return, std::ptr::null_mut());
        (!panel.is_null()).then_some(panel)
    });
    if restore_focus {
        // The panel had focus when the palette opened: give it back, rather than the editor.
        let target = panel_return.unwrap_or_else(|| {
            if tab_count(hwnd) > 0 {
                content_focus_target(hwnd).unwrap_or(hwnd)
            } else {
                hwnd
            }
        });
        unsafe {
            SetFocus(target);
        }
    }
}

pub(super) fn refilter_command_palette(hwnd: HWND) {
    let Some(query) = with_command_palette(hwnd, |palette| {
        palette.is_visible().then(|| palette.query_text())
    })
    .flatten() else {
        return;
    };
    let is_picker =
        with_command_palette(hwnd, |palette| palette.picker().is_some()).unwrap_or(false);
    if is_picker {
        let quick_open = with_command_palette(hwnd, |palette| {
            palette
                .picker()
                .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen)
        })
        .unwrap_or(false);
        // Built before the palette is borrowed: the rows read the tabs and the library.
        let quick_rows = quick_open.then(|| quick_open_rows(hwnd, &query));
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            match quick_rows {
                Some((rows, selected)) => palette.set_picker_rows(rows, selected),
                None => {
                    if let Some(picker) = palette.picker() {
                        let rows = command_palette::picker_rows(picker, &query);
                        let selected = (!rows.is_empty()).then_some(0);
                        palette.set_picker_rows(rows, selected);
                    }
                }
            }
        }
    } else {
        let has_tabs = tab_count(hwnd) > 0;
        let markdown = crate::window::preview_host::buttons_visible(hwnd);
        let image = crate::window::image_host::active_is_image(hwnd);
        // Not `buttons_visible`, which also holds for SVG.
        let markdown_document = active_language(hwnd) == crate::document::Language::Markdown;
        let sidebar = notes_mode_enabled(hwnd);
        // New note and New folder need a notebook, open or loading, to put the item in (inline
        // naming spec §3.1).
        let notebook = crate::window::library_host::folder(hwnd).is_some();
        let groups = unsafe { app_ptr(hwnd) }.map_or(1, |app| unsafe { app.as_ref() }.groups.len());
        let entries = command_palette::filter_entries(&query, |command| {
            (has_tabs || !command.needs_document())
                && (!image || !command.needs_text())
                && (markdown || !command.is_markdown_preview())
                && (markdown_document || !command.is_markdown_edit())
                && (sidebar || !command.is_sidebar())
                && (notebook || !matches!(command, CommandId::NoteNew | CommandId::NoteNewFolder))
                // Close Group with one empty group would do nothing.
                && (has_tabs || groups > 1 || command != CommandId::CloseGroup)
        });
        let keymap = keymap(hwnd);
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            palette.set_entries(entries, &keymap);
        }
    }
    with_command_palette(hwnd, CommandPalette::fill_list);
    layout_command_palette(hwnd);
}

/// `WM_PAINT` for a palette, find bar or name box panel.
pub(crate) fn paint_panel(hwnd: HWND, panel: HWND) {
    let fonts = title_chrome(hwnd).1;
    let (glyph_font, text_font) = (fonts.glyph(), fonts.text());
    let painted = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        if let Some(palette) = app.command_palette.as_ref().filter(|p| p.owns(panel)) {
            palette.paint_panel(panel);
            true
        } else if let Some(bar) = app
            .groups
            .iter()
            .filter_map(|group| group.find_bar.as_ref())
            .find(|bar| bar.owns(panel))
        {
            bar.paint_panel(panel, glyph_font, text_font);
            true
        } else if let Some(name_box) = app.name_box.as_ref().filter(|n| n.owns(panel)) {
            name_box.paint_panel(panel, text_font);
            true
        } else {
            false
        }
    });
    if !painted {
        // Validates the region so an orphaned panel does not repaint forever.
        unsafe {
            windows_sys::Win32::Graphics::Gdi::ValidateRect(panel, std::ptr::null());
        }
    }
}

pub(crate) fn run_command_palette_selection(hwnd: HWND) {
    let pick = with_command_palette(hwnd, |palette| {
        palette
            .picker()
            .map(|picker| (picker.kind, palette.selected_choice()))
    })
    .flatten();
    // A quick-open row that can't be picked ("No notebook is open"), or no row at all, leaves
    // the picker open (spec §3.1).
    if matches!(pick, Some((command_palette::PickerKind::QuickOpen, None))) {
        return;
    }
    let command = if pick.is_none() {
        with_command_palette(hwnd, CommandPalette::selected_command).flatten()
    } else {
        None
    };
    // Taken before closing moves focus off the sidebar panel, so a note-scoped command still
    // knows which row was focused when the palette opened (spec §6.3).
    let note = take_palette_note_target(hwnd);
    close_command_palette(hwnd, true);
    match pick {
        Some((kind, Some(choice))) => crate::window::library_host::picked(hwnd, kind, choice),
        Some((_, None)) => {}
        None => {
            if let Some(command) = command {
                execute_command_with_note(hwnd, command, note);
            }
        }
    }
}

pub(crate) fn move_command_palette_selection(hwnd: HWND, step: isize) {
    with_command_palette(hwnd, |palette| palette.move_selection(step));
}

pub(crate) fn select_command_palette_row(hwnd: HWND, lparam: LPARAM) -> bool {
    with_command_palette(hwnd, |palette| palette.select_row_at(lparam)).unwrap_or(false)
}

pub(crate) fn focus_command_palette(hwnd: HWND) {
    with_command_palette(hwnd, CommandPalette::focus_query);
}

pub(crate) fn command_palette_owns(hwnd: HWND, control: HWND) -> bool {
    !control.is_null()
        && with_command_palette(hwnd, |palette| palette.owns(control)).unwrap_or(false)
}
