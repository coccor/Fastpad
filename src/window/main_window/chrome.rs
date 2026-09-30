//! The window chrome: building it at startup, the logo icon, theme changes, the title bar's
//! pointer state and caption buttons, the status bar and notifications.

use super::*;

/// Runs only inside `WM_FASTPAD_BUILD_CHROME`: the first system theme query, the status model,
/// and a repaint that makes any queued notifications visible.
pub(super) fn build_chrome(hwnd: HWND) {
    let theme = crate::platform::theme::SystemTheme::detect();
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.theme = Some(theme);
        app.status = Some(crate::window::status::StatusModel::new(theme));
    }
    apply_theme(hwnd);
    layout_editor_and_find_bar(hwnd);
    load_and_show_logo(hwnd);
    unsafe {
        windows_sys::Win32::UI::Shell::DragAcceptFiles(hwnd, 1);
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
    crate::window::library_host::accept_editor_file_drops(hwnd);
}

/// Loads the activity bar's logo icon at the window's current DPI (the deferred chrome step, the
/// first time the icon is loaded at all: nothing before first paint touches it) and invalidates
/// only its rect on the bar, if the sidebar exists yet.
fn load_and_show_logo(hwnd: HWND) {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    ensure_logo_icon(hwnd, dpi);
    let bar = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .sidebar
            .as_ref()
            .map(|sidebar| sidebar.bar)
    });
    if let Some(bar) = bar {
        let mut client = RECT::default();
        unsafe { GetClientRect(bar, &mut client) };
        let rect = crate::window::activity_bar::logo_rect(client, dpi);
        unsafe { InvalidateRect(bar, &rect, 1) };
    }
}

/// Loads the logo icon for `dpi` unless it is already loaded at that DPI, replacing (and, by
/// dropping it, destroying) any icon loaded at a different one. Call only from `build_chrome`
/// (after first paint) and the `WM_DPICHANGED` handler; a paint must never trigger a load.
pub(super) fn ensure_logo_icon(hwnd: HWND, dpi: u32) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if app.logo_icon.as_ref().is_some_and(|logo| logo.dpi() == dpi) {
            return;
        }
        app.logo_icon = load_logo_icon(dpi).map(|icon| LogoIcon::new(dpi, icon));
    }
}

/// The logo icon loaded for `dpi`, or `None` before `build_chrome` has run, or momentarily while a
/// different DPI's icon hasn't been reloaded yet. Never loads; call it with nothing of the App
/// borrowed, from `activity_bar::paint`.
pub(crate) fn logo_icon(hwnd: HWND, dpi: u32) -> Option<HICON> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .logo_icon
            .as_ref()
            .filter(|logo| logo.dpi() == dpi)
            .map(LogoIcon::icon)
    })
}

/// Loads the app's icon resource (`APP_ICON_RESOURCE_ID`, embedded by `build.rs`) at `dpi`'s pixel
/// size. No file I/O: it is already resident in the module. `None` if the resource is missing
/// (e.g. a test binary built without it) or the load otherwise fails.
pub(super) fn load_logo_icon(dpi: u32) -> Option<HICON> {
    let px = crate::window::design::metrics::scale(20, dpi);
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let handle = unsafe {
        LoadImageW(
            instance,
            APP_ICON_RESOURCE_ID as *const u16,
            IMAGE_ICON,
            px,
            px,
            LR_DEFAULTCOLOR,
        )
    };
    (!handle.is_null()).then_some(handle)
}

/// Re-queries the system theme after chrome exists and restyles the editor only on a real change.
pub(super) fn refresh_theme(hwnd: HWND) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.theme.is_none() {
            return false;
        }
        let theme = crate::platform::theme::SystemTheme::detect();
        if app.theme == Some(theme) {
            return false;
        }
        app.theme = Some(theme);
        if let Some(status) = app.status.as_mut() {
            status.theme = theme;
        }
        true
    });
    if changed {
        apply_theme(hwnd);
    }
}

/// Before chrome exists there is no cached theme, so system-following preferences fall back to the
/// one-shot registry read `apply_language` has always used; fixed themes skip it.
pub(crate) fn effective_theme(hwnd: HWND) -> crate::platform::theme::Theme {
    let (theme, preference) = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            (app.theme, app.settings.theme)
        })
        .unwrap_or((None, crate::config::ThemePreference::System));
    match theme {
        Some(theme) => theme.effective_theme(preference),
        None => crate::platform::theme::Theme::resolve(
            preference,
            preference.follows_system() && crate::platform::theme::system_uses_dark_mode(),
        ),
    }
}

pub(super) fn apply_theme(hwnd: HWND) {
    let Some((editor, language, palette, frame_change)) =
        (unsafe { app_ptr(hwnd) }).and_then(|mut app| {
            let app = unsafe { app.as_mut() };
            let editor = app.editor().cloned()?;
            let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
            let frame_change = app.dark_frame_applied != palette.dark_frame;
            app.dark_frame_applied = palette.dark_frame;
            for group in &mut app.groups {
                if let Some(bar) = group.find_bar.as_mut() {
                    bar.set_colors(palette);
                }
            }
            if let Some(name_box) = app.name_box.as_mut() {
                name_box.set_colors(palette);
            }
            if let Some(command_palette) = app.command_palette.as_mut() {
                command_palette.set_colors(palette);
            }
            let language = app
                .tabs
                .active()
                .map_or(crate::document::Language::PlainText, |document| {
                    document.language
                });
            Some((editor, language, palette, frame_change))
        })
    else {
        return;
    };
    let highlight_current_line = unsafe { app_ptr(hwnd) }
        .is_none_or(|app| unsafe { app.as_ref() }.settings.highlight_current_line);
    for editor in all_editors(hwnd) {
        apply_colors_to(&editor, palette, highlight_current_line);
    }
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_ref() };
        for bar in app
            .groups
            .iter()
            .filter_map(|group| group.find_bar.as_ref())
        {
            bar.invalidate();
        }
        if let Some(name_box) = app.name_box.as_ref() {
            name_box.invalidate();
        }
        if let Some(command_palette) = app.command_palette.as_ref() {
            command_palette.invalidate();
        }
    }
    if frame_change {
        crate::window::titlebar::apply_frame_theme(hwnd, editor.hwnd(), palette.dark_frame);
    }
    if language != crate::document::Language::PlainText {
        apply_language(hwnd, language);
    }
    let others: Vec<GroupId> = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            let active = app.tabs.active_group();
            app.groups
                .iter()
                .map(|group| group.id)
                .filter(|id| *id != active)
                .collect()
        })
        .unwrap_or_default();
    for id in others {
        style_group_view(hwnd, id);
    }
    crate::window::preview_host::refresh_appearance(hwnd);
    crate::window::image_host::refresh_appearance(hwnd);
    crate::window::side_panel::refresh(hwnd);
}

/// Copies what a title-strip paint needs out of App, creating the per-DPI fonts on first use.
/// Before chrome is built the palette is the neutral compiled one (no theme queries).
pub(crate) fn title_chrome(hwnd: HWND) -> (Palette, TitleFontHandles, PointerState) {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    unsafe { app_ptr(hwnd) }
        .map(|mut app| {
            let app = unsafe { app.as_mut() };
            if app
                .title_fonts
                .as_ref()
                .is_none_or(|fonts| fonts.dpi() != dpi)
            {
                app.title_fonts = Some(crate::window::titlebar::TitleFonts::create(dpi));
            }
            (
                Palette::for_cached_theme(app.theme, app.settings.theme),
                app.title_fonts
                    .as_ref()
                    .map(crate::window::titlebar::TitleFonts::handles)
                    .unwrap_or_default(),
                app.title_pointer,
            )
        })
        .unwrap_or_else(|| {
            (
                Palette::neutral(),
                TitleFontHandles::default(),
                PointerState::default(),
            )
        })
}

pub(super) fn client_title_target(hwnd: HWND, lparam: LPARAM) -> Option<HitTarget> {
    let point = crate::window::titlebar::Point::new(
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    Some(title_layout(hwnd).hit_test(point))
}

pub(super) fn update_title_pointer(hwnd: HWND, update: impl FnOnce(PointerState) -> PointerState) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let next = update(app.title_pointer);
        let changed = next != app.title_pointer;
        app.title_pointer = next;
        changed
    });
    if changed {
        crate::window::titlebar::invalidate_strip(hwnd);
    }
}

pub(super) fn run_caption_button(hwnd: HWND, target: HitTarget) {
    let command = match target {
        HitTarget::Minimize => SC_MINIMIZE,
        HitTarget::Maximize if unsafe { IsZoomed(hwnd) } != 0 => SC_RESTORE,
        HitTarget::Maximize => SC_MAXIMIZE,
        HitTarget::Close => SC_CLOSE,
        _ => return,
    };
    unsafe {
        SendMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
    }
}

/// The pending-notification text shown on the bottom bar, which exists only after
/// `WM_FASTPAD_BUILD_CHROME`.
pub(super) fn current_status_text(hwnd: HWND) -> Option<String> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    app.status.as_ref()?;
    crate::window::status::status_text(&app.notifications)
}

/// Everything the bottom bar paints, or `None` before `WM_FASTPAD_BUILD_CHROME` builds it.
pub(super) fn current_status_bar(hwnd: HWND) -> Option<crate::window::status::StatusBarText> {
    if let Some(image) = crate::window::image_host::status(hwnd) {
        let app = unsafe { app_ptr(hwnd) }?;
        let app = unsafe { app.as_ref() };
        app.status.as_ref()?;
        return Some(crate::window::status::image_status_bar_text(
            &app.notifications,
            &image,
        ));
    }
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    app.status.as_ref()?;
    let active = app.tabs.active().and_then(|document| {
        Some(crate::window::status::ActiveDocumentStatus {
            caret: app.editor()?.caret_status().ok()?,
            language: document.language,
            encoding: document.encoding,
        })
    });
    let mut bar = crate::window::status::status_bar_text(&app.notifications, active);
    if app.notifications.pending().is_empty()
        && let Some(hint) = app
            .active_group()
            .and_then(|group| group.preview.status_hint())
    {
        bar.left = hint;
    }
    Some(bar)
}

pub(super) fn status_bar_height(hwnd: HWND) -> i32 {
    let built =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.status.is_some());
    if !built {
        return 0;
    }
    crate::window::status::status_height(unsafe {
        windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd)
    })
}

fn status_bar_rect(hwnd: HWND) -> Option<RECT> {
    let height = status_bar_height(hwnd);
    if height == 0 {
        return None;
    }
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    rect.top = (rect.bottom - height).max(rect.top);
    Some(rect)
}

/// Clicking the bar dismisses notifications only while one is showing.
pub(super) fn notice_contains(hwnd: HWND, y: i32) -> bool {
    current_status_text(hwnd).is_some() && status_bar_rect(hwnd).is_some_and(|rect| y >= rect.top)
}

/// Repaints just the bottom bar, for caret, selection and language changes.
pub(crate) fn invalidate_status_bar(hwnd: HWND) {
    if let Some(rect) = status_bar_rect(hwnd) {
        unsafe {
            InvalidateRect(hwnd, &rect, 0);
        }
    }
}

pub(super) fn dismiss_notifications(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.notifications.dismiss_all();
    }
    layout_editor_and_find_bar(hwnd);
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

/// Reports a rejected Open (missing file, unsupported encoding, NUL bytes) by naming the file.
pub(crate) fn report_open_failure(hwnd: HWND, path: &std::path::Path, error: &crate::FastPadError) {
    push_notice(
        hwnd,
        format!("FastPad could not open {}: {error}", path.display()),
    );
}

pub(crate) fn push_notice(hwnd: HWND, message: String) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.notifications.push(message);
    }
    refresh_notifications(hwnd);
}

pub(super) fn refresh_notifications(hwnd: HWND) {
    let chrome_built =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.status.is_some());
    if chrome_built {
        layout_editor_and_find_bar(hwnd);
    }
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}
