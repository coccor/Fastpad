//! The main window's half of the Live Markdown view (2026-10-18 Live editor design spec §4): which
//! tabs each group shows Live, creating and sizing each group's Live window, and choosing whether
//! a group's text shows in the Live window or in its Scintilla. No function holds an `App` borrow
//! while calling Win32 APIs that can send messages back to the main window.

use crate::document::{DocumentId, Language};
use crate::live::LIVE_MAX_BYTES;
use crate::live::layout::LiveFonts;
use crate::live::view::{LiveColors, LiveView, MenuRequest};
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use crate::window::palette::Palette;
use crate::window::split_tree::GroupId;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{MoveWindow, SW_HIDE, SW_SHOWNA, ShowWindow};

const NOT_MARKDOWN_NOTICE: &str = "Live Markdown is available for Markdown documents. \
                                   Choose View > Markdown to treat this tab as Markdown.";
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
    /// Whether the view was just created, so it already lays out the editor's text: showing it
    /// the first time needs no reload.
    fresh: bool,
    /// The editor's selection as last followed: (anchor, caret, selection count).
    last_selection: Option<(usize, usize, usize)>,
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
    let default = unsafe { host_window::app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.settings.markdown_live_default);
    // The default skips a file over the limit silently; a choice was checked when it was made.
    chosen.unwrap_or(default) && document_length(hwnd, id) <= LIVE_MAX_BYTES
}

/// Group `id`'s Live window while it is shown.
pub(crate) fn view_hwnd(hwnd: HWND, id: GroupId) -> Option<HWND> {
    with_live(hwnd, id, |host| {
        host.view.filter(|_| host.shown).map(|view| view.hwnd())
    })
    .flatten()
}

impl LiveHost {
    /// Whether `window` is this group's Live window and it is shown.
    pub(crate) fn shows(&self, window: HWND) -> bool {
        self.shown && self.view.is_some_and(|view| view.hwnd() == window)
    }
}

/// The group whose editor, or shown Live window, is `window`: where focus means "in the
/// editor" for keys, commands and the writing helpers.
pub(crate) fn group_of_text_view(hwnd: HWND, window: HWND) -> Option<GroupId> {
    // SAFETY: the App reference lives only inside this function, which makes no Win32 call.
    let app = unsafe { host_window::app_ptr(hwnd)?.as_ref() };
    app.groups
        .iter()
        .find(|group| group.editor.hwnd() == window || group.live.shows(window))
        .map(|group| group.id)
}

/// The window showing group `id`'s text: its Live window while shown, otherwise its editor.
pub(crate) fn text_view_hwnd(hwnd: HWND, id: GroupId) -> Option<HWND> {
    view_hwnd(hwnd, id).or_else(|| host_window::group_editor(hwnd, id).map(|editor| editor.hwnd()))
}

/// `markdown.toggleLive` (Ctrl+Alt+V): the active tab switches between Live and the source view.
pub(crate) fn toggle(hwnd: HWND) {
    let Some(app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return;
    };
    let id = unsafe { app.as_ref() }.tabs.active_group();
    set_live(hwnd, id, view_hwnd(hwnd, id).is_none());
}

/// Turns Live on or off for group `id`'s active tab, with a notice when it cannot be on.
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
    let (view, reload, fresh) = with_live(hwnd, id, |host| {
        let fresh = std::mem::take(&mut host.fresh);
        let reload = live && !fresh && (!host.shown || host.shown_for != document);
        if reload {
            host.last_selection = None;
        }
        host.shown = live;
        if live {
            host.shown_for = document;
        }
        (host.view, reload, fresh && live)
    })
    .unwrap_or((None, false, false));
    let focus = unsafe { GetFocus() };
    if live && let Some(view) = view {
        // Relative image paths resolve against the document's folder.
        let folder =
            crate::window::preview_host::group_document(hwnd, id).and_then(|(_, _, folder)| folder);
        view.set_document_dir(folder);
        if reload {
            view.reload();
        }
        if reload || fresh {
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
            with_live(hwnd, id, |host| {
                host.view = Some(view);
                host.fresh = true;
            });
            // Files dropped on the view open as they do over the rest of the group.
            let windows = unsafe { host_window::app_ptr(hwnd) }.and_then(|app| {
                let app = unsafe { app.as_ref() };
                let group = app.groups.iter().find(|group| group.id == id)?;
                app.file_drops_accepted
                    .then(|| (group.hwnd, group.editor.hwnd()))
            });
            if let Some((group, editor)) = windows {
                crate::window::library_host::wrap_live_drop_target(
                    hwnd,
                    group,
                    editor,
                    view.hwnd(),
                );
            }
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
    let (colors, fonts, line_numbers) = appearance(hwnd, &graphics, &editor);
    // Where the editor is now; the next group layout keeps the two together.
    let rect = client_rect_in_parent(editor.hwnd(), parent);
    let view = LiveView::create(parent, editor, graphics, colors, fonts, line_numbers)?;
    // The scroll bar follows the theme, as the editor's does.
    crate::window::titlebar::apply_scrollbar_theme(view.hwnd(), dark_scrollbars(hwnd));
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

/// The editor's colours, the prose font at the editor's zoom, and whether line numbers show.
fn appearance(
    hwnd: HWND,
    graphics: &crate::preview::dwrite::Graphics,
    editor: &crate::editor::Editor,
) -> (LiveColors, LiveFonts, bool) {
    let (line_numbers, highlight_block) =
        unsafe { host_window::app_ptr(hwnd) }.map_or((true, true), |app| {
            let settings = &unsafe { app.as_ref() }.settings;
            (settings.line_numbers, settings.highlight_current_line)
        });
    let zoom = editor.zoom().unwrap_or(0);
    let (palette, preview_font, code_font, preview_size, line_height, font_size) =
        unsafe { host_window::app_ptr(hwnd) }
            .map(|app| {
                let app = unsafe { app.as_ref() };
                (
                    Palette::for_cached_theme(app.theme, app.settings.theme),
                    app.settings.preview_font.clone(),
                    app.settings.font_face.clone(),
                    app.settings.preview_font_size,
                    app.settings.preview_line_height,
                    app.settings.font_size,
                )
            })
            .unwrap_or_else(|| {
                (
                    Palette::for_cached_theme(None, Default::default()),
                    crate::window::design::faces::current().text.to_owned(),
                    "Consolas".to_owned(),
                    crate::config::defaults::DEFAULT_PREVIEW_FONT_SIZE,
                    crate::config::defaults::DEFAULT_PREVIEW_LINE_HEIGHT,
                    crate::config::defaults::DEFAULT_FONT_SIZE,
                )
            });
    // The Markdown roles as the source view colours them; high contrast keeps to system pairs.
    let roles = crate::languages::syntax_colors(host_window::effective_theme(hwnd));
    let role = |color: u32| {
        if palette.high_contrast {
            palette.editor_foreground
        } else {
            color
        }
    };
    let colors = LiveColors {
        background: palette.editor_background,
        text: palette.editor_foreground,
        selection: palette.selection_background,
        inactive_selection: palette.inactive_selection_background,
        caret: palette.editor_foreground,
        line_number: palette.line_number_foreground,
        heading: role(roles.heading),
        emphasis: role(roles.emphasis),
        marker: role(roles.operator),
        link: role(roles.link),
        code: role(roles.code),
        code_background: if palette.high_contrast {
            palette.editor_background
        } else {
            roles.code_background
        },
        dim: role(roles.comment),
        // The edited block stands out as the current line does, and only when that does.
        active_block: if highlight_block {
            palette.caret_line_background
        } else {
            palette.editor_background
        },
    };
    let fonts = LiveFonts::new(
        graphics,
        &preview_font,
        &code_font,
        preview_size,
        line_height,
        font_size,
        zoom,
    );
    (colors, fonts, line_numbers)
}

/// Group `id`'s Live window while it is shown, as a handle.
fn shown_view(hwnd: HWND, id: GroupId) -> Option<LiveView> {
    with_live(hwnd, id, |host| host.view.filter(|_| host.shown)).flatten()
}

/// The Split preview beside group `id` was scrolled to document `line`: its shown Live view puts
/// that line at its top, without reporting a scroll of its own (which would sync the preview
/// back). False when the group shows no Live view.
pub(crate) fn follow_preview(hwnd: HWND, id: GroupId, line: usize) -> bool {
    let Some(view) = shown_view(hwnd, id) else {
        return false;
    };
    view.set_top_line(line);
    true
}

/// Group `id`'s Live view `view` asks for its context menu (phase 4 plan, Decision 5): the
/// source view's items, greyed as they cannot run, and two more over a link. The menu-only
/// items reuse command ids as local entries, as the sidebar menus do: Delete is
/// `DeleteLines`, Select All `SelectAllOccurrences`, Open link `NotesFollowLink` and Copy link
/// address `MarkdownLink`.
pub(crate) fn context_menu(hwnd: HWND, id: GroupId, view: HWND, request: &MenuRequest) {
    use crate::window::menus::{self, MenuEntry};
    let Some(editor) = host_window::group_editor(hwnd, id) else {
        return;
    };
    let selected = editor
        .anchor_and_caret()
        .is_ok_and(|(anchor, caret)| anchor != caret);
    let mut entries = vec![
        MenuEntry::command("&Undo", CommandId::Undo).enabled(editor.can_undo().unwrap_or(false)),
        MenuEntry::command("&Redo", CommandId::Redo).enabled(editor.can_redo().unwrap_or(false)),
        MenuEntry::Separator,
        MenuEntry::command("Cu&t", CommandId::Cut).enabled(selected),
        MenuEntry::command("&Copy", CommandId::Copy).enabled(selected),
        MenuEntry::command("&Paste", CommandId::Paste),
        MenuEntry::local("&Delete\tDel", CommandId::DeleteLines).enabled(selected),
        MenuEntry::Separator,
        MenuEntry::local("Select &All\tCtrl+A", CommandId::SelectAllOccurrences),
    ];
    if request.link.is_some() {
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::local("&Open link", CommandId::NotesFollowLink),
            MenuEntry::local("Copy link &address", CommandId::MarkdownLink),
        ]);
    }
    // `track_popup` takes main-window client coordinates.
    let mut at = windows_sys::Win32::Foundation::POINT {
        x: request.screen.0,
        y: request.screen.1,
    };
    unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut at) };
    match menus::track_popup(hwnd, &entries, at) {
        None => {}
        Some(CommandId::DeleteLines) => {
            if let Ok(selection) = editor.selection() {
                let start = selection.start;
                let _ = editor.replace_target(selection, "");
                let _ = editor.set_selection(start..start);
            }
        }
        Some(CommandId::SelectAllOccurrences) => {
            if let Ok(length) = editor.length() {
                let _ = editor.set_selection(0..length);
            }
        }
        Some(CommandId::NotesFollowLink) => {
            if let Some(link) = &request.link
                && !crate::window::markdown_host::follow_link_at(hwnd, view, request.position)
                && !link.wiki
            {
                let payload = Box::into_raw(Box::new(link.dest.clone()));
                crate::window::preview_host::follow_link(hwnd, id, payload as isize);
            }
        }
        Some(CommandId::MarkdownLink) => {
            if let Some(link) = &request.link {
                let _ = crate::platform::clipboard::set_text(hwnd, &link.dest);
            }
        }
        Some(command) => unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND,
                command as usize,
                0,
            );
        },
    }
    poll(hwnd, id, false);
}

/// `SCN_MODIFIED` from group `id`'s editor: its shown Live view lays out the touched lines.
pub(crate) fn edited(hwnd: HWND, id: GroupId, notification: &crate::editor::ScintillaNotification) {
    let Some(view) = shown_view(hwnd, id) else {
        return;
    };
    let Some(editor) = host_window::group_editor(hwnd, id) else {
        return;
    };
    let position = notification.position.max(0) as usize;
    let first_line = editor.line_from_position(position).unwrap_or(0);
    let inserted = notification.modification_type as u32
        & crate::editor::scintilla_constants::SC_MOD_INSERTTEXT
        != 0;
    let edit = crate::preview::incremental::Edit::from_change(
        position,
        notification.length.max(0) as usize,
        inserted,
        notification.lines_added,
    );
    view.edited(edit, first_line);
}

const MULTIPLE_SELECTIONS_NOTICE: &str =
    "Live Markdown works with one selection. Switched to the source view.";
const RECTANGULAR_SELECTION_NOTICE: &str =
    "Live Markdown cannot show a column selection. Switched to the source view.";

/// Commands that need Scintilla's own UI (several carets, folds): in Live they switch to the
/// source view before they run (spec §7.3), with the name the notice gives them.
pub(crate) const fn source_only(command: CommandId) -> Option<&'static str> {
    Some(match command {
        CommandId::AddNextOccurrence => "Add next occurrence",
        CommandId::SelectAllOccurrences => "Select all occurrences",
        CommandId::AddCursorAbove => "Add cursor above",
        CommandId::AddCursorBelow => "Add cursor below",
        CommandId::ToggleCodeFolding => "Code folding",
        CommandId::FoldAll => "Fold all",
        CommandId::UnfoldAll => "Unfold all",
        _ => return None,
    })
}

/// Before `command` runs: one that needs the source view switches the active group out of Live,
/// with a notice, so it acts on the shown editor.
pub(crate) fn before_command(hwnd: HWND, command: CommandId) {
    let Some(name) = source_only(command) else {
        return;
    };
    let Some(app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return;
    };
    let id = unsafe { app.as_ref() }.tabs.active_group();
    if shown_view(hwnd, id).is_some() {
        leave(
            hwnd,
            id,
            format!("{name} needs the source view. Switched to the source view."),
        );
    }
}

/// Switches group `id`'s active tab to the source view, saying why.
fn leave(hwnd: HWND, id: GroupId, notice: String) {
    if let Some((document, _)) = active_document(hwnd, id) {
        with_live(hwnd, id, |host| host.choices.insert(document, false));
    }
    host_window::push_notice(hwnd, notice);
    crate::window::preview_host::in_group(id, || {
        crate::window::preview_host::sync_visibility(hwnd)
    });
}

/// Moves group `id`'s caret to the ATX heading whose slug is `anchor` (a `#heading` link
/// followed in the Live view); false when no heading matches.
pub(crate) fn go_to_anchor(hwnd: HWND, id: GroupId, anchor: &str) -> bool {
    let Some(editor) = host_window::group_editor(hwnd, id) else {
        return false;
    };
    let Ok(count) = editor.line_count() else {
        return false;
    };
    let found = (0..count).find(|&line| {
        editor.line_text(line).is_ok_and(|text| {
            let trimmed = text.trim_start();
            let level = trimmed.bytes().take_while(|b| *b == b'#').count();
            (1..=6).contains(&level)
                && trimmed[level..].starts_with([' ', '\t'])
                && crate::preview::links::slug(trimmed[level..].trim().trim_end_matches('#').trim())
                    == anchor
        })
    });
    let Some(line) = found else {
        return false;
    };
    let Ok(start) = editor.line_start(line) else {
        return false;
    };
    let _ = editor.set_selection(start..start);
    poll(hwnd, id, false);
    true
}

/// Follows group `id`'s editor after anything that may have moved its selection or scrolled its
/// Live view (`scrolled`): a hidden Scintilla sends no `SCN_UPDATEUI` for either, so the Live
/// view, the status bar and the writing helpers' table tracking are told from here. Called after
/// Live view input, after every command, and on any `SCN_UPDATEUI` that does arrive: a selection
/// set from elsewhere (find, go to line, a link jump) arrives that way, from Scintilla's posted
/// idle work, before the next paint.
pub(crate) fn poll(hwnd: HWND, id: GroupId, scrolled: bool) {
    let (Some(view), Some(editor)) = (shown_view(hwnd, id), host_window::group_editor(hwnd, id))
    else {
        return;
    };
    if scrolled
        && let Ok(display) = editor.visible_from_doc_line(view.top_line())
        && editor.set_first_visible_line(display).is_ok()
    {
        // Scintilla reports no scroll while hidden; a Split preview follows from here.
        crate::window::preview_host::editor_scrolled(hwnd, id);
    }
    let (Ok((anchor, caret)), Ok(count)) = (editor.anchor_and_caret(), editor.selection_count())
    else {
        return;
    };
    let snapshot = (anchor, caret, count);
    let changed = with_live(hwnd, id, |host| {
        host.last_selection.replace(snapshot) != Some(snapshot)
    })
    .unwrap_or(false);
    if !changed {
        return;
    }
    // Several carets or a column selection, made by keys forwarded to Scintilla (Alt+Shift+
    // arrows), need the source view (phase 1 plan, deviation 5).
    if count > 1 {
        leave(hwnd, id, MULTIPLE_SELECTIONS_NOTICE.to_owned());
        return;
    }
    if editor.is_rectangular_selection().unwrap_or(false) {
        leave(hwnd, id, RECTANGULAR_SELECTION_NOTICE.to_owned());
        return;
    }
    view.selection_changed();
    let active = unsafe { host_window::app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.tabs.active_group() == id);
    if active {
        host_window::invalidate_status_bar(hwnd);
    }
    crate::window::markdown_host::selection_changed(hwnd, id);
}

/// `poll` for the active group, after a command ran.
pub(crate) fn poll_active(hwnd: HWND) {
    if let Some(app) = unsafe { host_window::app_ptr(hwnd) } {
        let id = unsafe { app.as_ref() }.tabs.active_group();
        poll(hwnd, id, false);
    }
}

/// The editors' colours, fonts or zoom changed: every group's Live view follows.
pub(crate) fn refresh_appearance(hwnd: HWND) {
    let Some(app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return;
    };
    let views = unsafe { app.as_ref() }
        .groups
        .iter()
        .filter_map(|group| Some((group.live.view?, group.editor.clone())))
        .collect::<Vec<_>>();
    // No Live view yet: nothing to refresh, and DirectWrite must not load for it.
    if views.is_empty() {
        return;
    }
    let Ok(graphics) = crate::window::preview_host::shared_graphics(hwnd) else {
        return;
    };
    let dark = dark_scrollbars(hwnd);
    for (view, editor) in views {
        let (colors, fonts, line_numbers) = appearance(hwnd, &graphics, &editor);
        view.set_appearance(colors, fonts, line_numbers);
        crate::window::titlebar::apply_scrollbar_theme(view.hwnd(), dark);
    }
}

/// Whether scroll bars take the dark system look, as the editor's do.
fn dark_scrollbars(hwnd: HWND) -> bool {
    unsafe { host_window::app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        Palette::for_cached_theme(app.theme, app.settings.theme).dark_frame
    })
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
