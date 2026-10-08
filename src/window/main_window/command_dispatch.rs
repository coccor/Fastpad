//! Running commands: `execute_command` and the note-target rules for commands chosen from the
//! command palette.

use super::*;

/// Commands that act on a note rather than a command in the general sense (spec §6.3): the row
/// recorded when the palette opened or currently focused in the sidebar's tree, else (except
/// Rename and Delete, whose no-target path looks up the active tab itself) the active tab's file.
fn is_note_command(command: CommandId) -> bool {
    matches!(
        command,
        CommandId::NoteTogglePin
            | CommandId::NoteMoveToNotebook
            | CommandId::NoteRevealInExplorer
            | CommandId::NoteRename
            | CommandId::NoteDelete
    )
}

/// Pin/Move to notebook/Reveal's target: `tree` (the row the palette recorded, or the tree's
/// currently focused row), else the active tab's file.
fn note_target(hwnd: HWND, tree: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    tree.map(std::path::Path::to_path_buf)
        .or_else(|| crate::window::library_host::active_file(hwnd))
}

pub(super) fn execute_command(hwnd: HWND, command: CommandId) {
    execute_command_with_note(hwnd, command, None);
}

#[cfg(test)]
thread_local! {
    /// The last command `execute_command_with_note` received, for the tests that check what a
    /// key chord runs.
    pub(super) static LAST_COMMAND: std::cell::Cell<Option<CommandId>> = const { std::cell::Cell::new(None) };
}

/// Counts `is_sidebar` commands that ran past the notes-mode guard below. The sidebar's own state
/// (`app.sidebar`, the Search view's options) already reads as empty/default with no sidebar to
/// hold it, so a test disabling notes mode has nothing else to observe; this hook makes the guard
/// itself a regression test rather than an untested `if`.
#[cfg(test)]
static SIDEBAR_COMMAND_RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(test)]
pub(super) fn sidebar_command_runs() -> u32 {
    SIDEBAR_COMMAND_RUNS.load(std::sync::atomic::Ordering::Relaxed)
}

/// `execute_command`, for a command chosen from the command palette: `recorded` is the sidebar
/// row that `capture_palette_focus` recorded when the palette opened, taken by
/// `run_command_palette_selection` before closing it moved focus off the panel (spec §6.3).
pub(super) fn execute_command_with_note(
    hwnd: HWND,
    command: CommandId,
    recorded: Option<std::path::PathBuf>,
) {
    #[cfg(test)]
    LAST_COMMAND.with(|last| last.set(Some(command)));
    exit_menu_mode(hwnd);
    if file_population_active(hwnd) {
        return;
    }
    let tree_note = is_note_command(command)
        .then(|| recorded.or_else(|| crate::window::notebook_view::focused_note(hwnd)))
        .flatten();
    // Rename and Delete on a focused folder row act on the folder (notebook folders spec §4.2).
    let tree_folder = (matches!(command, CommandId::NoteRename | CommandId::NoteDelete)
        && tree_note.is_none())
    .then(|| crate::window::notebook_view::focused_folder(hwnd))
    .flatten();
    // A focused (or recorded) note or folder lets a note-scoped command through even with no
    // tab open.
    if command.needs_document()
        && tab_count(hwnd) == 0
        && tree_note.is_none()
        && tree_folder.is_none()
    {
        return;
    }
    // An image tab has no text to save, edit, search or relabel (image preview spec §5).
    if command.needs_text()
        && tree_note.is_none()
        && crate::window::image_host::active_is_image(hwnd)
    {
        return;
    }
    // Sidebar commands do nothing with notes mode off: there is no sidebar to act on (spec §5).
    if command.is_sidebar() && !notes_mode_enabled(hwnd) {
        return;
    }
    #[cfg(test)]
    if command.is_sidebar() {
        SIDEBAR_COMMAND_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(index) = command.tab_index() {
        if index < tab_count(hwnd) {
            activate_tab(hwnd, index);
        }
        return;
    }
    if let Some(index) = command.group_index() {
        focus_group_number(hwnd, index);
        return;
    }
    match command {
        CommandId::Open => {
            let identity = unsafe { window_identity(hwnd) };
            // Modal Show reenters the window procedure. Only an owned identity crosses it.
            let selection = crate::window::modal::choose_open_path(hwnd);
            if identity
                .as_ref()
                .is_some_and(|identity| identity.is_live_for(hwnd))
            {
                match selection {
                    Ok(Some(path)) => {
                        if let Err(error) = App::open_path(hwnd, &path) {
                            report_open_failure(hwnd, &path, &error);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => push_notice(
                        hwnd,
                        format!("FastPad could not show the Open dialog: {error}"),
                    ),
                }
            }
        }
        CommandId::New => crate::window::library_host::new_note_in(hwnd),
        CommandId::CloseTab => close_active_document(hwnd),
        CommandId::CloseAllTabs => close_all_documents(hwnd),
        CommandId::Save => crate::window::library_host::save_command(hwnd),
        CommandId::SaveAs => crate::window::library_host::save_as_command(hwnd),
        CommandId::Undo => with_editor(hwnd, |editor| {
            let _ = editor.undo();
        }),
        CommandId::Redo => with_editor(hwnd, |editor| {
            let _ = editor.redo();
        }),
        CommandId::Cut => with_editor(hwnd, |editor| {
            let _ = editor.cut();
        }),
        CommandId::Copy => with_editor(hwnd, |editor| {
            let _ = editor.copy();
        }),
        CommandId::Paste => with_editor(hwnd, |editor| {
            let _ = editor.paste();
        }),
        CommandId::MoveLinesUp => with_editor(hwnd, |editor| {
            let _ = editor.move_lines(true);
        }),
        CommandId::MoveLinesDown => with_editor(hwnd, |editor| {
            let _ = editor.move_lines(false);
        }),
        CommandId::CopyLinesUp => with_editor(hwnd, |editor| {
            let _ = editor.copy_lines(false);
        }),
        CommandId::CopyLinesDown => with_editor(hwnd, |editor| {
            let _ = editor.copy_lines(true);
        }),
        CommandId::DeleteLines => with_editor(hwnd, |editor| {
            let _ = editor.delete_lines();
        }),
        CommandId::InsertLineBelow => with_editor(hwnd, |editor| {
            let _ = editor.insert_line(true);
        }),
        CommandId::InsertLineAbove => with_editor(hwnd, |editor| {
            let _ = editor.insert_line(false);
        }),
        CommandId::IndentLines => with_editor(hwnd, |editor| {
            let _ = editor.indent_lines(false);
        }),
        CommandId::OutdentLines => with_editor(hwnd, |editor| {
            let _ = editor.indent_lines(true);
        }),
        CommandId::ExpandLineSelection => with_editor(hwnd, |editor| {
            let _ = editor.expand_line_selection();
        }),
        CommandId::ToggleLineComment => {
            let syntax = active_language(hwnd).comment_syntax();
            with_editor(hwnd, |editor| {
                let _ = editor.toggle_line_comment(syntax);
            });
        }
        CommandId::ToggleBlockComment => {
            let syntax = active_language(hwnd).comment_syntax();
            with_editor(hwnd, |editor| {
                let _ = editor.toggle_block_comment(syntax);
            });
        }
        CommandId::AddNextOccurrence => with_editor(hwnd, |editor| {
            let _ = editor.add_next_occurrence();
        }),
        CommandId::SelectAllOccurrences => with_editor(hwnd, |editor| {
            let _ = editor.select_all_occurrences();
        }),
        CommandId::AddCursorAbove => with_editor(hwnd, |editor| {
            let _ = editor.add_cursor(true);
        }),
        CommandId::AddCursorBelow => with_editor(hwnd, |editor| {
            let _ = editor.add_cursor(false);
        }),
        CommandId::Find => open_find_bar(hwnd, find_bar::FindBarMode::Find),
        CommandId::FindNext => find_again(hwnd, false),
        CommandId::FindPrevious => find_again(hwnd, true),
        CommandId::CommandPalette => open_command_palette(hwnd),
        CommandId::About => show_about(hwnd),
        CommandId::OpenSettings => show_settings(hwnd),
        CommandId::EditSettingsFile => edit_settings_file(hwnd),
        CommandId::OpenKeyboardShortcuts => show_keyboard_shortcuts(hwnd),
        CommandId::QuickOpen => open_quick_open(hwnd),
        CommandId::NoteNewFolder => crate::window::inline_name::new_folder(hwnd, None),
        CommandId::NoteNew => crate::window::inline_name::new_note(hwnd, None),
        CommandId::ThemeSystem => set_theme(hwnd, crate::config::ThemePreference::System),
        CommandId::ThemeLight => set_theme(hwnd, crate::config::ThemePreference::Light),
        CommandId::FileIconsMaterial => set_file_icons(hwnd, crate::config::FileIconSet::Material),
        CommandId::FileIconsMinimal => set_file_icons(hwnd, crate::config::FileIconSet::Minimal),
        CommandId::FileIconsSolid => set_file_icons(hwnd, crate::config::FileIconSet::Solid),
        CommandId::ThemeDark => set_theme(hwnd, crate::config::ThemePreference::Dark),
        CommandId::ThemeCatppuccin => set_theme(hwnd, crate::config::ThemePreference::Catppuccin),
        CommandId::ThemeCatppuccinLatte => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinLatte)
        }
        CommandId::ThemeCatppuccinFrappe => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinFrappe)
        }
        CommandId::ThemeCatppuccinMacchiato => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinMacchiato)
        }
        CommandId::ThemeCatppuccinMocha => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinMocha)
        }
        CommandId::ThemePaperLamp => set_theme(hwnd, crate::config::ThemePreference::PaperLamp),
        CommandId::ThemePaper => set_theme(hwnd, crate::config::ThemePreference::Paper),
        CommandId::ThemeLamp => set_theme(hwnd, crate::config::ThemePreference::Lamp),
        CommandId::ToggleWordWrap => change_setting(hwnd, |settings| {
            settings.word_wrap = !settings.word_wrap;
            Some(("word_wrap", settings.word_wrap.to_string()))
        }),
        CommandId::ToggleLineNumbers => change_setting(hwnd, |settings| {
            settings.line_numbers = !settings.line_numbers;
            Some(("line_numbers", settings.line_numbers.to_string()))
        }),
        CommandId::ToggleInsertSpaces => change_setting(hwnd, |settings| {
            settings.insert_spaces = !settings.insert_spaces;
            Some(("insert_spaces", settings.insert_spaces.to_string()))
        }),
        CommandId::ToggleShowWhitespace => change_setting(hwnd, |settings| {
            settings.show_whitespace = !settings.show_whitespace;
            Some(("show_whitespace", settings.show_whitespace.to_string()))
        }),
        CommandId::ToggleHighlightCurrentLine => change_setting(hwnd, |settings| {
            settings.highlight_current_line = !settings.highlight_current_line;
            Some((
                "highlight_current_line",
                settings.highlight_current_line.to_string(),
            ))
        }),
        CommandId::ToggleCodeFolding => change_setting(hwnd, |settings| {
            settings.code_folding = !settings.code_folding;
            Some(("code_folding", settings.code_folding.to_string()))
        }),
        CommandId::FoldAll => with_editor(hwnd, |editor| {
            let _ = editor.fold_all(true);
        }),
        CommandId::UnfoldAll => with_editor(hwnd, |editor| {
            let _ = editor.fold_all(false);
        }),
        CommandId::ToggleAlwaysOnTop => {
            change_setting(hwnd, |settings| {
                settings.always_on_top = !settings.always_on_top;
                Some(("always_on_top", settings.always_on_top.to_string()))
            });
            apply_always_on_top(hwnd);
        }
        CommandId::ToggleRestoreSession => {
            change_setting(hwnd, |settings| {
                settings.restore_session = !settings.restore_session;
                Some(("restore_session", settings.restore_session.to_string()))
            });
            let enabled = unsafe { app_ptr(hwnd) }
                .is_some_and(|app| unsafe { app.as_ref() }.settings.restore_session);
            push_notice(hwnd, crate::session::toggle_notice(enabled).to_owned());
        }
        CommandId::ToggleNotesMode => {
            change_setting(hwnd, |settings| {
                settings.notes_mode = !settings.notes_mode;
                Some(("notes_mode", settings.notes_mode.to_string()))
            });
            let enabled = unsafe { app_ptr(hwnd) }
                .is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode);
            crate::window::library_host::notes_mode_changed(hwnd, enabled);
            crate::window::side_panel::notes_mode_changed(hwnd, enabled);
            if enabled {
                // The library step ran before the sidebar existed; its view catches up here.
                crate::window::side_panel::refresh(hwnd);
                crate::window::library_host::show_labels(hwnd);
            } else {
                crate::window::library_host::clear_labels(hwnd);
            }
            push_notice(
                hwnd,
                crate::window::library_host::notes_mode_notice(enabled).to_owned(),
            );
        }
        CommandId::OpenFolder => crate::window::library_host::choose_and_open_folder(hwnd),
        CommandId::OpenRecentFolder => crate::window::library_host::open_recent_folder_picker(hwnd),
        CommandId::ToggleFolderAutosave => {
            crate::window::library_host::toggle_folder_autosave(hwnd);
        }
        CommandId::NoteReloadFromDisk => crate::window::library_host::reload_from_disk(hwnd),
        CommandId::NoteKeepMine => crate::window::library_host::keep_mine(hwnd),
        CommandId::NoteTogglePin => {
            if let Some(path) = note_target(hwnd, tree_note.as_deref()) {
                crate::window::library_host::toggle_pin(hwnd, &path);
            }
        }
        CommandId::NoteMoveToNotebook => {
            if let Some(path) = note_target(hwnd, tree_note.as_deref()) {
                crate::window::library_host::move_to_notebook(hwnd, &path);
            }
        }
        CommandId::NoteRevealInExplorer => {
            if let Some(path) = note_target(hwnd, tree_note.as_deref()) {
                crate::window::library_host::reveal(hwnd, &path);
            }
        }
        CommandId::NoteRename => {
            if crate::window::library_host::ready_library(hwnd) {
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::inline_name::rename_note_at(hwnd, path),
                    (None, Some(folder)) => crate::window::inline_name::rename(
                        hwnd,
                        &crate::library::tree::RowKind::Folder(folder.clone()),
                    ),
                    (None, None) => crate::window::inline_name::rename_active(hwnd),
                }
            }
        }
        CommandId::NoteDelete => {
            if crate::window::library_host::ready_library(hwnd) {
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::library_host::delete_file(hwnd, path),
                    (None, Some(folder)) => {
                        crate::window::library_host::delete_folder(hwnd, folder);
                    }
                    (None, None) => crate::window::library_host::delete_note(hwnd),
                }
            }
        }
        CommandId::CloseNotebook => crate::window::library_host::close_notebook(hwnd),
        CommandId::SplitRight => {
            split_active_group(hwnd, crate::window::split_tree::Direction::Right);
        }
        CommandId::MoveTabToNextGroup => move_active_view(hwnd, true),
        CommandId::MoveTabToPreviousGroup => move_active_view(hwnd, false),
        CommandId::SplitDown => {
            split_active_group(hwnd, crate::window::split_tree::Direction::Down);
        }
        CommandId::CloseGroup => {
            if let Some(group) =
                unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
            {
                close_group(hwnd, group);
            }
        }
        CommandId::ToggleNotebookFavorite => {
            crate::window::library_host::toggle_notebook_favorite(hwnd);
        }
        CommandId::FontSizeIncrease => {
            set_font_size(hwnd, |size| {
                size.saturating_add(1).min(MAX_FONT_SIZE.max(size))
            });
        }
        CommandId::FontSizeDecrease => {
            set_font_size(hwnd, |size| {
                size.saturating_sub(1).max(MIN_FONT_SIZE.min(size))
            });
        }
        CommandId::FontSizeReset => {
            set_font_size(hwnd, |_| crate::config::defaults::DEFAULT_FONT_SIZE);
        }
        CommandId::TabWidth2 => set_tab_width(hwnd, 2),
        CommandId::TabWidth4 => set_tab_width(hwnd, 4),
        CommandId::TabWidth8 => set_tab_width(hwnd, 8),
        CommandId::Replace => open_find_bar(hwnd, find_bar::FindBarMode::Replace),
        CommandId::LanguagePlainText
        | CommandId::LanguageJson
        | CommandId::LanguageMarkdown
        | CommandId::LanguageBash
        | CommandId::LanguageBatch
        | CommandId::LanguageC
        | CommandId::LanguageCSharp
        | CommandId::LanguageCpp
        | CommandId::LanguageCss
        | CommandId::LanguageEnv
        | CommandId::LanguageHtml
        | CommandId::LanguageIni
        | CommandId::LanguageJavaScript
        | CommandId::LanguagePowerShell
        | CommandId::LanguageProperties
        | CommandId::LanguagePython
        | CommandId::LanguageRust
        | CommandId::LanguageSql
        | CommandId::LanguageSvg
        | CommandId::LanguageToml
        | CommandId::LanguageTypeScript
        | CommandId::LanguageXml
        | CommandId::LanguageYaml => {
            if let Some(language) = command.language() {
                apply_language(hwnd, language);
            }
        }
        CommandId::ValidateJson => validate_active_json(hwnd),
        CommandId::FormatJson => format_active_json(hwnd),
        CommandId::NextTab => cycle_tab(hwnd, true),
        CommandId::PreviousTab => cycle_tab(hwnd, false),
        CommandId::ZoomIn | CommandId::ZoomOut | CommandId::ZoomReset
            // An image tab never zooms the hidden editor, even with no image view to zoom.
            if crate::window::image_host::zoom(hwnd, command)
                || crate::window::image_host::active_is_image(hwnd)
                || crate::window::preview_host::zoom_svg(hwnd, command) => {}
        // One zoom for every group (split editors plan amendment 11).
        CommandId::ZoomIn => {
            for editor in all_editors(hwnd) {
                let _ = editor.zoom_in();
            }
        }
        CommandId::ZoomOut => {
            for editor in all_editors(hwnd) {
                let _ = editor.zoom_out();
            }
        }
        CommandId::ZoomReset => {
            for editor in all_editors(hwnd) {
                let _ = editor.reset_zoom();
            }
        }
        CommandId::MarkdownPreviewCycle
        | CommandId::MarkdownPreviewSide
        | CommandId::MarkdownPreviewFull
        | CommandId::MarkdownPreviewClose => {
            crate::window::preview_host::run_command(hwnd, command)
        }
        CommandId::MarkdownBold
        | CommandId::MarkdownItalic
        | CommandId::MarkdownCode
        | CommandId::MarkdownLink => crate::window::markdown_host::format(hwnd, command),
        CommandId::ToggleSidebar => crate::window::side_panel::toggle(hwnd),
        CommandId::FocusNextPane => cycle_focus(hwnd, false),
        CommandId::FocusPreviousPane => cycle_focus(hwnd, true),
        CommandId::ShowNotebookView => {
            crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, true)
        }
        CommandId::ShowSearchView => show_search_view(hwnd),
        CommandId::ReplaceInNotes => crate::window::search_view::show_replace(hwnd),
        CommandId::SearchToggleCase
        | CommandId::SearchToggleWholeWord
        | CommandId::SearchToggleRegex => {
            if let Some(option) = command.search_option() {
                toggle_search_option(hwnd, option);
            }
        }
        CommandId::ShowFavoritesView => {
            crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Favorites, true)
        }
        _ => {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.execute(command);
            }
        }
    }
}
