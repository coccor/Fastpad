//! A notebook's conventions on the window side (second brain phase 1 spec §9): `notebook.ini`,
//! daily notes and note templates.

use super::*;
use crate::library::dates::{self, Date, Time};
use crate::library::notebook_ini::{self, NotebookSettings};
use crate::library::template::{self, Filled};
use crate::window::command_palette::{Picker, PickerKind};
use crate::window::main_window::push_notice;
use std::path::Path;

/// What the daily note commands say with no notebook open.
pub(crate) const NO_NOTEBOOK_DAILY: &str = "Open a notebook to use daily notes.";

#[cfg(test)]
thread_local! {
    static FIXED_NOW: std::cell::Cell<Option<(Date, Time)>> = const { std::cell::Cell::new(None) };
}

/// Makes "now" `date` at `time` for this thread's tests.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn set_now(date: Date, time: Time) {
    FIXED_NOW.with(|now| now.set(Some((date, time))));
}

fn now() -> (Date, Time) {
    #[cfg(test)]
    if let Some(now) = FIXED_NOW.with(std::cell::Cell::get) {
        return now;
    }
    crate::platform::clock::local_now()
}

/// The open notebook's conventions: its `notebook.ini`, or the defaults.
pub(crate) fn notebook_settings(hwnd: HWND) -> NotebookSettings {
    host(hwnd, |host| host.notebook.clone()).unwrap_or_default()
}

/// Reads the open notebook's `notebook.ini` again (on load, after a rescan, and when the watcher
/// reports it). A file whose text changed reports its bad lines, once.
pub(crate) fn reload_notebook_settings(hwnd: HWND) {
    let Some(root) = folder(hwnd) else {
        return;
    };
    let text = std::fs::read(notebook_ini::notebook_file(&root))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let changed = host(hwnd, |host| {
        if host.notebook_text == text {
            return false;
        }
        host.notebook_text = text.clone();
        true
    })
    .unwrap_or(false);
    if !changed {
        return;
    }
    let (settings, warnings) = notebook_ini::parse(text.as_deref().unwrap_or_default());
    host(hwnd, |host| host.notebook = settings);
    for warning in warnings {
        push_notice(
            hwnd,
            format!("notebook.ini line {}: {}", warning.line, warning.message),
        );
    }
    crate::window::side_panel::index_changed(hwnd);
}

/// Forgets the notebook's conventions: it closed or another opened.
pub(crate) fn forget_notebook_settings(hwnd: HWND) {
    host(hwnd, |host| {
        host.notebook = NotebookSettings::default();
        host.notebook_text = None;
        host.shown_templates = None;
    });
}

/// Notes: Edit notebook settings: opens `.fastpad\notebook.ini`, made first with every setting
/// commented out at its default when the notebook has none.
pub(crate) fn edit_notebook_settings(hwnd: HWND) {
    let Some(root) = folder(hwnd) else {
        push_notice(hwnd, "Open a notebook first.".to_owned());
        return;
    };
    let path = notebook_ini::notebook_file(&root);
    if !path.exists() {
        let made = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(crate::FastPadError::Io)
            .and_then(|()| {
                crate::file::saver::save_atomic_new(&path, notebook_ini::starter_text().as_bytes())
            });
        if let Err(error) = made {
            push_notice(
                hwnd,
                format!("FastPad could not create notebook.ini: {error}"),
            );
            return;
        }
    }
    if let Err(error) = crate::window::main_window::open_note(
        hwnd,
        &path,
        crate::window::main_window::OpenMode::Permanent,
        true,
    ) {
        crate::window::main_window::report_open_failure(hwnd, &path, &error);
    }
}

/// Puts the active editor's caret at byte `position`, and scrolls to it.
pub(crate) fn place_caret(hwnd: HWND, position: usize) {
    let Some(editor) = (unsafe { crate::window::main_window::app_ptr(hwnd) })
        .and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    if editor.set_selection(position..position).is_ok() {
        editor.scroll_caret_into_view();
    }
}

/// `template` (an absolute path) filled in for a new note named `title`.
pub(crate) fn fill_template(hwnd: HWND, template: &Path, title: &str) -> Result<Filled, String> {
    let file = crate::file::loader::load(template).map_err(|error| {
        format!(
            "FastPad could not read the template {}: {error}",
            template.display()
        )
    })?;
    let settings = notebook_settings(hwnd);
    let (date, time) = now();
    Ok(template::fill(
        &file.text,
        &template::Context {
            date,
            time,
            daily_format: &settings.daily_format,
            title,
        },
    ))
}

/// Writes `filled` as the new note at `path` (never over a file), lists it and opens it with
/// the caret in place. The error is for a notice.
fn create_note(hwnd: HWND, path: &Path, filled: &Filled) -> Result<(), String> {
    let made = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(crate::FastPadError::Io)
        .and_then(|()| crate::file::saver::save_atomic_new(path, filled.text.as_bytes()));
    if let Err(error) = made {
        return Err(format!(
            "FastPad could not create {}: {error}",
            path.display()
        ));
    }
    with_state(hwnd, |state| state.add_note(path));
    crate::window::side_panel::refresh(hwnd);
    if let Err(error) = crate::window::main_window::open_note(
        hwnd,
        path,
        crate::window::main_window::OpenMode::Permanent,
        true,
    ) {
        crate::window::main_window::report_open_failure(hwnd, path, &error);
        return Ok(());
    }
    place_caret(hwnd, filled.cursor.unwrap_or(filled.text.len()));
    Ok(())
}

/// The daily note for `date` in the notebook at `root`.
fn daily_path(root: &Path, settings: &NotebookSettings, date: Date) -> PathBuf {
    let name = crate::library::title::file_name(&dates::format(date, &settings.daily_format), "md");
    root.join(&settings.daily_folder).join(name)
}

/// Notes: Open today's note (spec §9.2): opens it, or makes it from the daily template.
pub(crate) fn open_today(hwnd: HWND) {
    let Some(root) = folder(hwnd) else {
        push_notice(hwnd, NO_NOTEBOOK_DAILY.to_owned());
        return;
    };
    let settings = notebook_settings(hwnd);
    let (date, _) = now();
    let path = daily_path(&root, &settings, date);
    if path.exists() {
        links::open_at_heading(hwnd, &path, None);
        return;
    }
    let template = root.join(&settings.daily_template);
    let title = dates::format(date, &settings.daily_format);
    let filled = if template.is_file() {
        match fill_template(hwnd, &template, &title) {
            Ok(filled) => filled,
            Err(error) => {
                push_notice(hwnd, error);
                template::default_daily(date)
            }
        }
    } else {
        if settings.daily_template != NotebookSettings::default().daily_template {
            push_notice(
                hwnd,
                format!(
                    "The daily note template {} doesn't exist, so today's note starts with its date.",
                    settings.daily_template.display()
                ),
            );
        }
        template::default_daily(date)
    };
    if let Err(error) = create_note(hwnd, &path, &filled) {
        push_notice(hwnd, error);
    }
}

/// The daily notes the notebook lists, by date.
fn daily_notes(hwnd: HWND, settings: &NotebookSettings) -> Vec<(Date, PathBuf)> {
    let mut found: Vec<(Date, PathBuf)> = with_state(hwnd, |state| {
        state
            .notes
            .iter()
            .filter(|note| {
                note.path.parent() == Some(settings.daily_folder.as_path())
                    && note
                        .path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            })
            .filter_map(|note| {
                let stem = note.path.file_stem()?.to_string_lossy();
                Some((
                    dates::parse(&stem, &settings.daily_format)?,
                    note.path.clone(),
                ))
            })
            .collect()
    })
    .unwrap_or_default();
    found.sort();
    found
}

/// Notes: Previous daily note / Next daily note (spec §9.2): the nearest daily note before or
/// after the active one, or today when the active tab isn't a daily note.
pub(crate) fn step_daily(hwnd: HWND, forward: bool) {
    let Some(root) = folder(hwnd) else {
        push_notice(hwnd, NO_NOTEBOOK_DAILY.to_owned());
        return;
    };
    let settings = notebook_settings(hwnd);
    let notes = daily_notes(hwnd, &settings);
    let active = unsafe { crate::window::main_window::app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    let from = active
        .and_then(|path| {
            notes
                .iter()
                .find(|(_, note)| crate::library::model::same_path(&root.join(note), &path))
                .map(|(date, _)| *date)
        })
        .unwrap_or_else(|| now().0);
    let next = if forward {
        notes.iter().find(|(date, _)| *date > from)
    } else {
        notes.iter().rev().find(|(date, _)| *date < from)
    };
    match next {
        Some((_, note)) => links::open_at_heading(hwnd, &root.join(note), None),
        None => push_notice(
            hwnd,
            if forward {
                "There is no later daily note."
            } else {
                "There is no earlier daily note."
            }
            .to_owned(),
        ),
    }
}

/// The templates in the notebook's templates folder (absolute paths), by name.
fn templates(root: &Path, settings: &NotebookSettings) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root.join(&settings.templates_folder))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path.extension().is_some_and(|extension| {
                            ["md", "markdown", "txt"]
                                .iter()
                                .any(|known| extension.eq_ignore_ascii_case(known))
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by(|a, b| {
        crate::library::tree::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())
    });
    found
}

/// Notes: New note from template… (spec §9.4): a picker of the templates, for a new note in
/// `parent` (relative), or the selected folder when `None`.
pub(crate) fn new_note_from_template(hwnd: HWND, parent: Option<PathBuf>) {
    if !ready_library(hwnd) {
        return;
    }
    let Some(root) = folder(hwnd) else {
        return;
    };
    let settings = notebook_settings(hwnd);
    let found = templates(&root, &settings);
    if found.is_empty() {
        push_notice(
            hwnd,
            format!(
                "No templates in {}. Add .md files there to use them.",
                settings.templates_folder.display()
            ),
        );
        return;
    }
    let items = found
        .iter()
        .map(|path| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    host(hwnd, |host| host.shown_templates = Some((parent, found)));
    crate::window::main_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::Template,
            items,
            create: None,
        },
    );
}

/// A template row was chosen: the field for the new note's name.
pub(super) fn template_picked(hwnd: HWND, index: usize) {
    let Some((parent, found)) = host(hwnd, |host| host.shown_templates.take()).flatten() else {
        return;
    };
    if let Some(template) = found.get(index) {
        crate::window::inline_name::new_note_from(hwnd, parent, template.clone());
    }
}
