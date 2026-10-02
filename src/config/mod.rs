pub mod defaults;
pub mod persisted;

pub use defaults::{
    DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, clamp_sidebar_width,
    default_settings,
};
pub use persisted::{
    FileIconSet, SettingWarning, Settings, SettingsDelta, SidebarView, ThemePreference,
    WindowPlacement, load, parse, remove_setting, remove_setting_to, save_setting, save_setting_to,
};
