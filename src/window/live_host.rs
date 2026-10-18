//! The main window's half of the Live Markdown view (2026-10-18 Live editor design spec §4): which
//! tabs each group shows Live, creating and sizing each group's Live window, and choosing whether
//! a group's text shows in the Live window or in its Scintilla. No function holds an `App` borrow
//! while calling Win32 APIs that can send messages back to the main window.

use crate::document::{DocumentId, Language};
use crate::live::LIVE_MAX_BYTES;
use crate::live::layout::LiveFonts;
use crate::live::view::{LiveColors, LiveView};
use crate::window::main_window as host_window;
use crate::window::palette::Palette;
use crate::window::split_tree::GroupId;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{MoveWindow, SW_HIDE, SW_SHOWNA, ShowWindow};

#[allow(
    dead_code,
    reason = "set_live's first caller outside markdown_live is the toggle command"
)]
const NOT_MARKDOWN_NOTICE: &str = "Live Markdown is available for Markdown documents. \
                                   Choose View > Markdown to treat this tab as Markdown.";
#[allow(
    dead_code,
    reason = "set_live's first caller outside markdown_live is the toggle command"
)]
const TOO_LARGE_NOTICE: &str = "Live Markdown is available for documents up to 4 MB. This one \
                                stays in the source view.";

/// One editor group's Live state.
#[derive(Debug, Default)]
pub(crate) struct LiveHost {
    /// The Live choice made for each of the group's documents; absent follows the default.
    choices: HashMap<DocumentId, bool>,
    /// Created the first time the group shows a tab Live.
    view: Option<LiveView>,
    /// The document the view showed when it was last shown; another one means a reload.
    shown_for: Option<DocumentId>,
    /// Whether the view is shown in place of the editor.
    shown: bool,
}

fn with_live<R>(hwnd: HWND, id: GroupId, action: impl FnOnce(&mut LiveHost) -> R) -> Option<R> {
    // SAFETY: the App pointer is used only for the immediate field access inside `action`, which
    // never calls back into Win32.
    unsafe { host_window::app_ptr(hwnd) }
        .and_then(|mut app| Some(action(&mut unsafe { app.as_mut() }.group_mut(id)?.live)))
}

/// Group `id`'s active document and its language.
fn active_document(hwnd: HWND, id: GroupId) -> Option<(DocumentId, Language)> {
    crate::window::preview_host::group_document(hwnd, id)
        .map(|(document, language, _)| (document, language))
}

fn document_length(hwnd: HWND, id: GroupId) -> usize {
    host_window::group_editor(hwnd, id)
        .and_then(|editor| editor.length().ok())
        .unwrap_or(0)
}

/// Whether group `id`'s active tab should show Live: a Markdown tab chosen Live, or not chosen
/// either way while the default is Live, that is not over the size limit.
pub(crate) fn wants_live(hwnd: HWND, id: GroupId) -> bool {
    let Some((document, Language::Markdown)) = active_document(hwnd, id) else {
        return false;
    };
    let chosen = with_live(hwnd, id, |host| host.choices.get(&document).copied()).flatten();
    chosen.unwrap_or(false) && document_length(hwnd, id) <= LIVE_MAX_BYTES
}

/// Group `id`'s Live window while it is shown.
pub(crate) fn view_hwnd(hwnd: HWND, id: GroupId) -> Option<HWND> {
    with_live(hwnd, id, |host| {
        host.view.filter(|_| host.shown).map(|view| view.hwnd())
    })
    .flatten()
}

/// The window showing group `id`'s text: its Live window while shown, otherwise its editor.
pub(crate) fn text_view_hwnd(hwnd: HWND, id: GroupId) -> Option<HWND> {
    view_hwnd(hwnd, id).or_else(|| host_window::group_editor(hwnd, id).map(|editor| editor.hwnd()))
}

/// Turns Live on or off for group `id`'s active tab, with a notice when it cannot be on.
#[allow(
    dead_code,
    reason = "first caller outside markdown_live is the toggle command (phase 1 plan, Task 11)"
)]
pub(crate) fn set_live(hwnd: HWND, id: GroupId, on: bool) {
    let Some((document, language)) = active_document(hwnd, id) else {
        return;
    };
    if on && language != Language::Markdown {
        host_window::push_notice(hwnd, NOT_MARKDOWN_NOTICE.to_owned());
        return;
    }
    if on && document_length(hwnd, id) > LIVE_MAX_BYTES {
        host_window::push_notice(hwnd, TOO_LARGE_NOTICE.to_owned());
        return;
    }
    with_live(hwnd, id, |host| host.choices.insert(document, on));
    crate::window::preview_host::in_group(id, || {
        crate::window::preview_host::sync_visibility(hwnd)
    });
}

/// Shows group `id`'s text in its Live window or its editor, as its active tab wants, or hides
/// both when `shown` is false (an image tab, the Full preview). Focus on the window being hidden
/// moves to the one being shown.
pub(crate) fn show_text(hwnd: HWND, id: GroupId, shown: bool) {
    prune_choices(hwnd, id);
    let Some(editor) = host_window::group_editor(hwnd, id) else {
        return;
    };
    let live = shown && wants_live(hwnd, id) && ensure_view(hwnd, id).is_some();
    let document = active_document(hwnd, id).map(|(document, _)| document);
    let (view, reload) = with_live(hwnd, id, |host| {
        let reload = live && (!host.shown || host.shown_for != document);
        host.shown = live;
        if live {
            host.shown_for = document;
        }
        (host.view, reload)
    })
    .unwrap_or((None, false));
    let focus = unsafe { GetFocus() };
    if live && let Some(view) = view {
        if reload {
            view.reload();
            view.scroll_to_caret();
        }
        unsafe {
            ShowWindow(editor.hwnd(), SW_HIDE);
            ShowWindow(view.hwnd(), SW_SHOWNA);
        }
        if focus == editor.hwnd() {
            unsafe { SetFocus(view.hwnd()) };
        }
        return;
    }
    if let Some(view) = view {
        let had_focus = focus == view.hwnd();
        let top = view.top_line();
        let was_visible =
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(view.hwnd()) }
                != 0;
        unsafe { ShowWindow(view.hwnd(), SW_HIDE) };
        if was_visible
            && shown
            && let Ok(display) = editor.visible_from_doc_line(top)
        {
            // Back to the source view where Live was scrolled to.
            let _ = editor.set_first_visible_line(display);
        }
        if had_focus && shown {
            unsafe { SetFocus(editor.hwnd()) };
        }
    }
    unsafe { ShowWindow(editor.hwnd(), if shown { SW_SHOWNA } else { SW_HIDE }) };
}

/// Sizes group `id`'s Live window to the editor's rectangle.
pub(crate) fn layout(hwnd: HWND, id: GroupId, rect: Option<RECT>) {
    let (Some(view), Some(rect)) = (with_live(hwnd, id, |host| host.view).flatten(), rect) else {
        return;
    };
    unsafe {
        MoveWindow(
            view.hwnd(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            1,
        );
    }
}

/// `document`'s text or language changed without reaching the view as edits (a reload, a
/// background Replace, a language change), or it closed: every Live view showing it lays it out
/// again when next shown.
pub(crate) fn forget(hwnd: HWND, document: DocumentId) {
    let Some(app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return;
    };
    let ids = unsafe { app.as_ref() }
        .groups
        .iter()
        .map(|group| group.id)
        .collect::<Vec<_>>();
    for id in ids {
        let view = with_live(hwnd, id, |host| {
            if host.shown_for == Some(document) {
                host.shown_for = None;
                host.view.filter(|_| host.shown)
            } else {
                None
            }
        })
        .flatten();
        if let Some(view) = view {
            view.reload();
        }
    }
}

/// Drops the choices for documents no longer among group `id`'s tabs.
fn prune_choices(hwnd: HWND, id: GroupId) {
    let Some(mut app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return;
    };
    let app = unsafe { app.as_mut() };
    let Some(open) = app.tabs.group(id).map(|tabs| tabs.document_ids()) else {
        return;
    };
    if let Some(group) = app.group_mut(id) {
        group
            .live
            .choices
            .retain(|document, _| open.contains(document));
    }
}

/// Group `id`'s Live window, created on first use. A failure turns Live off for the tab with a
/// notice.
fn ensure_view(hwnd: HWND, id: GroupId) -> Option<LiveView> {
    if let Some(view) = with_live(hwnd, id, |host| host.view).flatten() {
        return Some(view);
    }
    let created = create_view(hwnd, id);
    match created {
        Ok(view) => {
            with_live(hwnd, id, |host| host.view = Some(view));
            Some(view)
        }
        Err(error) => {
            if let Some((document, _)) = active_document(hwnd, id) {
                with_live(hwnd, id, |host| host.choices.insert(document, false));
            }
            host_window::push_notice(
                hwnd,
                format!("FastPad could not open the Live Markdown view: {error}"),
            );
            None
        }
    }
}

fn create_view(hwnd: HWND, id: GroupId) -> crate::Result<LiveView> {
    let editor = host_window::group_editor(hwnd, id).ok_or(crate::FastPadError::Invariant(
        "Live view without an editor",
    ))?;
    let parent = host_window::with_group_id(hwnd, id, |group| group.hwnd)
        .ok_or(crate::FastPadError::Invariant("Live view without a group"))?;
    let graphics = crate::window::preview_host::shared_graphics(hwnd)?;
    let (colors, fonts) = appearance(hwnd, &graphics, &editor);
    // Where the editor is now; the next group layout keeps the two together.
    let rect = client_rect_in_parent(editor.hwnd(), parent);
    let view = LiveView::create(parent, editor, graphics, colors, fonts)?;
    unsafe {
        MoveWindow(
            view.hwnd(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            0,
        );
    }
    Ok(view)
}

/// `child`'s rectangle in `parent`'s client coordinates.
fn client_rect_in_parent(child: HWND, parent: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(child, &mut rect);
        windows_sys::Win32::Graphics::Gdi::MapWindowPoints(
            std::ptr::null_mut(),
            parent,
            (&mut rect as *mut RECT).cast(),
            2,
        );
    }
    rect
}

/// The editor's colours and the prose font at the editor's zoom.
fn appearance(
    hwnd: HWND,
    graphics: &crate::preview::dwrite::Graphics,
    editor: &crate::editor::Editor,
) -> (LiveColors, LiveFonts) {
    let zoom = editor.zoom().unwrap_or(0);
    let (palette, preview_font, preview_size, line_height, font_size) =
        unsafe { host_window::app_ptr(hwnd) }
            .map(|app| {
                let app = unsafe { app.as_ref() };
                (
                    Palette::for_cached_theme(app.theme, app.settings.theme),
                    app.settings.preview_font.clone(),
                    app.settings.preview_font_size,
                    app.settings.preview_line_height,
                    app.settings.font_size,
                )
            })
            .unwrap_or_else(|| {
                (
                    Palette::for_cached_theme(None, Default::default()),
                    crate::window::design::faces::current().text.to_owned(),
                    crate::config::defaults::DEFAULT_PREVIEW_FONT_SIZE,
                    crate::config::defaults::DEFAULT_PREVIEW_LINE_HEIGHT,
                    crate::config::defaults::DEFAULT_FONT_SIZE,
                )
            });
    let colors = LiveColors {
        background: palette.editor_background,
        text: palette.editor_foreground,
        selection: palette.selection_background,
        inactive_selection: palette.inactive_selection_background,
        caret: palette.editor_foreground,
    };
    let fonts = LiveFonts::new(
        graphics,
        &preview_font,
        preview_size,
        line_height,
        font_size,
        zoom,
    );
    (colors, fonts)
}

#[cfg(test)]
#[allow(dead_code, reason = "used by the markdown_live integration target")]
impl LiveHost {
    /// The group's Live window, shown or not.
    pub(crate) fn view_for_test(&self) -> Option<LiveView> {
        self.view
    }

    /// How many of the group's documents have a Live choice.
    pub(crate) fn choice_count(&self) -> usize {
        self.choices.len()
    }
}
