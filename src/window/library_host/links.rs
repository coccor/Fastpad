//! Links on the window side (second brain phase 1 spec §7): resolving with the live note list
//! and index, following a link, creating the note a link names, and writing link updates.

use super::*;
use crate::library::links::{NoteEdits, Resolver};
use crate::window::main_window::{OpenMode, push_notice};
use std::path::Path;

/// A resolver over the open notebook's notes and the index's aliases, or `None` without a
/// loaded notebook.
pub(crate) fn resolver(hwnd: HWND) -> Option<Resolver> {
    let index = note_index(hwnd);
    let templates = notebook_settings(hwnd).templates_folder;
    with_state(hwnd, |state| {
        Resolver::of(
            state
                .notes
                .iter()
                .map(|note| note.path.as_path())
                .filter(|path| !crate::library::links::in_folder(path, &templates)),
            &index,
        )
    })
}

/// What `[[` completion offers (spec §7.4): every note, by the shortest target that reaches it
/// from the note at `from` (absolute), sorted.
pub(crate) fn completion_items(hwnd: HWND, from: Option<&Path>) -> Vec<String> {
    let (Some(root), Some(resolver)) = (folder(hwnd), resolver(hwnd)) else {
        return Vec::new();
    };
    let from = from
        .and_then(|from| from.strip_prefix(&root).ok())
        .map_or_else(|| PathBuf::from("_"), Path::to_path_buf);
    let mut items: Vec<String> = note_paths(hwnd)
        .iter()
        .filter(|path| !library::model::same_path(path, &from) && resolver.contains(path))
        .map(|path| crate::library::links::wiki_target(&resolver, path, &from))
        .collect();
    items.sort_by(|a, b| crate::library::tree::natural_cmp(a, b));
    items.dedup();
    items
}

/// The open notebook's note paths, relative.
pub(crate) fn note_paths(hwnd: HWND) -> Vec<PathBuf> {
    with_state(hwnd, |state| {
        state.notes.iter().map(|note| note.path.clone()).collect()
    })
    .unwrap_or_default()
}

/// Whether a file is at `relative` in the open notebook: link updates keep links to files the
/// notebook doesn't list, such as PDFs (spec §10.5).
fn root_file_exists(hwnd: HWND, relative: &Path) -> bool {
    folder(hwnd).is_some_and(|root| root.join(relative).is_file())
}

/// Follows the wikilink `target` (with `heading`, if any) written in the note at `from`
/// (absolute; `None` for a tab outside the notebook): opens the note it names, else offers to
/// create it (spec §7.3).
pub(crate) fn follow_wiki(hwnd: HWND, from: Option<&Path>, target: &str, heading: Option<&str>) {
    let Some(root) = folder(hwnd) else {
        push_notice(hwnd, "Open a notebook to follow links.".to_owned());
        return;
    };
    let from = from
        .and_then(|from| from.strip_prefix(&root).ok())
        .map_or_else(|| PathBuf::from("_"), Path::to_path_buf);
    let found = resolver(hwnd).and_then(|resolver| resolver.resolve_wiki(target, &from));
    match found {
        Some(note) => open_at_heading(hwnd, &root.join(note), heading),
        None => create_linked_note(hwnd, target),
    }
}

/// Opens `path` in a tab and, with `heading`, puts the caret on the first heading line whose
/// text matches it, ignoring case.
pub(crate) fn open_at_heading(hwnd: HWND, path: &Path, heading: Option<&str>) {
    if let Err(error) = crate::window::main_window::open_note(hwnd, path, OpenMode::Permanent, true)
    {
        crate::window::main_window::report_open_failure(hwnd, path, &error);
        return;
    }
    let Some(heading) = heading.map(str::trim).filter(|heading| !heading.is_empty()) else {
        return;
    };
    let Some(editor) = (unsafe { crate::window::main_window::app_ptr(hwnd) })
        .and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let line = editor
        .with_document_text(|text| {
            text.lines().position(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with('#')
                    && trimmed
                        .trim_start_matches('#')
                        .trim()
                        .eq_ignore_ascii_case(heading)
            })
        })
        .ok()
        .flatten();
    if let Some(line) = line
        && let Ok(start) = editor.line_start(line)
    {
        let _ = editor.set_selection(start..start);
        editor.scroll_caret_into_view();
    }
}

/// Creates `<target>.md` in the notebook root (a path form creates its folders) and opens it
/// (spec §7.3). The name is sanitized as a first save's is, and never overwrites.
pub(crate) fn create_linked_note(hwnd: HWND, target: &str) {
    let Some(root) = folder(hwnd) else {
        push_notice(hwnd, "Open a notebook to create linked notes.".to_owned());
        return;
    };
    let parts: Vec<String> = target
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(crate::library::title::sanitize_stem)
        .collect();
    let Some((stem, folders)) = parts.split_last() else {
        return;
    };
    let mut path = root.clone();
    for folder in folders {
        path.push(folder);
    }
    let name = if crate::library::title::is_note_extension(
        Path::new(stem)
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_default()
            .as_str(),
    ) {
        stem.clone()
    } else {
        crate::library::title::file_name(stem, "md")
    };
    path.push(name);
    if !path.exists() {
        let created = path
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
            .and_then(|_| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map(|_| ())
            });
        if let Err(error) = created {
            push_notice(
                hwnd,
                format!("FastPad could not create {}: {error}", path.display()),
            );
            return;
        }
        with_state(hwnd, |state| state.add_note(&path));
        crate::window::side_panel::refresh(hwnd);
    }
    open_at_heading(hwnd, &path, None);
}

/// What writing link edits did.
#[derive(Debug, Default)]
pub(crate) struct LinksWritten {
    pub links: usize,
    pub notes: usize,
    /// Notes left alone, by name, with why.
    pub skipped: Vec<String>,
}

/// Writes `edits` (spec §7.6). Each note is rewritten on disk only if it still has the size and
/// write time the index read; a note open in a tab with unsaved edits is left alone, since its
/// text no longer matches the ranges. Open clean tabs then take the change as one undoable edit
/// (spec §5.3). The writes are FastPad's own, so they mark nothing.
pub(crate) fn write_link_edits(hwnd: HWND, edits: &[NoteEdits]) -> LinksWritten {
    let mut result = LinksWritten::default();
    let Some(root) = folder(hwnd) else {
        return result;
    };
    let dirty: Vec<PathBuf> = unsafe { crate::window::main_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| document.dirty)
                .filter_map(|document| document.path.clone())
                .collect()
        })
        .unwrap_or_default();
    let paths: Vec<PathBuf> = edits.iter().map(|edit| root.join(&edit.note)).collect();
    begin_own_writes(hwnd, paths.iter().map(PathBuf::as_path));
    for (edit, path) in edits.iter().zip(&paths) {
        let name = crate::library::title::note_title(path);
        if dirty
            .iter()
            .any(|open| library::model::same_path(open, path))
        {
            result.skipped.push(format!("{name} (unsaved edits)"));
            continue;
        }
        match rewrite(path, edit) {
            Ok(()) => {
                result.links += edit.edits.len();
                result.notes += 1;
                with_state(hwnd, |state| state.add_note(path));
            }
            Err(reason) => result.skipped.push(format!("{name} ({reason})")),
        }
    }
    end_own_writes(hwnd, paths.iter().map(PathBuf::as_path));
    // Open clean tabs show the new text as one undoable edit.
    check_paths(hwnd, &paths);
    request_index(hwnd);
    result
}

fn rewrite(path: &Path, edit: &NoteEdits) -> Result<(), &'static str> {
    let stamp = library::disk_stamp(path).ok_or("gone")?;
    if stamp.size != edit.size || library::filetime_ticks(stamp.modified) != edit.mtime {
        return Err("changed since it was read");
    }
    let file = crate::file::loader::load(path).map_err(|_| "unreadable")?;
    let mut text = file.text;
    for (range, replacement) in edit.edits.iter().rev() {
        if range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err("changed since it was read");
        }
        text.replace_range(range.clone(), replacement);
    }
    crate::file::saver::save_atomic(path, &crate::file::encoding::encode(&text, file.encoding))
        .map_err(|_| "could not be written")
}

/// After FastPad renamed or moved notes (`moves`, relative old and new paths; a folder move
/// lists every note under it): a moved note's own relative links are fixed at once, and links
/// in other notes that would no longer reach it are offered for updating (spec §7.6, §10.5).
/// A single Markdown note renamed to another name may keep its old name as an alias.
pub(crate) fn notes_moved(hwnd: HWND, moves: &[(PathBuf, PathBuf)]) {
    if moves.is_empty() || !index_loaded(hwnd) {
        return;
    }
    let index = note_index(hwnd);
    let notes = note_paths(hwnd);
    let edits = crate::library::links::edits_for_moves_with(&index, &notes, moves, &|path| {
        root_file_exists(hwnd, path)
    });
    let (own, inbound): (Vec<NoteEdits>, Vec<NoteEdits>) =
        edits.into_iter().partition(|edit| edit.own);
    let mut written = LinksWritten::default();
    if !own.is_empty() {
        written = write_link_edits(hwnd, &own);
    }
    if inbound.is_empty() {
        return;
    }
    let links: usize = inbound.iter().map(|edit| edit.edits.len()).sum();
    let renamed = match moves {
        [(old, new)]
            if crate::library::index::is_indexed(new) && old.file_stem() != new.file_stem() =>
        {
            old.file_stem()
                .map(|stem| (new.clone(), stem.to_string_lossy().into_owned()))
        }
        _ => None,
    };
    let target = match moves {
        [(_, new)] => format!("\u{201C}{}\u{201D}", crate::library::title::note_title(new)),
        _ => "the moved notes".to_owned(),
    };
    let question = format!(
        "Update {links} {} in {} {} to point to {target}?",
        if links == 1 { "link" } else { "links" },
        inbound.len(),
        if inbound.len() == 1 { "note" } else { "notes" },
    );
    let mut buttons = Vec::new();
    if renamed.is_some() {
        buttons.push("Update links and keep alias");
    }
    buttons.push("Update links");
    buttons.push("Leave them");
    let choice = crate::window::modal::choose(hwnd, &question, &buttons);
    if choice + 1 == buttons.len() {
        return;
    }
    let more = write_link_edits(hwnd, &inbound);
    written.links += more.links;
    written.notes += more.notes;
    written.skipped.extend(more.skipped);
    if choice == 0
        && let Some((note, alias)) = renamed
        && let Some(root) = folder(hwnd)
    {
        add_alias_to(hwnd, &root.join(note), &alias);
    }
    push_notice(hwnd, links_written_notice(&written));
}

/// Adds `alias` to the note at `path`'s front matter, unless the note is open with unsaved edits
/// or its aliases have a form FastPad doesn't edit.
fn add_alias_to(hwnd: HWND, path: &Path, alias: &str) {
    let dirty = unsafe { crate::window::main_window::app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .documents()
            .any(|document| document.dirty && document.path.as_deref() == Some(path))
    });
    let Ok(file) = crate::file::loader::load(path) else {
        return;
    };
    let added = (!dirty)
        .then(|| crate::library::frontmatter::add_alias(&file.text, alias))
        .flatten();
    let Some(text) = added else {
        push_notice(
            hwnd,
            format!(
                "\u{201C}{alias}\u{201D} wasn't added as an alias of {}: edit its front matter by hand.",
                crate::library::title::note_title(path)
            ),
        );
        return;
    };
    begin_own_writes(hwnd, [path]);
    let saved =
        crate::file::saver::save_atomic(path, &crate::file::encoding::encode(&text, file.encoding));
    end_own_writes(hwnd, [path]);
    if saved.is_ok() {
        with_state(hwnd, |state| state.add_note(path));
        check_paths(hwnd, &[path.to_path_buf()]);
        request_index(hwnd);
    }
}

/// The edits that would point links in other notes at the notes renamed outside FastPad.
fn outside_rename_edits(hwnd: HWND) -> Vec<NoteEdits> {
    let moves = host(hwnd, |host| host.renamed_outside.clone()).unwrap_or_default();
    if moves.is_empty() || !index_loaded(hwnd) {
        return Vec::new();
    }
    let index = note_index(hwnd);
    let notes = note_paths(hwnd);
    crate::library::links::edits_for_moves_with(&index, &notes, &moves, &|path| {
        root_file_exists(hwnd, path)
    })
    .into_iter()
    .filter(|edit| !edit.own)
    .collect()
}

/// An index build landed: notes renamed outside FastPad that links still point to are named in
/// a notice, once (spec §7.6). Renames that break no link are forgotten.
pub(crate) fn outside_renames_check(hwnd: HWND) {
    let told = host(hwnd, |host| {
        host.renamed_outside.is_empty() || host.renamed_outside_told
    })
    .unwrap_or(true);
    if told || !index_loaded(hwnd) {
        return;
    }
    let edits = outside_rename_edits(hwnd);
    let moves = host(hwnd, |host| {
        if edits.is_empty() {
            host.renamed_outside.clear();
        } else {
            host.renamed_outside_told = true;
        }
        host.renamed_outside.clone()
    })
    .unwrap_or_default();
    if edits.is_empty() {
        return;
    }
    push_notice(hwnd, outside_renames_notice(&moves, &edits));
}

/// The notice naming notes renamed outside FastPad that links still point to.
pub(crate) fn outside_renames_notice(moves: &[(PathBuf, PathBuf)], edits: &[NoteEdits]) -> String {
    let links: usize = edits.iter().map(|edit| edit.edits.len()).sum();
    let renamed = match moves {
        [(old, new)] => format!(
            "\u{201C}{}\u{201D} was renamed to \u{201C}{}\u{201D} outside FastPad.",
            crate::library::title::note_title(old),
            crate::library::title::note_title(new)
        ),
        _ => format!("{} notes were renamed outside FastPad.", moves.len()),
    };
    format!(
        "{renamed} {links} {} in {} {} still {} to the old {}. Use Notes: Update links to renamed notes.",
        if links == 1 { "link" } else { "links" },
        edits.len(),
        if edits.len() == 1 { "note" } else { "notes" },
        if links == 1 { "points" } else { "point" },
        if moves.len() == 1 { "name" } else { "names" },
    )
}

/// Notes: Update links to renamed notes: offers to point the links in other notes at the notes
/// renamed outside FastPad, then forgets those renames either way.
pub(crate) fn update_renamed_links(hwnd: HWND) {
    let edits = outside_rename_edits(hwnd);
    if edits.is_empty() {
        host(hwnd, |host| host.renamed_outside.clear());
        push_notice(
            hwnd,
            "No links point to notes renamed outside FastPad.".to_owned(),
        );
        return;
    }
    let links: usize = edits.iter().map(|edit| edit.edits.len()).sum();
    let question = format!(
        "Update {links} {} in {} {} to point to the renamed notes?",
        if links == 1 { "link" } else { "links" },
        edits.len(),
        if edits.len() == 1 { "note" } else { "notes" },
    );
    let choice = crate::window::modal::choose(hwnd, &question, &["Update links", "Leave them"]);
    host(hwnd, |host| host.renamed_outside.clear());
    if choice == 0 {
        let written = write_link_edits(hwnd, &edits);
        push_notice(hwnd, links_written_notice(&written));
    }
}

/// The moves a folder rename made: every note under `old` (relative) now under `new`.
pub(crate) fn folder_moves(hwnd: HWND, old: &Path, new: &Path) -> Vec<(PathBuf, PathBuf)> {
    note_paths(hwnd)
        .into_iter()
        .filter_map(|note| {
            let rest = note.strip_prefix(new).ok()?;
            Some((old.join(rest), note))
        })
        .collect()
}

/// The notice after links were updated.
pub(crate) fn links_written_notice(written: &LinksWritten) -> String {
    let mut text = format!(
        "Updated {} {} in {} {}.",
        written.links,
        if written.links == 1 { "link" } else { "links" },
        written.notes,
        if written.notes == 1 { "note" } else { "notes" }
    );
    if !written.skipped.is_empty() {
        text.push_str(&format!(" Not updated: {}.", written.skipped.join(", ")));
    }
    text
}
