//! The menu band, the find bar band, setting commands and toggles, tab scrolling and
//! modal-scope seams.

use super::*;

#[test]
fn alt_shows_a_painted_menu_band_that_pushes_the_editor_down_and_runs_dropdown_commands() {
    // Break caught: Alt attached a native menu bar, which Windows drew unthemed over the editor
    // (the reclaimed caption leaves it no room) and left a "File" remnant behind after Escape.
    use crate::window::menus::{DropdownExit, answer_next_dropdown};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenu, GetWindowRect, SC_KEYMENU};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    super::super::build_chrome(window.hwnd);
    let editor_top = || {
        let mut rect = RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            GetWindowRect(editor.hwnd(), &mut rect);
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
        }
        rect.top - origin.y
    };
    let key_menu = |letter: u8| unsafe {
        SendMessageW(
            window.hwnd,
            super::super::WM_SYSCOMMAND,
            SC_KEYMENU as usize,
            letter as isize,
        )
    };
    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    // The editor group's tab strip is the title row, so the editor starts right below it.
    let title_height = super::super::title_layout(window.hwnd).height;
    let band = super::super::menu_band::band_height(dpi);

    key_menu(0);
    assert_eq!(
        app_mut(window.hwnd).menu_mode,
        Some(super::super::MenuMode {
            hot: 0,
            open: false
        })
    );
    assert!(
        unsafe { GetMenu(window.hwnd) }.is_null(),
        "no native menu bar"
    );
    assert_eq!(editor_top(), title_height + band);
    assert_eq!(super::super::menu_headings(window.hwnd).len(), 5);

    key_menu(0);
    assert_eq!(app_mut(window.hwnd).menu_mode, None);
    assert_eq!(editor_top(), title_height);

    // Alt+E opens Edit; Right moves to Search, whose Escape leaves Search highlighted.
    answer_next_dropdown(|hwnd, heading| {
        assert_eq!(heading, 1);
        assert_eq!(
            app_mut(hwnd).menu_mode,
            Some(super::super::MenuMode { hot: 1, open: true })
        );
        DropdownExit::Switch(2)
    });
    answer_next_dropdown(|_, heading| {
        assert_eq!(heading, 2);
        DropdownExit::Escape
    });
    key_menu(b'e');
    assert_eq!(
        app_mut(window.hwnd).menu_mode,
        Some(super::super::MenuMode {
            hot: 2,
            open: false
        })
    );

    // Down opens the highlighted heading; a picked command leaves menu mode before it runs.
    answer_next_dropdown(|_, heading| {
        assert_eq!(heading, 2);
        DropdownExit::Command(CommandId::Find)
    });
    assert!(super::super::handle_menu_key(
        window.hwnd,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
        super::super::VK_DOWN as usize,
    ));
    assert_eq!(app_mut(window.hwnd).menu_mode, None);
    assert!(app_mut(window.hwnd).find_bar().unwrap().is_visible());
    assert_eq!(
        editor_top(),
        title_height + super::super::find_bar::find_bar_height(dpi)
    );
}

#[test]
fn the_find_bar_panel_reserves_its_band_above_the_editor_and_follows_theme_changes() {
    // Break caught: a find bar whose painted band is not reserved (the editor draws over it),
    // or that keeps the old colors after the theme changes while it is open.
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    super::super::build_chrome(window.hwnd);
    let top_of = |child| {
        let mut rect = RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            GetWindowRect(child, &mut rect);
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
        }
        (rect.top - origin.y, rect.bottom - rect.top)
    };
    let visible = |child| {
        (unsafe { GetWindowLongPtrW(child, super::super::GWL_STYLE) }) as u32
            & super::super::WS_VISIBLE
            != 0
    };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window.hwnd) }.max(96);
    // The editor group's tab strip is the title row, so the editor starts right below it.
    let title_height = super::super::title_layout(window.hwnd).height;

    execute_command(window.hwnd, CommandId::Find);
    let panel = app_mut(window.hwnd).find_bar().unwrap().panel_hwnd();
    assert!(visible(panel));
    let band = super::super::find_bar::find_bar_height(dpi);
    assert_eq!(top_of(panel), (title_height, band));
    assert_eq!(top_of(editor.hwnd()).0, title_height + band);

    let brush_before = {
        let bar = app_mut(window.hwnd).find_bar().unwrap();
        bar.control_color(std::ptr::null_mut())
    };
    execute_command(window.hwnd, CommandId::ThemeCatppuccinMocha);
    let bar = app_mut(window.hwnd).find_bar().unwrap();
    assert!(bar.is_visible());
    assert_ne!(bar.control_color(std::ptr::null_mut()), brush_before);

    assert_eq!(bar.placeholder(bar.query_hwnd()), Some("Find"));
    assert_eq!(bar.placeholder(bar.replace_hwnd()), Some("Replace"));

    // A click released on the close button at the bar's right end closes it.
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let point = |x: i32, y: i32| ((y as u32) << 16 | (x as u32 & 0xffff)) as super::super::LPARAM;
    let close_point = point(client.right - band / 2, band / 2);
    super::super::panel_pointer(
        window.hwnd,
        panel,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
        0,
        point(client.right / 2, band / 2),
    );
    assert!(
        visible(panel),
        "a click on the field area must not close the bar"
    );
    super::super::panel_pointer(
        window.hwnd,
        panel,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
        0,
        close_point,
    );
    assert!(!visible(panel));
    assert_eq!(top_of(editor.hwnd()).0, title_height);
}

#[test]
fn with_no_tab_open_the_command_palette_lists_only_commands_that_need_no_document() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    install_test_editor(&window);
    execute_command(window.hwnd, CommandId::CloseAllTabs);
    assert!(app_mut(window.hwnd).tabs.is_empty());

    execute_command(window.hwnd, CommandId::CommandPalette);
    let shown = app_mut(window.hwnd)
        .command_palette
        .as_ref()
        .unwrap()
        .shown()
        .iter()
        .map(|entry| entry.command)
        .collect::<Vec<_>>();
    assert!(shown.iter().all(|command| !command.needs_document()));
    assert!(shown.contains(&CommandId::Open));
    assert!(shown.contains(&CommandId::ThemeDark));
    assert!(!shown.contains(&CommandId::Save));
}

#[test]
fn file_icon_commands_save_the_set_and_repaint_the_tree_without_restyling_the_editor() {
    // Break caught: a set that is lost on restart, a tree left showing the old set until
    // something else repaints it, fastpad.ini rewritten beyond its own line (icon sets spec
    // §4), or a file-icon command that leaks into the editor's own styling.
    use crate::editor::scintilla_constants::STYLE_DEFAULT;
    use windows_sys::Win32::Graphics::Gdi::{GetUpdateRect, ValidateRect};
    const SCI_STYLEGETSIZEFRACTIONAL: u32 = 2062;
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("file-icons");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let panel = sidebar_panel(window.hwnd);
    // An update region is only tracked for windows under a visible ancestor chain; without
    // this, GetUpdateRect below would read 0 no matter what InvalidateRect did. SW_SHOWNA
    // shows the window without activating it, so it doesn't steal focus from the test run.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
            window.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNA,
        )
    };
    let editor_style = || unsafe {
        SendMessageW(
            editor.hwnd(),
            SCI_STYLEGETSIZEFRACTIONAL,
            STYLE_DEFAULT as usize,
            0,
        )
    };
    let style_before = editor_style();

    // Material -> Minimal: a real change, so the panel must repaint.
    unsafe { ValidateRect(panel, std::ptr::null()) };
    execute_command(window.hwnd, CommandId::FileIconsMinimal);
    assert_ne!(
        unsafe { GetUpdateRect(panel, std::ptr::null_mut(), 0) },
        0,
        "switching to Minimal must invalidate the tree panel"
    );
    assert_eq!(
        app_mut(window.hwnd).settings.file_icons,
        crate::config::FileIconSet::Minimal
    );

    // Minimal -> Minimal: no-op for the setting, but set_file_icons still invalidates
    // unconditionally (main_window.rs set_file_icons), so assert what the code does.
    unsafe { ValidateRect(panel, std::ptr::null()) };
    execute_command(window.hwnd, CommandId::FileIconsMinimal);
    assert_ne!(
        unsafe { GetUpdateRect(panel, std::ptr::null_mut(), 0) },
        0,
        "the repeat command still repaints (set_file_icons invalidates unconditionally)"
    );

    // Minimal -> Solid: a real change, so the panel must repaint.
    unsafe { ValidateRect(panel, std::ptr::null()) };
    execute_command(window.hwnd, CommandId::FileIconsSolid);
    assert_ne!(
        unsafe { GetUpdateRect(panel, std::ptr::null_mut(), 0) },
        0,
        "switching to Solid must invalidate the tree panel"
    );
    assert_eq!(
        app_mut(window.hwnd).settings.file_icons,
        crate::config::FileIconSet::Solid
    );

    // Solid -> Material: a real change, so the panel must repaint again.
    unsafe { ValidateRect(panel, std::ptr::null()) };
    execute_command(window.hwnd, CommandId::FileIconsMaterial);
    assert_ne!(
        unsafe { GetUpdateRect(panel, std::ptr::null_mut(), 0) },
        0,
        "switching to Material must invalidate the tree panel"
    );

    assert_eq!(
        editor_style(),
        style_before,
        "file-icon commands must not restyle the editor"
    );

    super::super::save_settings_to(None);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nfile_icons=material\r\n"
    );
    assert!(!CommandId::FileIconsSolid.needs_document());
    assert!(!CommandId::FileIconsMaterial.needs_document());
}

#[test]
fn the_editor_display_toggles_apply_to_the_editor_and_a_theme_change_keeps_them() {
    // Break caught: a toggle that saves but leaves the editor unchanged, a caret line that a
    // theme change turns back on, or a toggle that rewrites the rest of fastpad.ini
    // (settings dialog spec §4.4).
    use crate::editor::scintilla_constants::{
        SC_ELEMENT_CARET_LINE_BACK, SCI_GETELEMENTISSET, SCI_GETUSETABS, SCI_GETVIEWWS,
        SCWS_INVISIBLE, SCWS_VISIBLEALWAYS,
    };
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("display-toggles");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    super::super::build_chrome(window.hwnd);
    let send = |message, wparam| unsafe { SendMessageW(editor.hwnd(), message, wparam, 0) };
    let caret_line_set = || send(SCI_GETELEMENTISSET, SC_ELEMENT_CARET_LINE_BACK as usize);
    assert_eq!(send(SCI_GETUSETABS, 0), 1, "tab characters by default");
    assert_eq!(send(SCI_GETVIEWWS, 0), SCWS_INVISIBLE as isize);
    assert_eq!(
        caret_line_set(),
        1,
        "the current line is highlighted by default"
    );

    execute_command(window.hwnd, CommandId::ToggleInsertSpaces);
    execute_command(window.hwnd, CommandId::ToggleShowWhitespace);
    execute_command(window.hwnd, CommandId::ToggleHighlightCurrentLine);
    assert_eq!(send(SCI_GETUSETABS, 0), 0);
    assert_eq!(send(SCI_GETVIEWWS, 0), SCWS_VISIBLEALWAYS as isize);
    assert_eq!(caret_line_set(), 0);

    execute_command(window.hwnd, CommandId::ThemeDark);
    assert_eq!(caret_line_set(), 0, "a theme change keeps it off");
    execute_command(window.hwnd, CommandId::ToggleHighlightCurrentLine);
    assert_eq!(caret_line_set(), 1);
    super::super::save_settings_to(None);

    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\ninsert_spaces=true\r\nshow_whitespace=true\r\n\
         highlight_current_line=true\r\ntheme=dark\r\n"
    );
}

#[test]
fn always_on_top_pins_the_window_and_saves_only_its_own_line() {
    // Break caught: a toggle that saves but never changes the window's z-order, one that
    // leaves the window topmost after switching off, or one that rewrites the rest of
    // fastpad.ini.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, SW_SHOWNA, ShowWindow, WS_EX_TOPMOST,
    };
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("always-on-top");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(
        &ini, "# kept
",
    )
    .unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    super::super::build_chrome(window.hwnd);
    // Windows keeps a hidden window's z-order as it was, so the window must be showing.
    unsafe { ShowWindow(window.hwnd, SW_SHOWNA) };
    let topmost =
        || unsafe { GetWindowLongPtrW(window.hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST != 0 };
    assert!(!topmost(), "off by default");

    execute_command(window.hwnd, CommandId::ToggleAlwaysOnTop);
    assert!(topmost());
    execute_command(window.hwnd, CommandId::ToggleAlwaysOnTop);
    assert!(!topmost());
    execute_command(window.hwnd, CommandId::ToggleAlwaysOnTop);
    super::super::save_settings_to(None);

    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept
always_on_top=true
"
    );
}

#[test]
fn settings_actions_apply_and_save_only_their_own_lines() {
    // Break caught: a dialog change that updates the window but is lost on restart, one that
    // rewrites the user's fastpad.ini, or a re-pick of the current value that writes anyway
    // (settings dialog spec §4.2).
    use crate::config::{FileIconSet, ThemePreference};
    use crate::window::settings_model::{SettingsAction, Toggle};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings-actions");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    super::super::build_chrome(window.hwnd);

    for action in [
        SettingsAction::SetTheme(ThemePreference::CatppuccinMocha),
        SettingsAction::SetFileIcons(FileIconSet::Solid),
        SettingsAction::SetFontFace("Cascadia Mono".to_owned()),
        SettingsAction::SetFontSize(14),
        SettingsAction::SetTabWidth(2),
        SettingsAction::Toggle(Toggle::WordWrap),
        // Picking what is already set writes nothing.
        SettingsAction::SetFontSize(14),
        SettingsAction::SetFontFace("Cascadia Mono".to_owned()),
    ] {
        super::super::apply_settings_action(window.hwnd, action);
    }

    let settings = app_mut(window.hwnd).settings.clone();
    assert_eq!(settings.theme, ThemePreference::CatppuccinMocha);
    assert_eq!(settings.file_icons, FileIconSet::Solid);
    assert_eq!(settings.font_face, "Cascadia Mono");
    assert_eq!(settings.font_size, 14);
    assert_eq!(settings.tab_width, 2);
    assert!(settings.word_wrap);
    let view = super::super::settings_view(window.hwnd);
    assert_eq!(view.settings, settings);
    assert_eq!(view.notebook_autosave, None, "no notebook is open");
    super::super::save_settings_to(None);

    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\ntheme=catppuccin-mocha\r\nfile_icons=solid\r\nfont_face=Cascadia Mono\r\n\
         font_size=14\r\ntab_width=2\r\nword_wrap=true\r\n"
    );
}

#[test]
fn setting_commands_apply_to_the_editor_and_save_only_their_own_ini_lines() {
    // Break caught: a palette setting that changes the editor but is lost on restart, or that
    // rewrites fastpad.ini and drops what the user wrote there by hand.
    use crate::editor::scintilla_constants::{SCI_GETTABWIDTH, SCI_STYLEGETBACK, STYLE_DEFAULT};
    const SCI_GETWRAPMODE: u32 = 2269;
    const SCI_STYLEGETSIZEFRACTIONAL: u32 = 2062;
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\nfont_face=Cascadia Mono\r\ntab_width=4\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let send = |message, wparam| unsafe { SendMessageW(editor.hwnd(), message, wparam, 0) };

    execute_command(window.hwnd, CommandId::ToggleWordWrap);
    assert_ne!(send(SCI_GETWRAPMODE, 0), 0);
    execute_command(window.hwnd, CommandId::TabWidth8);
    assert_eq!(send(SCI_GETTABWIDTH, 0), 8);
    execute_command(window.hwnd, CommandId::FontSizeIncrease);
    execute_command(window.hwnd, CommandId::FontSizeIncrease);
    assert_eq!(
        send(SCI_STYLEGETSIZEFRACTIONAL, STYLE_DEFAULT as usize),
        1300
    );
    execute_command(window.hwnd, CommandId::FontSizeReset);
    assert_eq!(
        send(SCI_STYLEGETSIZEFRACTIONAL, STYLE_DEFAULT as usize),
        1100
    );
    execute_command(window.hwnd, CommandId::ToggleLineNumbers);
    assert!(!app_mut(window.hwnd).settings.line_numbers);

    super::super::build_chrome(window.hwnd);
    execute_command(window.hwnd, CommandId::ThemeDark);
    assert_eq!(
        send(SCI_STYLEGETBACK, STYLE_DEFAULT as usize) as u32,
        crate::window::palette::Palette::for_theme(crate::platform::theme::Theme::Dark, false,)
            .editor_background
    );
    execute_command(window.hwnd, CommandId::ThemeCatppuccinMocha);
    assert_eq!(
        send(SCI_STYLEGETBACK, STYLE_DEFAULT as usize) as u32,
        crate::window::palette::Palette::for_theme(
            crate::platform::theme::Theme::CatppuccinMocha,
            false,
        )
        .editor_background
    );
    execute_command(window.hwnd, CommandId::ThemeLight);
    assert_eq!(
        send(SCI_STYLEGETBACK, STYLE_DEFAULT as usize) as u32,
        crate::window::palette::Palette::for_theme(crate::platform::theme::Theme::Light, false,)
            .editor_background
    );
    super::super::save_settings_to(None);

    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nfont_face=Cascadia Mono\r\ntab_width=8\r\nword_wrap=true\r\n\
         font_size=11\r\nline_numbers=false\r\ntheme=light\r\n"
    );
    let (reloaded, warnings) = {
        let mut settings = crate::config::default_settings();
        let delta = crate::config::parse(&std::fs::read_to_string(&ini).unwrap());
        settings.apply_delta(&delta);
        (settings, delta.warnings)
    };
    assert!(warnings.is_empty());
    // The window never loaded this file, so only the hand-written font differs.
    assert_eq!(reloaded.font_face, "Cascadia Mono");
    assert_eq!(
        crate::config::Settings {
            font_face: app_mut(window.hwnd).settings.font_face.clone(),
            ..reloaded
        },
        app_mut(window.hwnd).settings
    );
}

#[test]
fn session_toggle_saves_only_its_line_and_says_so() {
    // Break caught: a toggle that flips the flag but is lost on restart, rewrites the user's
    // fastpad.ini, or leaves no sign of which state it chose (menus show no checkmarks).
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("session-toggle");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    assert!(app_mut(window.hwnd).settings.restore_session);

    execute_command(window.hwnd, CommandId::ToggleRestoreSession);

    assert!(!app_mut(window.hwnd).settings.restore_session);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nrestore_session=false\r\n"
    );
    assert!(
        app_mut(window.hwnd)
            .notifications
            .pending()
            .iter()
            .any(|notice| notice.message == crate::session::toggle_notice(false))
    );
    super::super::save_settings_to(None);
}

#[test]
fn notes_mode_toggle_saves_only_its_line_and_says_so() {
    // Break caught: a toggle lost on restart, or one that rewrites the rest of fastpad.ini.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("notes-toggle");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::ToggleNotesMode);
    assert!(!app_mut(window.hwnd).settings.notes_mode);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nnotes_mode=false\r\n"
    );
    assert!(
        notices(window.hwnd)
            .contains(&crate::window::library_host::notes_mode_notice(false).to_owned())
    );
    super::super::save_settings_to(None);
}

#[test]
fn an_untitled_tab_is_labelled_by_its_first_line_as_you_type() {
    // Break caught: every untitled tab reading "Untitled", or the label recomputed on every
    // keystroke far below the first line.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    super::super::create_new_document(window.hwnd).unwrap();
    editor.set_text("\n## Meeting notes\nbody").unwrap();
    pump_posted_messages(window.hwnd);
    let title = || app_mut(window.hwnd).tabs.active().unwrap().title();
    assert_eq!(title(), "Meeting notes *");
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().label_watch, 1);

    app_mut(window.hwnd).settings.notes_mode = false;
    crate::window::library_host::clear_labels(window.hwnd);
    assert_eq!(title(), "Untitled *");
}

#[test]
fn dragging_the_tab_scroll_thumb_scrolls_the_tabs_without_activating_one() {
    // Break caught: a scroll bar that is only painted, so overflowing tabs cannot be reached
    // without a mouse wheel, or a drag release that also clicks the tab under the pointer.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    for _ in 0..40 {
        execute_command(window.hwnd, CommandId::New);
    }
    super::super::activate_tab(window.hwnd, 0);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let scroll = || app_mut(window.hwnd).tabs.scroll_offset();
    let layout = super::super::strip_layout(window.hwnd).unwrap();
    assert_eq!(layout.scroll, 0);
    let thumb = layout.scroll_thumb().expect("40 tabs overflow the strip");
    let pack = |x: i32, y: i32| (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
    let y = thumb.center().y;
    let far_right = layout.tabs.right + 500;

    unsafe {
        SendMessageW(group, WM_LBUTTONDOWN, 1, pack(thumb.center().x, y));
        SendMessageW(group, WM_MOUSEMOVE, 1, pack(far_right, y));
    }
    assert_eq!(scroll(), layout.max_scroll);
    unsafe {
        SendMessageW(group, WM_LBUTTONUP, 0, pack(far_right, y));
        SendMessageW(group, WM_MOUSEMOVE, 0, pack(layout.tabs.left, y));
    }
    assert_eq!(
        scroll(),
        layout.max_scroll,
        "moving after the release must not keep dragging"
    );
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 0);

    // Pressing the track away from the thumb jumps there.
    let track = super::super::strip_layout(window.hwnd)
        .unwrap()
        .scroll_bar
        .unwrap();
    unsafe {
        SendMessageW(group, WM_LBUTTONDOWN, 1, pack(track.left, y));
        SendMessageW(group, WM_LBUTTONUP, 0, pack(track.left, y));
    }
    assert_eq!(scroll(), 0);
}

#[test]
fn queued_ipc_requests_wait_for_the_modal_prompt_to_close() {
    // Break caught: a forwarded launch is dispatched inside a close prompt (opening tabs the
    // review never saw) or dropped entirely instead of staying queued.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("dirty").unwrap();
    app_mut(window.hwnd)
        .ipc_requests
        .push(crate::ipc::IpcRequest::New);
    let before = app_mut(window.hwnd).tabs.len();

    answer_next_close_prompt(|hwnd| {
        unsafe {
            SendMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
        }
        CloseDecision::Cancel
    });
    execute_command(window.hwnd, CommandId::CloseTab);

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        before,
        "a forwarded request was handled inside the modal loop"
    );
    assert_eq!(
        app_mut(window.hwnd).ipc_requests.len(),
        1,
        "the request must stay queued while a modal loop runs"
    );

    pump_posted_messages(window.hwnd);

    assert_eq!(app_mut(window.hwnd).tabs.len(), before + 1);
    assert!(app_mut(window.hwnd).ipc_requests.is_empty());
}

#[test]
fn recovery_snapshot_ticks_are_skipped_inside_a_modal_prompt() {
    // Break caught: the recovery WM_TIMER fires inside a modal loop and swaps documents in and
    // out of the view under the operation the modal dialog is about to complete.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("modal-snapshot");
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    editor.set_text("typed").unwrap();
    let snapshot = snapshot_path(
        root.path(),
        app_mut(window.hwnd).tabs.active().unwrap().recovery_id,
    );

    answer_next_close_prompt(|hwnd| {
        super::super::snapshot_next_document(hwnd);
        CloseDecision::Cancel
    });
    execute_command(window.hwnd, CommandId::CloseTab);

    assert!(
        !snapshot.exists(),
        "a snapshot tick ran inside the modal loop"
    );

    super::super::snapshot_next_document(window.hwnd);

    assert!(snapshot.exists(), "snapshots must resume after the modal");
}

#[test]
fn save_as_writes_the_document_chosen_before_the_dialog_opened() {
    // Break caught: the Save As dialog's modal loop activates another tab (a recovered one, a
    // forwarded open), and complete_save then renames and overwrites whatever is active now.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("modal-save-as");
    let target = root.path().join("chosen.txt");
    editor.set_text("alpha").unwrap();
    let chosen = app_mut(window.hwnd).tabs.active().unwrap().id;
    let destination = target.clone();
    answer_next_save_dialog(move |hwnd| {
        super::super::create_new_document(hwnd).unwrap();
        Some(destination)
    });

    assert!(super::super::save_active_document_as(window.hwnd));

    assert_eq!(std::fs::read(&target).unwrap(), b"alpha");
    let app = app_mut(window.hwnd);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.tabs.active().unwrap().id, chosen);
    assert_eq!(
        app.tabs.document(chosen).unwrap().path.as_deref(),
        Some(target.as_path())
    );
}

#[test]
fn folder_and_confirm_seams_answer_inside_their_modal_scope() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    crate::window::answer_next_folder_dialog(|hwnd| {
        assert!(crate::window::modal::modal_active(hwnd));
        Some(std::path::PathBuf::from(r"D:\Notes"))
    });
    assert_eq!(
        crate::window::modal::choose_folder(window.hwnd).unwrap(),
        Some(std::path::PathBuf::from(r"D:\Notes"))
    );
    crate::window::answer_next_confirm(|hwnd| {
        assert!(crate::window::modal::modal_active(hwnd));
        false
    });
    assert!(!crate::window::modal::confirm(
        window.hwnd,
        "Delete?",
        "Delete"
    ));
}
