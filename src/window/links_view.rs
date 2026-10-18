//! The Links view (second brain phase 1 spec §7.5): for the active note, the notes that link to
//! it (with the linking lines) and the notes it links to; for the whole notebook, the link
//! targets no note matches and the notes nothing links to.

use crate::window::design::metrics::{panel_header, scale, sidebar_row};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::panel::fill;
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{ViewPaint, draw_text, point_of};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC,
    InvalidateRect,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_LEFT, VK_RETURN, VK_RIGHT,
    VK_SPACE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

const PADDING_AT_96_DPI: i32 = 12;
const INDENT_AT_96_DPI: i32 = 16;
const CHEVRON_AT_96_DPI: i32 = 16;
const HINT_AT_96_DPI: i32 = 96;
/// How many linking lines a backlink shows under its note.
const SNIPPETS: usize = 3;

const CHEVRON_RIGHT: &str = "\u{E76C}";
const CHEVRON_DOWN: &str = "\u{E70D}";

pub(crate) const HEADER_TEXT: &str = "Links";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Section {
    LinkedFrom,
    LinksTo,
    Missing,
    Unlinked,
}

impl Section {
    pub(crate) const ALL: [Self; 4] = [
        Self::LinkedFrom,
        Self::LinksTo,
        Self::Missing,
        Self::Unlinked,
    ];

    const fn title(self) -> &'static str {
        match self {
            Self::LinkedFrom => "Linked from",
            Self::LinksTo => "Links to",
            Self::Missing => "Missing notes",
            Self::Unlinked => "Not linked from anywhere",
        }
    }

    /// The notebook-wide sections start collapsed: they cost work only when looked at.
    const fn starts_expanded(self) -> bool {
        matches!(self, Self::LinkedFrom | Self::LinksTo)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LinkRowKind {
    Section {
        section: Section,
        expanded: bool,
        count: usize,
    },
    /// A note to open, at a line if there is one.
    Note { path: PathBuf, line: Option<u32> },
    /// A linking line under its note.
    Snippet { path: PathBuf, line: u32 },
    /// A link target no note matches; opening it creates the note.
    Missing { target: String },
    /// A line of text in place of rows ("No links yet…").
    Message,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LinkRow {
    pub kind: LinkRowKind,
    pub depth: u16,
    pub text: String,
    /// Dim text at the right: "not created", "linked from 3 notes".
    pub hint: Option<String>,
}

/// What the view shows for the active note and the notebook. Built by `refresh`.
#[derive(Clone, Debug)]
pub(crate) struct LinksModel {
    /// The active note, relative to the notebook, or why there is none.
    pub note: Result<PathBuf, &'static str>,
    pub linked_from: Vec<(PathBuf, Vec<(u32, String)>)>,
    pub links_to: Vec<(String, Option<PathBuf>)>,
    pub missing: Vec<(String, usize)>,
    pub unlinked: Vec<PathBuf>,
}

impl Default for LinksModel {
    fn default() -> Self {
        Self {
            note: Err(NO_NOTEBOOK),
            linked_from: Vec::new(),
            links_to: Vec::new(),
            missing: Vec::new(),
            unlinked: Vec::new(),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The rows for `model` with `expanded` sections open (indexed like `Section::ALL`).
pub(crate) fn link_rows(model: &LinksModel, expanded: [bool; 4]) -> Vec<LinkRow> {
    let mut rows = Vec::new();
    let section = |rows: &mut Vec<LinkRow>, section: Section, count: usize| -> bool {
        let open = expanded[section as usize];
        rows.push(LinkRow {
            kind: LinkRowKind::Section {
                section,
                expanded: open,
                count,
            },
            depth: 0,
            text: format!("{} ({count})", section.title()),
            hint: None,
        });
        open
    };
    let message = |text: &str| LinkRow {
        kind: LinkRowKind::Message,
        depth: 1,
        text: text.to_owned(),
        hint: None,
    };
    match &model.note {
        Err(why) => rows.push(LinkRow {
            depth: 0,
            ..message(why)
        }),
        Ok(_) => {
            if section(&mut rows, Section::LinkedFrom, model.linked_from.len()) {
                for (path, lines) in &model.linked_from {
                    rows.push(LinkRow {
                        kind: LinkRowKind::Note {
                            path: path.clone(),
                            line: lines.first().map(|(line, _)| *line),
                        },
                        depth: 1,
                        text: file_name(path),
                        hint: None,
                    });
                    rows.extend(lines.iter().take(SNIPPETS).map(|(line, text)| LinkRow {
                        kind: LinkRowKind::Snippet {
                            path: path.clone(),
                            line: *line,
                        },
                        depth: 2,
                        text: text.trim().to_owned(),
                        hint: None,
                    }));
                }
                if model.linked_from.is_empty() {
                    rows.push(message("No other note links here."));
                }
            }
            if section(&mut rows, Section::LinksTo, model.links_to.len()) {
                for (target, path) in &model.links_to {
                    rows.push(match path {
                        Some(path) => LinkRow {
                            kind: LinkRowKind::Note {
                                path: path.clone(),
                                line: None,
                            },
                            depth: 1,
                            text: file_name(path),
                            hint: None,
                        },
                        None => LinkRow {
                            kind: LinkRowKind::Missing {
                                target: target.clone(),
                            },
                            depth: 1,
                            text: target.clone(),
                            hint: Some("not created".to_owned()),
                        },
                    });
                }
                if model.links_to.is_empty() {
                    rows.push(message("No links yet. Type [[ to link to another note."));
                }
            }
        }
    }
    if model.note != Err(crate::window::links_view::NO_NOTEBOOK) {
        if section(&mut rows, Section::Missing, model.missing.len()) {
            rows.extend(model.missing.iter().map(|(target, count)| LinkRow {
                kind: LinkRowKind::Missing {
                    target: target.clone(),
                },
                depth: 1,
                text: target.clone(),
                hint: Some(format!(
                    "linked from {count} {}",
                    if *count == 1 { "note" } else { "notes" }
                )),
            }));
        }
        if section(&mut rows, Section::Unlinked, model.unlinked.len()) {
            rows.extend(model.unlinked.iter().map(|path| LinkRow {
                kind: LinkRowKind::Note {
                    path: path.clone(),
                    line: None,
                },
                depth: 1,
                text: file_name(path),
                hint: None,
            }));
        }
    }
    rows
}

pub(crate) const NO_NOTEBOOK: &str = "No notebook open.";
pub(crate) const NOT_IN_NOTEBOOK: &str = "This file isn't in the open notebook.";
pub(crate) const NO_NOTE: &str = "Open a note to see its links.";
pub(crate) const INDEXING: &str = "Indexing notes\u{2026}";

#[derive(Debug)]
pub(crate) struct LinksView {
    pub(crate) rows: Vec<LinkRow>,
    pub(crate) list: RowListState,
    pub(crate) expanded: [bool; 4],
    order: u64,
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

const fn height(rect: RECT) -> i32 {
    let height = rect.bottom - rect.top;
    if height > 0 { height } else { 0 }
}

fn row_key(row: &LinkRow) -> String {
    format!("{:?}|{}", row.kind, row.depth)
}

impl LinksView {
    pub(crate) fn new(dpi: u32) -> Self {
        Self {
            rows: Vec::new(),
            list: RowListState::new(sidebar_row(dpi)),
            expanded: Section::ALL.map(Section::starts_expanded),
            order: 0,
        }
    }

    fn set_rows(&mut self, rows: Vec<LinkRow>, dpi: u32, height: i32) {
        let kept = self
            .list
            .selected
            .and_then(|index| self.rows.get(index))
            .map(row_key);
        if rows.len() != self.rows.len()
            || rows
                .iter()
                .zip(&self.rows)
                .any(|(a, b)| row_key(a) != row_key(b))
        {
            self.order = self.order.wrapping_add(1);
        }
        self.rows = rows;
        self.list.row_height = sidebar_row(dpi);
        self.list.set_count(self.rows.len());
        if let Some(index) =
            kept.and_then(|key| self.rows.iter().position(|row| row_key(row) == key))
        {
            self.list.select(index, height);
        } else if self
            .list
            .selected
            .is_some_and(|index| index >= self.rows.len())
        {
            self.list.selected = None;
        }
        self.list.scroll_lines(0, height);
    }

    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        RECT {
            top: (client.top + panel_header(dpi)).min(client.bottom),
            ..client
        }
    }

    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let header = RECT {
            bottom: client.top + panel_header(dpi),
            ..client
        };
        unsafe {
            fill(paint.hdc, client, paint.background);
            draw_text(
                paint.hdc,
                HEADER_TEXT,
                RECT {
                    left: header.left + pad,
                    ..header
                },
                paint.fonts.bold,
                palette.muted_foreground,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT,
            );
        }
        row_list::paint(
            paint.hdc,
            self.list_area(client, dpi),
            &self.list,
            &palette,
            paint.focused,
            paint.focused,
            &|_| paint.background,
            dpi,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }

    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let dpi = paint.dpi;
        let palette = paint.palette;
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let foreground = row_foreground(look, &palette);
        let muted = if look.selected && look.focused {
            foreground
        } else {
            palette.muted_foreground
        };
        let left = rect.left
            + scale(PADDING_AT_96_DPI, dpi)
            + i32::from(row.depth) * scale(INDENT_AT_96_DPI, dpi);
        let right = rect.right - scale(PADDING_AT_96_DPI, dpi);
        let hint = row.hint.as_ref().map(|hint| {
            (
                hint,
                RECT {
                    left: (right - scale(HINT_AT_96_DPI, dpi)).max(left),
                    right,
                    ..rect
                },
            )
        });
        let text_right = hint.as_ref().map_or(right, |(_, rect)| rect.left);
        unsafe {
            let (text_left, font, color) = match &row.kind {
                LinkRowKind::Section { expanded, .. } => {
                    let chevron = RECT {
                        left,
                        right: left + scale(CHEVRON_AT_96_DPI, dpi),
                        ..rect
                    };
                    let arrow = if *expanded {
                        CHEVRON_DOWN
                    } else {
                        CHEVRON_RIGHT
                    };
                    draw_text(
                        hdc,
                        arrow,
                        chevron,
                        paint.fonts.glyph,
                        muted,
                        line | DT_CENTER,
                    );
                    (chevron.right, paint.fonts.bold, foreground)
                }
                LinkRowKind::Snippet { .. } | LinkRowKind::Message => {
                    (left, paint.fonts.text, muted)
                }
                LinkRowKind::Missing { .. } => (left, paint.fonts.italic, foreground),
                LinkRowKind::Note { .. } => (left, paint.fonts.text, foreground),
            };
            draw_text(
                hdc,
                &row.text,
                RECT {
                    left: text_left,
                    right: text_right,
                    ..rect
                },
                font,
                color,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if let Some((hint, rect)) = hint {
                draw_text(
                    hdc,
                    hint,
                    rect,
                    paint.fonts.text,
                    muted,
                    line | DT_RIGHT | DT_END_ELLIPSIS,
                );
            }
        }
    }

    /// The action the row at `index` asks for when opened (`keep`: a permanent tab).
    fn action(&self, index: usize, keep: bool) -> Option<LinkAction> {
        Some(match &self.rows.get(index)?.kind {
            LinkRowKind::Section { section, .. } => LinkAction::Toggle(*section),
            LinkRowKind::Note { path, line } => LinkAction::Open {
                path: path.clone(),
                line: *line,
                keep,
            },
            LinkRowKind::Snippet { path, line } => LinkAction::Open {
                path: path.clone(),
                line: Some(*line),
                keep,
            },
            LinkRowKind::Missing { target } => LinkAction::Create(target.clone()),
            LinkRowKind::Message => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LinkAction {
    Toggle(Section),
    Open {
        path: PathBuf,
        line: Option<u32>,
        keep: bool,
    },
    Create(String),
}

fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut LinksView) -> R) -> Option<R> {
    let mut app = unsafe { super::main_window::app_ptr(hwnd) }?;
    unsafe { app.as_mut() }
        .sidebar
        .as_mut()
        .map(|sidebar| f(&mut sidebar.links))
}

fn geometry(panel: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(panel, &mut client);
    }
    (client, unsafe { GetDpiForWindow(panel) }.max(96))
}

fn invalidate(panel: HWND) {
    unsafe {
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

/// Builds what the view shows now. Reads the linking lines of the active note's backlinks
/// from their files; the notebook-wide sections are computed only while expanded.
fn model(hwnd: HWND, expanded: [bool; 4]) -> LinksModel {
    let Some(root) = library_host::folder(hwnd) else {
        return LinksModel {
            note: Err(NO_NOTEBOOK),
            ..LinksModel::default()
        };
    };
    if !library_host::index_loaded(hwnd) {
        return LinksModel {
            note: Err(INDEXING),
            ..LinksModel::default()
        };
    }
    let settings = library_host::notebook_settings(hwnd);
    let index = library_host::note_index(hwnd);
    let Some(resolver) = library_host::resolver(hwnd) else {
        return LinksModel {
            note: Err(INDEXING),
            ..LinksModel::default()
        };
    };
    let active = unsafe { super::main_window::app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    let note = match &active {
        None => Err(NO_NOTE),
        Some(path) => path
            .strip_prefix(&root)
            .map(Path::to_path_buf)
            .map_err(|_| NOT_IN_NOTEBOOK),
    };
    let mut model = LinksModel {
        note: note.clone(),
        ..LinksModel::default()
    };
    if let Ok(note) = &note {
        model.linked_from = crate::library::links::backlinks(&index, &resolver, note)
            .into_iter()
            .map(|backlink| {
                let text = crate::file::loader::load(&root.join(&backlink.from))
                    .map(|file| file.text)
                    .unwrap_or_default();
                let lines: Vec<&str> = text.lines().collect();
                let mut seen = Vec::new();
                let snippets = backlink
                    .links
                    .iter()
                    .filter(|link| {
                        let new = !seen.contains(&link.line);
                        seen.push(link.line);
                        new
                    })
                    .map(|link| {
                        (
                            link.line,
                            lines
                                .get(link.line as usize)
                                .copied()
                                .unwrap_or_default()
                                .to_owned(),
                        )
                    })
                    .collect();
                (backlink.from, snippets)
            })
            .collect();
        model.links_to = crate::library::links::outgoing(&index, &resolver, note)
            .into_iter()
            .map(|out| (out.link.target, out.target))
            .collect();
    }
    if expanded[Section::Missing as usize] {
        model.missing =
            crate::library::links::missing(&index, &resolver, &settings.templates_folder)
                .into_iter()
                .map(|gap| (gap.target, gap.from.len()))
                .collect();
    }
    if expanded[Section::Unlinked as usize] {
        model.unlinked = crate::library::links::unlinked(
            &index,
            &resolver,
            &crate::library::links::Conventions {
                daily_folder: &settings.daily_folder,
                inbox: &settings.inbox,
                templates: &settings.templates_folder,
            },
        );
    }
    model
}

/// Rebuilds the rows. Part of `side_panel::refresh`, and after the active tab or the index
/// changed.
pub(crate) fn refresh(hwnd: HWND, panel: HWND) {
    let expanded = with_view(hwnd, |view| view.expanded).unwrap_or([true, true, false, false]);
    let model = model(hwnd, expanded);
    let rows = link_rows(&model, expanded);
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        view.set_rows(rows, dpi, height(area));
    });
    invalidate(panel);
}

pub(crate) fn refresh_metrics(hwnd: HWND, panel: HWND) {
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let height = height(view.list_area(client, dpi));
        view.list.row_height = sidebar_row(dpi);
        view.list.scroll_lines(0, height);
    });
    invalidate(panel);
}

pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    if let Some(app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.links.paint(paint);
    }
}

/// Shows the Links view with `section` expanded (the Missing notes and Not linked commands).
pub(crate) fn reveal(hwnd: HWND, section: Section) {
    with_view(hwnd, |view| view.expanded[section as usize] = true);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Links, true);
    if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
        refresh(hwnd, panel);
        let (client, dpi) = geometry(panel);
        with_view(hwnd, |view| {
            let area = view.list_area(client, dpi);
            if let Some(index) = view.rows.iter().position(|row| {
                matches!(row.kind, LinkRowKind::Section { section: shown, .. } if shown == section)
            }) {
                view.list.select(index, height(area));
            }
        });
    }
}

fn run(hwnd: HWND, panel: HWND, action: LinkAction) {
    match action {
        LinkAction::Toggle(section) => {
            with_view(hwnd, |view| {
                view.expanded[section as usize] = !view.expanded[section as usize];
            });
            refresh(hwnd, panel);
        }
        LinkAction::Open { path, line, keep } => {
            let Some(root) = library_host::folder(hwnd) else {
                return;
            };
            let path = root.join(path);
            let (mode, focus) = if keep {
                (OpenMode::Permanent, true)
            } else {
                (OpenMode::Preview, false)
            };
            if let Err(error) = crate::window::main_window::open_note(hwnd, &path, mode, focus) {
                crate::window::main_window::report_open_failure(hwnd, &path, &error);
                return;
            }
            if let Some(line) = line
                && let Some(editor) = unsafe { super::main_window::app_ptr(hwnd) }
                    .and_then(|app| unsafe { app.as_ref() }.editor().cloned())
                && let Ok(start) = editor.line_start(line as usize)
            {
                let _ = editor.set_selection(start..start);
                editor.scroll_caret_into_view();
            }
        }
        LinkAction::Create(target) => library_host::create_linked_note(hwnd, &target),
    }
}

/// Input for the Links view. `None` leaves the message to the panel's default handling.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (client, dpi) = geometry(panel);
    let point = || {
        let (x, y) = point_of(lparam);
        POINT { x, y }
    };
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let at = point();
            let changed = with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                let row = inside(area, at)
                    .then(|| view.list.row_at(at.y - area.top))
                    .flatten();
                view.list.set_hover(row)
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            if with_view(hwnd, |view| view.list.set_hover(None)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if unsafe { GetFocus() } != panel {
                unsafe {
                    SetFocus(panel);
                }
            }
            let at = point();
            let double = message == WM_LBUTTONDBLCLK;
            let action = with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                if !inside(area, at) {
                    return None;
                }
                let index = view.list.row_at(at.y - area.top)?;
                view.list.select(index, height(area));
                let action = view.action(index, double)?;
                // The second press of a double click on a section would undo the first.
                (!(double && matches!(action, LinkAction::Toggle(_)))).then_some(action)
            })
            .flatten();
            invalidate(panel);
            if let Some(action) = action {
                run(hwnd, panel, action);
            }
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            if with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.wheel(delta, lines, height(area))
            })
            .unwrap_or(false)
            {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            if let Some(movement) = ListKey::from_virtual_key(u32::from(key)) {
                with_view(hwnd, |view| {
                    let area = view.list_area(client, dpi);
                    view.list.move_selection(movement, height(area));
                });
                invalidate(panel);
                return Some(0);
            }
            let action = with_view(hwnd, |view| {
                let index = view.list.selected?;
                let row = view.rows.get(index)?;
                match (key, &row.kind) {
                    (
                        VK_RIGHT,
                        LinkRowKind::Section {
                            expanded: false, ..
                        },
                    )
                    | (VK_LEFT, LinkRowKind::Section { expanded: true, .. }) => {
                        view.action(index, false)
                    }
                    (VK_RETURN | VK_SPACE, _) => view.action(index, true),
                    _ => None,
                }
            })
            .flatten();
            let action = action?;
            invalidate(panel);
            run(hwnd, panel, action);
            Some(0)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn shown_rows(hwnd: HWND) -> Vec<LinkRow> {
    with_view(hwnd, |view| view.rows.clone()).unwrap_or_default()
}

impl crate::window::sidebar_accessibility::AccessibleView for LinksView {
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        self.rows.len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{outline_item, row_rect};
        let row = self.rows.get(index)?;
        let (rect, visible) = row_rect(self.list_area(client, dpi), &self.list, index);
        let selected = self.list.selected == Some(index);
        let expanded = match &row.kind {
            LinkRowKind::Section { expanded, .. } => Some(*expanded),
            _ => None,
        };
        let mut name = row.text.clone();
        if let Some(hint) = &row.hint {
            name.push_str(", ");
            name.push_str(hint);
        }
        Some(outline_item(
            name, expanded, row.depth, selected, focused, rect, visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let area = self.list_area(client, dpi);
        inside(area, point)
            .then(|| self.list.row_at(point.y - area.top))
            .flatten()
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.list.selected
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if index < self.rows.len() {
            let area = self.list_area(client, dpi);
            self.list.select(index, area.bottom - area.top);
        }
    }

    fn accessible_identity(&self, index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        Some(crate::window::sidebar_accessibility::identity_of(&row_key(
            self.rows.get(index)?,
        )))
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_sections_and_their_state() {
        let model = LinksModel {
            note: Ok(PathBuf::from("atlas.md")),
            linked_from: vec![(
                PathBuf::from("2026-10-14.md"),
                vec![(2, "  kickoff for [[Atlas]] moved".to_owned())],
            )],
            links_to: vec![
                ("Roadmap".to_owned(), Some(PathBuf::from("Roadmap.md"))),
                ("Vendor list".to_owned(), None),
            ],
            missing: vec![("Vendor list".to_owned(), 3)],
            unlinked: vec![],
        };
        let rows = link_rows(&model, [true, true, true, false]);
        let shown: Vec<(&str, u16, Option<&str>)> = rows
            .iter()
            .map(|row| (row.text.as_str(), row.depth, row.hint.as_deref()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Linked from (1)", 0, None),
                ("2026-10-14.md", 1, None),
                ("kickoff for [[Atlas]] moved", 2, None),
                ("Links to (2)", 0, None),
                ("Roadmap.md", 1, None),
                ("Vendor list", 1, Some("not created")),
                ("Missing notes (1)", 0, None),
                ("Vendor list", 1, Some("linked from 3 notes")),
                ("Not linked from anywhere (0)", 0, None),
            ]
        );
    }

    #[test]
    fn without_a_note_only_the_reason_and_the_notebook_sections_show() {
        let model = LinksModel {
            note: Err(NO_NOTE),
            ..LinksModel::default()
        };
        let rows = link_rows(&model, [true, true, false, false]);
        assert_eq!(rows[0].text, NO_NOTE);
        assert_eq!(rows.len(), 3);
        let none = LinksModel {
            note: Err(NO_NOTEBOOK),
            ..LinksModel::default()
        };
        assert_eq!(link_rows(&none, [true; 4]).len(), 1);
    }
}
