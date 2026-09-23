//! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
//! folder commands, first-save naming, autosave, and the organizing commands.

use super::main_window::{app_ptr, push_notice, window_identity};
use crate::library::ids::{NotebookId, TagId};
use crate::library::model::{LibraryError, NotebookColor};
use crate::library::ops::PendingOp;
use crate::library::title;
use crate::library::{self, LibraryState, Metadata, ids::IdSource};
use crate::window::command_palette::{Picker, PickerChoice, PickerKind};
use crate::window::commands::CommandId;
use crate::window::name_box::{NameBox, NamePurpose};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

pub(crate) const LIBRARY_WRITE_TIMER_ID: usize = 0x4650_4C57;
pub(crate) const RESCAN_AFTER: Duration = Duration::from_secs(5);
const WRITE_DELAY_MS: u32 = 500;
pub(crate) const AUTOSAVE_TIMER_ID: usize = 0x4650_4153;
pub(crate) const AUTOSAVE_DELAY_MS: u32 = 1_000;

/// What one autosave attempt did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Autosave {
    /// Nothing to do: notes mode or the folder's autosave is off, or the active tab is clean,
    /// untitled, outside the folder, or paused.
    NotEligible,
    Saved,
    /// The file changed on disk since FastPad loaded or saved it; autosave is now paused for it.
    Paused,
    /// The write failed; the tab stays dirty.
    Failed,
}

#[derive(Debug)]
pub(crate) struct LibraryHost {
    /// The open folder. Set as soon as the step runs, before its state has loaded.
    pub(crate) folder: Option<PathBuf>,
    pub(crate) state: Option<LibraryState>,
    /// `%LOCALAPPDATA%\FastPad`, resolved lazily. Under `cfg(test)` it is only ever pre-seeded.
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) generation: u64,
    pub(crate) scanning: bool,
    pub(crate) rescan_requested: bool,
    pub(crate) inactive_since: Option<Instant>,
    /// IDs for new notes, notebooks and tags.
    pub(crate) ids: IdSource,
    notified: Option<PathBuf>,
    /// The folders the open recent-folder picker lists, in its row order.
    shown_recent_folders: Vec<PathBuf>,
    /// The rows the open organizing picker lists, in its row order, so a pick acts on what was
    /// shown even if the library changed meanwhile. `None` in a row is the "Notes" row.
    shown_targets: Option<(PickerKind, Vec<Option<Target>>)>,
    /// The notebook or tag a multi-step picker flow acts on (the notebook whose color is chosen).
    pending_target: Option<Target>,
}

/// A notebook or tag an organizing picker row or flow acts on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    Notebook(NotebookId),
    Tag(TagId),
}

impl LibraryHost {
    pub(crate) fn new(process_start: u64) -> Self {
        Self {
            folder: None,
            state: None,
            data_dir: None,
            generation: 0,
            scanning: false,
            rescan_requested: false,
            inactive_since: None,
            ids: IdSource::new(process_start, std::process::id()),
            notified: None,
            shown_recent_folders: Vec::new(),
            shown_targets: None,
            pending_target: None,
        }
    }
}

struct Loaded {
    generation: u64,
    folder: PathBuf,
    result: Result<LibraryState, String>,
}

fn host<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryHost) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.map(|mut app| f(&mut unsafe { app.as_mut() }.library))
}

fn notes_mode(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode)
}

pub(crate) fn folder(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| host.folder.clone()).flatten()
}

pub(crate) fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryState) -> R) -> Option<R> {
    host(hwnd, |host| host.state.as_mut().map(f)).flatten()
}

fn data_dir(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| {
        if host.data_dir.is_none() && !cfg!(test) {
            host.data_dir = crate::platform::fastpad_data_dir().ok();
        }
        host.data_dir.clone()
    })
    .flatten()
}

/// The folder to open at startup: a directory named on the command line, else the most recent
/// folder that still exists, else `Documents\FastPad`.
fn startup_folder(hwnd: HWND, data: &Path) -> Option<PathBuf> {
    let launch_dir =
        unsafe { app_ptr(hwnd) }.and_then(|app| match &unsafe { app.as_ref() }.launch.request {
            crate::launch::LaunchRequest::Open(path) => {
                let path = std::path::absolute(path).ok()?;
                path.is_dir().then_some(path)
            }
            crate::launch::LaunchRequest::New => None,
        });
    if let Some(path) = launch_dir {
        remember_folder(data, &path);
        return Some(path);
    }
    let recent = library::local::read_folders(&library::local::folders_file(data));
    if let Some(first) = recent.folders.first() {
        if first.is_dir() {
            return Some(first.clone());
        }
        push_notice(
            hwnd,
            format!(
                "FastPad could not find the folder {}. Using Documents\\FastPad instead.",
                first.display()
            ),
        );
    }
    crate::platform::paths::default_notes_folder().ok()
}

pub(crate) fn remember_folder(data: &Path, folder: &Path) {
    let path = library::local::folders_file(data);
    let mut recent = library::local::read_folders(&path);
    recent.push(folder.to_path_buf());
    let _ = library::local::write_folders(&path, &recent);
}

/// `WM_FASTPAD_OPEN_LIBRARY`: picks the folder and starts the worker. Reads only `folders.ini`.
pub(crate) fn open_library_step(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some(folder) = startup_folder(hwnd, &data) else {
        return;
    };
    host(hwnd, |host| host.folder = Some(folder));
    start_load(hwnd);
}

pub(crate) fn start_load(hwnd: HWND) {
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some((folder, generation)) = host(hwnd, |host| {
        let folder = host.folder.clone()?;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = true;
        host.rescan_requested = false;
        Some((folder, host.generation))
    })
    .flatten() else {
        return;
    };
    let local_path = library::local::local_file(&data, &folder);
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let result = library::load(&folder, &local_path, library::now_unix())
            .map_err(|error| error.to_string());
        let payload = Box::into_raw(Box::new(Loaded {
            generation,
            folder,
            result,
        }));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_LIBRARY_READY,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

#[cfg(test)]
pub(crate) fn test_ready_payload(generation: u64, result: Result<LibraryState, String>) -> LPARAM {
    Box::into_raw(Box::new(Loaded {
        generation,
        folder: PathBuf::new(),
        result,
    })) as LPARAM
}

/// `WM_FASTPAD_LIBRARY_READY`: installs the worker's result unless a newer load superseded it.
pub(crate) fn library_ready(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let loaded = unsafe { Box::from_raw(lparam as *mut Loaded) };
    let current = host(hwnd, |host| {
        if host.generation != loaded.generation {
            return false;
        }
        host.scanning = false;
        true
    })
    .unwrap_or(false);
    if !current {
        return;
    }
    let Loaded { folder, result, .. } = *loaded;
    match result {
        Ok(fresh) => install(hwnd, fresh),
        Err(error) => push_notice(
            hwnd,
            format!(
                "FastPad could not load the folder {}: {error}",
                folder.display()
            ),
        ),
    }
    if host(hwnd, |host| std::mem::take(&mut host.rescan_requested)).unwrap_or(false) {
        start_load(hwnd);
    }
}

fn install(hwnd: HWND, fresh: LibraryState) {
    let Some((relocated, first_time, truncated, unreadable, has_pending)) = host(hwnd, |host| {
        let state = match host.state.take() {
            Some(previous) if library::model::same_path(&previous.folder, &fresh.folder) => {
                library::merge_rescan(previous, fresh)
            }
            _ => fresh,
        };
        let first_time = !host
            .notified
            .as_ref()
            .is_some_and(|f| library::model::same_path(f, &state.folder));
        host.notified = Some(state.folder.clone());
        let state = host.state.insert(state);
        (
            std::mem::take(&mut state.relocated),
            first_time,
            state.truncated,
            state.metadata == Metadata::Unreadable,
            !state.pending.is_empty(),
        )
    }) else {
        return;
    };
    let folder = folder(hwnd).unwrap_or_default();
    for (old, new) in relocated {
        rebind_open_tab(hwnd, &folder.join(old), folder.join(new));
    }
    if first_time && truncated {
        push_notice(
            hwnd,
            "This folder has more than 10,000 notes. FastPad indexed the first 10,000.".to_owned(),
        );
    }
    if first_time && unreadable {
        push_notice(hwnd, "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so notebooks and tags are read-only.".to_owned());
    }
    if has_pending {
        schedule_write(hwnd);
    }
    with_state(hwnd, |state| library::write_local(state));
}

/// A note moved outside FastPad: an open tab for it follows the file.
fn rebind_open_tab(hwnd: HWND, old: &Path, new: PathBuf) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        match app.tabs.find_path(old) {
            Some(id) => app.tabs.rebind_path(id, new).is_ok(),
            None => false,
        }
    });
    if changed {
        super::main_window::invalidate_title_strip(hwnd);
    }
}

pub(crate) fn request_rescan(hwnd: HWND) {
    let scanning = host(hwnd, |host| {
        if host.scanning {
            host.rescan_requested = true;
        }
        host.scanning
    })
    .unwrap_or(true);
    if !scanning && folder(hwnd).is_some() {
        start_load(hwnd);
    }
}

/// `WM_ACTIVATEAPP`. Coming back after at least `RESCAN_AFTER` rescans, to catch Explorer edits.
pub(crate) fn activation_changed(hwnd: HWND, active: bool) {
    if !active {
        host(hwnd, |host| host.inactive_since = Some(Instant::now()));
        autosave_active(hwnd);
        return;
    }
    let long_enough = host(hwnd, |host| {
        host.inactive_since
            .take()
            .is_some_and(|since| since.elapsed() >= RESCAN_AFTER)
    })
    .unwrap_or(false);
    if long_enough && notes_mode(hwnd) {
        request_rescan(hwnd);
    }
}

pub(crate) fn schedule_write(hwnd: HWND) {
    unsafe {
        SetTimer(hwnd, LIBRARY_WRITE_TIMER_ID, WRITE_DELAY_MS, None);
    }
}

/// Writes pending metadata now (timer, folder switch, close).
pub(crate) fn flush_now(hwnd: HWND) {
    if let Err(error) = try_flush(hwnd) {
        push_notice(
            hwnd,
            format!("FastPad could not save this folder's notebooks and tags: {error}"),
        );
    }
}

fn try_flush(hwnd: HWND) -> crate::Result<()> {
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    with_state(hwnd, |state| {
        let result = library::flush(state);
        library::write_local(state);
        result.map(|_| ())
    })
    .unwrap_or(Ok(()))
}

/// Opens `path` as the library, flushing the current one first. Open tabs stay open.
pub(crate) fn open_folder(hwnd: HWND, path: &Path) {
    if !notes_mode(hwnd) {
        push_notice(
            hwnd,
            "Notes mode is off. Turn it on with Notes: Toggle notes mode to open folders."
                .to_owned(),
        );
        return;
    }
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    if !path.is_dir() {
        push_notice(hwnd, format!("{} is not a folder.", path.display()));
        return;
    }
    // Switching would drop the unsaved notebooks and tags, so a failed write keeps the old folder.
    if let Err(error) = try_flush(hwnd) {
        schedule_write(hwnd);
        push_notice(
            hwnd,
            format!(
                "FastPad kept this folder open because it could not save its notebooks and tags: {error}"
            ),
        );
        return;
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = Some(path.clone());
    });
    if let Some(data) = data_dir(hwnd) {
        remember_folder(&data, &path);
    }
    start_load(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
}

pub(crate) fn choose_and_open_folder(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let choice = crate::window::modal::choose_folder(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    match choice {
        Ok(Some(path)) => open_folder(hwnd, &path),
        Ok(None) => {}
        Err(error) => push_notice(
            hwnd,
            format!("FastPad could not open the folder picker: {error}"),
        ),
    }
}

fn recent_folders(hwnd: HWND) -> Vec<PathBuf> {
    data_dir(hwnd)
        .map(|data| library::local::read_folders(&library::local::folders_file(&data)).folders)
        .unwrap_or_default()
}

pub(crate) fn open_recent_folder_picker(hwnd: HWND) {
    let folders = recent_folders(hwnd);
    if folders.is_empty() {
        push_notice(
            hwnd,
            "No recent folders yet. Use File: Open folder.".to_owned(),
        );
        return;
    }
    let items = folders.iter().map(|f| f.display().to_string()).collect();
    host(hwnd, |host| host.shown_recent_folders = folders);
    super::main_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::RecentFolder,
            items,
            create: None,
        },
    );
}

/// Scintilla's own OLE drop target refuses files and wins over `WM_DROPFILES`, so the editor gets
/// a wrapper that posts dropped files here as `WM_FASTPAD_FILES_DROPPED`. Runs in `BUILD_CHROME`.
pub(crate) fn accept_editor_file_drops(hwnd: HWND) {
    let Some(editor) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.editor.as_ref().map(|e| e.hwnd()))
    else {
        return;
    };
    let target = hwnd as isize;
    // Text drag-and-drop still works without the wrapper; only file drops on the editor are lost.
    let _ = crate::editor::file_drop::accept_file_drops(editor, move |paths| {
        let payload = Box::into_raw(Box::new(paths));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_FILES_DROPPED,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// `WM_FASTPAD_FILES_DROPPED`: frees the posted paths and opens them. A drop that lands while a
/// modal dialog runs is ignored, as `WM_DROPFILES` is for a disabled window.
pub(crate) fn editor_files_dropped(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let paths = *unsafe { Box::from_raw(lparam as *mut Vec<PathBuf>) };
    if !super::modal::modal_active(hwnd) {
        files_dropped(hwnd, paths);
    }
}

/// Dropped folders open as the library (the last one wins); dropped files open as tabs.
pub(crate) fn files_dropped(hwnd: HWND, paths: Vec<PathBuf>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let mut folder = None;
    for path in paths {
        if !identity.is_live_for(hwnd) {
            return;
        }
        if path.is_dir() {
            folder = Some(path);
        } else if let Err(error) = super::main_window::open_path(hwnd, &path) {
            super::main_window::report_open_failure(hwnd, &path, &error);
        }
    }
    if let Some(folder) = folder
        && identity.is_live_for(hwnd)
    {
        open_folder(hwnd, &folder);
    }
}

/// Notes mode was toggled: load the last folder, or flush and forget the library.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    if enabled {
        if folder(hwnd).is_none() {
            open_library_step(hwnd);
        }
        return;
    }
    flush_now(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
}

/// Recomputes the active untitled tab's label from its first lines.
pub(crate) fn refresh_label(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let Some(editor) = app.editor.as_ref() else {
            return false;
        };
        let Some(active) = app.tabs.active() else {
            return false;
        };
        if active.path.is_some() {
            return false;
        }
        let id = active.id;
        let count = editor
            .line_count()
            .unwrap_or(0)
            .min(title::LABEL_SCAN_LINES);
        let lines: Vec<String> = (0..count)
            .map(|line| editor.line_text(line).unwrap_or_default())
            .collect();
        let label = title::untitled_label(lines.iter().map(String::as_str));
        let Some(document) = app.tabs.document_mut(id) else {
            return false;
        };
        document.label_watch = label.watch_through;
        if document.untitled_label == label.text {
            return false;
        }
        document.untitled_label = label.text;
        true
    });
    if changed {
        super::main_window::refresh_tab_view(hwnd);
    }
}

/// `SCN_MODIFIED`: only edits at or above the label's line can change it.
pub(crate) fn text_changed(hwnd: HWND, position: usize) {
    let relevant = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        let (Some(editor), Some(active)) = (app.editor.as_ref(), app.tabs.active()) else {
            return false;
        };
        active.path.is_none()
            && editor
                .line_from_position(position)
                .is_ok_and(|line| line <= active.label_watch)
    });
    if relevant {
        refresh_label(hwnd);
    }
}

/// Notes mode turned off: untitled tabs go back to "Untitled".
pub(crate) fn clear_labels(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let ids: Vec<_> = app.tabs.documents().map(|document| document.id).collect();
        for id in ids {
            if let Some(document) = app.tabs.document_mut(id) {
                document.untitled_label = None;
            }
        }
    }
    super::main_window::refresh_tab_view(hwnd);
}

fn active_untitled(hwnd: HWND) -> Option<crate::document::DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        active.path.is_none().then_some(active.id)
    })
}

/// "<sanitized label>.<extension for the tab's language>".
pub(crate) fn suggested_file_name(hwnd: HWND) -> String {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let active = unsafe { app.as_ref() }.tabs.active()?;
            let stem = title::sanitize_stem(active.untitled_label.as_deref().unwrap_or("Untitled"));
            Some(format!(
                "{stem}.{}",
                title::default_extension(active.language)
            ))
        })
        .unwrap_or_else(|| "Untitled.md".to_owned())
}

/// Ctrl+S: an untitled tab in notes mode is named in the name box; everything else as before.
pub(crate) fn save_command(hwnd: HWND) {
    if notes_mode(hwnd)
        && folder(hwnd).is_some()
        && let Some(id) = active_untitled(hwnd)
    {
        // Ctrl+S again while the box is open for this tab keeps what was typed.
        if name_box_purpose(hwnd) == Some(NamePurpose::FirstSave(id)) {
            focus_name_box(hwnd);
            return;
        }
        refresh_label(hwnd);
        let suffix = format!("in {}", folder_display_name(hwnd));
        open_name_box(
            hwnd,
            NamePurpose::FirstSave(id),
            &suggested_file_name(hwnd),
            suffix,
            true,
        );
        return;
    }
    let _ = super::main_window::save_active_document(hwnd);
}

pub(crate) fn save_as_command(hwnd: HWND) {
    if notes_mode(hwnd) && folder(hwnd).is_some() && active_untitled(hwnd).is_some() {
        save_command(hwnd);
        return;
    }
    let _ = super::main_window::save_active_document_as(hwnd);
}

fn folder_display_name(hwnd: HWND) -> String {
    folder(hwnd)
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "the notes folder".to_owned())
}

pub(crate) fn open_name_box(
    hwnd: HWND,
    purpose: NamePurpose,
    text: &str,
    suffix: String,
    browse: bool,
) {
    super::main_window::close_find_bar(hwnd);
    let colors = super::main_window::current_palette(hwnd);
    let shown = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.name_box.is_none() {
            app.name_box = NameBox::create(hwnd).ok();
        }
        match app.name_box.as_mut() {
            Some(name_box) => {
                name_box.show(purpose, text, suffix, browse, colors);
                true
            }
            None => false,
        }
    });
    if !shown {
        push_notice(hwnd, "FastPad could not show the name box.".to_owned());
        return;
    }
    super::main_window::layout_editor_and_find_bar(hwnd);
    focus_name_box(hwnd);
}

fn focus_name_box(hwnd: HWND) {
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_ref() }.name_box.as_ref()
    {
        name_box.focus();
    }
}

/// The purpose of the visible name box, if one is showing.
fn name_box_purpose(hwnd: HWND) -> Option<NamePurpose> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        if !name_box.is_visible() {
            return None;
        }
        name_box.purpose().cloned()
    })
}

/// Closes a name box whose tab is gone or no longer active, or whose first save already happened.
pub(crate) fn close_stale_name_box(hwnd: HWND) {
    let Some(purpose) = name_box_purpose(hwnd) else {
        return;
    };
    let Some(id) = purpose.document() else {
        return;
    };
    let stale = unsafe { app_ptr(hwnd) }.is_none_or(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let Some(document) = tabs.document(id) else {
            return true;
        };
        tabs.active().is_none_or(|active| active.id != id)
            || (matches!(purpose, NamePurpose::FirstSave(_)) && document.path.is_some())
    });
    if stale {
        close_name_box(hwnd);
    }
}

pub(crate) fn close_name_box(hwnd: HWND) {
    let hidden = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .name_box
            .as_mut()
            .is_some_and(|name_box| {
                let was = name_box.is_visible();
                name_box.hide();
                was
            })
    });
    if hidden {
        super::main_window::layout_editor_and_find_bar(hwnd);
        super::main_window::focus_content(hwnd);
    }
}

fn name_box_state(hwnd: HWND) -> Option<(NamePurpose, String)> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        Some((name_box.purpose()?.clone(), name_box.text()))
    })
}

fn name_box_error(hwnd: HWND, error: String) {
    let stored = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .name_box
            .as_mut()
            .map(|name_box| name_box.set_error(Some(error)))
            .is_some()
    });
    // The error is wider than the suffix, so the field shrinks to make room.
    if stored {
        super::main_window::layout_editor_and_find_bar(hwnd);
    }
}

/// "<name> already exists. Try <first free name>."
fn name_taken_error(folder: &Path, stem: &str, extension: &str) -> String {
    let free = title::free_name(stem, extension, |candidate| folder.join(candidate).exists());
    format!("{stem}.{extension} already exists. Try {free}.")
}

/// Enter or Save in the name box.
pub(crate) fn name_box_submit(hwnd: HWND) {
    let Some((purpose, text)) = name_box_state(hwnd) else {
        return;
    };
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::NewNotebook { .. }
        | NamePurpose::RenameNotebook(_)
        | NamePurpose::RenameTag(_)
        | NamePurpose::RenameNote(_)
            if !ready_library(hwnd) =>
        {
            close_name_box(hwnd);
        }
        NamePurpose::NewNotebook { then_move } => submit_new_notebook(hwnd, then_move, text),
        NamePurpose::RenameNotebook(id) => {
            let result = apply_op(hwnd, |_, _| {
                Some(PendingOp::RenameNotebook {
                    id,
                    name: text,
                    now: library::now_unix(),
                })
            });
            close_name_box_unless_error(hwnd, result);
        }
        NamePurpose::RenameTag(id) => {
            let result = apply_op(hwnd, |_, _| Some(PendingOp::RenameTag { id, name: text }));
            close_name_box_unless_error(hwnd, result);
        }
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
    }
}

/// Closes the name box after a successful change; after a failed one, shows why and keeps it open.
fn close_name_box_unless_error(hwnd: HWND, result: Result<(), LibraryError>) {
    match result {
        Ok(()) => close_name_box(hwnd),
        Err(error) => name_box_error(hwnd, error.to_string()),
    }
}

fn submit_new_notebook(hwnd: HWND, then_move: Option<crate::document::DocumentId>, name: String) {
    let Some(id) = host(hwnd, |host| NotebookId(host.ids.next())) else {
        return;
    };
    let result = apply_op(hwnd, |_, _| {
        Some(PendingOp::CreateNotebook {
            id,
            name,
            now: library::now_unix(),
        })
    });
    if result.is_err() {
        close_name_box_unless_error(hwnd, result);
        return;
    }
    close_name_box(hwnd);
    if let Some(document) = then_move
        && super::main_window::activate_document_by_id(hwnd, document)
        && let Some(path) = active_file(hwnd)
    {
        report(
            hwnd,
            apply_op(hwnd, |state, ids| {
                Some(PendingOp::SetNoteNotebook {
                    note: state.note_ref(ids, &path),
                    notebook: Some(id),
                })
            }),
        );
    }
}

fn submit_first_save(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(folder) = folder(hwnd) else {
        return;
    };
    let language = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.document(id)?.language))
        .unwrap_or(crate::document::Language::Markdown);
    let (stem, extension) = title::split_typed_name(text, title::default_extension(language));
    let name = format!("{stem}.{extension}");
    let target = folder.join(&name);
    if target.exists() {
        name_box_error(hwnd, name_taken_error(&folder, &stem, &extension));
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&folder) {
        name_box_error(
            hwnd,
            format!("FastPad could not create {}: {error}", folder.display()),
        );
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if !super::main_window::activate_document_by_id(hwnd, id) {
        return;
    }
    let outcome = super::main_window::complete_first_save(hwnd, &identity, target);
    if !identity.is_live_for(hwnd) {
        return;
    }
    match outcome {
        super::main_window::SaveOutcome::Saved => close_name_box(hwnd),
        // A file took the name after the check above; it is left alone.
        super::main_window::SaveOutcome::NameTaken => {
            name_box_error(hwnd, name_taken_error(&folder, &stem, &extension));
        }
        super::main_window::SaveOutcome::Failed => {}
    }
}

/// Note: Rename...: the name box, prefilled with the file's current name.
pub(crate) fn rename_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    let Some(id) =
        unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
    else {
        return;
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    open_name_box(
        hwnd,
        NamePurpose::RenameNote(id),
        &name,
        "Rename".to_owned(),
        false,
    );
}

/// Renames the tab's file in place, never over another file, then follows it in the tab and the
/// library.
fn submit_rename(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(old) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.document(id)?.path.clone())
    else {
        close_name_box(hwnd);
        return;
    };
    let current_extension = old
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "md".into());
    let (stem, extension) = title::split_typed_name(text, &current_extension);
    let new = old.with_file_name(format!("{stem}.{extension}"));
    if new == old {
        close_name_box(hwnd);
        return;
    }
    // A change of letter case only names the same file, so it is not a clash.
    let case_only = library::model::same_path(&new, &old);
    let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
    if new.exists() && !case_only {
        name_box_error(hwnd, name_taken_error(&parent, &stem, &extension));
        return;
    }
    if let Err(error) = crate::platform::files::rename_no_replace(&old, &new) {
        // A file may have taken the name after the check above; it is left alone.
        let error = if new.exists() && !case_only {
            name_taken_error(&parent, &stem, &extension)
        } else {
            format!("FastPad could not rename the file: {error}")
        };
        name_box_error(hwnd, error);
        return;
    }
    let rebound = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .tabs
            .rebind_path(id, new.clone())
            .is_ok()
    });
    if !rebound {
        // Undo, so the tab and the disk agree.
        let _ = crate::platform::files::rename_no_replace(&new, &old);
        name_box_error(hwnd, "Another tab already has that file open.".to_owned());
        return;
    }
    with_state(hwnd, |state| state.rename_note(&old, &new));
    schedule_write(hwnd);
    close_name_box(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    // The extension may have changed, and with it the language.
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
}

/// Note: Delete: after a confirm, sends the file to the Recycle Bin and closes its tab. Any
/// record stays, flagged deleted and marked missing, so the 30-day purge removes it.
pub(crate) fn delete_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    let Some(id) =
        unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
    else {
        return;
    };
    let question = format!(
        "Move \u{201c}{}\u{201d} to the Recycle Bin?",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    if !confirmed(hwnd, &question) {
        return;
    }
    if let Err(error) = crate::platform::files::recycle(&path) {
        push_notice(
            hwnd,
            format!("FastPad could not delete {}: {error}", path.display()),
        );
        return;
    }
    let now = library::now_unix();
    let record = with_state(hwnd, |state| state.record_for(&path).map(|r| r.id)).flatten();
    if let Some(note_id) = record {
        report(
            hwnd,
            apply_op(hwnd, |state, ids| {
                Some(PendingOp::SetDeleted {
                    note: state.note_ref(ids, &path),
                    value: true,
                })
            }),
        );
        with_state(hwnd, |state| state.local.set_missing(note_id, now));
    }
    with_state(hwnd, |state| state.remove_note(&path));
    super::main_window::close_document_without_prompt(hwnd, id);
}

/// Browse… in the name box: the system Save As dialog, starting in the folder.
pub(crate) fn name_box_browse(hwnd: HWND) {
    let Some((NamePurpose::FirstSave(id), _)) = name_box_state(hwnd) else {
        return;
    };
    close_name_box(hwnd);
    if super::main_window::activate_document_by_id(hwnd, id) {
        let _ = super::main_window::save_active_document_as(hwnd);
    }
}

fn folder_autosave(hwnd: HWND) -> bool {
    host(hwnd, |host| {
        // Until the folder's state has loaded, its autosave setting is unknown: do not save.
        host.state
            .as_ref()
            .is_some_and(|state| state.local.autosave)
    })
    .unwrap_or(false)
}

/// The active tab's path, if autosave applies to it right now.
fn autosave_target(hwnd: HWND) -> Option<PathBuf> {
    if !notes_mode(hwnd)
        || !folder_autosave(hwnd)
        || super::main_window::file_population_active(hwnd)
    {
        return None;
    }
    let folder = folder(hwnd)?;
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        let path = active.path.clone()?;
        (active.dirty && !active.autosave_paused && library::is_inside(&folder, &path))
            .then_some(path)
    })
}

/// After an edit: (re)starts the idle timer when the active tab would autosave.
pub(crate) fn schedule_autosave(hwnd: HWND) {
    if autosave_target(hwnd).is_some() {
        unsafe {
            SetTimer(hwnd, AUTOSAVE_TIMER_ID, AUTOSAVE_DELAY_MS, None);
        }
    }
}

/// Saves the active tab if autosave applies to it, unless its file changed on disk since FastPad
/// loaded or saved it: then autosave pauses for that tab until the user reloads or keeps theirs.
/// Only a successful write marks the tab clean.
pub(crate) fn autosave_active(hwnd: HWND) -> Autosave {
    unsafe {
        KillTimer(hwnd, AUTOSAVE_TIMER_ID);
    }
    let Some(path) = autosave_target(hwnd) else {
        return Autosave::NotEligible;
    };
    // The guard runs right before the write: nothing between here and `complete_save` yields.
    let known =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.active()?.disk_stamp);
    let now = library::disk_stamp(&path);
    // No known stamp (a tab restored from a snapshot): FastPad cannot tell whether the file
    // changed, or was deleted on purpose, so it is treated as changed and never re-created.
    let changed = known.is_none() || known != now;
    if changed {
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            if let Some(id) = app.tabs.active().map(|document| document.id)
                && let Some(document) = app.tabs.document_mut(id)
            {
                document.autosave_paused = true;
            }
        }
        push_notice(
            hwnd,
            format!(
                "{} changed on disk. Autosave is paused for it: use Note: Reload from disk or Note: Keep my version.",
                title::note_title(&path)
            ),
        );
        return Autosave::Paused;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return Autosave::Failed;
    };
    // Quiet: on failure the named notice below replaces the generic save-failure one.
    if super::main_window::complete_autosave(hwnd, &identity) {
        Autosave::Saved
    } else {
        if identity.is_live_for(hwnd) {
            push_notice(
                hwnd,
                format!(
                    "Autosave failed for {}. Your text is kept in recovery and FastPad will try again.",
                    title::note_title(&path)
                ),
            );
        }
        Autosave::Failed
    }
}

/// Before the window closes: save every eligible dirty tab. Failures fall back to the prompt.
pub(crate) fn autosave_all(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = folder(hwnd) else {
        return;
    };
    if !notes_mode(hwnd) || !folder_autosave(hwnd) {
        return;
    }
    let (active, ids): (Option<_>, Vec<_>) = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let ids = tabs
                .documents()
                .filter(|document| {
                    document.dirty
                        && !document.autosave_paused
                        && document
                            .path
                            .as_deref()
                            .is_some_and(|path| library::is_inside(&folder, path))
                })
                .map(|document| document.id)
                .collect();
            (tabs.active().map(|document| document.id), ids)
        })
        .unwrap_or_default();
    for id in ids {
        if !identity.is_live_for(hwnd) || !super::main_window::activate_document_by_id(hwnd, id) {
            return;
        }
        autosave_active(hwnd);
    }
    // The session records the tab the user had active, not the last one saved.
    if let Some(active) = active
        && identity.is_live_for(hwnd)
    {
        super::main_window::activate_document_by_id(hwnd, active);
    }
}

pub(crate) fn toggle_folder_autosave(hwnd: HWND) {
    let Some(enabled) = with_state(hwnd, |state| {
        state.local.autosave = !state.local.autosave;
        library::write_local(state);
        state.local.autosave
    }) else {
        push_notice(hwnd, "Loading folder…".to_owned());
        return;
    };
    if !enabled {
        unsafe {
            KillTimer(hwnd, AUTOSAVE_TIMER_ID);
        }
    }
    push_notice(
        hwnd,
        if enabled {
            "Autosave is on for this folder.".to_owned()
        } else {
            "Autosave is off for this folder. Use Ctrl+S to save.".to_owned()
        },
    );
}

/// Replaces the tab's text with the file on disk and resumes autosave.
pub(crate) fn reload_from_disk(hwnd: HWND) {
    let Some(path) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone())
    else {
        return;
    };
    // Read before the load: a change that lands during it then still pauses the next autosave.
    let stamp = library::disk_stamp(&path);
    let loaded = crate::file::loader::load(&path).and_then(|loaded| {
        // A NUL byte cannot round-trip through Scintilla's UTF-8 buffer: the file is unsupported.
        std::ffi::CString::new(loaded.text.as_str())
            .map_err(|_| crate::FastPadError::UnsupportedEncoding)?;
        Ok(loaded)
    });
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            super::main_window::report_open_failure(hwnd, &path, &error);
            return;
        }
    };
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    if editor.set_text(&loaded.text).is_err() || !identity.is_live_for(hwnd) {
        return;
    }
    editor.set_save_point();
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.tabs.set_active_dirty(false);
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
        {
            document.encoding = loaded.encoding;
            document.disk_stamp = stamp;
            document.autosave_paused = false;
        }
    }
    super::main_window::remove_saved_document_snapshots(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
}

/// Saves the tab over the changed file and resumes autosave.
pub(crate) fn keep_mine(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // An untitled tab has no file on disk to keep its text over.
    let has_path = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .active()
            .is_some_and(|d| d.path.is_some())
    });
    if !has_path {
        return;
    }
    let _ = super::main_window::complete_save(hwnd, &identity, None);
}

/// After a file is opened into a tab: remember its disk stamp, read before the load so a change
/// landing during it still pauses the next autosave, and a note among recent notes.
pub(crate) fn document_loaded(hwnd: HWND, stamp: Option<library::DiskStamp>) {
    let path = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.tabs.active()?.id;
        let document = app.tabs.document_mut(id)?;
        let path = document.path.clone()?;
        document.disk_stamp = stamp;
        Some(path)
    });
    if let Some(path) = path
        && folder(hwnd).is_some_and(|folder| library::is_inside(&folder, &path))
    {
        with_state(hwnd, |state| {
            let record = library::record_path(&state.folder, &path);
            state.local.note_opened(&record, library::now_unix());
        });
    }
}

/// After any successful save: remember the file's disk stamp and index it if it is a note.
pub(crate) fn document_saved(hwnd: HWND) {
    let path = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.tabs.active()?.id;
        let document = app.tabs.document_mut(id)?;
        let path = document.path.clone()?;
        document.disk_stamp = library::disk_stamp(&path);
        document.autosave_paused = false;
        Some(path)
    });
    if let Some(path) = path {
        with_state(hwnd, |state| state.add_note(&path));
    }
    close_stale_name_box(hwnd);
}

#[cfg(test)]
pub(crate) fn install_for_test(hwnd: HWND, state: LibraryState) {
    host(hwnd, |host| {
        host.folder = Some(state.folder.clone());
        host.generation = host.generation.wrapping_add(1);
    });
    install(hwnd, state);
}

pub(crate) fn notes_mode_notice(enabled: bool) -> &'static str {
    if enabled {
        "Notes mode is on. The open folder is your note library."
    } else {
        "Notes mode is off. FastPad works as a plain file editor."
    }
}

const READ_ONLY: &str = "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so notebooks and tags are read-only.";

/// True when organizing can proceed; otherwise explains why not.
pub(crate) fn ready_library(hwnd: HWND) -> bool {
    match host(hwnd, |host| host.state.as_ref().map(|state| state.metadata)).flatten() {
        Some(Metadata::Ready) => true,
        Some(Metadata::Unreadable) => {
            push_notice(hwnd, READ_ONLY.to_owned());
            false
        }
        None => {
            let notice = if folder(hwnd).is_some() {
                "Loading folder…"
            } else {
                "Open a folder first."
            };
            push_notice(hwnd, notice.to_owned());
            false
        }
    }
}

/// The active tab's file; for an untitled tab, explains that it must be saved first.
pub(crate) fn active_file(hwnd: HWND) -> Option<PathBuf> {
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    if path.is_none() {
        push_notice(hwnd, "Save this note first to organize it.".to_owned());
    }
    path
}

/// Applies one operation built from the host's ID source, then schedules the write. The model
/// validates before the operation is recorded, so a failed one leaves nothing pending.
fn apply_op(
    hwnd: HWND,
    build: impl FnOnce(&mut LibraryState, &mut IdSource) -> Option<PendingOp>,
) -> Result<(), LibraryError> {
    let result = host(hwnd, |host| {
        let LibraryHost { state, ids, .. } = host;
        let state = state.as_mut()?;
        let op = build(state, ids)?;
        Some(state.apply(op))
    })
    .flatten()
    .unwrap_or(Ok(()));
    if result.is_ok() {
        schedule_write(hwnd);
    }
    result
}

fn report(hwnd: HWND, result: Result<(), LibraryError>) {
    if let Err(error) = result {
        push_notice(hwnd, error.to_string());
    }
}

type Row = (Option<Target>, String);

fn notebook_rows(hwnd: HWND) -> Vec<Row> {
    with_state(hwnd, |state| {
        state
            .library
            .notebooks_in_order()
            .iter()
            .map(|n| (Some(Target::Notebook(n.id)), n.name.clone()))
            .collect()
    })
    .unwrap_or_default()
}

/// Every tag, the most used first, then by name.
fn tag_rows(hwnd: HWND) -> Vec<Row> {
    with_state(hwnd, |state| {
        let mut tags: Vec<_> = state
            .library
            .tags
            .iter()
            .map(|t| (t.id, t.name.clone()))
            .collect();
        tags.sort_by(|a, b| {
            state
                .library
                .tag_count(b.0)
                .cmp(&state.library.tag_count(a.0))
                .then_with(|| a.1.cmp(&b.1))
        });
        tags.into_iter()
            .map(|(id, name)| (Some(Target::Tag(id)), name))
            .collect()
    })
    .unwrap_or_default()
}

fn note_tags(hwnd: HWND, path: &Path) -> Vec<TagId> {
    with_state(hwnd, |state| {
        state
            .record_for(path)
            .map_or_else(Vec::new, |record| record.tags.clone())
    })
    .unwrap_or_default()
}

fn note_tag_rows(hwnd: HWND, path: &Path) -> Vec<Row> {
    with_state(hwnd, |state| {
        state.record_for(path).map_or_else(Vec::new, |record| {
            record
                .tags
                .iter()
                .filter_map(|id| {
                    Some((Some(Target::Tag(*id)), state.library.tag(*id)?.name.clone()))
                })
                .collect()
        })
    })
    .unwrap_or_default()
}

/// "Notes", then every notebook.
fn move_rows(hwnd: HWND) -> Vec<Row> {
    let mut rows = vec![(None, "Notes".to_owned())];
    rows.extend(notebook_rows(hwnd));
    rows
}

/// The tags the note does not have yet.
fn add_tag_rows(hwnd: HWND, path: &Path) -> Vec<Row> {
    let on_note = note_tags(hwnd, path);
    tag_rows(hwnd)
        .into_iter()
        .filter(|(target, _)| !matches!(target, Some(Target::Tag(id)) if on_note.contains(id)))
        .collect()
}

/// Opens a picker over `rows` and remembers what each row stands for.
fn picker(hwnd: HWND, kind: PickerKind, rows: Vec<Row>, create: Option<&'static str>) {
    let (targets, items): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    host(hwnd, |host| host.shown_targets = Some((kind, targets)));
    super::main_window::open_picker(
        hwnd,
        Picker {
            kind,
            items,
            create,
        },
    );
}

/// What row `index` of a `kind` picker stands for: the row that picker showed, or, when no such
/// picker was shown, the row it would show now (`rows`). `None` when the row does not exist.
fn shown_row(
    hwnd: HWND,
    kind: PickerKind,
    index: usize,
    rows: impl FnOnce() -> Vec<Row>,
) -> Option<Option<Target>> {
    let shown = host(hwnd, |host| host.shown_targets.take()).flatten();
    let targets = match shown {
        Some((shown_kind, targets)) if shown_kind == kind => targets,
        _ => rows().into_iter().map(|(target, _)| target).collect(),
    };
    targets.get(index).copied()
}

fn shown_notebook(hwnd: HWND, kind: PickerKind, index: usize) -> Option<(NotebookId, String)> {
    let Some(Some(Target::Notebook(id))) = shown_row(hwnd, kind, index, || notebook_rows(hwnd))
    else {
        return None;
    };
    let name = with_state(hwnd, |state| Some(state.library.notebook(id)?.name.clone())).flatten();
    if name.is_none() {
        report(hwnd, Err(LibraryError::NotFound));
    }
    Some((id, name?))
}

fn shown_tag(
    hwnd: HWND,
    kind: PickerKind,
    index: usize,
    rows: impl FnOnce() -> Vec<Row>,
) -> Option<(TagId, String)> {
    let Some(Some(Target::Tag(id))) = shown_row(hwnd, kind, index, rows) else {
        return None;
    };
    let name = with_state(hwnd, |state| Some(state.library.tag(id)?.name.clone())).flatten();
    if name.is_none() {
        report(hwnd, Err(LibraryError::NotFound));
    }
    Some((id, name?))
}

/// Asks `question`; false also when the window went away meanwhile.
fn confirmed(hwnd: HWND, question: &str) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    crate::window::modal::confirm(hwnd, question) && identity.is_live_for(hwnd)
}

fn toggle_flag(hwnd: HWND, favorite: bool) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    let mut now_on = false;
    let result = apply_op(hwnd, |state, ids| {
        let current = state
            .record_for(&path)
            .is_some_and(|r| if favorite { r.favorite } else { r.pinned });
        now_on = !current;
        let note = state.note_ref(ids, &path);
        Some(if favorite {
            PendingOp::SetFavorite {
                note,
                value: now_on,
            }
        } else {
            PendingOp::SetPinned {
                note,
                value: now_on,
            }
        })
    });
    if result.is_err() {
        report(hwnd, result);
        return;
    }
    let notice = match (favorite, now_on) {
        (true, true) => "Added to Favorites.",
        (true, false) => "Removed from Favorites.",
        (false, true) => "Pinned.",
        (false, false) => "Unpinned.",
    };
    push_notice(hwnd, notice.to_owned());
}

/// Every organizing command.
pub(crate) fn organize(hwnd: HWND, command: CommandId) {
    if !ready_library(hwnd) {
        return;
    }
    match command {
        CommandId::NoteToggleFavorite => toggle_flag(hwnd, true),
        CommandId::NoteTogglePin => toggle_flag(hwnd, false),
        CommandId::NoteMoveToNotebook => {
            if active_file(hwnd).is_some() {
                let rows = move_rows(hwnd);
                picker(hwnd, PickerKind::MoveToNotebook, rows, Some("New notebook"));
            }
        }
        CommandId::NoteAddTag => {
            if let Some(path) = active_file(hwnd) {
                let rows = add_tag_rows(hwnd, &path);
                picker(hwnd, PickerKind::AddTag, rows, Some("Add tag"));
            }
        }
        CommandId::NoteRemoveTag => {
            let Some(path) = active_file(hwnd) else {
                return;
            };
            let rows = note_tag_rows(hwnd, &path);
            if rows.is_empty() {
                push_notice(hwnd, "This note has no tags.".to_owned());
                return;
            }
            picker(hwnd, PickerKind::RemoveTag, rows, None);
        }
        CommandId::NotebookNew => {
            open_name_box(
                hwnd,
                NamePurpose::NewNotebook { then_move: None },
                "",
                "New notebook".to_owned(),
                false,
            );
        }
        CommandId::NotebookRename | CommandId::NotebookChangeColor | CommandId::NotebookDelete => {
            let rows = notebook_rows(hwnd);
            if rows.is_empty() {
                push_notice(
                    hwnd,
                    "There are no notebooks yet. Use Notebook: New.".to_owned(),
                );
                return;
            }
            let kind = match command {
                CommandId::NotebookRename => PickerKind::RenameNotebook,
                CommandId::NotebookChangeColor => PickerKind::RecolorNotebook,
                _ => PickerKind::DeleteNotebook,
            };
            picker(hwnd, kind, rows, None);
        }
        CommandId::TagRename | CommandId::TagRemoveEverywhere => {
            let rows = tag_rows(hwnd);
            if rows.is_empty() {
                push_notice(hwnd, "There are no tags yet. Use Note: Add tag.".to_owned());
                return;
            }
            let kind = if command == CommandId::TagRename {
                PickerKind::RenameTag
            } else {
                PickerKind::RemoveTagEverywhere
            };
            picker(hwnd, kind, rows, None);
        }
        _ => {}
    }
}

#[cfg(test)]
thread_local! {
    static LAST_PICK: std::cell::RefCell<Option<(PickerKind, PickerChoice)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn take_last_pick() -> Option<(PickerKind, PickerChoice)> {
    LAST_PICK.with(|last| last.borrow_mut().take())
}

/// A picker row was chosen. Every kind resolves the row against what that picker showed.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    if kind != PickerKind::RecentFolder && !ready_library(hwnd) {
        host(hwnd, |host| {
            host.shown_targets = None;
            host.pending_target = None;
        });
        return;
    }
    match (kind, choice) {
        (PickerKind::RecentFolder, PickerChoice::Item(index)) => {
            let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
            if let Some(folder) = shown.unwrap_or_default().get(index) {
                open_folder(hwnd, folder);
            }
        }
        (PickerKind::MoveToNotebook, choice) => {
            let Some(path) = active_file(hwnd) else {
                return;
            };
            let notebook = match choice {
                PickerChoice::Item(index) => {
                    match shown_row(hwnd, kind, index, || move_rows(hwnd)) {
                        Some(None) => None,
                        Some(Some(Target::Notebook(id))) => Some(id),
                        _ => return,
                    }
                }
                PickerChoice::Create(name) => {
                    host(hwnd, |host| host.shown_targets = None);
                    let then_move = unsafe { app_ptr(hwnd) }
                        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id));
                    open_name_box(
                        hwnd,
                        NamePurpose::NewNotebook { then_move },
                        &name,
                        "New notebook".to_owned(),
                        false,
                    );
                    return;
                }
            };
            report(
                hwnd,
                apply_op(hwnd, |state, ids| {
                    Some(PendingOp::SetNoteNotebook {
                        note: state.note_ref(ids, &path),
                        notebook,
                    })
                }),
            );
        }
        (PickerKind::AddTag, choice) => {
            let Some(path) = active_file(hwnd) else {
                return;
            };
            let (tag, name) = match choice {
                PickerChoice::Create(name) => {
                    host(hwnd, |host| host.shown_targets = None);
                    (None, name)
                }
                PickerChoice::Item(index) => {
                    let Some((id, name)) =
                        shown_tag(hwnd, kind, index, || add_tag_rows(hwnd, &path))
                    else {
                        return;
                    };
                    (Some(id), name)
                }
            };
            report(
                hwnd,
                apply_op(hwnd, |state, ids| {
                    let tag = tag
                        .or_else(|| {
                            let bare = name.trim().trim_start_matches('#');
                            state.library.tag_by_name(bare).map(|t| t.id)
                        })
                        .unwrap_or_else(|| TagId(ids.next()));
                    Some(PendingOp::AddTag {
                        note: state.note_ref(ids, &path),
                        tag,
                        name,
                    })
                }),
            );
        }
        (PickerKind::RemoveTag, PickerChoice::Item(index)) => {
            let Some(path) = active_file(hwnd) else {
                return;
            };
            let Some(Some(Target::Tag(tag))) =
                shown_row(hwnd, kind, index, || note_tag_rows(hwnd, &path))
            else {
                return;
            };
            report(
                hwnd,
                apply_op(hwnd, |state, ids| {
                    Some(PendingOp::RemoveTag {
                        note: state.note_ref(ids, &path),
                        tag,
                    })
                }),
            );
        }
        (PickerKind::RenameNotebook, PickerChoice::Item(index)) => {
            let Some((id, name)) = shown_notebook(hwnd, kind, index) else {
                return;
            };
            open_name_box(
                hwnd,
                NamePurpose::RenameNotebook(id),
                &name,
                "Rename notebook".to_owned(),
                false,
            );
        }
        (PickerKind::RecolorNotebook, PickerChoice::Item(index)) => {
            let Some((id, _)) = shown_notebook(hwnd, kind, index) else {
                return;
            };
            host(hwnd, |host| {
                host.pending_target = Some(Target::Notebook(id))
            });
            let mut rows = vec![(None, "No color".to_owned())];
            rows.extend(
                NotebookColor::ALL
                    .iter()
                    .map(|c| (None, c.name().to_owned())),
            );
            picker(hwnd, PickerKind::ChooseColor, rows, None);
        }
        (PickerKind::ChooseColor, PickerChoice::Item(index)) => {
            host(hwnd, |host| host.shown_targets = None);
            let Some(Some(Target::Notebook(id))) = host(hwnd, |host| host.pending_target.take())
            else {
                return;
            };
            let color = index
                .checked_sub(1)
                .and_then(|i| NotebookColor::ALL.get(i).copied());
            report(
                hwnd,
                apply_op(hwnd, |_, _| {
                    Some(PendingOp::SetNotebookColor {
                        id,
                        color,
                        now: library::now_unix(),
                    })
                }),
            );
        }
        (PickerKind::DeleteNotebook, PickerChoice::Item(index)) => {
            let Some((id, name)) = shown_notebook(hwnd, kind, index) else {
                return;
            };
            let count = with_state(hwnd, |state| {
                state
                    .library
                    .notes
                    .iter()
                    .filter(|n| n.notebook == Some(id) && !n.deleted)
                    .count()
            })
            .unwrap_or(0);
            let question = format!(
                "Delete the notebook \u{201c}{name}\u{201d}? Its {count} notes move to Notes. No note is deleted."
            );
            if confirmed(hwnd, &question) {
                report(
                    hwnd,
                    apply_op(hwnd, |_, _| Some(PendingOp::DeleteNotebook { id })),
                );
            }
        }
        (PickerKind::RenameTag, PickerChoice::Item(index)) => {
            let Some((id, name)) = shown_tag(hwnd, kind, index, || tag_rows(hwnd)) else {
                return;
            };
            open_name_box(
                hwnd,
                NamePurpose::RenameTag(id),
                &name,
                "Rename tag".to_owned(),
                false,
            );
        }
        (PickerKind::RemoveTagEverywhere, PickerChoice::Item(index)) => {
            let Some((id, name)) = shown_tag(hwnd, kind, index, || tag_rows(hwnd)) else {
                return;
            };
            let count = with_state(hwnd, |state| state.library.tag_count(id)).unwrap_or(0);
            let question = format!("Remove the tag \u{201c}{name}\u{201d} from {count} notes?");
            if confirmed(hwnd, &question) {
                report(
                    hwnd,
                    apply_op(hwnd, |_, _| Some(PendingOp::RemoveTagEverywhere { id })),
                );
            }
        }
        _ => {}
    }
}
