pub(crate) mod accessibility;
pub(crate) mod activity_bar;
pub(crate) mod command_palette;
pub mod commands;
pub mod find_bar;
pub(crate) mod library_host;
mod main_window;
pub(crate) mod menu_band;
pub(crate) mod menus;
mod messages;
pub(crate) mod modal;
pub(crate) mod name_box;
pub(crate) mod notebook_view;
pub mod notification;
pub mod palette;
pub(crate) mod panel;
pub(crate) mod preview_host;
pub(crate) mod row_list;
pub(crate) mod side_panel;
pub mod status;
pub mod tabs;
pub mod titlebar;
pub(crate) mod tooltip;

pub(crate) use main_window::{
    INPUT_MESSAGE_FIRST, INPUT_MESSAGE_LAST, MainWindowClass, WindowCreateContext,
    clear_input_priority, initialize_editor_with, input_priority_requested,
    input_queue_status_mask, ipc_wait_handle, maybe_post_deferred_start, open_path, service_ipc,
    translate_accelerator,
};
#[cfg(test)]
#[allow(
    unused_imports,
    reason = "consumed by the source-linked save_file integration target"
)]
pub(crate) use main_window::{save_path_as, with_test_input_queue_status};
pub use messages::{
    WM_FASTPAD_APPLY_LANGUAGE, WM_FASTPAD_BUILD_CHROME, WM_FASTPAD_DIAGNOSTIC_JSON_COUNT,
    WM_FASTPAD_DIAGNOSTIC_PREVIEW, WM_FASTPAD_FILES_DROPPED, WM_FASTPAD_IPC_REQUEST,
    WM_FASTPAD_LIBRARY_READY, WM_FASTPAD_LOAD_SETTINGS, WM_FASTPAD_NOTEBOOK_CHECKED,
    WM_FASTPAD_OPEN_LIBRARY, WM_FASTPAD_OPEN_REQUEST, WM_FASTPAD_PREVIEW_ACTIVATE,
    WM_FASTPAD_PREVIEW_ESCAPE, WM_FASTPAD_PREVIEW_HOVER, WM_FASTPAD_PREVIEW_IMAGE,
    WM_FASTPAD_PREVIEW_LINK, WM_FASTPAD_PREVIEW_PARSED, WM_FASTPAD_PREVIEW_REFRESH,
    WM_FASTPAD_PREVIEW_SCROLLED, WM_FASTPAD_RECOVERY, WM_FASTPAD_RESTORE_SESSION,
    WM_FASTPAD_START_IPC,
};
#[cfg(test)]
#[allow(
    unused_imports,
    reason = "consumed by the source-linked save_file integration target"
)]
pub(crate) use modal::{
    answer_next_close_prompt, answer_next_confirm, answer_next_folder_dialog,
    answer_next_save_dialog,
};
