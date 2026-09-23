pub mod defaults;
pub mod persisted;

pub use defaults::{
    DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, clamp_sidebar_width,
    default_settings,
};
pub use persisted::{
    SettingWarning, Settings, SettingsDelta, SidebarView, ThemePreference, load, parse,
    save_setting, save_setting_to,
};
