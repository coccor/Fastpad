//! Naming notes and folders in the Notebook tree (inline naming spec): an `EDIT` field over a
//! row's name for New note, New folder and both renames. The pure half comes first; the field,
//! the edit it shows and the commits follow.

use super::main_window::{app_ptr, push_notice};
use super::notebook_view::{self, NotebookView, with_view};
use super::side_panel;
use crate::config::SidebarView;
use crate::library;
use crate::library::title;
use crate::library::tree::{self, RowKind, TreeRow};
use crate::platform::{last_error, wide_null};
use crate::window::file_icons::{NoteKind, note_kind};
use crate::window::icon_sets::TreeItem;
use crate::window::library_host::{self, with_state};
use crate::window::palette::Palette;
use crate::window::panel::{create_child, scale};
use crate::window::tree_move::{self, MoveError};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, HBRUSH, HDC, InvalidateRect, SetBkColor, SetTextColor,
};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_TAB,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, EVENT_OBJECT_DESCRIPTIONCHANGE, GWL_STYLE, GetParent,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, MoveWindow,
    PostMessageW, SW_HIDE, SW_SHOWNA, SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS, WM_NCDESTROY, WM_SETFONT, WS_CHILD, WS_VISIBLE,
};

// Sizes at 96 DPI; everything is scaled with `panel::scale`.
/// How far the frame starts before the row's name.
const FRAME_OUTSET: i32 = 3;
/// The frame's gap to the row's top and bottom edges.
const FRAME_INSET_Y: i32 = 2;
/// Where the text starts inside the frame.
const TEXT_INSET: i32 = 3;
const FIELD_HOOK_ID: usize = 0x4650_494E;

/// What the field names. Paths are relative to the notebook; a new item's is the folder it
/// goes in, empty for the root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Purpose {
    NewNote(PathBuf),
    NewFolder(PathBuf),
    RenameNote(PathBuf),
    RenameFolder(PathBuf),
}

impl Purpose {
    /// The folder the name goes in (empty for the notebook root).
    pub(crate) fn parent(&self) -> &Path {
        match self {
            Self::NewNote(parent) | Self::NewFolder(parent) => parent,
            Self::RenameNote(path) | Self::RenameFolder(path) => {
                path.parent().unwrap_or(Path::new(""))
            }
        }
    }

    /// A new item's folder: the edit shows a draft row there (spec §3.1).
    pub(crate) fn draft_parent(&self) -> Option<&Path> {
        match self {
            Self::NewNote(parent) | Self::NewFolder(parent) => Some(parent),
            Self::RenameNote(_) | Self::RenameFolder(_) => None,
        }
    }

    /// The row being renamed.
    pub(crate) fn own_row(&self) -> Option<RowKind> {
        match self {
            Self::RenameNote(path) => Some(RowKind::Note(path.clone())),
            Self::RenameFolder(path) => Some(RowKind::Folder(path.clone())),
            Self::NewNote(_) | Self::NewFolder(_) => None,
        }
    }

    pub(crate) fn is_folder(&self) -> bool {
        matches!(self, Self::NewFolder(_) | Self::RenameFolder(_))
    }

    /// A rename's current name, which the field starts with (spec §3.3).
    pub(crate) fn current_name(&self) -> Option<String> {
        match self {
            Self::RenameNote(path) | Self::RenameFolder(path) => {
                Some(path.file_name()?.to_string_lossy().into_owned())
            }
            Self::NewNote(_) | Self::NewFolder(_) => None,
        }
    }
}

/// The name `text` gives the item (spec §4), or `None` when it cancels: nothing left once
/// cleaned, or a rename to the name it already has. A change of letter case is a rename.
pub(crate) fn typed_name(purpose: &Purpose, text: &str) -> Option<String> {
    let name = match purpose {
        Purpose::NewNote(_) => title::new_note_name(text)?,
        Purpose::NewFolder(_) | Purpose::RenameFolder(_) => title::folder_name(text)?,
        Purpose::RenameNote(path) => {
            let current = path
                .extension()
                .map(|extension| extension.to_string_lossy());
            title::renamed_note_name(text, current.as_deref())?
        }
    };
    match purpose.current_name() {
        Some(current) if current == name => None,
        _ => Some(name),
    }
}

/// "<name> already exists here." (spec §4.4).
pub(crate) fn taken_message(name: &str) -> String {
    format!("{name} already exists here.")
}

/// The live check (spec §4.4): the problem the field shows for `text`, or `None`. `siblings`
/// holds the lowercased names listed beside the item, its own row left out. A name that
/// cancels has no problem.
pub(crate) fn check(purpose: &Purpose, text: &str, siblings: &HashSet<String>) -> Option<String> {
    let name = typed_name(purpose, text)?;
    if purpose.is_folder() && crate::library::scan::skip_directory(&name) {
        return Some(super::library_host::hidden_folder_error(&name));
    }
    siblings
        .contains(&name.to_lowercase())
        .then(|| taken_message(&name))
}

/// The part of `name` a rename selects, in UTF-16 units (spec §3.3): a note's name before its
/// last `.`, all of a name whose only `.` starts it, and all of a folder's.
pub(crate) fn rename_selection(name: &str, folder: bool) -> (usize, usize) {
    let all = name.encode_utf16().count();
    if folder {
        return (0, all);
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => (0, name[..dot].encode_utf16().count()),
        _ => (0, all),
    }
}

/// What the draft row shows an icon for (spec §3.1): a closed folder, or the note type of the
/// note extension typed so far, Markdown until one is.
pub(crate) fn draft_icon(purpose: &Purpose, text: &str) -> TreeItem {
    if purpose.is_folder() {
        return TreeItem::Folder { expanded: false };
    }
    let extension = text
        .trim()
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| title::is_note_extension(extension))
        .unwrap_or("md");
    TreeItem::Note(note_kind(Some(extension)))
}

/// The field's accessible name (spec §6). `notebook` names the root.
pub(crate) fn accessible_name(purpose: &Purpose, notebook: &str) -> String {
    let place = |parent: &Path| {
        parent.file_name().map_or_else(
            || notebook.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    };
    match purpose {
        Purpose::NewNote(parent) => format!("New note name, in {}", place(parent)),
        Purpose::NewFolder(parent) => format!("New folder name, in {}", place(parent)),
        Purpose::RenameNote(path) | Purpose::RenameFolder(path) => format!(
            "Rename {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ),
    }
}

/// The row of the folder `parent`: `Some(None)` for the notebook root, `None` when the folder
/// has no row.
pub(crate) fn parent_row(rows: &[TreeRow], parent: &Path) -> Option<Option<usize>> {
    if parent.as_os_str().is_empty() {
        return Some(None);
    }
    tree::row_index(rows, &RowKind::Folder(parent.to_path_buf())).map(Some)
}

/// The lowercased names of the notes and folders listed in the folder at row `parent` (`None`:
/// the root), leaving out row `own`: what a name typed there may not take (spec §4.4).
pub(crate) fn sibling_names(
    rows: &[TreeRow],
    parent: Option<usize>,
    own: Option<usize>,
) -> HashSet<String> {
    let (start, depth) = match parent {
        Some(index) => match rows.get(index) {
            Some(folder) => (index + 1, folder.depth.saturating_add(1)),
            None => return HashSet::new(),
        },
        None => (0, 0),
    };
    rows.iter()
        .enumerate()
        .skip(start)
        .take_while(|(_, row)| row.depth >= depth)
        .filter(|&(index, row)| {
            row.depth == depth
                && Some(index) != own
                && matches!(row.kind, RowKind::Folder(_) | RowKind::Note(_))
        })
        .map(|(_, row)| row.name.to_lowercase())
        .collect()
}

/// Puts the draft row in `rows` as the first child of the folder at row `parent`, one level
/// deeper, or at the root below the unsaved rows (spec §3.1). `None`, changing nothing, when
/// that folder is collapsed or gone. Returns the draft row's index.
pub(crate) fn insert_draft(rows: &mut Vec<TreeRow>, parent: Option<usize>) -> Option<usize> {
    let (at, depth) = match parent {
        None => (
            rows.iter()
                .take_while(|row| matches!(row.kind, RowKind::Unsaved(_)))
                .count(),
            0,
        ),
        Some(index) => {
            let folder = rows.get(index)?;
            if !folder.expanded {
                return None;
            }
            (index + 1, folder.depth.saturating_add(1))
        }
    };
    rows.insert(
        at,
        TreeRow {
            kind: RowKind::Draft,
            depth,
            name: String::new(),
            pinned: false,
            expanded: false,
        },
    );
    Some(at)
}

/// Where Ctrl+Backspace deletes back to from `caret` in `text` (UTF-16): past any spaces, then
/// past one run of letters and digits, or of other characters.
pub(crate) fn word_start(text: &[u16], caret: usize) -> usize {
    // 0 space, 1 letter or digit (a surrogate half counts as one), 2 anything else.
    let class = |unit: u16| match char::from_u32(u32::from(unit)) {
        Some(ch) if ch.is_whitespace() => 0,
        Some(ch) if !ch.is_alphanumeric() => 2,
        _ => 1,
    };
    let mut start = caret.min(text.len());
    while start > 0 && class(text[start - 1]) == 0 {
        start -= 1;
    }
    if start > 0 {
        let run = class(text[start - 1]);
        while start > 0 && class(text[start - 1]) == run {
            start -= 1;
        }
    }
    start
}

/// The field's frame and the `Edit` inside it, in panel coordinates. (`RECT` is only `Clone` and
/// `Copy`, so this is too.)
#[derive(Clone, Copy)]
pub(crate) struct FieldLayout {
    pub(crate) frame: RECT,
    pub(crate) edit: RECT,
}

/// Where the field goes for the row at `row` (depth `depth`), clipped to the `list` area so it
/// never covers the header (spec §5.4). The frame covers the name, from just before it to the
/// pin; the chevron, icon and pin stay in view (§3.3). `None` when the row is out of view, or
/// too narrow for any text.
pub(crate) fn field_layout(
    row: RECT,
    list: RECT,
    depth: u16,
    dpi: u32,
    text_height: i32,
) -> Option<FieldLayout> {
    if row.top < list.top || row.top >= list.bottom {
        return None;
    }
    let name = super::notebook_view::row_parts(row, depth, dpi).name;
    let frame = RECT {
        left: (name.left - scale(FRAME_OUTSET, dpi)).max(row.left),
        top: row.top + scale(FRAME_INSET_Y, dpi),
        right: name.right,
        bottom: (row.bottom - scale(FRAME_INSET_Y, dpi)).min(list.bottom),
    };
    let text_height = text_height.clamp(1, (frame.bottom - frame.top - 2).max(1));
    let top = frame.top + (frame.bottom - frame.top - text_height) / 2;
    let edit = RECT {
        left: frame.left + scale(TEXT_INSET, dpi),
        top,
        right: frame.right - 1,
        bottom: (top + text_height).min(frame.bottom - 1),
    };
    (edit.right > edit.left && edit.bottom > edit.top).then_some(FieldLayout { frame, edit })
}

/// Where a problem `height` tall goes (spec §4.4): under the frame, over the row beneath, or
/// above it when the list has no room below. Never above the list's top.
pub(crate) fn message_rect(frame: RECT, list: RECT, height: i32) -> RECT {
    if frame.bottom + height <= list.bottom {
        return RECT {
            top: frame.bottom,
            bottom: frame.bottom + height,
            ..frame
        };
    }
    RECT {
        top: (frame.top - height).max(list.top),
        bottom: frame.top,
        ..frame
    }
}

/// How an edit is being committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum How {
    /// Enter in the field: a problem keeps the field open with its message (spec §5.2).
    Enter,
    /// Focus left the field for another FastPad window, a click in the tree, or another edit
    /// starting (spec §3.4, §5.3): a problem closes the field with a notice, and nothing moves
    /// the focus from where it went.
    FocusLeft,
}

/// The edit the field shows.
#[derive(Debug)]
struct Edit {
    purpose: Purpose,
    /// The field's text as `EN_CHANGE` last reported it.
    text: String,
    /// The lowercased names listed beside the edited name (`sibling_names`), from the rows last
    /// built.
    siblings: HashSet<String>,
    /// The message under the field: the live check's, or a failed Enter's.
    problem: Option<String>,
    /// The field's accessible name (spec §6).
    accessible: String,
    /// `problem` changed since screen readers last heard it.
    announce: bool,
    /// Focus left FastPad while the field had it: it goes back when the window is active again
    /// (spec §5.3).
    refocus: bool,
}

impl Edit {
    fn recheck(&mut self) {
        let problem = check(&self.purpose, &self.text, &self.siblings);
        self.show(problem);
    }

    fn show(&mut self, problem: Option<String>) {
        if problem != self.problem {
            self.problem = problem;
            self.announce = true;
        }
    }
}

/// The Notebook view's inline name field and the one edit it shows, owned by `NotebookView`.
#[derive(Debug)]
pub(crate) struct InlineName {
    /// The field, made on the first edit (spec §7), a child of the side panel.
    field: Option<HWND>,
    /// The field could not be made; it is not tried again.
    failed: bool,
    edit: Option<Edit>,
    /// The edited row's index in the view's rows, from the last `fit`.
    row: Option<usize>,
    /// `row` is the draft row.
    draft: bool,
    /// The field font's text height, measured by `place`.
    text_height: i32,
    colors: Palette,
    /// The field's background for `WM_CTLCOLOREDIT`, made on its first use.
    brush: HBRUSH,
}

impl InlineName {
    pub(crate) fn new() -> Self {
        Self {
            field: None,
            failed: false,
            edit: None,
            row: None,
            draft: false,
            text_height: 0,
            colors: Palette::neutral(),
            brush: std::ptr::null_mut(),
        }
    }

    fn begin(&mut self, purpose: Purpose, text: String, accessible: String) {
        self.edit = Some(Edit {
            purpose,
            text,
            siblings: HashSet::new(),
            problem: None,
            accessible,
            announce: false,
            refocus: false,
        });
        self.row = None;
        self.draft = false;
    }

    /// Ends the open edit, if any; `place` hides the field afterwards, with nothing borrowed.
    /// Returns whether there was one.
    pub(crate) fn end(&mut self) -> bool {
        self.row = None;
        self.draft = false;
        self.edit.take().is_some()
    }

    /// A New note or New folder edit is open: its draft row needs the tree.
    pub(crate) fn wants_draft(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|edit| edit.purpose.draft_parent().is_some())
    }

    /// Fits the edit to freshly built `rows` (spec §5.4): the draft row goes back in as the
    /// first child of its folder, or the renamed row is found; the sibling names and the live
    /// check follow the new rows. False, changing nothing, when the folder or the row is gone:
    /// the caller ends the edit. True with no edit open.
    pub(crate) fn fit(&mut self, rows: &mut Vec<TreeRow>) -> bool {
        self.row = None;
        self.draft = false;
        let Some(edit) = self.edit.as_mut() else {
            return true;
        };
        let Some(parent) = parent_row(rows, edit.purpose.parent()) else {
            return false;
        };
        let row = match edit.purpose.own_row() {
            Some(own) => tree::row_index(rows, &own),
            None => insert_draft(rows, parent),
        };
        let Some(row) = row else {
            return false;
        };
        edit.siblings = sibling_names(rows, parent, Some(row));
        edit.recheck();
        self.draft = edit.purpose.draft_parent().is_some();
        self.row = Some(row);
        true
    }

    pub(crate) fn row(&self) -> Option<usize> {
        self.row
    }

    pub(crate) fn draft_at(&self) -> Option<usize> {
        self.row.filter(|_| self.draft)
    }

    pub(crate) fn draft_icon(&self) -> TreeItem {
        self.edit
            .as_ref()
            .map_or(TreeItem::Note(NoteKind::Markdown), |edit| {
                draft_icon(&edit.purpose, &edit.text)
            })
    }

    pub(crate) fn problem(&self) -> Option<&str> {
        self.edit.as_ref()?.problem.as_deref()
    }

    pub(crate) fn text_height(&self) -> i32 {
        self.text_height
    }

    /// Recolors for a theme change (the panel's paint calls it).
    pub(crate) fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        self.colors = colors;
        if !self.brush.is_null() {
            unsafe { DeleteObject(self.brush) };
            self.brush = std::ptr::null_mut();
        }
    }

    fn brush(&mut self) -> HBRUSH {
        if self.brush.is_null() {
            self.brush = unsafe { CreateSolidBrush(self.colors.editor_background) };
        }
        self.brush
    }
}

impl Drop for InlineName {
    fn drop(&mut self) {
        if !self.brush.is_null() {
            unsafe { DeleteObject(self.brush) };
        }
    }
}

fn with_inline<R>(hwnd: HWND, f: impl FnOnce(&mut InlineName) -> R) -> Option<R> {
    with_view(hwnd, |view| f(&mut view.inline))
}

fn field_of(hwnd: HWND) -> Option<HWND> {
    with_inline(hwnd, |inline| inline.field).flatten()
}

/// Whether an edit is open.
pub(crate) fn is_open(hwnd: HWND) -> bool {
    with_inline(hwnd, |inline| inline.edit.is_some()).unwrap_or(false)
}

/// Whether `control` is the field.
pub(crate) fn owns(hwnd: HWND, control: HWND) -> bool {
    !control.is_null() && with_inline(hwnd, |inline| inline.field == Some(control)).unwrap_or(false)
}

fn field_text(field: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(field);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(field, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(copied.max(0) as usize);
        String::from_utf16_lossy(&buffer)
    }
}

fn set_field_text(field: HWND, text: &str) {
    let wide = wide_null(text);
    unsafe {
        SetWindowTextW(field, wide.as_ptr());
    }
}

/// The field, made now if the view has none yet. A failure is reported once.
fn ensure_field(hwnd: HWND) -> Option<HWND> {
    let (panel, field, failed) = with_view(hwnd, |view| {
        (view.panel, view.inline.field, view.inline.failed)
    })?;
    if field.is_some() || failed {
        return field;
    }
    // Made with nothing of the App borrowed: creating the Edit sends messages to the panel.
    match create_field(panel) {
        Ok(field) => {
            if with_inline(hwnd, |inline| inline.field = Some(field)).is_none() {
                unsafe { DestroyWindow(field) };
                return None;
            }
            Some(field)
        }
        Err(error) => {
            with_inline(hwnd, |inline| inline.failed = true);
            push_notice(
                hwnd,
                format!("FastPad could not show the name field: {error}"),
            );
            None
        }
    }
}

/// A hidden single-line `Edit` inside `panel`, subclassed by `field_proc`.
fn create_field(panel: HWND) -> crate::Result<HWND> {
    let field = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
    if unsafe { SetWindowSubclass(field, Some(field_proc), FIELD_HOOK_ID, 0) } == 0 {
        let error = last_error();
        unsafe {
            DestroyWindow(field);
        }
        return Err(error);
    }
    Ok(field)
}

fn key_down(key: u16) -> bool {
    let state = unsafe { GetKeyState(i32::from(key)) };
    state < 0
}

/// The field's keys (spec §5.1): Enter commits, Esc cancels, Tab does nothing, Ctrl+A selects
/// all and Ctrl+Backspace deletes the word before the caret. Everything else is the Edit's own.
unsafe extern "system" fn field_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(field_proc), subclass_id);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let main = unsafe { GetParent(GetParent(hwnd)) };
    // A single-line Edit beeps at Enter, Escape and Tab, and types a box for Ctrl+A and
    // Ctrl+Backspace: all are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b | 0x09 | 0x01 | 0x7f) {
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = key_down(VK_CONTROL) && !key_down(VK_MENU);
        match wparam as u16 {
            VK_RETURN => {
                commit(main, How::Enter);
                return 0;
            }
            VK_ESCAPE => {
                cancel(main);
                notebook_view::focus_tree(main);
                return 0;
            }
            VK_TAB => return 0,
            key if ctrl && key == u16::from(b'A') => {
                unsafe { SendMessageW(hwnd, EM_SETSEL, 0, -1) };
                return 0;
            }
            VK_BACK if ctrl => {
                delete_word_before(hwnd);
                return 0;
            }
            _ => {}
        }
    }
    if message == WM_KILLFOCUS {
        focus_leaving(main, wparam as HWND);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// Ctrl+Backspace: deletes the selection, or back to `word_start`, as one undoable change.
fn delete_word_before(field: HWND) {
    let (mut start, mut end) = (0_u32, 0_u32);
    unsafe {
        SendMessageW(
            field,
            EM_GETSEL,
            &mut start as *mut u32 as WPARAM,
            &mut end as *mut u32 as LPARAM,
        );
    }
    if start == end {
        let text: Vec<u16> = field_text(field).encode_utf16().collect();
        start = word_start(&text, end as usize) as u32;
        unsafe { SendMessageW(field, EM_SETSEL, start as WPARAM, end as LPARAM) };
    }
    let empty = [0u16];
    unsafe { SendMessageW(field, EM_REPLACESEL, 1, empty.as_ptr() as LPARAM) };
}

/// Starts an edit for `purpose` (spec §3): the one already open commits first (§3.4), the find
/// bar and the name bar close, the Notebook view shows, a draft's folder expands, and the field
/// takes the keyboard focus over its row. Nothing touches the disk.
fn start(hwnd: HWND, purpose: Purpose) {
    if !library_host::ready_library(hwnd) || side_panel::windows(hwnd).is_none() {
        return;
    }
    commit(hwnd, How::FocusLeft);
    super::main_window::close_find_bar(hwnd);
    library_host::close_name_box(hwnd);
    if side_panel::current_view(hwnd) != SidebarView::Notebook {
        side_panel::show_view(hwnd, SidebarView::Notebook, false);
    }
    if let Some(parent) = purpose.draft_parent() {
        let mut folders = tree::ancestors(parent);
        if !parent.as_os_str().is_empty() {
            folders.push(parent.to_path_buf());
        }
        for folder in folders {
            library_host::set_expanded(hwnd, &folder, true);
        }
    }
    let Some(field) = ensure_field(hwnd) else {
        return;
    };
    let text = purpose.current_name().unwrap_or_default();
    let (select_from, select_to) = rename_selection(&text, purpose.is_folder());
    let notebook = library_host::folder(hwnd)
        .map(|root| library_host::notebook_name(&root))
        .unwrap_or_default();
    let accessible = accessible_name(&purpose, &notebook);
    with_inline(hwnd, |inline| {
        inline.begin(purpose, text.clone(), accessible.clone())
    });
    side_panel::with_accessible_events(hwnd, || notebook_view::rebuild(hwnd));
    // The rebuild found no folder or row for it.
    if !is_open(hwnd) {
        return;
    }
    with_view(hwnd, NotebookView::reveal_edit);
    // Filled and focused with nothing of the App borrowed: the Edit sends EN_CHANGE to the
    // panel, and SetFocus sends focus messages.
    set_field_text(field, &text);
    let _ = crate::platform::annotation::annotate(field, &accessible, "");
    place(hwnd);
    unsafe {
        SetFocus(field);
        SendMessageW(field, EM_SETSEL, select_from, select_to as LPARAM);
    }
}

/// The folder a new item goes in, relative to the notebook (spec §3.1): `parent`, else the
/// folder of the selected row, else the root. `None` for a path that is not a plain relative
/// folder, so nothing is expanded or drafted outside the notebook.
fn target_folder(hwnd: HWND, parent: Option<PathBuf>) -> Option<PathBuf> {
    let parent = parent.unwrap_or_else(|| {
        let root = library_host::folder(hwnd);
        notebook_view::selected_folder(hwnd)
            .zip(root)
            .map(|(selected, root)| library_host::relative_folder(&root, &selected))
            .unwrap_or_default()
    });
    (parent.as_os_str().is_empty() || tree::is_plain_relative_folder(&parent)).then_some(parent)
}

/// The header's New folder, "New folder here" (`parent`) and Notebook: New folder… (spec §3.2).
pub(crate) fn new_folder(hwnd: HWND, parent: Option<PathBuf>) {
    if let Some(parent) = target_folder(hwnd, parent) {
        start(hwnd, Purpose::NewFolder(parent));
    }
}

/// The header's "+", "New note here" (`parent`) and Notebook: New note… (spec §3.1).
pub(crate) fn new_note(hwnd: HWND, parent: Option<PathBuf>) {
    if let Some(parent) = target_folder(hwnd, parent) {
        start(hwnd, Purpose::NewNote(parent));
    }
}

/// F2 or Rename… on a note or folder row (spec §3.3): the field over that row's name.
pub(crate) fn rename(hwnd: HWND, row: &RowKind) {
    let purpose = match row {
        RowKind::Note(relative) => Purpose::RenameNote(relative.clone()),
        RowKind::Folder(relative) => Purpose::RenameFolder(relative.clone()),
        RowKind::Unsaved(_) | RowKind::Draft => return,
    };
    start(hwnd, purpose);
}

/// Note: Rename… on the note at `path` (absolute; the palette's recorded row): its row,
/// revealed, else the name bar for a file the tree has no row for (spec §3.3).
pub(crate) fn rename_note_at(hwnd: HWND, path: &Path) {
    match reveal(hwnd, path) {
        Some(relative) => start(hwnd, Purpose::RenameNote(relative)),
        // Only the active tab's file goes to the name bar: it renames the active tab, and any
        // other note is not the one the user chose.
        None if is_active_file(hwnd, path) => library_host::rename_note(hwnd),
        None => {}
    }
}

/// Whether `path` is the active tab's file. No notice for an untitled tab.
fn is_active_file(hwnd: HWND, path: &Path) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .active()
            .and_then(|active| active.path.as_deref())
            .is_some_and(|active| library::model::same_path(active, path))
    })
}

/// Note: Rename… with no row focused: the active tab's note (spec §3.3).
pub(crate) fn rename_active(hwnd: HWND) {
    if let Some(path) = library_host::active_file(hwnd) {
        rename_note_at(hwnd, &path);
    }
}

/// Shows the note at `path` in the Notebook view (opening the sidebar on it if hidden or on
/// another view), its folders expanded and its row selected and scrolled into view (spec
/// §3.3). `None` when it has no row there: outside the notebook, not a note, not listed yet, or
/// no sidebar, which change nothing; or a listed note whose row is still not found once the view
/// is shown and its folders expanded, which stay so.
fn reveal(hwnd: HWND, path: &Path) -> Option<PathBuf> {
    let root = library_host::folder(hwnd)?;
    if !library::is_inside(&root, path) || side_panel::windows(hwnd).is_none() {
        return None;
    }
    let relative = library::record_path(&root, path);
    let listed = with_state(hwnd, |state| {
        state
            .notes
            .iter()
            .any(|note| library::model::same_path(&note.path, &relative))
    })
    .unwrap_or(false);
    if !listed {
        return None;
    }
    if side_panel::current_view(hwnd) != SidebarView::Notebook {
        side_panel::show_view(hwnd, SidebarView::Notebook, false);
    }
    for folder in tree::ancestors(&relative) {
        library_host::set_expanded(hwnd, &folder, true);
    }
    if notebook_view::stale(hwnd) {
        side_panel::with_accessible_events(hwnd, || notebook_view::rebuild(hwnd));
    }
    notebook_view::select_row(hwnd, &RowKind::Note(relative.clone())).then_some(relative)
}

/// `EN_CHANGE` from the field: the live check runs on what is typed now, against the rows in
/// memory (spec §4.4). No disk access.
pub(crate) fn changed(hwnd: HWND) {
    let Some(field) = field_of(hwnd) else {
        return;
    };
    // Read with nothing of the App borrowed.
    let text = field_text(field);
    let panel = with_view(hwnd, |view| {
        let edit = view.inline.edit.as_mut()?;
        edit.text = text;
        edit.recheck();
        Some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
    announce(hwnd);
}

/// Tells screen readers about a problem that appeared, changed or went (spec §6): the field's
/// description, and `EVENT_OBJECT_DESCRIPTIONCHANGE`.
fn announce(hwnd: HWND) {
    let Some((field, name, description)) = with_inline(hwnd, |inline| {
        let field = inline.field?;
        let edit = inline.edit.as_mut()?;
        std::mem::take(&mut edit.announce).then(|| {
            (
                field,
                edit.accessible.clone(),
                edit.problem.clone().unwrap_or_default(),
            )
        })
    })
    .flatten() else {
        return;
    };
    let _ = crate::platform::annotation::annotate(field, &name, &description);
    super::sidebar_accessibility::raise(field, &[(EVENT_OBJECT_DESCRIPTIONCHANGE, 0)]);
}

/// The field is losing the focus to `to` (spec §5.3). Another window of FastPad commits the
/// edit, once the focus change is over; none (another app, or the window deactivating) keeps it
/// and arms `refocus`. Both only post `WM_FASTPAD_INLINE_NAME_LEFT`: the focus change may come
/// from a `SetFocus` made while its caller still holds the app, so nothing is borrowed here.
fn focus_leaving(hwnd: HWND, to: HWND) {
    let ours = !to.is_null()
        && unsafe { GetWindowThreadProcessId(to, std::ptr::null_mut()) }
            == unsafe { GetCurrentThreadId() };
    let wparam = if ours { 0 } else { LEFT_FASTPAD };
    unsafe { PostMessageW(hwnd, crate::window::WM_FASTPAD_INLINE_NAME_LEFT, wparam, 0) };
}

/// `WM_FASTPAD_INLINE_NAME_LEFT`'s wparam when the focus left FastPad. The other value, 0, is
/// also what a message held through a modal prompt is re-posted with.
pub(crate) const LEFT_FASTPAD: usize = 1;

/// `WM_FASTPAD_INLINE_NAME_LEFT` from the focus leaving FastPad: the open edit stays and takes
/// the focus back when the window is activated again (`refocus`).
pub(crate) fn focus_left_fastpad(hwnd: HWND) {
    with_inline(hwnd, |inline| {
        if let Some(edit) = inline.edit.as_mut() {
            edit.refocus = true;
        }
    });
}

/// `WM_FASTPAD_INLINE_NAME_LEFT` from the focus moving to another window of FastPad: commits the
/// open edit unless the focus is back in the field (a menu closed, or another edit started
/// meanwhile).
pub(crate) fn focus_left(hwnd: HWND) {
    let Some(field) = with_inline(hwnd, |inline| inline.edit.as_ref().and(inline.field)).flatten()
    else {
        return;
    };
    if unsafe { GetFocus() } != field {
        commit(hwnd, How::FocusLeft);
    }
}

/// The frame got the focus back (the window was activated again): the field takes it, if focus
/// left FastPad from it (spec §5.3). Returns whether it did.
pub(crate) fn refocus(hwnd: HWND) -> bool {
    let field = with_inline(hwnd, |inline| {
        let edit = inline.edit.as_mut()?;
        std::mem::take(&mut edit.refocus)
            .then_some(inline.field)
            .flatten()
    })
    .flatten();
    match field {
        Some(field) => {
            unsafe { SetFocus(field) };
            true
        }
        None => false,
    }
}

/// `WM_CTLCOLOREDIT` for the field.
pub(crate) fn control_color(hwnd: HWND, dc: HDC) -> HBRUSH {
    with_inline(hwnd, |inline| {
        unsafe {
            SetTextColor(dc, inline.colors.editor_foreground);
            SetBkColor(dc, inline.colors.editor_background);
        }
        inline.brush()
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Moves the field over its row's name, clipped to the list (spec §5.4), or hides it: when no
/// edit is open, when it has no row, or when the row is out of view. A field scrolled out of
/// view keeps the focus and goes on editing; one whose edit ended hands the focus to the tree.
/// Runs after every rebuild, scroll and layout, with nothing of the App borrowed.
pub(crate) fn place(hwnd: HWND) {
    let Some((field, editing)) = with_inline(hwnd, |inline| {
        inline.field.map(|field| (field, inline.edit.is_some()))
    })
    .flatten() else {
        return;
    };
    let font = super::main_window::ui_fonts(hwnd).text;
    let text_height = crate::window::panel::text_height(field, font);
    let layout = with_view(hwnd, |view| {
        view.inline.text_height = text_height;
        view.inline_layout()
    })
    .flatten();
    unsafe {
        match layout {
            Some(layout) => {
                if !font.is_null() {
                    SendMessageW(field, WM_SETFONT, font as WPARAM, 0);
                }
                let edit = layout.edit;
                MoveWindow(
                    field,
                    edit.left,
                    edit.top,
                    edit.right - edit.left,
                    edit.bottom - edit.top,
                    1,
                );
                ShowWindow(field, SW_SHOWNA);
            }
            None => {
                let focused = GetFocus() == field;
                if editing && focused {
                    hide_keeping_focus(field);
                } else {
                    if focused {
                        SetFocus(GetParent(field));
                    }
                    ShowWindow(field, SW_HIDE);
                }
            }
        }
    }
    announce(hwnd);
}

/// Hides the focused field without taking its focus: `ShowWindow(SW_HIDE)` would move the
/// focus to the panel, and the typing with it, while the field is only scrolled out of view
/// (inline naming spec §5.4). It shrinks away first, so the panel repaints where it was, then
/// turns invisible by its style alone; `ShowWindow` shows it again.
unsafe fn hide_keeping_focus(field: HWND) {
    unsafe {
        MoveWindow(field, 0, 0, 0, 0, 1);
        let style = GetWindowLongPtrW(field, GWL_STYLE);
        SetWindowLongPtrW(field, GWL_STYLE, style & !(WS_VISIBLE as isize));
    }
}

/// Ends the edit with no disk access; returns whether one was open.
fn end(hwnd: HWND) -> bool {
    with_inline(hwnd, InlineName::end).unwrap_or(false)
}

/// Cancels the open edit, if any: the draft row goes and the field hides (spec §5.1, §5.4).
pub(crate) fn cancel(hwnd: HWND) {
    if end(hwnd) {
        side_panel::with_accessible_events(hwnd, || notebook_view::rebuild(hwnd));
    }
}

/// An empty or unchanged name: the edit cancels without a message (spec §5.2). Enter returns
/// the focus to the tree, as Esc does.
fn cancelled(hwnd: HWND, how: How) {
    cancel(hwnd);
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
}

/// A commit that cannot go ahead (spec §5.2, §5.3): after Enter the message shows under the
/// field, which stays; after focus left, the field closes and the message is a notice.
fn fail(hwnd: HWND, how: How, message: String) {
    match how {
        How::Enter => {
            let panel = with_view(hwnd, |view| {
                if let Some(edit) = view.inline.edit.as_mut() {
                    edit.show(Some(message));
                }
                view.panel
            });
            if let Some(panel) = panel {
                unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
            }
            announce(hwnd);
        }
        How::FocusLeft => {
            cancel(hwnd);
            push_notice(hwnd, message);
        }
    }
}

/// Commits the open edit, if any (spec §5.2): refused while a problem shows, else the one disk
/// call its purpose makes.
pub(crate) fn commit(hwnd: HWND, how: How) {
    let Some((purpose, problem, field)) = with_inline(hwnd, |inline| {
        let edit = inline.edit.as_ref()?;
        Some((edit.purpose.clone(), edit.problem.clone(), inline.field?))
    })
    .flatten() else {
        return;
    };
    if let Some(problem) = problem {
        fail(hwnd, how, problem);
        return;
    }
    // Read with nothing of the App borrowed.
    let text = field_text(field);
    match purpose {
        Purpose::NewNote(parent) => commit_new_note(hwnd, how, &parent, &text),
        Purpose::NewFolder(parent) => commit_new_folder(hwnd, how, &parent, &text),
        Purpose::RenameNote(relative) => commit_rename_note(hwnd, how, &relative, &text),
        Purpose::RenameFolder(relative) => commit_rename_folder(hwnd, how, &relative, &text),
    }
}

/// Opens an edit for `purpose` with `text` typed, skipping `start`'s checks and its row, and
/// commits it with Enter: reaches the commits' own path guards with a purpose no row could give.
#[cfg(test)]
pub(crate) fn commit_unchecked(hwnd: HWND, purpose: Purpose, text: &str) {
    let Some(field) = ensure_field(hwnd) else {
        return;
    };
    with_inline(hwnd, |inline| {
        inline.begin(purpose, String::new(), String::new())
    });
    set_field_text(field, text);
    commit(hwnd, How::Enter);
}

/// New folder (spec §5.2): the folder is made with the one disk call, listed, and its row
/// selected. After Enter the focus stays in the tree.
fn commit_new_folder(hwnd: HWND, how: How, parent: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let Some(name) = title::folder_name(text) else {
        cancelled(hwnd, how);
        return;
    };
    let relative = parent.join(&name);
    // Defence in depth: the folder made must be a plain relative one inside the notebook.
    if !tree::is_plain_relative_folder(&relative) {
        cancel(hwnd);
        return;
    }
    if let Err(error) = std::fs::create_dir(root.join(&relative)) {
        // A file, or a folder the tree doesn't list, may already have the name.
        let error = if error.kind() == std::io::ErrorKind::AlreadyExists {
            library_host::folder_taken_error(&name)
        } else {
            format!("FastPad could not create the folder: {error}")
        };
        fail(hwnd, how, error);
        return;
    }
    with_state(hwnd, |state| state.add_folder(&relative));
    for ancestor in tree::ancestors(&relative) {
        library_host::set_expanded(hwnd, &ancestor, true);
    }
    end(hwnd);
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Folder(relative.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
}

/// New note (spec §4.1, §5.2): the empty file is made with the one disk call, never over an
/// existing file, listed, and opened as a normal tab. After Enter the focus goes to the editor.
fn commit_new_note(hwnd: HWND, how: How, parent: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let Some(name) = title::new_note_name(text) else {
        cancelled(hwnd, how);
        return;
    };
    // Defence in depth: the note made must be inside the notebook.
    if !parent.as_os_str().is_empty() && !tree::is_plain_relative_folder(parent) {
        cancel(hwnd);
        return;
    }
    let folder = root.join(parent);
    let path = folder.join(&name);
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(drop);
    if let Err(error) = created {
        // A file the tree doesn't list may already have the name.
        let error = if error.kind() == std::io::ErrorKind::AlreadyExists {
            let (stem, extension) = title::split_typed_name(text, "md");
            library_host::name_taken_error(&folder, &stem, &extension)
        } else {
            format!("FastPad could not create {name}: {error}")
        };
        fail(hwnd, how, error);
        return;
    }
    with_state(hwnd, |state| state.add_note(&path));
    end(hwnd);
    side_panel::refresh(hwnd);
    let mode = super::main_window::OpenMode::Permanent;
    if let Err(error) = super::main_window::open_note(hwnd, &path, mode, how == How::Enter) {
        super::main_window::report_open_failure(hwnd, &path, &error);
    }
}

/// A note rename (spec §4.3, §5.2): the one disk call, never over another file, then the tab
/// that has it open follows (dirty or preview alike) and the library follows it. A tab that
/// cannot follow undoes the rename. No tab is opened. After Enter the focus stays in the tree.
fn commit_rename_note(hwnd: HWND, how: How, relative: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let current = relative
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());
    let Some(name) = title::renamed_note_name(text, current.as_deref()) else {
        cancelled(hwnd, how);
        return;
    };
    let new = relative.with_file_name(&name);
    if new == relative {
        cancelled(hwnd, how);
        return;
    }
    let moved = match tree_move::move_note(hwnd, &root, relative, &new) {
        Ok(moved) => moved,
        Err(error) => {
            let message = match error {
                MoveError::Taken => {
                    let (stem, extension) = title::split_rename(text, current.as_deref());
                    let parent = root
                        .join(relative)
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    library_host::name_taken_error(&parent, &stem, &extension.unwrap_or_default())
                }
                MoveError::Missing(error) | MoveError::Failed(error) => {
                    format!("FastPad could not rename the file: {error}")
                }
                MoveError::TabCantFollow => "Another tab already has that file open.".to_owned(),
            };
            fail(hwnd, how, message);
            return;
        }
    };
    end(hwnd);
    let row = library::record_path(&root, &root.join(&new));
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Note(row.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
    if !moved.stuck.is_empty() {
        let old_name = relative.file_name().unwrap_or_default().to_string_lossy();
        push_notice(
            hwnd,
            library_host::rename_undo_failed_notice(&old_name, &name, &moved.stuck),
        );
    }
    if moved.rebound {
        // The extension may have changed, and with it the language.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd,
                crate::window::WM_FASTPAD_APPLY_LANGUAGE,
                0,
                0,
            );
        }
    }
}

/// A folder rename (spec §4.3, §5.2): the one disk call, never onto another name, then the open
/// tabs under it follow and the library follows it. A tab that cannot follow undoes the whole
/// rename; if the undo fails, the rename stands and a notice names the tabs left on their old
/// paths. After Enter the focus stays in the tree.
fn commit_rename_folder(hwnd: HWND, how: How, old: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    // Defence in depth: an empty or escaping path would rename the notebook root or a folder
    // outside it.
    if !tree::is_plain_relative_folder(old) {
        cancel(hwnd);
        return;
    }
    let Some(name) = title::folder_name(text) else {
        cancelled(hwnd, how);
        return;
    };
    let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
    let new = parent.join(&name);
    if new.as_os_str() == old.as_os_str() || !tree::is_plain_relative_folder(&new) {
        cancelled(hwnd, how);
        return;
    }
    let moved = match tree_move::move_folder(hwnd, &root, old, &new) {
        Ok(moved) => moved,
        Err(error) => {
            let message = match error {
                MoveError::Taken => library_host::folder_taken_error(&name),
                MoveError::Missing(error) | MoveError::Failed(error) => {
                    format!("FastPad could not rename the folder: {error}")
                }
                MoveError::TabCantFollow => "Another tab already has that file open.".to_owned(),
            };
            fail(hwnd, how, message);
            return;
        }
    };
    end(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Folder(new.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
    if !moved.stuck.is_empty() {
        let old_name = old.file_name().unwrap_or_default().to_string_lossy();
        push_notice(
            hwnd,
            library_host::rename_undo_failed_notice(&old_name, &name, &moved.stuck),
        );
    }
}

#[cfg(test)]
pub(crate) fn field_hwnd(hwnd: HWND) -> Option<HWND> {
    field_of(hwnd)
}

#[cfg(test)]
pub(crate) fn purpose(hwnd: HWND) -> Option<Purpose> {
    with_inline(hwnd, |inline| {
        inline.edit.as_ref().map(|edit| edit.purpose.clone())
    })
    .flatten()
}

#[cfg(test)]
pub(crate) fn problem(hwnd: HWND) -> Option<String> {
    with_inline(hwnd, |inline| inline.problem().map(str::to_owned)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: RowKind, name: &str, depth: u16, expanded: bool) -> TreeRow {
        TreeRow {
            kind,
            depth,
            name: name.to_owned(),
            pinned: false,
            expanded,
        }
    }

    fn names(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn purpose_accessors_read_the_parent_draft_parent_and_own_row() {
        // Break caught: a new item's folder, draft-row parent or own row read wrong, which
        // would misplace the draft row or check a rename against its own name.
        let new_note = Purpose::NewNote("sub".into());
        assert_eq!(new_note.parent(), Path::new("sub"));
        assert_eq!(new_note.draft_parent(), Some(Path::new("sub")));
        assert_eq!(new_note.own_row(), None);

        let new_folder = Purpose::NewFolder(PathBuf::new());
        assert_eq!(new_folder.parent(), Path::new(""));
        assert_eq!(new_folder.draft_parent(), Some(Path::new("")));
        assert_eq!(new_folder.own_row(), None);

        let rename_note = Purpose::RenameNote(r"sub\plan.md".into());
        assert_eq!(rename_note.parent(), Path::new("sub"));
        assert_eq!(rename_note.draft_parent(), None);
        assert_eq!(
            rename_note.own_row(),
            Some(RowKind::Note(r"sub\plan.md".into()))
        );

        let rename_folder = Purpose::RenameFolder("archive".into());
        assert_eq!(rename_folder.parent(), Path::new(""));
        assert_eq!(rename_folder.draft_parent(), None);
        assert_eq!(
            rename_folder.own_row(),
            Some(RowKind::Folder("archive".into()))
        );
    }

    #[test]
    fn typed_names_cancel_when_empty_or_unchanged_and_a_case_change_is_a_rename() {
        // Break caught: an empty draft creating "Untitled.md", Enter on an unchanged rename
        // renaming onto itself, or "plan.md" to "Plan.md" treated as no change (spec §4).
        let note = Purpose::NewNote(PathBuf::new());
        assert_eq!(typed_name(&note, "todo").as_deref(), Some("todo.md"));
        assert_eq!(typed_name(&note, "  "), None);
        let folder = Purpose::NewFolder("sub".into());
        assert_eq!(typed_name(&folder, " a/b: c?. ").as_deref(), Some("ab c"));
        assert_eq!(typed_name(&folder, "..."), None);
        let rename = Purpose::RenameNote(r"sub\plan.md".into());
        assert_eq!(typed_name(&rename, "plan.md"), None);
        assert_eq!(typed_name(&rename, "Plan.md").as_deref(), Some("Plan.md"));
        assert_eq!(typed_name(&rename, "draft").as_deref(), Some("draft.md"));
        let rename_folder = Purpose::RenameFolder("v1.2".into());
        assert_eq!(typed_name(&rename_folder, "v1.2"), None);
        assert_eq!(typed_name(&rename_folder, "V1.2").as_deref(), Some("V1.2"));
    }

    #[test]
    fn the_live_check_finds_a_taken_name_ignoring_case_and_refuses_hidden_folder_names() {
        // Break caught: "TODO" slipping past a listed todo.md, a note's own name reported as
        // taken, or a ".git" folder created that the next rescan hides (spec §4.4).
        let siblings = names(&["todo.md", "archive"]);
        let note = Purpose::NewNote(PathBuf::new());
        assert_eq!(
            check(&note, "TODO", &siblings).as_deref(),
            Some("TODO.md already exists here.")
        );
        assert_eq!(check(&note, "other", &siblings), None);
        assert_eq!(check(&note, "", &siblings), None, "an empty name cancels");
        let folder = Purpose::NewFolder(PathBuf::new());
        assert_eq!(
            check(&folder, "Archive", &siblings).as_deref(),
            Some("Archive already exists here.")
        );
        assert_eq!(
            check(&folder, ".git", &siblings).as_deref(),
            Some("FastPad hides folders named \u{201c}.git\u{201d}. Choose another name.")
        );
        // A rename's own row is left out of `siblings` (`sibling_names`' `own`).
        let rename = Purpose::RenameNote("plan.md".into());
        assert_eq!(check(&rename, "PLAN.md", &names(&["b.md"])), None);
        assert_eq!(
            check(&rename, "b", &names(&["b.md"])).as_deref(),
            Some("b.md already exists here.")
        );
    }

    #[test]
    fn a_rename_selects_the_stem_of_a_note_and_all_of_a_folder() {
        // Break caught: typing over "a.md" also replacing ".md", ".gitignore" opening with
        // nothing selected, or a "v1.2" folder keeping ".2" (spec §3.3).
        assert_eq!(rename_selection("a.md", false), (0, 1));
        assert_eq!(rename_selection(".gitignore", false), (0, 10));
        assert_eq!(rename_selection("archive.tar.gz", false), (0, 11));
        assert_eq!(rename_selection("README", false), (0, 6));
        assert_eq!(rename_selection("v1.2", true), (0, 4));
        assert_eq!(rename_selection("é.md", false), (0, 1), "UTF-16 units");
    }

    #[test]
    fn the_draft_icon_follows_the_typed_note_extension() {
        // Break caught: a new JSON note drawn as Markdown, or a folder draft drawn as a note.
        let note = Purpose::NewNote(PathBuf::new());
        let markdown = TreeItem::Note(NoteKind::Markdown);
        assert_eq!(draft_icon(&note, ""), markdown);
        assert_eq!(
            draft_icon(&note, "data.json"),
            TreeItem::Note(note_kind(Some("json")))
        );
        assert_eq!(draft_icon(&note, "v1.2"), markdown);
        assert_eq!(
            draft_icon(&Purpose::NewFolder(PathBuf::new()), "x.json"),
            TreeItem::Folder { expanded: false }
        );
    }

    #[test]
    fn the_field_is_named_for_what_it_names_and_where() {
        // Break caught: a screen reader hearing a bare "edit", or the root named "" (spec §6).
        assert_eq!(
            accessible_name(&Purpose::NewNote(PathBuf::new()), "Notes"),
            "New note name, in Notes"
        );
        assert_eq!(
            accessible_name(&Purpose::NewFolder(r"a\sub".into()), "Notes"),
            "New folder name, in sub"
        );
        assert_eq!(
            accessible_name(&Purpose::RenameNote(r"a\b.md".into()), "Notes"),
            "Rename b.md"
        );
    }

    #[test]
    fn siblings_are_the_rows_directly_in_the_folder_without_the_own_row() {
        // Break caught: a name in a subfolder, an unsaved tab's label or the renamed row itself
        // counted as taken, or a sibling below a nested folder missed.
        let rows = vec![
            row(RowKind::Unsaved(1), "Untitled", 0, false),
            row(RowKind::Folder("sub".into()), "sub", 0, true),
            row(RowKind::Note(r"sub\A.md".into()), "A.md", 1, false),
            row(RowKind::Folder(r"sub\deep".into()), "deep", 1, true),
            row(RowKind::Note(r"sub\deep\x.md".into()), "x.md", 2, false),
            row(RowKind::Note(r"sub\b.md".into()), "b.md", 1, false),
            row(RowKind::Note("top.md".into()), "top.md", 0, false),
        ];
        assert_eq!(parent_row(&rows, Path::new("sub")), Some(Some(1)));
        assert_eq!(parent_row(&rows, Path::new("")), Some(None));
        assert_eq!(parent_row(&rows, Path::new("gone")), None);
        assert_eq!(
            sibling_names(&rows, Some(1), None),
            names(&["a.md", "deep", "b.md"])
        );
        assert_eq!(
            sibling_names(&rows, Some(1), Some(2)),
            names(&["deep", "b.md"])
        );
        assert_eq!(sibling_names(&rows, None, None), names(&["sub", "top.md"]));
    }

    #[test]
    fn the_draft_row_is_the_first_child_of_an_expanded_folder_or_below_the_unsaved_rows() {
        // Break caught: a draft row at the end of its folder, at the wrong depth, above the
        // unsaved rows, or inside a collapsed folder (spec §3.1).
        let mut rows = vec![
            row(RowKind::Unsaved(1), "Untitled", 0, false),
            row(RowKind::Folder("sub".into()), "sub", 0, true),
            row(RowKind::Note(r"sub\a.md".into()), "a.md", 1, false),
            row(RowKind::Folder("shut".into()), "shut", 0, false),
        ];
        assert_eq!(insert_draft(&mut rows, Some(1)), Some(2));
        assert_eq!((rows[2].kind.clone(), rows[2].depth), (RowKind::Draft, 1));
        rows.remove(2);
        assert_eq!(insert_draft(&mut rows, None), Some(1));
        assert_eq!((rows[1].kind.clone(), rows[1].depth), (RowKind::Draft, 0));
        rows.remove(1);
        assert_eq!(insert_draft(&mut rows, Some(3)), None, "collapsed");
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn ctrl_backspace_deletes_spaces_then_one_run_of_word_or_punctuation() {
        // Break caught: Ctrl+Backspace typing a box character or deleting the whole name.
        let wide = |text: &str| text.encode_utf16().collect::<Vec<_>>();
        assert_eq!(word_start(&wide("my note.md"), 10), 8);
        assert_eq!(word_start(&wide("my note.md"), 8), 7);
        assert_eq!(word_start(&wide("my note  "), 9), 3);
        assert_eq!(word_start(&wide("my"), 0), 0);
        assert_eq!(word_start(&wide("my"), 99), 0, "a caret past the end");
    }

    #[test]
    fn the_field_covers_the_name_up_to_the_pin_and_stays_inside_the_list() {
        // Break caught: the field drawn over the chevron, icon or pin, over the header when its
        // row is scrolled up, or below the list's bottom edge (spec §3.3, §5.4).
        let list = RECT {
            left: 0,
            top: 38,
            right: 240,
            bottom: 400,
        };
        let row_at = |top: i32| RECT {
            left: 0,
            top,
            right: 240,
            bottom: top + 26,
        };
        let parts = super::super::notebook_view::row_parts(row_at(60), 1, 96);
        let layout = field_layout(row_at(60), list, 1, 96, 16).unwrap();
        assert!(layout.frame.left > parts.icon.right - 1);
        assert_eq!(layout.frame.right, parts.pin.left);
        assert!(layout.edit.left > layout.frame.left && layout.edit.right < layout.frame.right);
        assert!(layout.edit.top > layout.frame.top && layout.edit.bottom < layout.frame.bottom);
        assert!(
            field_layout(row_at(12), list, 1, 96, 16).is_none(),
            "under the header"
        );
        assert!(
            field_layout(row_at(400), list, 1, 96, 16).is_none(),
            "below the list"
        );
        let cut = field_layout(row_at(390), list, 1, 96, 16).unwrap();
        assert!(cut.frame.bottom <= list.bottom && cut.edit.bottom <= list.bottom);
    }

    #[test]
    fn the_problem_goes_under_the_field_or_above_it_on_the_last_row() {
        // Break caught: a message drawn past the list's bottom, hidden under the next paint, or
        // over the header (spec §4.4).
        let list = RECT {
            left: 0,
            top: 38,
            right: 240,
            bottom: 400,
        };
        let frame = RECT {
            left: 55,
            top: 62,
            right: 216,
            bottom: 84,
        };
        let below = message_rect(frame, list, 30);
        assert_eq!((below.top, below.bottom), (84, 114));
        let last = RECT {
            top: 380,
            bottom: 398,
            ..frame
        };
        let above = message_rect(last, list, 30);
        assert_eq!((above.top, above.bottom), (350, 380));
        let tiny = RECT {
            top: 38,
            bottom: 70,
            ..list
        };
        let first = RECT {
            top: 40,
            bottom: 60,
            ..frame
        };
        assert_eq!(message_rect(first, tiny, 40).top, 38);
    }

    #[test]
    fn only_ctrl_z_and_ctrl_y_among_the_field_keys_are_accelerators() {
        // Break caught: an accelerator on Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V, Del, Home, End,
        // Ctrl+Left, Ctrl+Right or Ctrl+Backspace taking the key from the field, which keeps
        // only Ctrl+Z and Ctrl+Y from the table (inline naming spec §5.1, §11).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_BACK, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RIGHT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{FCONTROL, FSHIFT};
        let specs = crate::window::menus::accelerator_specs();
        let bound = |modifiers: u8, key: u16| {
            specs
                .iter()
                .any(|spec| spec.modifiers == modifiers && spec.key == key)
        };
        let letter = |key: u8| u16::from(key);
        for (modifiers, key) in [
            (FCONTROL, letter(b'A')),
            (FCONTROL, letter(b'C')),
            (FCONTROL, letter(b'X')),
            (FCONTROL, letter(b'V')),
            (0, VK_DELETE),
            (0, VK_HOME),
            (0, VK_END),
            (FSHIFT, VK_HOME),
            (FSHIFT, VK_END),
            (FCONTROL, VK_LEFT),
            (FCONTROL, VK_RIGHT),
            (FCONTROL | FSHIFT, VK_LEFT),
            (FCONTROL | FSHIFT, VK_RIGHT),
            (FCONTROL, VK_BACK),
        ] {
            assert!(!bound(modifiers, key), "{modifiers:#x} {key:#x}");
        }
        assert!(
            bound(FCONTROL, letter(b'Z')),
            "Ctrl+Z is Undo: the field keeps it"
        );
        assert!(
            bound(FCONTROL, letter(b'Y')),
            "Ctrl+Y is Redo: the field keeps it"
        );
    }
}
