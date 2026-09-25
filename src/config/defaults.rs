use super::persisted::{FileIconSet, Settings, SidebarView, ThemePreference};

/// FastPad's compiled fallback font face, used until a settings file overrides it. Kept separate
/// from `languages::DEFAULT_FONT_FACE` (the per-lexer-style face used while highlighting is
/// deliberately unconfigurable) even though both currently name the same font: this one is a
/// user-facing default meant to be overridden by `fastpad.ini`, that one is an internal constant.
pub const DEFAULT_FONT_FACE: &str = "Consolas";
pub const DEFAULT_FONT_SIZE: u16 = 11;
pub const DEFAULT_TAB_WIDTH: u8 = 4;
pub const DEFAULT_WORD_WRAP: bool = false;
pub const DEFAULT_LINE_NUMBERS: bool = true;
pub const DEFAULT_THEME: ThemePreference = ThemePreference::System;
pub const DEFAULT_RECOVERY_INTERVAL_SECONDS: u32 = 30;
pub const DEFAULT_RESTORE_SESSION: bool = true;
pub const DEFAULT_NOTES_MODE: bool = true;
pub const DEFAULT_SIDEBAR_VIEW: SidebarView = SidebarView::Notebook;
/// The side panel's width in 96-DPI pixels: the default, and the range a drag or `fastpad.ini`
/// can set.
pub const DEFAULT_SIDEBAR_WIDTH: u16 = 260;
pub const MIN_SIDEBAR_WIDTH: u16 = 180;
pub const MAX_SIDEBAR_WIDTH: u16 = 480;
pub const DEFAULT_FILE_ICONS: FileIconSet = FileIconSet::Material;

/// `width` pulled into `MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH`.
pub const fn clamp_sidebar_width(width: u16) -> u16 {
    if width < MIN_SIDEBAR_WIDTH {
        MIN_SIDEBAR_WIDTH
    } else if width > MAX_SIDEBAR_WIDTH {
        MAX_SIDEBAR_WIDTH
    } else {
        width
    }
}

/// FastPad's compiled defaults: Consolas 11pt, 4-wide tabs, word wrap off, line numbers on, system
/// theme, a 30-second crash-recovery interval, session restore on, notes mode on, the side
/// panel showing the Notebook view at 260 pixels, and Material Icon Theme file icons. Every value
/// a settings file does not (validly) specify keeps whatever `default_settings()` produced.
pub fn default_settings() -> Settings {
    Settings {
        font_face: DEFAULT_FONT_FACE.to_owned(),
        font_size: DEFAULT_FONT_SIZE,
        tab_width: DEFAULT_TAB_WIDTH,
        word_wrap: DEFAULT_WORD_WRAP,
        line_numbers: DEFAULT_LINE_NUMBERS,
        theme: DEFAULT_THEME,
        recovery_interval_seconds: DEFAULT_RECOVERY_INTERVAL_SECONDS,
        restore_session: DEFAULT_RESTORE_SESSION,
        notes_mode: DEFAULT_NOTES_MODE,
        sidebar_view: DEFAULT_SIDEBAR_VIEW,
        sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        file_icons: DEFAULT_FILE_ICONS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_match_the_compiled_defaults() {
        // Break caught: default_settings drifting from the documented compiled defaults silently
        // changes what a brand-new profile (no fastpad.ini yet) looks like.
        let settings = default_settings();
        assert_eq!(settings.font_face, "Consolas");
        assert_eq!(settings.font_size, 11);
        assert_eq!(settings.tab_width, 4);
        assert!(!settings.word_wrap);
        assert!(settings.line_numbers);
        assert_eq!(settings.theme, ThemePreference::System);
        assert_eq!(settings.recovery_interval_seconds, 30);
        assert!(settings.restore_session);
        assert!(settings.notes_mode);
        assert_eq!(settings.sidebar_view, SidebarView::Notebook);
        assert_eq!(settings.sidebar_width, 260);
    }

    #[test]
    fn sidebar_widths_clamp_to_the_range() {
        assert_eq!(clamp_sidebar_width(0), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(179), 180);
        assert_eq!(clamp_sidebar_width(260), 260);
        assert_eq!(clamp_sidebar_width(481), 480);
        assert_eq!(clamp_sidebar_width(u16::MAX), MAX_SIDEBAR_WIDTH);
    }
}
