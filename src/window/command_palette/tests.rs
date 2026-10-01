use super::{
    ENTRIES, PanelLayout, Picker, PickerKind, PickerRow, filter_entries, hit_runs, match_rank,
    picker_row_label, picker_rows,
};
use crate::window::commands::CommandId;

fn shortcut_text(command: CommandId) -> Option<String> {
    crate::window::keymap::Keymap::defaults().first_text(command)
}

#[test]
fn palette_rows_show_the_windows_keys() {
    // Break caught: the palette hinting the default key after the user rebound it.
    use crate::window::keymap::{KeyStroke, Keymap};
    let keymap =
        Keymap::defaults().with_keys(CommandId::Save, vec![KeyStroke::parse("F9").unwrap()]);
    let entries = filter_entries("file: save", |_| true);
    assert_eq!(entries[0].command, CommandId::Save);
    let shown = entries
        .iter()
        .map(|entry| keymap.first_text(entry.command))
        .collect::<Vec<_>>();
    assert_eq!(shown[0].as_deref(), Some("F9"));
}

#[test]
fn the_editor_display_toggles_are_listed_once_under_editor() {
    // Break caught: a new setting reachable only from the Settings dialog.
    for (label, command) in [
        (
            "Editor: Toggle indent with spaces",
            CommandId::ToggleInsertSpaces,
        ),
        (
            "Editor: Toggle show whitespace",
            CommandId::ToggleShowWhitespace,
        ),
        (
            "Editor: Toggle highlight current line",
            CommandId::ToggleHighlightCurrentLine,
        ),
    ] {
        let labels = ENTRIES
            .iter()
            .filter(|entry| entry.command == command)
            .map(|entry| entry.label)
            .collect::<Vec<_>>();
        assert_eq!(labels, [label]);
    }
}

#[test]
fn hit_runs_cut_at_char_positions_not_bytes() {
    // Break caught (review focus 2): hits used as byte offsets, so "Über Straße" bolds "S"
    // and "t" one char late, or a run split inside a multi-byte char (a panic).
    assert_eq!(
        hit_runs("Über Straße", &[5, 6]),
        [("Über ", false), ("St", true), ("raße", false)]
    );
    assert_eq!(hit_runs("abc", &[0, 1, 2]), [("abc", true)]);
    assert_eq!(hit_runs("abc", &[]), [("abc", false)]);
    assert!(hit_runs("", &[]).is_empty());
}

#[test]
fn settings_is_listed_under_preferences_with_ctrl_comma() {
    // Break caught: the dialog reachable only by mouse, or its row showing no shortcut.
    assert_eq!(labels("open settings")[0], "Preferences: Open Settings");
    assert_eq!(
        shortcut_text(CommandId::OpenSettings).as_deref(),
        Some("Ctrl+,")
    );
    assert_eq!(labels("fastpad.ini")[0], "Preferences: Edit fastpad.ini");
    assert_eq!(shortcut_text(CommandId::EditSettingsFile), None);
}

fn labels(query: &str) -> Vec<&'static str> {
    filter_entries(query, |_| true)
        .into_iter()
        .map(|entry| entry.label)
        .collect()
}

#[test]
fn go_to_note_is_listed_once_with_ctrl_p() {
    // Break caught: Ctrl+P working but the palette never offering it, or its row showing no
    // shortcut (quick-open spec §3.1).
    assert_eq!(labels("go to note")[0], "Go to note\u{2026}");
    assert_eq!(
        shortcut_text(CommandId::QuickOpen).as_deref(),
        Some("Ctrl+P")
    );
    assert_eq!(ENTRIES.len(), 112);
}

#[test]
fn every_language_is_in_the_palette() {
    for row in crate::languages::LANGUAGES.iter() {
        let command = CommandId::for_language(row.language);
        let entry = ENTRIES
            .iter()
            .find(|entry| entry.command == command)
            .unwrap_or_else(|| panic!("{} missing from the palette", row.name));
        assert_eq!(entry.label, format!("Language: {}", row.name));
    }
}

#[test]
fn new_folder_is_listed_under_notebook_without_a_shortcut() {
    // Break caught: the palette never offering New folder, or showing a shortcut it doesn't
    // have (spec §6).
    assert_eq!(labels("new folder")[0], "Notebook: New folder\u{2026}");
    assert_eq!(shortcut_text(CommandId::NoteNewFolder), None);
}

#[test]
fn new_note_is_listed_right_before_new_folder_without_a_shortcut() {
    // Break caught: the palette never offering New note, listing it away from New folder,
    // or showing Ctrl+N (which opens an untitled tab) beside it (inline naming spec §8).
    assert_eq!(
        labels("notebook: new note")[0],
        "Notebook: New note\u{2026}"
    );
    assert_eq!(shortcut_text(CommandId::NoteNew), None);
    let position = |command| ENTRIES.iter().position(|entry| entry.command == command);
    assert_eq!(
        position(CommandId::NoteNew).map(|index| index + 1),
        position(CommandId::NoteNewFolder)
    );
}

#[test]
fn about_is_listed_under_help_without_a_shortcut() {
    // Break caught: the About box reachable only from the menu band, or its row showing a
    // shortcut it doesn't have.
    assert_eq!(labels("about")[0], "Help: About FastPad");
    assert_eq!(shortcut_text(CommandId::About), None);
}

#[test]
fn close_tab_is_listed_with_ctrl_w() {
    // Break caught: the palette's Close tab row still showing no shortcut after Ctrl+W.
    assert_eq!(labels("close tab")[0], "File: Close tab");
    assert_eq!(
        shortcut_text(CommandId::CloseTab).as_deref(),
        Some("Ctrl+W")
    );
}

#[test]
fn every_command_except_tab_positions_and_the_palette_is_listed_once() {
    // Break caught: a command added to the menus and shortcuts that the palette never offers.
    for value in 100..300u16 {
        let Ok(command) = CommandId::try_from(value) else {
            continue;
        };
        let listed = ENTRIES
            .iter()
            .filter(|entry| entry.command == command)
            .count();
        let expected = usize::from(
            command.tab_index().is_none()
                && command.group_index().is_none()
                && command != CommandId::CommandPalette
                && command != CommandId::MarkdownPreviewCycle
                && command != CommandId::FocusNextPane
                && command != CommandId::FocusPreviousPane,
        );
        assert_eq!(listed, expected, "{command:?}");
    }
}

#[test]
fn split_shortcuts_are_spelled_with_a_backslash() {
    // Break caught: Ctrl+\ shown as "Ctrl+Ü", VK_OEM_5's code read as a character.
    assert_eq!(
        shortcut_text(CommandId::SplitRight).as_deref(),
        Some("Ctrl+\\")
    );
    assert_eq!(
        shortcut_text(CommandId::SplitDown).as_deref(),
        Some("Ctrl+Shift+\\")
    );
}

#[test]
fn the_sidebar_commands_are_listed_with_their_shortcuts() {
    assert_eq!(labels("sidebar")[0], "View: Toggle sidebar");
    assert_eq!(labels("show search")[0], "View: Show search");
    assert_eq!(
        shortcut_text(CommandId::ToggleSidebar).as_deref(),
        Some("Ctrl+B")
    );
    assert_eq!(
        shortcut_text(CommandId::ShowNotebookView).as_deref(),
        Some("Ctrl+Shift+E")
    );
    // Break caught: the palette row still reading Ctrl+K after the shortcut moved.
    assert_eq!(
        shortcut_text(CommandId::ShowSearchView).as_deref(),
        Some("Ctrl+Shift+F")
    );
    assert_eq!(shortcut_text(CommandId::ShowFavoritesView), None);
}

#[test]
fn the_search_option_rows_are_listed_without_shortcuts() {
    // Break caught: a toggle the palette never offers, or one showing a shortcut it doesn't
    // have (the Alt keys work only inside the Search box and the find bar).
    assert_eq!(labels("toggle match case")[0], "Search: Toggle match case");
    assert_eq!(labels("whole word")[0], "Search: Toggle whole word");
    assert_eq!(
        labels("regular expression")[0],
        "Search: Toggle regular expression"
    );
    for command in [
        CommandId::SearchToggleCase,
        CommandId::SearchToggleWholeWord,
        CommandId::SearchToggleRegex,
    ] {
        assert_eq!(shortcut_text(command), None, "{command:?}");
    }
}

#[test]
fn replace_in_notes_is_listed_with_its_shortcut() {
    // Break caught: the palette never offering 3b's replace, or its row showing Ctrl+H, the
    // find bar's Replace.
    assert_eq!(labels("replace in notes")[0], "Search: Replace in notes");
    assert_eq!(
        shortcut_text(CommandId::ReplaceInNotes).as_deref(),
        Some("Ctrl+Shift+H")
    );
    assert_eq!(shortcut_text(CommandId::Replace).as_deref(), Some("Ctrl+H"));
}

#[test]
fn an_empty_query_lists_every_available_command_in_catalog_order() {
    assert_eq!(labels("").len(), ENTRIES.len());
    assert_eq!(labels("   ")[0], ENTRIES[0].label);
    let without_documents = filter_entries("", |command| !command.needs_document());
    assert!(
        without_documents
            .iter()
            .all(|entry| !entry.command.needs_document())
    );
    assert!(
        without_documents
            .iter()
            .any(|entry| entry.command == CommandId::Open)
    );
    assert!(
        !without_documents
            .iter()
            .any(|entry| entry.command == CommandId::Save)
    );
}

#[test]
fn matches_are_case_insensitive_and_ranked_prefix_word_substring_then_scattered() {
    assert_eq!(match_rank("FILE: s", "File: Save"), Some(0));
    assert_eq!(match_rank("zoom in", "View: Zoom in"), Some(1));
    assert_eq!(match_rank("oom", "View: Zoom in"), Some(2));
    assert_eq!(match_rank("vzi", "View: Zoom in"), Some(3));
    assert_eq!(match_rank("xyz", "View: Zoom in"), None);
    // Break caught: a scattered match outranking the command whose word the user typed.
    let found = labels("save");
    assert_eq!(found[..2], ["File: Save", "File: Save as..."]);
    assert_eq!(labels("json")[0], "JSON: Format document");
    assert_eq!(labels("zoom")[0], "View: Zoom in");
}

#[test]
fn the_palette_rows_follow_the_text_size() {
    let _factor = crate::window::design::text_scale::FactorGuard::new();
    // Break caught: palette rows that stay 26 px tall while the Windows text size grows.
    use crate::window::design::text_scale::{scale_text, set_factor_for_test};
    set_factor_for_test(225);
    let layout = PanelLayout::calculate(560, 96, 16, 3);
    let expected = 3 * scale_text(26, 96);
    set_factor_for_test(100);
    let list = layout.list.unwrap();
    assert_eq!(list.bottom - list.top, expected);
    assert_eq!(expected, 3 * 59);
}

#[test]
fn the_panel_centers_the_query_text_and_ends_with_the_list_on_its_bottom_border() {
    // Break caught: query text stuck to the top of its box, or a list that overhangs (or stops
    // short of) the panel's border.
    let layout = PanelLayout::calculate(560, 96, 16, 3);
    let field_middle = (layout.field.top + layout.field.bottom) / 2;
    let edit_middle = (layout.edit.top + layout.edit.bottom) / 2;
    assert!((field_middle - edit_middle).abs() <= 1);
    assert!(layout.edit.left > layout.field.left && layout.edit.right < layout.field.right);
    let list = layout.list.unwrap();
    assert_eq!(list.bottom - list.top, 3 * 26);
    assert_eq!((list.left, list.right), (1, 559));
    assert_eq!(layout.height, list.bottom + 1);

    let many = PanelLayout::calculate(560, 144, 24, 40);
    assert_eq!(many.list.map(|list| list.bottom - list.top), Some(12 * 39));

    let empty = PanelLayout::calculate(560, 96, 16, 0);
    assert!(empty.list.is_none());
    assert_eq!(empty.height, empty.field.bottom + 6);
}

#[test]
fn shortcuts_are_spelled_from_the_accelerator_table() {
    assert_eq!(shortcut_text(CommandId::Save).as_deref(), Some("Ctrl+S"));
    assert_eq!(
        shortcut_text(CommandId::SaveAs).as_deref(),
        Some("Ctrl+Shift+S")
    );
    assert_eq!(
        shortcut_text(CommandId::NextTab).as_deref(),
        Some("Ctrl+Tab")
    );
    assert_eq!(shortcut_text(CommandId::ZoomIn).as_deref(), Some("Ctrl+="));
    assert_eq!(shortcut_text(CommandId::ZoomOut).as_deref(), Some("Ctrl+-"));
    assert_eq!(
        shortcut_text(CommandId::CommandPalette).as_deref(),
        Some("Ctrl+Shift+P")
    );
    // Break caught: Format JSON's row still showing Ctrl+Shift+F, now Search's shortcut.
    assert_eq!(
        shortcut_text(CommandId::FormatJson).as_deref(),
        Some("Shift+Alt+F")
    );
    assert_eq!(shortcut_text(CommandId::FindNext).as_deref(), Some("F3"));
    assert_eq!(
        shortcut_text(CommandId::FindPrevious).as_deref(),
        Some("Shift+F3")
    );
    assert_eq!(shortcut_text(CommandId::Copy), None);
}

fn picker(create: Option<&'static str>) -> Picker {
    Picker {
        kind: PickerKind::RecentFolder,
        items: vec!["idea".into(), "reference".into(), "todo".into()],
        create,
    }
}

#[test]
fn picker_rows_filter_items_like_commands_and_offer_to_create_a_new_name() {
    // Break caught: typing a new name leaving nothing to press Enter on, or offering to
    // create a name that already exists under another case.
    let with_create = picker(Some("Create"));
    assert_eq!(
        picker_rows(&with_create, ""),
        vec![PickerRow::Item(0), PickerRow::Item(1), PickerRow::Item(2)]
    );
    assert_eq!(
        picker_rows(&with_create, "ref"),
        vec![PickerRow::Item(1), PickerRow::Create("ref".into())]
    );
    assert_eq!(picker_rows(&with_create, "TODO"), vec![PickerRow::Item(2)]);
    assert_eq!(picker_rows(&picker(None), "zzz"), vec![]);
    assert_eq!(
        picker_row_label(&with_create, &PickerRow::Create("urgent".into())),
        "Create \u{201c}urgent\u{201d}"
    );
    assert_eq!(picker_row_label(&with_create, &PickerRow::Item(0)), "idea");
}

#[test]
fn markdown_preview_cycles_with_ctrl_shift_v_and_lists_three_palette_entries() {
    assert_eq!(
        shortcut_text(CommandId::MarkdownPreviewCycle).as_deref(),
        Some("Ctrl+Shift+V")
    );
    let labels = filter_entries("markdown preview", |_| true)
        .into_iter()
        .map(|entry| entry.command)
        .collect::<Vec<_>>();
    for command in [
        CommandId::MarkdownPreviewSide,
        CommandId::MarkdownPreviewFull,
        CommandId::MarkdownPreviewClose,
    ] {
        assert!(labels.contains(&command), "{command:?}");
    }
    assert!(
        filter_entries("markdown preview", |command| !command.is_markdown_preview()).is_empty()
    );
}

mod painting {
    use super::super::{paint_field, paint_row_background};
    use crate::platform::theme::Theme;
    use crate::window::palette::Palette;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel, HDC,
        ReleaseDC, SelectObject,
    };

    /// Fills a 200x80 memory bitmap with `colors.strip_background`, runs `draw`, then `read`.
    fn with_canvas(
        colors: &Palette,
        draw: impl FnOnce(HDC),
        read: impl FnOnce(&dyn Fn(i32, i32) -> u32),
    ) {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 200, 80);
            let previous = SelectObject(dc, bitmap);
            let all = RECT {
                left: 0,
                top: 0,
                right: 200,
                bottom: 80,
            };
            crate::window::panel::fill(dc, all, colors.strip_background);
            draw(dc);
            read(&|x, y| GetPixel(dc, x, y));
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }

    const FIELD: RECT = RECT {
        left: 10,
        top: 10,
        right: 110,
        bottom: 40,
    };
    const ROW: RECT = RECT {
        left: 0,
        top: 0,
        right: 100,
        bottom: 26,
    };

    #[test]
    fn the_field_is_a_rounded_box_with_an_accent_border() {
        // Break caught: a square field, or a rounded one that lost its border.
        let colors = Palette::for_theme(Theme::ALL[0], false);
        with_canvas(
            &colors,
            |dc| paint_field(dc, FIELD, &colors, 96),
            |pixel| {
                assert_eq!(pixel(10, 10), colors.strip_background, "corner");
                assert_eq!(pixel(60, 10), colors.selection_background, "border");
                assert_eq!(pixel(60, 11), colors.editor_background, "inside");
            },
        );
    }

    #[test]
    fn the_selected_row_is_an_inset_rounded_fill() {
        // Break caught: a full-bleed selection, or a fill with square corners.
        let colors = Palette::for_theme(Theme::ALL[0], false);
        with_canvas(
            &colors,
            |dc| paint_row_background(dc, ROW, true, &colors, 96),
            |pixel| {
                assert_eq!(pixel(1, 13), colors.strip_background, "left inset");
                assert_eq!(pixel(7, 13), colors.hover_background, "fill edge");
                assert_eq!(pixel(50, 0), colors.strip_background, "top gap");
                assert_eq!(pixel(4, 1), colors.strip_background, "rounded corner");
            },
        );
    }

    #[test]
    fn an_unselected_row_stays_the_strip_color() {
        let colors = Palette::for_theme(Theme::ALL[0], false);
        with_canvas(
            &colors,
            |dc| paint_row_background(dc, ROW, false, &colors, 96),
            |pixel| {
                for (x, y) in [(0, 0), (4, 1), (50, 13), (99, 25)] {
                    assert_eq!(pixel(x, y), colors.strip_background, "({x}, {y})");
                }
            },
        );
    }

    #[test]
    fn high_contrast_keeps_a_square_field_and_a_full_row_selection() {
        // Break caught: rounding or insetting under a high-contrast palette.
        let colors = Palette::for_theme(Theme::ALL[0], true);
        with_canvas(
            &colors,
            |dc| {
                paint_field(dc, FIELD, &colors, 96);
                paint_row_background(dc, ROW, true, &colors, 96);
            },
            |pixel| {
                assert_eq!(pixel(10, 10), colors.selection_background, "field corner");
                assert_eq!(pixel(0, 0), colors.hover_background, "row corner");
                assert_eq!(pixel(5, 13), colors.hover_background, "no accent bar");
            },
        );
    }

    #[test]
    fn the_selected_row_has_an_accent_bar() {
        // Break caught: a selected row with no accent bar, or a bar over the wrong pixels.
        let colors = Palette::for_theme(Theme::ALL[0], false);
        with_canvas(
            &colors,
            |dc| paint_row_background(dc, ROW, true, &colors, 96),
            |pixel| {
                assert_eq!(pixel(5, 13), colors.accent, "bar");
                assert_eq!(pixel(7, 13), colors.hover_background, "past the bar");
            },
        );
    }
}
