//! Applying and saving settings: font size, tab width, theme and icon set, key bindings, the
//! Settings dialog's actions, loading settings at startup and applying the editor settings.

use super::*;

pub(super) fn set_font_size(hwnd: HWND, next: impl FnOnce(u16) -> u16) {
    change_setting(hwnd, |settings| {
        let size = next(settings.font_size);
        (size != settings.font_size).then(|| {
            settings.font_size = size;
            ("font_size", size.to_string())
        })
    });
}

pub(super) fn set_tab_width(hwnd: HWND, width: u8) {
    change_setting(hwnd, |settings| {
        (settings.tab_width != width).then(|| {
            settings.tab_width = width;
            ("tab_width", width.to_string())
        })
    });
}

pub(super) fn set_theme(hwnd: HWND, theme: crate::config::ThemePreference) {
    change_setting(hwnd, |settings| {
        (settings.theme != theme).then(|| {
            settings.theme = theme;
            ("theme", theme.ini_value().to_owned())
        })
    });
}

/// Switches the Notebook tree's icon set and repaints the tree (icon sets spec §4). No rescan.
pub(super) fn set_file_icons(hwnd: HWND, set: crate::config::FileIconSet) {
    change_setting(hwnd, |settings| {
        (settings.file_icons != set).then(|| {
            settings.file_icons = set;
            ("file_icons", set.token().to_owned())
        })
    });
    if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
}

/// Makes one Settings dialog change through the same code the palette commands use, so it
/// applies at once and saves its one `fastpad.ini` line (settings dialog spec §4.2).
pub(crate) fn apply_settings_action(
    hwnd: HWND,
    action: crate::window::settings_model::SettingsAction,
) {
    use crate::window::settings_model::SettingsAction;
    match action {
        SettingsAction::SetTheme(theme) => set_theme(hwnd, theme),
        SettingsAction::SetFileIcons(set) => set_file_icons(hwnd, set),
        SettingsAction::SetFontFace(face) => change_setting(hwnd, |settings| {
            (settings.font_face != face).then(|| {
                settings.font_face.clone_from(&face);
                ("font_face", face)
            })
        }),
        SettingsAction::SetFontSize(size) => set_font_size(hwnd, |_| size),
        SettingsAction::SetTabWidth(width) => set_tab_width(hwnd, width),
        SettingsAction::Toggle(toggle) => execute_command(hwnd, toggle.command()),
    }
}

/// What the Settings dialog shows. Call it with nothing of the App borrowed.
pub(crate) fn settings_view(hwnd: HWND) -> crate::window::settings_model::SettingsView {
    let settings = unsafe { app_ptr(hwnd) }.map_or_else(crate::config::default_settings, |app| {
        unsafe { app.as_ref() }.settings.clone()
    });
    crate::window::settings_model::SettingsView {
        settings,
        notebook_autosave: crate::window::library_host::notebook_autosave(hwnd),
    }
}

/// Whether the Notebook view's Open Editors section is expanded (open editors spec §3.3).
pub(crate) fn open_editors_expanded(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }
        .is_none_or(|app| unsafe { app.as_ref() }.settings.open_editors_expanded)
}

/// Collapses or expands the Open Editors section and saves it. Only the panel repaints.
pub(crate) fn set_open_editors_expanded(hwnd: HWND, expanded: bool) {
    change_setting(hwnd, |settings| {
        (settings.open_editors_expanded != expanded).then(|| {
            settings.open_editors_expanded = expanded;
            ("open_editors_expanded", expanded.to_string())
        })
    });
    if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
}

/// The shortcuts in force for `hwnd`'s window (the defaults before it has an App).
pub(crate) fn keymap(hwnd: HWND) -> crate::window::keymap::Keymap {
    unsafe { app_ptr(hwnd) }.map_or_else(crate::window::keymap::Keymap::defaults, |app| {
        unsafe { app.as_ref() }.keymap.clone()
    })
}

/// The text menus show for `command`'s key in `hwnd`'s keymap, without copying the keymap.
pub(crate) fn first_key_text(hwnd: HWND, command: CommandId) -> Option<String> {
    match unsafe { app_ptr(hwnd) } {
        Some(app) => unsafe { app.as_ref() }.keymap.first_text(command),
        None => crate::window::keymap::Keymap::defaults().first_text(command),
    }
}

/// The commands with a `key.<id>=` line in `hwnd`'s settings, including lines the keymap ignored.
pub(crate) fn key_line_commands(hwnd: HWND) -> Vec<CommandId> {
    unsafe { app_ptr(hwnd) }.map_or_else(Vec::new, |app| {
        unsafe { app.as_ref() }
            .settings
            .key_overrides
            .keys()
            .filter_map(|id| crate::window::keymap::command_for_id(id))
            .collect()
    })
}

/// Puts `keymap` in force: the accelerator table is rebuilt now; the menu bar, which spells the
/// keys, is rebuilt the next time it opens.
fn install_keymap(app: &mut App, keymap: crate::window::keymap::Keymap) {
    app.accelerators = crate::window::menus::AcceleratorTable::create(&keymap).ok();
    if app.menu_mode.is_none() {
        app.menu_bar = None;
    }
    app.keymap = keymap;
}

/// Gives `command` exactly `keys` (keyboard shortcuts spec 6.6): applies at once and saves its
/// `key.<id>=` line, or removes the line when `keys` are the defaults.
pub(crate) fn set_command_keys(
    hwnd: HWND,
    command: CommandId,
    keys: Vec<crate::window::keymap::KeyStroke>,
) {
    use crate::window::keymap::{ini_key, ini_value};
    let Some(key) = ini_key(command) else {
        return;
    };
    let id = key["key.".len()..].to_owned();
    // The App borrow ends before saving: a failed save pushes a notice, which borrows it again.
    let Some(saved) = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let keymap = app.keymap.with_keys(command, keys);
        // A line the keymap ignored (`key.file.save=Bogus`) is still in the settings and the
        // file; it is stale, and resetting the command removes it.
        let stale = !keymap.is_user(command) && app.settings.key_overrides.contains_key(&id);
        if keymap == app.keymap && !stale {
            return None;
        }
        let value = keymap
            .is_user(command)
            .then(|| ini_value(&keymap.keys_of(command)));
        match &value {
            Some(value) => app.settings.key_overrides.insert(id, value.clone()),
            None => app.settings.key_overrides.remove(&id),
        };
        install_keymap(app, keymap);
        Some(value)
    }) else {
        return;
    };
    let result = match saved {
        Some(value) => save_setting(&key, &value),
        None => remove_setting(&key),
    };
    if let Err(error) = result {
        push_notice(hwnd, format!("FastPad could not save fastpad.ini: {error}"));
    }
}

/// Gives `command` its default keys back and removes its `key.<id>=` line.
pub(crate) fn reset_command_keys(hwnd: HWND, command: CommandId) {
    set_command_keys(hwnd, command, crate::window::keymap::default_keys(command));
}

/// Applies one settings change from a command and saves it to `fastpad.ini`. `change` edits the
/// in-memory settings and names the `key=value` it made, or returns `None` when nothing changed.
pub(crate) fn change_setting(
    hwnd: HWND,
    change: impl FnOnce(&mut crate::config::Settings) -> Option<(&'static str, String)>,
) {
    let Some((previous_theme, (key, value))) = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let settings = &mut unsafe { app.as_mut() }.settings;
        let previous_theme = settings.theme;
        Some((previous_theme, change(settings)?))
    }) else {
        return;
    };
    let theme_changed = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.settings.theme != previous_theme);
    // The sidebar's view, width and icon set change only the sidebar, which their callers redo,
    // and the Settings dialog's size only that dialog; the editor and the Markdown preview are
    // not restyled for them.
    let sidebar_only = matches!(
        key,
        "sidebar_view"
            | "sidebar_width"
            | "file_icons"
            | "open_editors_expanded"
            | "settings_size"
            | "always_on_top"
    );
    if theme_changed {
        apply_theme(hwnd);
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    } else if !sidebar_only {
        apply_editor_settings(hwnd);
    }
    if let Err(error) = save_setting(key, &value) {
        push_notice(hwnd, format!("FastPad could not save fastpad.ini: {error}"));
    }
}

#[cfg(not(test))]
fn remove_setting(key: &str) -> Result<()> {
    crate::config::remove_setting(key)
}

/// Like `save_setting` in tests: only a path a test chose with `save_settings_to` is touched.
#[cfg(test)]
fn remove_setting(key: &str) -> Result<()> {
    TEST_SETTINGS_PATH.with(|path| match path.borrow().as_deref() {
        Some(path) => crate::config::remove_setting_to(path, key),
        None => Ok(()),
    })
}

#[cfg(not(test))]
fn save_setting(key: &str, value: &str) -> Result<()> {
    crate::config::save_setting(key, value)
}

/// Tests never touch the real `%LocalAppData%\FastPadastpad.ini`: a setting is saved only to a
/// path a test chose with `save_settings_to`.
#[cfg(test)]
fn save_setting(key: &str, value: &str) -> Result<()> {
    TEST_SETTINGS_PATH.with(|path| match path.borrow().as_deref() {
        Some(path) => crate::config::save_setting_to(path, key, value),
        None => Ok(()),
    })
}

#[cfg(test)]
thread_local! {
    pub(super) static TEST_SETTINGS_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "not every source-linked test target saves settings"
)]
pub(crate) fn save_settings_to(path: Option<std::path::PathBuf>) {
    TEST_SETTINGS_PATH.with(|slot| *slot.borrow_mut() = path);
}

pub(in crate::window) fn file_population_active(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.populating_file)
}

/// Runs only inside `WM_FASTPAD_LOAD_SETTINGS`: applies the settings `bootstrap::run` read before
/// the window existed (or resolves and parses `fastpad.ini` now when nothing was preloaded),
/// applies the editor view settings in place, and queues every rejected line as a non-modal
/// notification.
pub(super) fn load_settings(hwnd: HWND) {
    let preloaded = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let warnings = app.preloaded_settings_warnings.take()?;
        Some((app.settings.clone(), warnings))
    });
    let (settings, warnings) = preloaded.unwrap_or_else(crate::config::load);
    apply_loaded_settings(hwnd, settings, warnings);
    start_recovery_timer(hwnd);
}

/// Applies an already-loaded settings/warnings pair, split out of `load_settings` so tests can drive
/// the reporting path directly instead of mutating the process-wide `LOCALAPPDATA` environment
/// variable to fake a corrupt `fastpad.ini` on disk.
pub(super) fn apply_loaded_settings(
    hwnd: HWND,
    settings: crate::config::Settings,
    warnings: Vec<crate::config::SettingWarning>,
) {
    let mut sidebar_changed = true;
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let sidebar = |settings: &crate::config::Settings| {
            (
                settings.notes_mode,
                settings.sidebar_view,
                settings.sidebar_width,
            )
        };
        // Settings `bootstrap::run` preloaded already made the sidebar the first frame shows.
        sidebar_changed = sidebar(&app.settings) != sidebar(&settings)
            || app.sidebar.is_some() != settings.notes_mode;
        app.settings = settings;
        for warning in &warnings {
            app.notifications.push(settings_warning_message(warning));
        }
        let (keymap, problems) =
            crate::window::keymap::Keymap::from_ini(&app.settings.key_overrides);
        for problem in problems {
            app.notifications.push(format!("fastpad.ini: {problem}"));
        }
        if keymap != app.keymap {
            install_keymap(app, keymap);
        }
    }
    apply_editor_settings(hwnd);
    apply_always_on_top(hwnd);
    if sidebar_changed {
        let notes_mode = notes_mode_enabled(hwnd);
        crate::window::side_panel::notes_mode_changed(hwnd, notes_mode);
    }
}

/// Puts the window above every non-topmost window, or back among them, to match the
/// `always_on_top` setting. Neither move nor resize nor activate: only the z-order changes.
pub(super) fn apply_always_on_top(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, HWND_TOPMOST};
    let Some(app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let insert_after = if unsafe { app.as_ref() }.settings.always_on_top {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    unsafe {
        SetWindowPos(
            hwnd,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

fn settings_warning_message(warning: &crate::config::SettingWarning) -> String {
    if warning.line == 0 {
        format!("fastpad.ini: {}", warning.message)
    } else {
        format!("fastpad.ini line {}: {}", warning.line, warning.message)
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static EDITOR_SETTINGS_APPLIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has applied the editor settings, for the tests that check a
/// sidebar change does not.
#[cfg(test)]
pub(super) fn editor_settings_applied() -> usize {
    EDITOR_SETTINGS_APPLIED.with(std::cell::Cell::get)
}

/// Also recolors the line numbers: this runs after every lexer change, whose style reset gives
/// the gutter full-contrast text. Before chrome exists the neutral palette matches Scintilla's own
/// black-on-white defaults.
pub(super) fn apply_editor_settings(hwnd: HWND) {
    #[cfg(test)]
    EDITOR_SETTINGS_APPLIED.with(|count| count.set(count.get() + 1));
    let Some((settings, palette)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
        (app.settings.clone(), palette)
    }) else {
        return;
    };
    for editor in all_editors(hwnd) {
        apply_settings_to(&editor, &settings, palette);
    }
    crate::window::preview_host::refresh_appearance(hwnd);
    crate::window::image_host::refresh_appearance(hwnd);
}
