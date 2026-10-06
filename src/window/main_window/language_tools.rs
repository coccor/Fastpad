//! Language detection for the active document and the JSON validate and format commands.

use super::*;

/// Detects the active document's language from its path (an untitled document has no path and
/// stays whatever it already is, i.e. plain text) and applies it. Reached only after `input_pending`
/// is false for `WM_FASTPAD_APPLY_LANGUAGE` (see `handle_deferred`), so this never runs ahead of
/// queued user input.
pub(super) fn apply_detected_language(hwnd: HWND) {
    if crate::window::image_host::active_is_image(hwnd) {
        return;
    }
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    let Some(path) = path else {
        return;
    };
    apply_language(hwnd, crate::languages::detect_language(&path));
}

/// Applies `language`'s lexer to the active editor via the (lazily created, per Task 13's
/// `find_bar`/`menu_bar`-style `Option<T>` precedent) `App::language_manager`, then records the
/// outcome on the active document's metadata: success updates `Document::language` to match what
/// is now actually shown; failure leaves the document's language metadata unchanged (the editor
/// itself is also left unchanged by `LanguageManager::apply` on failure) and surfaces the error.
pub(super) fn apply_language(hwnd: HWND, language: crate::document::Language) {
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let theme = effective_theme(hwnd);
    let result = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.language_manager.is_none() {
            app.language_manager = Some(crate::languages::LanguageManager::new());
        }
        app.language_manager
            .as_mut()
            .expect("just populated above if it was absent")
            .apply(&editor, language, theme)
    });
    match result {
        Some(Ok(())) => {
            set_active_language(hwnd, language);
            invalidate_status_bar(hwnd);
            // Lexer style tables reset every style's font face; restore the configured one.
            apply_editor_settings(hwnd);
            crate::window::preview_host::sync_visibility(hwnd);
        }
        Some(Err(_)) => {
            // An SVG's preview does not need Lexilla, so the tab stays an SVG in plain text.
            if language == crate::document::Language::Svg {
                set_active_language(hwnd, language);
                invalidate_status_bar(hwnd);
                crate::window::preview_host::sync_visibility(hwnd);
            }
            push_notice(
                hwnd,
                "FastPad could not enable syntax highlighting for this file. It will remain in \
                 plain text."
                    .to_owned(),
            );
        }
        None => {}
    }
}

/// The active tab's language; plain text while no tab is open.
/// Sets the active document's language. A changed language drops Live's state for it: edits
/// made under another language never reached it.
fn set_active_language(hwnd: HWND, language: crate::document::Language) {
    let changed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let tabs = &mut unsafe { app.as_mut() }.tabs;
        let id = tabs.active()?.id;
        tabs.set_active_language(language).then_some(id)
    });
    if let Some(id) = changed {
        crate::window::live_host::forget(hwnd, id);
    }
}

pub(crate) fn active_language(hwnd: HWND) -> crate::document::Language {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            unsafe { app.as_ref() }
                .tabs
                .active()
                .map(|document| document.language)
        })
        .unwrap_or(crate::document::Language::PlainText)
}

/// Validates the active document's current text as JSON and reports the outcome. Read-only: never
/// touches the editor's text, selection, or undo stack either way.
pub(super) fn validate_active_json(hwnd: HWND) {
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let Ok(text) = editor.text() else {
        return;
    };
    match crate::languages::validate_json(&text) {
        Ok(()) => push_notice(hwnd, "This document contains valid JSON.".to_owned()),
        Err(issue) => push_notice(hwnd, json_issue_message(&issue)),
    }
}

/// Formats the active document's full JSON text in place. Reads the current text and selection,
/// formats completely in memory first, and only on success mutates the editor: one full-buffer
/// `replace_target` bracketed by `begin_undo_action`/`end_undo_action` (Task 12's
/// `Editor::replace_all` precedent), so a single Undo restores the exact original bytes. The
/// selection is restored afterward, clamped to the (likely different) new length and then snapped
/// down to the nearest UTF-8 character boundary (`floor_char_boundary`): the pre-format byte
/// offsets have no guaranteed relationship to character boundaries in the reformatted text (JSON
/// string values keep their literal, possibly multi-byte, UTF-8 content), so clamping alone is not
/// enough to avoid handing Scintilla a mid-character position. Invalid JSON never starts an undo
/// action and leaves the document's bytes completely unchanged: the failure is detected before any
/// editor mutation is attempted.
pub(super) fn format_active_json(hwnd: HWND) {
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let Ok(text) = editor.text() else {
        return;
    };
    let selection = editor.selection().unwrap_or(0..0);
    match crate::languages::format_json(&text) {
        Ok(formatted) => {
            editor.begin_undo_action();
            let result = editor.replace_target(0..text.len(), &formatted);
            editor.end_undo_action();
            if result.is_err() {
                return;
            }
            let new_length = formatted.len();
            let start = floor_char_boundary(&formatted, selection.start.min(new_length));
            let end = floor_char_boundary(&formatted, selection.end.min(new_length));
            let _ = editor.set_selection(start..end);
        }
        Err(issue) => push_notice(hwnd, json_issue_message(&issue)),
    }
}

/// The largest UTF-8 character boundary in `text` at or before `position`. `position` may be
/// `text.len()` (a valid boundary, the end of the string) but must not exceed it. Used to snap a
/// byte offset carried over from a *different* string (the pre-format text) into a valid position
/// in `text` (the post-format text): after formatting, an old offset has no guaranteed
/// relationship to character boundaries in the reformatted bytes — `serde_json` passes multi-byte
/// UTF-8 through unescaped, so it can coincidentally land mid-character. `0` is always a valid
/// boundary, so this loop always terminates.
fn floor_char_boundary(text: &str, mut position: usize) -> usize {
    while !text.is_char_boundary(position) {
        position -= 1;
    }
    position
}

fn json_issue_message(issue: &crate::languages::JsonIssue) -> String {
    if issue.line == 0 && issue.column == 0 {
        format!("FastPad could not process this JSON: {}", issue.message)
    } else {
        format!(
            "This document is not valid JSON (line {}, column {}): {}",
            issue.line, issue.column, issue.message
        )
    }
}
