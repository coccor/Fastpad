//! Opening files, images, notes and search results into tabs, preview tabs, and the empty
//! startup tab a launch with a file replaces.

use super::*;

/// A launch with no file leaves no tab: the empty untitled tab the window started with closes
/// once the session restore has had its turn, unless it is not alone or has text by now.
fn close_unused_startup_tab(hwnd: HWND) {
    let alone = unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.tabs.len() == 1);
    if alone && empty_startup_tab(hwnd).is_some() {
        close_active_document(hwnd);
    }
}

pub(super) fn handle_open_request(hwnd: HWND) -> LRESULT {
    let request = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.launch_open_completed {
            return None;
        }
        app.launch_open_completed = true;
        Some(app.launch.request.clone())
    });
    let Some(request) = request else {
        return 0;
    };
    match request {
        crate::launch::LaunchRequest::Open(path) => {
            let path = std::path::Path::new(&path);
            if path.is_dir() {
                // OPEN_LIBRARY already opened it as the folder.
                if !notes_mode_enabled(hwnd) {
                    push_notice(
                        hwnd,
                        format!(
                            "{} is a folder. Turn on notes mode to open it as a notebook.",
                            path.display()
                        ),
                    );
                }
                unsafe {
                    let _ = record_milestone(hwnd, Milestone::FileLoaded);
                }
            } else {
                match App::open_path(hwnd, path) {
                    Ok(()) => return 0,
                    Err(error) => {
                        report_open_failure(hwnd, path, &error);
                        // The requested-file unit is finished either way; the milestone stays honest.
                        unsafe {
                            let _ = record_milestone(hwnd, Milestone::FileLoaded);
                        }
                    }
                }
            }
        }
        crate::launch::LaunchRequest::New => {
            close_unused_startup_tab(hwnd);
            unsafe {
                let _ = record_milestone(hwnd, Milestone::FileLoaded);
            }
        }
    }
    if unsafe { window_identity(hwnd) }.is_some_and(|identity| identity.is_live_for(hwnd)) {
        unsafe {
            PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
        }
    }
    0
}

pub(super) fn notes_mode_enabled(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode)
}

pub(crate) fn open_path(hwnd: HWND, path: &std::path::Path) -> Result<()> {
    open_path_placed(hwnd, path, false)
}

fn open_path_placed(hwnd: HWND, path: &std::path::Path, preview: bool) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    if file_population_active(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "file population is already active",
        ));
    }
    let existing =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.find_path(path));
    if let Some(id) = existing {
        return if open_in_active_group(hwnd, id) {
            unsafe {
                PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
            }
            Ok(())
        } else {
            Err(crate::FastPadError::Invariant(
                "existing file could not be activated",
            ))
        };
    }
    if crate::library::title::is_raster_image_path(path) {
        return open_image_placed(hwnd, path, preview);
    }

    // The tab being left saves first; a failed or paused autosave leaves it dirty and open.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    // Read before the load: a change that lands during it then still pauses the next autosave.
    let stamp = crate::library::disk_stamp(path);
    // All fallible disk/decode/text validation occurs before touching active state.
    // A file that is not text but starts with an image signature opens in an image tab (image
    // preview spec §4).
    let loaded = match crate::file::loader::load(path) {
        Err(crate::FastPadError::UnsupportedEncoding)
            if crate::file::sniff::file_looks_like_image(path) =>
        {
            return open_image_placed(hwnd, path, preview);
        }
        loaded => loaded?,
    };
    // A NUL byte cannot round-trip through Scintilla's UTF-8 buffer: the file is unsupported.
    if std::ffi::CString::new(loaded.text.as_str()).is_err() {
        return if crate::file::sniff::file_looks_like_image(path) {
            open_image_placed(hwnd, path, preview)
        } else {
            Err(crate::FastPadError::UnsupportedEncoding)
        };
    }
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        // A preview goes where the preview tab is. Without one it is placed like any new tab,
        // reusing an empty start tab.
        let replace_preview = preview && app.tabs.preview_id().is_some();
        // An untitled tab another group also shows stays: that view keeps it.
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| !replace_preview && !active.dirty && active.path.is_none())
            .filter(|active| app.tabs.views_of(active.id).len() == 1)
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
    remember_active_view(hwnd);
    // With no tab open this is the hidden placeholder document.
    let previous = editor.current_document()?;
    let reused_ids = match candidate_ids {
        Some(ids) if editor.text()?.is_empty() => Some(ids),
        _ => None,
    };
    let reuse = reused_ids.is_some();
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    let (id, recovery_id) = if let Some(ids) = reused_ids {
        ids
    } else {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        unsafe { app.as_mut() }.allocate_document_identity()
    };
    let mut document = Document::untitled(id, recovery_id, editor.create_document()?);
    document.path = Some(loaded.path);
    document.encoding = loaded.encoding;
    document.preview = preview;
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    unsafe { app_ptr(hwnd).unwrap().as_mut() }.populating_file = true;
    let result = document
        .expect_text()
        .and_then(|handle| editor.use_document(handle))
        .and_then(|_| editor.populate_clean(&loaded.text));
    if result.is_err() && identity.is_live_for(hwnd) {
        let _ = editor.use_document(&previous);
    }
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file population",
        ));
    }
    let (commit, retired) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        app.populating_file = false;
        result?;
        if replace_preview {
            // The old preview is never dirty, so dropping it loses nothing.
            (Ok(()), app.tabs.replace_preview(document))
        } else if reuse {
            let retired = app.tabs.replace_active_untitled(document);
            let commit = if retired.is_some() {
                Ok(())
            } else {
                Err(crate::FastPadError::Invariant(
                    "the reused tab closed during file open",
                ))
            };
            (commit, retired)
        } else {
            (
                app.tabs
                    .push(document)
                    .map_err(|_| crate::FastPadError::Invariant("duplicate document path")),
                None,
            )
        }
    };
    if let Some(retired) = retired {
        crate::window::markdown_host::forget(hwnd, retired.id);
    }
    if commit.is_err() {
        let _ = editor.use_document(&previous);
    }
    commit?;
    unsafe {
        let _ = record_milestone(hwnd, Milestone::FileLoaded);
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
    // Population suppressed SCN_MODIFIED, and a reused tab keeps its document id.
    crate::window::preview_host::document_reloaded(hwnd);
    refresh_tabs(hwnd);
    crate::window::library_host::document_loaded(hwnd, stamp);
    Ok(())
}

/// Opens `path` in an image tab (image preview spec §5). Like a text open it reuses an empty start
/// tab or the preview tab, but reads no bytes: the image view decodes on a worker.
fn open_image_placed(hwnd: HWND, path: &std::path::Path, preview: bool) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    if !path.is_file() {
        return Err(crate::FastPadError::Io(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )));
    }
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    let stamp = crate::library::disk_stamp(path);
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let replace_preview = preview && app.tabs.preview_id().is_some();
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| {
                !replace_preview && !active.is_image() && !active.dirty && active.path.is_none()
            })
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
    remember_active_view(hwnd);
    let reused_ids = match candidate_ids {
        Some(ids) if editor.text()?.is_empty() => Some(ids),
        _ => None,
    };
    let reuse = reused_ids.is_some();
    let (id, recovery_id) = match reused_ids {
        Some(ids) => ids,
        None => {
            let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
                "main window app state was not available",
            ))?;
            unsafe { app.as_mut() }.allocate_document_identity()
        }
    };
    let mut document = Document::image(id, recovery_id, path.to_path_buf());
    document.preview = preview;
    document.disk_stamp = stamp;
    let (commit, retired) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        if replace_preview {
            (Ok(()), app.tabs.replace_preview(document))
        } else if reuse {
            let retired = app.tabs.replace_active_untitled(document);
            let commit = if retired.is_some() {
                Ok(())
            } else {
                Err(crate::FastPadError::Invariant(
                    "the reused tab closed during file open",
                ))
            };
            (commit, retired)
        } else {
            (
                app.tabs
                    .push(document)
                    .map_err(|_| crate::FastPadError::Invariant("duplicate document path")),
                None,
            )
        }
    };
    commit?;
    // The retired tab's text document leaves the editor for an empty placeholder.
    let blank = editor.create_document()?;
    editor.use_document(&blank)?;
    if let Some(retired) = retired {
        crate::window::markdown_host::forget(hwnd, retired.id);
    }
    unsafe {
        let _ = record_milestone(hwnd, Milestone::FileLoaded);
    }
    refresh_tabs(hwnd);
    crate::window::library_host::document_loaded(hwnd, stamp);
    Ok(())
}

/// How `open_note` places a note that is not open yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenMode {
    /// In the preview tab, replaced in place by the next preview.
    Preview,
    /// In a normal tab. An open preview of the same note becomes normal.
    Permanent,
}

/// Opens `path` from the sidebar (spec §6.4). An already-open note is switched to, and a
/// `Permanent` open keeps it. Otherwise `Preview` replaces the preview tab in place and
/// `Permanent` opens a normal tab. `focus_editor` then moves the keyboard focus to the editor.
pub(crate) fn open_note(
    hwnd: HWND,
    path: &std::path::Path,
    mode: OpenMode,
    focus_editor: bool,
) -> Result<()> {
    let open = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.find_stored_path(path));
    match open {
        Some(id) => {
            if !open_in_active_group(hwnd, id) {
                return Err(crate::FastPadError::Invariant(
                    "the note's tab could not be activated",
                ));
            }
            unsafe {
                PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
            }
            if mode == OpenMode::Permanent {
                promote_tab(hwnd, id);
            }
        }
        None => open_path_placed(hwnd, path, mode == OpenMode::Preview)?,
    }
    if focus_editor {
        focus_content(hwnd);
    }
    Ok(())
}

/// Opens a Search result (spec §8). The note opens as `open_note` opens it. The find bar then
/// opens in Find mode with the query and options the shown results ran with, and selects the
/// first match from the start of the note. The find bar searches the live text: a phrase gone
/// since the search leaves the note open and the bar in its no-match state. `focus_editor` then
/// moves the focus to the editor, so F3 and Shift+F3 step on from the selected match. A note
/// that can't be opened (moved or deleted since the search) gets a notice, and the search runs
/// again.
pub(crate) fn open_search_result(
    hwnd: HWND,
    relative: &std::path::Path,
    mode: OpenMode,
    focus_editor: bool,
) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    // The results' query, not the box's text, which may be newer while the debounce runs.
    let search = crate::window::search_view::run_query(hwnd);
    if let Err(error) = open_note(hwnd, &path, mode, false) {
        push_notice(
            hwnd,
            format!("FastPad could not open {}: {error}", path.display()),
        );
        crate::window::text_search_host::run_now(hwnd);
        return;
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some((query, options)) = search.filter(|(query, _)| !query.is_empty()) {
        seed_find_bar(hwnd, &identity, &query, options);
    }
    if focus_editor && identity.is_live_for(hwnd) {
        focus_content(hwnd);
    }
}

/// Shows the find bar with `query` and `options` and selects the first match from position 0.
fn seed_find_bar(
    hwnd: HWND,
    identity: &WindowIdentity,
    query: &str,
    options: crate::search::MatchOptions,
) {
    crate::window::library_host::close_name_box(hwnd);
    if !identity.is_live_for(hwnd) || !ensure_find_bar(hwnd) {
        return;
    }
    let colors = title_chrome(hwnd).0;
    let pending = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar_mut()?;
        Some(bar.show_with(find_bar::FindBarMode::Find, query, options, colors))
    });
    let Some(pending) = pending else {
        return;
    };
    // Applied with nothing borrowed: the field's EN_CHANGE borrows the bar again.
    pending.apply();
    if !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let _ = editor.set_selection(0..0);
    select_match(
        hwnd,
        identity,
        &editor,
        query,
        options,
        0,
        find_bar::SearchDirection::Forward,
    );
}

/// Makes `id` a normal tab and repaints its label.
pub(super) fn promote_tab(hwnd: HWND, id: DocumentId) {
    let promoted =
        unsafe { app_ptr(hwnd) }.is_some_and(|mut app| unsafe { app.as_mut() }.tabs.promote(id));
    if promoted {
        invalidate_title_strip(hwnd);
    }
}

/// Whether this click on tab `index` is the second of a double-click.
pub(super) fn tab_double_click(hwnd: HWND, index: usize) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime;
    let now = unsafe { GetMessageTime() } as u32;
    let limit = unsafe { GetDoubleClickTime() };
    let Some(id) = tab_id_at(hwnd, index) else {
        return false;
    };
    with_group(hwnd, |group| {
        let double = group
            .last_tab_click
            .is_some_and(|(last, at)| last == id && now.wrapping_sub(at) <= limit);
        group.last_tab_click = if double { None } else { Some((id, now)) };
        double
    })
    .unwrap_or(false)
}
