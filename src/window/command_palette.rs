//! Ctrl+Shift+P command palette: a filter field over a list of every command the window can run,
//! overlaid at the top of the editor. A small painted panel hosts a native `Edit` and an
//! owner-drawn `ListBox` in the editor's palette colors; the main window owns showing, layout,
//! and running the chosen command.

use crate::library::quick_open::QuickMatch;
use crate::platform::{last_error, wide_null};
use crate::window::commands::CommandId;
use crate::window::menus::{AcceleratorSpec, accelerator_specs};
use crate::window::palette::Palette;
use crate::window::panel::{create_child, create_panel, fill, inset, scale, text_height};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontIndirectW, CreateSolidBrush, DEFAULT_GUI_FONT, DT_CALCRECT,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject,
    DrawTextW, EndPaint, FW_BOLD, GetCurrentObject, GetObjectW, GetStockObject, HBRUSH, HDC, HFONT,
    InvalidateRect, LOGFONTW, OBJ_FONT, PAINTSTRUCT, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE,
    RedrawWindow, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_GETMARGINS, EM_REPLACESEL, EM_SETSEL, EM_UNDO, ODS_SELECTED, SetWindowTheme,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN,
    VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetWindowTextLengthW, GetWindowTextW, HWND_TOP,
    LB_ADDSTRING, LB_GETCURSEL, LB_ITEMFROMPOINT, LB_RESETCONTENT, LB_SETCURSEL, LB_SETITEMHEIGHT,
    LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT, LBS_OWNERDRAWFIXED, MoveWindow, SW_HIDE, SW_SHOWNA,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow,
    WM_CHAR, WM_CLEAR, WM_CUT, WM_GETFONT, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_NCDESTROY, WM_PAINT, WM_PASTE, WM_SETFOCUS, WM_SETFONT, WM_SETTEXT, WM_UNDO,
    WS_CHILD, WS_VISIBLE, WS_VSCROLL,
};

/// One runnable row: the command and what the palette calls it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PaletteEntry {
    pub command: CommandId,
    pub label: &'static str,
}

const fn entry(label: &'static str, command: CommandId) -> PaletteEntry {
    PaletteEntry { command, label }
}

/// Every command reachable from the palette, in the order an empty query lists them. `SelectTabN`
/// is positional and the palette itself is already open, so neither is listed.
pub(crate) const ENTRIES: [PaletteEntry; 74] = [
    entry("File: New tab", CommandId::New),
    entry("File: Open...", CommandId::Open),
    entry("File: Open notebook...", CommandId::OpenFolder),
    entry("File: Open recent notebook...", CommandId::OpenRecentFolder),
    entry("Go to note\u{2026}", CommandId::QuickOpen),
    entry("File: Save", CommandId::Save),
    entry("File: Save as...", CommandId::SaveAs),
    entry("File: Close tab", CommandId::CloseTab),
    entry("File: Close all tabs", CommandId::CloseAllTabs),
    entry(
        "File: Toggle session restore",
        CommandId::ToggleRestoreSession,
    ),
    entry("Notes: Toggle notes mode", CommandId::ToggleNotesMode),
    entry(
        "Notes: Toggle autosave for this notebook",
        CommandId::ToggleFolderAutosave,
    ),
    entry("Notebook: Close", CommandId::CloseNotebook),
    entry(
        "Notebook: Toggle favorite",
        CommandId::ToggleNotebookFavorite,
    ),
    entry("Notebook: New note\u{2026}", CommandId::NoteNew),
    entry("Notebook: New folder\u{2026}", CommandId::NoteNewFolder),
    entry("Note: Reload from disk", CommandId::NoteReloadFromDisk),
    entry("Note: Keep my version", CommandId::NoteKeepMine),
    entry("Note: Toggle pin", CommandId::NoteTogglePin),
    entry("Note: Move to notebook...", CommandId::NoteMoveToNotebook),
    entry("Note: Reveal in Explorer", CommandId::NoteRevealInExplorer),
    entry("Note: Rename...", CommandId::NoteRename),
    entry("Note: Delete", CommandId::NoteDelete),
    entry("Edit: Undo", CommandId::Undo),
    entry("Edit: Redo", CommandId::Redo),
    entry("Edit: Cut", CommandId::Cut),
    entry("Edit: Copy", CommandId::Copy),
    entry("Edit: Paste", CommandId::Paste),
    entry("Search: Find", CommandId::Find),
    entry("Search: Find next", CommandId::FindNext),
    entry("Search: Find previous", CommandId::FindPrevious),
    entry("Search: Replace", CommandId::Replace),
    entry("Search: Replace in notes", CommandId::ReplaceInNotes),
    entry("Search: Toggle match case", CommandId::SearchToggleCase),
    entry(
        "Search: Toggle whole word",
        CommandId::SearchToggleWholeWord,
    ),
    entry(
        "Search: Toggle regular expression",
        CommandId::SearchToggleRegex,
    ),
    entry("JSON: Format document", CommandId::FormatJson),
    entry("JSON: Validate document", CommandId::ValidateJson),
    entry("Language: Plain text", CommandId::LanguagePlainText),
    entry("Language: JSON", CommandId::LanguageJson),
    entry("Language: Markdown", CommandId::LanguageMarkdown),
    entry(
        "Markdown Preview: Side by Side",
        CommandId::MarkdownPreviewSide,
    ),
    entry("Markdown Preview: Full", CommandId::MarkdownPreviewFull),
    entry("Close Markdown Preview", CommandId::MarkdownPreviewClose),
    entry("View: Next tab", CommandId::NextTab),
    entry("View: Previous tab", CommandId::PreviousTab),
    entry("View: Toggle sidebar", CommandId::ToggleSidebar),
    entry("View: Show notebook", CommandId::ShowNotebookView),
    entry("View: Show search", CommandId::ShowSearchView),
    entry("View: Show favorites", CommandId::ShowFavoritesView),
    entry("View: Zoom in", CommandId::ZoomIn),
    entry("View: Zoom out", CommandId::ZoomOut),
    entry("View: Reset zoom", CommandId::ZoomReset),
    entry("View: Left-to-right text", CommandId::TextLeftToRight),
    entry("View: Right-to-left text", CommandId::TextRightToLeft),
    entry("View: Toggle word wrap", CommandId::ToggleWordWrap),
    entry("View: Toggle line numbers", CommandId::ToggleLineNumbers),
    entry("View: Increase font size", CommandId::FontSizeIncrease),
    entry("View: Decrease font size", CommandId::FontSizeDecrease),
    entry("View: Reset font size", CommandId::FontSizeReset),
    entry("Theme: System", CommandId::ThemeSystem),
    entry("Theme: Light", CommandId::ThemeLight),
    entry("Theme: Dark", CommandId::ThemeDark),
    entry(
        "Theme: Catppuccin (follow system)",
        CommandId::ThemeCatppuccin,
    ),
    entry("Theme: Catppuccin Latte", CommandId::ThemeCatppuccinLatte),
    entry("Theme: Catppuccin Frappe", CommandId::ThemeCatppuccinFrappe),
    entry(
        "Theme: Catppuccin Macchiato",
        CommandId::ThemeCatppuccinMacchiato,
    ),
    entry("Theme: Catppuccin Mocha", CommandId::ThemeCatppuccinMocha),
    entry("File icons: Material", CommandId::FileIconsMaterial),
    entry("File icons: Minimal", CommandId::FileIconsMinimal),
    entry("Editor: Tab width 2", CommandId::TabWidth2),
    entry("Editor: Tab width 4", CommandId::TabWidth4),
    entry("Editor: Tab width 8", CommandId::TabWidth8),
    entry("File: Exit", CommandId::Exit),
];

/// What the activity bar's Settings button lists: every command that changes a `fastpad.ini`
/// setting or the open notebook's autosave switch. The palette shows them in catalog order.
pub(crate) const SETTINGS_COMMANDS: &[CommandId] = &[
    CommandId::ToggleRestoreSession,
    CommandId::ToggleNotesMode,
    CommandId::ToggleFolderAutosave,
    CommandId::ToggleWordWrap,
    CommandId::ToggleLineNumbers,
    CommandId::FontSizeIncrease,
    CommandId::FontSizeDecrease,
    CommandId::FontSizeReset,
    CommandId::ThemeSystem,
    CommandId::ThemeLight,
    CommandId::ThemeDark,
    CommandId::ThemeCatppuccin,
    CommandId::ThemeCatppuccinLatte,
    CommandId::ThemeCatppuccinFrappe,
    CommandId::ThemeCatppuccinMacchiato,
    CommandId::ThemeCatppuccinMocha,
    CommandId::FileIconsMaterial,
    CommandId::FileIconsMinimal,
    CommandId::TabWidth2,
    CommandId::TabWidth4,
    CommandId::TabWidth8,
];

/// How well `query` matches `label`, lower is better; `None` when it does not match at all.
/// Case-insensitive and whitespace-insensitive in the query: a label prefix beats a word prefix,
/// which beats a substring, which beats the query's characters merely appearing in order.
fn match_rank(query: &str, label: &str) -> Option<u8> {
    let query = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if query.is_empty() {
        return Some(0);
    }
    let label = label.to_lowercase();
    let compact = label
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    if compact.starts_with(&query) {
        return Some(0);
    }
    // Each word, and the text after the category prefix, counts as a word start.
    let word_start = label
        .char_indices()
        .filter(|&(index, _)| {
            index == 0 || label[..index].ends_with(|c: char| c.is_whitespace() || c == ':')
        })
        .any(|(index, _)| {
            label[index..]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .starts_with(&query)
        });
    if word_start {
        return Some(1);
    }
    if compact.contains(&query) {
        return Some(2);
    }
    let mut remaining = compact.chars();
    query
        .chars()
        .all(|wanted| remaining.any(|c| c == wanted))
        .then_some(3)
}

/// The commands `query` lists, best match first and in catalog order within a rank, skipping the
/// ones `available` rejects.
pub(crate) fn filter_entries(
    query: &str,
    available: impl Fn(CommandId) -> bool,
) -> Vec<PaletteEntry> {
    let mut ranked = ENTRIES
        .iter()
        .filter(|entry| available(entry.command))
        .filter_map(|entry| match_rank(query, entry.label).map(|rank| (rank, *entry)))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|&(rank, _)| rank);
    ranked.into_iter().map(|(_, entry)| entry).collect()
}

/// What a picker is choosing; decides what `library_host::picked` does with the choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    RecentFolder,
    MoveToNotebook,
    /// Ctrl+P (quick-open spec §3): notes by name. Its rows come from `main_window`, not from
    /// `Picker::items`.
    QuickOpen,
}

/// The most rows quick open lists (spec §3.3).
pub(crate) const QUICK_OPEN_ROWS: usize = 50;
/// Quick open's one row while no notebook is open; it can't be picked (spec §3.1).
pub(crate) const NO_NOTEBOOK: &str = "No notebook is open";
/// What quick open's empty field shows (spec §3.1).
pub(crate) const QUICK_OPEN_PLACEHOLDER: &str = "Go to note by name";

/// A list of runtime items shown in the palette instead of commands.
#[derive(Clone, Debug)]
pub(crate) struct Picker {
    pub kind: PickerKind,
    pub items: Vec<String>,
    /// When set, a typed name that matches no item exactly is offered as "<create> "<name>"".
    pub create: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerRow {
    Item(usize),
    Create(String),
    /// A quick-open note, and the line the query's `:<n>` names.
    Note {
        found: QuickMatch,
        line: Option<u32>,
    },
    /// A quick-open query that is only `:<n>`: that line of the current tab.
    GoToLine(u32),
    /// A row that can't be picked, such as `NO_NOTEBOOK`.
    Notice(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerChoice {
    Item(usize),
    Create(String),
    /// A quick-open note, relative to the notebook.
    Note {
        path: PathBuf,
        line: Option<u32>,
    },
    GoToLine(u32),
}

pub(crate) fn picker_rows(picker: &Picker, query: &str) -> Vec<PickerRow> {
    let query = query.trim();
    let mut ranked: Vec<(u8, usize)> = picker
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if query.is_empty() {
                Some((0, index))
            } else {
                match_rank(query, item).map(|rank| (rank, index))
            }
        })
        .collect();
    ranked.sort_by_key(|&(rank, index)| (rank, index));
    let mut rows: Vec<PickerRow> = ranked
        .into_iter()
        .map(|(_, index)| PickerRow::Item(index))
        .collect();
    let exact = picker
        .items
        .iter()
        .any(|item| item.to_lowercase() == query.to_lowercase());
    if picker.create.is_some() && !query.is_empty() && !exact {
        rows.push(PickerRow::Create(query.to_owned()));
    }
    rows
}

pub(crate) fn picker_row_label(picker: &Picker, row: &PickerRow) -> String {
    match row {
        PickerRow::Item(index) => picker.items.get(*index).cloned().unwrap_or_default(),
        PickerRow::Create(name) => {
            format!(
                "{} \u{201c}{name}\u{201d}",
                picker.create.unwrap_or("Create")
            )
        }
        PickerRow::Note { found, .. } if found.folder.is_empty() => found.name.clone(),
        PickerRow::Note { found, .. } => format!("{}, in {}", found.name, found.folder),
        PickerRow::GoToLine(line) => format!("Go to line {line}"),
        PickerRow::Notice(text) => (*text).to_owned(),
    }
}

/// The first keyboard shortcut bound to `command`, spelled the way the menus spell shortcuts.
pub(crate) fn shortcut_text(command: CommandId) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};
    let spec: AcceleratorSpec = accelerator_specs()
        .into_iter()
        .find(|spec| spec.command == command)?;
    let mut text = String::new();
    for (flag, name) in [(FCONTROL, "Ctrl+"), (FSHIFT, "Shift+"), (FALT, "Alt+")] {
        if spec.modifiers & flag != 0 {
            text.push_str(name);
        }
    }
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_F1, VK_F24, VK_OEM_MINUS, VK_OEM_PLUS,
    };
    match spec.key {
        VK_TAB => text.push_str("Tab"),
        VK_OEM_PLUS => text.push('+'),
        VK_OEM_MINUS => text.push('-'),
        key @ VK_F1..=VK_F24 => text.push_str(&format!("F{}", key - VK_F1 + 1)),
        key => text.push(char::from_u32(u32::from(key))?),
    }
    Some(text)
}

const WIDTH_AT_96_DPI: i32 = 560;
const PADDING_AT_96_DPI: i32 = 6;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const ROW_HEIGHT_AT_96_DPI: i32 = 26;
const VISIBLE_ROWS: usize = 12;
const MARGIN_AT_96_DPI: i32 = 8;

/// Where the panel's parts sit, in panel client coordinates.
#[derive(Clone, Copy)]
struct PanelLayout {
    width: i32,
    height: i32,
    /// The painted field box, border included.
    field: RECT,
    /// The borderless `Edit`, inset in the field and vertically centered on its text.
    edit: RECT,
    /// `None` while no command matches.
    list: Option<RECT>,
}

impl std::fmt::Debug for PanelLayout {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PanelLayout")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl PanelLayout {
    fn calculate(width: i32, dpi: u32, text_height: i32, rows: usize) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let field_height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
        let inset = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
        let field = RECT {
            left: padding,
            top: padding,
            right: (width - padding).max(padding),
            bottom: padding + field_height,
        };
        let text_height = text_height.clamp(1, field_height - 2);
        let edit_top = field.top + (field_height - text_height) / 2;
        let edit = RECT {
            left: field.left + inset,
            top: edit_top,
            right: (field.right - inset).max(field.left + inset),
            bottom: edit_top + text_height,
        };
        let rows = rows.min(VISIBLE_ROWS) as i32;
        let list = (rows > 0).then(|| RECT {
            left: 1,
            top: field.bottom + padding,
            right: (width - 1).max(1),
            bottom: field.bottom + padding + rows * scale(ROW_HEIGHT_AT_96_DPI, dpi),
        });
        // The list runs to the bottom border; the field alone keeps its padding below.
        let height = list.map_or(field.bottom + padding, |list| list.bottom + 1);
        Self {
            width,
            height,
            field,
            edit,
            list,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CommandPalette {
    /// Paints the frame and field box, and owns the two controls.
    panel: HWND,
    query_edit: HWND,
    list: HWND,
    shown: Vec<PaletteEntry>,
    /// `Some` while the palette lists runtime items instead of commands.
    picker: Option<Picker>,
    /// The rows `picker` currently shows, filtered by the query.
    picker_rows: Vec<PickerRow>,
    /// The row `fill_list` selects in picker mode; `None` selects nothing.
    picker_selected: Option<usize>,
    /// `Some` while command mode lists only these commands (the Settings button).
    subset: Option<&'static [CommandId]>,
    visible: bool,
    colors: Palette,
    layout: Option<PanelLayout>,
    field_brush: HBRUSH,
    list_brush: HBRUSH,
    /// The list's font when the bold one was made, and the bold one; null until a quick-open
    /// row is first drawn.
    bold: Cell<(HFONT, HFONT)>,
    /// How many times a mode switch marked the field for its hint to come or go, for
    /// in-process tests (their windows are never shown, so they have no update region).
    #[cfg(test)]
    hint_repaints: usize,
}

impl CommandPalette {
    pub(crate) fn create(parent: HWND) -> crate::Result<Self> {
        let panel = create_panel(parent)?;
        let controls = (|| {
            let query_edit = create_child(
                panel,
                &wide_null("Edit"),
                WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
            )?;
            let list = create_child(
                panel,
                &wide_null("ListBox"),
                WS_CHILD
                    | WS_VSCROLL
                    | (LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32,
            )?;
            install_hook(query_edit, parent, PaletteControl::Query)?;
            install_hook(list, parent, PaletteControl::List)?;
            Ok((query_edit, list))
        })();
        let (query_edit, list) = match controls {
            Ok(controls) => controls,
            Err(error) => {
                unsafe {
                    DestroyWindow(panel);
                }
                return Err(error);
            }
        };
        let colors = Palette::neutral();
        Ok(Self {
            panel,
            query_edit,
            list,
            shown: Vec::new(),
            picker: None,
            picker_rows: Vec::new(),
            picker_selected: None,
            subset: None,
            visible: false,
            colors,
            layout: None,
            field_brush: unsafe { CreateSolidBrush(colors.editor_background) },
            list_brush: unsafe { CreateSolidBrush(colors.strip_background) },
            bold: Cell::new((std::ptr::null_mut(), std::ptr::null_mut())),
            #[cfg(test)]
            hint_repaints: 0,
        })
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn owns(&self, hwnd: HWND) -> bool {
        hwnd == self.query_edit || hwnd == self.list || hwnd == self.panel
    }

    pub(crate) fn query_text(&self) -> String {
        let length = unsafe { GetWindowTextLengthW(self.query_edit) };
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied =
            unsafe { GetWindowTextW(self.query_edit, buffer.as_mut_ptr(), buffer.len() as i32) };
        buffer.truncate(copied.max(0) as usize);
        String::from_utf16_lossy(&buffer)
    }

    /// Marks the palette shown in `colors`; returns whether it was hidden before. Makes no calls
    /// that re-enter the window procedure, so the caller may hold the App borrow across it.
    pub(crate) fn mark_shown(&mut self, colors: Palette) -> bool {
        self.set_colors(colors);
        !std::mem::replace(&mut self.visible, true)
    }

    /// Recolors for a theme change; the caller repaints with `invalidate`.
    pub(crate) fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        unsafe {
            DeleteObject(self.field_brush);
            DeleteObject(self.list_brush);
            self.field_brush = CreateSolidBrush(colors.editor_background);
            self.list_brush = CreateSolidBrush(colors.strip_background);
        }
        self.colors = colors;
    }

    pub(crate) fn invalidate(&self) {
        unsafe {
            RedrawWindow(
                self.panel,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }

    /// Returns whether it was visible; `hide_controls` then removes it from the screen.
    pub(crate) fn mark_hidden(&mut self) -> bool {
        self.picker = None;
        self.picker_rows = Vec::new();
        self.picker_selected = None;
        self.subset = None;
        std::mem::take(&mut self.visible)
    }

    /// Sends `EN_CHANGE` to the parent.
    pub(crate) fn clear_query(&self) {
        let empty = wide_null("");
        unsafe {
            SetWindowTextW(self.query_edit, empty.as_ptr());
        }
    }

    pub(crate) fn hide_controls(&self) {
        unsafe {
            ShowWindow(self.panel, SW_HIDE);
        }
    }

    /// Records the rows to list; `fill_list` then puts them in the list box.
    pub(crate) fn set_entries(&mut self, entries: Vec<PaletteEntry>) {
        self.shown = entries;
    }

    /// Switches between command mode (`None`) and picker mode; also clears any rows from a
    /// previous filter, so a stale selection index can't leak into the new mode.
    pub(crate) fn set_picker(&mut self, picker: Option<Picker>) {
        let hint = self.placeholder();
        self.picker = picker;
        self.picker_rows = Vec::new();
        self.picker_selected = None;
        // An empty field repaints only on an edit, and switching modes may make none: the hint
        // must come or go here. InvalidateRect only marks the region; it sends nothing.
        if self.placeholder() != hint {
            unsafe {
                InvalidateRect(self.query_edit, std::ptr::null(), 1);
            }
            #[cfg(test)]
            {
                self.hint_repaints += 1;
            }
        }
    }

    pub(crate) fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    /// Limits command mode to `subset`, or lifts the limit with `None`.
    pub(crate) fn set_subset(&mut self, subset: Option<&'static [CommandId]>) {
        self.subset = subset;
    }

    pub(crate) fn subset(&self) -> Option<&'static [CommandId]> {
        self.subset
    }

    /// Records the picker rows to list and the one to select; `fill_list` then puts them in the
    /// list box.
    pub(crate) fn set_picker_rows(&mut self, rows: Vec<PickerRow>, selected: Option<usize>) {
        self.picker_rows = rows;
        self.picker_selected = selected;
    }

    /// Refills the list box from the recorded rows and selects the best match.
    pub(crate) fn fill_list(&self) {
        unsafe {
            SendMessageW(self.list, LB_RESETCONTENT, 0, 0);
        }
        let (count, selected) = if let Some(picker) = &self.picker {
            // Owner-drawn, but the strings are what a screen reader reads (spec §3.7).
            for row in &self.picker_rows {
                let label = wide_null(&picker_row_label(picker, row));
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, label.as_ptr() as LPARAM);
                }
            }
            (self.picker_rows.len(), self.picker_selected)
        } else {
            for entry in &self.shown {
                let label = wide_null(entry.label);
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, label.as_ptr() as LPARAM);
                }
            }
            (self.shown.len(), Some(0))
        };
        if let Some(selected) = selected.filter(|&selected| selected < count) {
            unsafe {
                SendMessageW(self.list, LB_SETCURSEL, selected, 0);
            }
        }
    }

    /// Records the panel geometry for `width` of parent client area, so painting and `apply_layout`
    /// agree on it.
    pub(crate) fn measure(&mut self, width: i32, dpi: u32, font: HFONT) {
        let margin = scale(MARGIN_AT_96_DPI, dpi);
        let width = scale(WIDTH_AT_96_DPI, dpi).min(width - 2 * margin).max(0);
        let text_height = text_height(self.query_edit, font);
        self.layout = Some(PanelLayout::calculate(
            width,
            dpi,
            text_height,
            self.row_count(),
        ));
    }

    /// Centers the measured panel horizontally in the `parent_width` wide editor area starting at
    /// `left`, from `top`, and places the field and list inside it.
    pub(crate) fn apply_layout(
        &self,
        left: i32,
        parent_width: i32,
        top: i32,
        dpi: u32,
        font: HFONT,
    ) {
        let Some(layout) = self.layout.filter(|_| self.visible) else {
            return;
        };
        let left = left + ((parent_width - layout.width) / 2).max(0);
        let top = top + scale(MARGIN_AT_96_DPI, dpi);
        let move_to = |hwnd, rect: RECT| unsafe {
            MoveWindow(
                hwnd,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                1,
            );
        };
        unsafe {
            if !font.is_null() {
                SendMessageW(self.query_edit, WM_SETFONT, font as WPARAM, 0);
                SendMessageW(self.list, WM_SETFONT, font as WPARAM, 0);
            }
            SendMessageW(
                self.list,
                LB_SETITEMHEIGHT,
                0,
                scale(ROW_HEIGHT_AT_96_DPI, dpi) as LPARAM,
            );
            // Dark scrollbar under the dark palettes, the system one otherwise.
            let dark_theme = wide_null("DarkMode_Explorer");
            SetWindowTheme(
                self.list,
                if self.colors.dark_frame {
                    dark_theme.as_ptr()
                } else {
                    std::ptr::null()
                },
                std::ptr::null(),
            );
            move_to(self.query_edit, layout.edit);
            match layout.list {
                Some(list) => {
                    move_to(self.list, list);
                    ShowWindow(self.list, SW_SHOWNA);
                }
                None => {
                    ShowWindow(self.list, SW_HIDE);
                }
            }
            // Above the editor in z-order; the editor clips itself against this sibling.
            SetWindowPos(
                self.panel,
                HWND_TOP,
                left,
                top,
                layout.width,
                layout.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    pub(crate) fn focus_query(&self) {
        unsafe {
            SetFocus(self.query_edit);
            SendMessageW(self.query_edit, EM_SETSEL, 0, -1);
        }
    }

    /// The number of rows currently listed: `picker_rows` in picker mode, `shown` otherwise.
    fn row_count(&self) -> usize {
        if self.picker.is_some() {
            self.picker_rows.len()
        } else {
            self.shown.len()
        }
    }

    /// Moves the selection by `delta` rows, clamped to the list.
    pub(crate) fn move_selection(&self, delta: isize) {
        let Some(last) = self.row_count().checked_sub(1) else {
            return;
        };
        let current = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) }.max(0);
        let next = current.saturating_add(delta).clamp(0, last as isize);
        unsafe {
            SendMessageW(self.list, LB_SETCURSEL, next as WPARAM, 0);
        }
    }

    pub(crate) fn selected_command(&self) -> Option<CommandId> {
        let index = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) };
        usize::try_from(index)
            .ok()
            .and_then(|index| self.shown.get(index))
            .map(|entry| entry.command)
    }

    /// The picker row under the list's current selection, converted into a choice; `None` outside
    /// picker mode or with nothing selected.
    pub(crate) fn selected_choice(&self) -> Option<PickerChoice> {
        let index = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) };
        let row = usize::try_from(index)
            .ok()
            .and_then(|index| self.picker_rows.get(index))?;
        Some(match row {
            PickerRow::Item(index) => PickerChoice::Item(*index),
            PickerRow::Create(name) => PickerChoice::Create(name.clone()),
            PickerRow::Note { found, line } => PickerChoice::Note {
                path: found.path.clone(),
                line: *line,
            },
            PickerRow::GoToLine(line) => PickerChoice::GoToLine(*line),
            PickerRow::Notice(_) => return None,
        })
    }

    /// Selects the row under a list-client point from `WM_LBUTTONDOWN`'s `lparam`.
    pub(crate) fn select_row_at(&self, lparam: LPARAM) -> bool {
        let hit = unsafe { SendMessageW(self.list, LB_ITEMFROMPOINT, 0, lparam) } as usize;
        // The high word is nonzero when the point lies outside every item.
        if (hit >> 16) & 0xffff != 0 || (hit & 0xffff) >= self.row_count() {
            return false;
        }
        unsafe {
            SendMessageW(self.list, LB_SETCURSEL, hit & 0xffff, 0);
        }
        true
    }

    /// `WM_CTLCOLOREDIT`/`WM_CTLCOLORLISTBOX` for one of the palette's controls.
    pub(crate) fn control_color(&self, dc: HDC, control: HWND) -> HBRUSH {
        let (background, brush) = if control == self.query_edit {
            (self.colors.editor_background, self.field_brush)
        } else {
            (self.colors.strip_background, self.list_brush)
        };
        unsafe {
            SetTextColor(dc, self.colors.editor_foreground);
            SetBkColor(dc, background);
        }
        brush
    }

    /// `WM_PAINT` for the panel: the strip-colored card with a hairline border, and the field box
    /// in the editor's colors with an accent outline around the borderless `Edit`.
    pub(crate) fn paint_panel(&self, panel: HWND) {
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(panel, &mut paint) };
        if dc.is_null() {
            return;
        }
        let mut client = RECT::default();
        unsafe {
            GetClientRect(panel, &mut client);
        }
        let colors = self.colors;
        if let Some(layout) = self.layout {
            unsafe {
                fill(dc, client, colors.pressed_background);
                fill(dc, inset(client, 1), colors.strip_background);
                fill(dc, layout.field, colors.selection_background);
                fill(dc, inset(layout.field, 1), colors.editor_background);
            }
        }
        unsafe {
            EndPaint(panel, &paint);
        }
    }

    /// `WM_DRAWITEM` for the list: label on the left, shortcut right-aligned in the muted color.
    /// A picker row has no shortcut and its label comes from `picker_row_label`.
    pub(crate) fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        let index = usize::try_from(item.itemID).ok();
        let quick_open = self
            .picker
            .as_ref()
            .is_some_and(|picker| picker.kind == PickerKind::QuickOpen);
        if quick_open {
            if let Some(row) = index.and_then(|index| self.picker_rows.get(index)) {
                self.draw_quick_open_row(item, row);
            }
            return;
        }
        let (label, shortcut) = if let Some(picker) = &self.picker {
            let Some(label) = index
                .and_then(|index| self.picker_rows.get(index))
                .map(|row| picker_row_label(picker, row))
            else {
                return;
            };
            (label, None)
        } else {
            let Some(entry) = index.and_then(|index| self.shown.get(index)) else {
                return;
            };
            (entry.label.to_owned(), shortcut_text(entry.command))
        };
        let selected = item.itemState & ODS_SELECTED != 0;
        let colors = self.colors;
        let (background, foreground, muted) = if selected {
            (
                colors.hover_background,
                colors.hover_foreground,
                colors.hover_foreground,
            )
        } else {
            (
                colors.strip_background,
                colors.editor_foreground,
                colors.muted_foreground,
            )
        };
        let dc = item.hDC;
        // Row text lines up with the query text above it; the list starts one pixel in.
        let padding = self.layout.map_or(8, |layout| layout.edit.left - 1);
        let mut text = RECT {
            left: item.rcItem.left + padding,
            right: item.rcItem.right - padding,
            ..item.rcItem
        };
        unsafe {
            fill(dc, item.rcItem, background);
            SetBkMode(dc, TRANSPARENT as i32);
            let font = SendMessageW(self.list, WM_GETFONT, 0, 0);
            let previous = (font != 0).then(|| SelectObject(dc, font as _));
            let flags = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
            if let Some(shortcut) = shortcut {
                let mut shortcut = shortcut.encode_utf16().collect::<Vec<_>>();
                SetTextColor(dc, muted);
                let mut measured = text;
                DrawTextW(
                    dc,
                    shortcut.as_mut_ptr(),
                    shortcut.len() as i32,
                    &mut measured,
                    flags | DT_RIGHT | DT_CALCRECT,
                );
                DrawTextW(
                    dc,
                    shortcut.as_mut_ptr(),
                    shortcut.len() as i32,
                    &mut text,
                    flags | DT_RIGHT,
                );
                // Keep the label clear of the shortcut column.
                text.right -= (measured.right - measured.left) + padding;
            }
            let mut label = label.encode_utf16().collect::<Vec<_>>();
            SetTextColor(dc, foreground);
            DrawTextW(
                dc,
                label.as_mut_ptr(),
                label.len() as i32,
                &mut text,
                flags | DT_LEFT | DT_END_ELLIPSIS,
            );
            if let Some(previous) = previous {
                SelectObject(dc, previous);
            }
        }
    }

    /// A quick-open row (spec §3.3): the name with its matched letters in bold, then the folder
    /// in the muted color, its matched letters bold too. The notice row can't be picked, so it is
    /// muted and never drawn selected.
    fn draw_quick_open_row(&self, item: &DRAWITEMSTRUCT, row: &PickerRow) {
        let notice = matches!(row, PickerRow::Notice(_));
        let selected = item.itemState & ODS_SELECTED != 0 && !notice;
        let colors = self.colors;
        let (background, foreground, muted) = if selected {
            (
                colors.hover_background,
                colors.hover_foreground,
                colors.hover_foreground,
            )
        } else {
            (
                colors.strip_background,
                colors.editor_foreground,
                colors.muted_foreground,
            )
        };
        let dc = item.hDC;
        let padding = self.layout.map_or(8, |layout| layout.edit.left - 1);
        let mut text = RECT {
            left: item.rcItem.left + padding,
            right: item.rcItem.right - padding,
            ..item.rcItem
        };
        let list_font = unsafe { SendMessageW(self.list, WM_GETFONT, 0, 0) } as HFONT;
        let bold = self.bold_font(list_font);
        // A list with no font draws in the DC's own; `draw_runs` switches fonts per run, so the
        // DC's font is always put back, whichever the last run selected.
        let font = if list_font.is_null() {
            unsafe { GetCurrentObject(dc, OBJ_FONT as u32) }
        } else {
            list_font
        };
        unsafe {
            fill(dc, item.rcItem, background);
            SetBkMode(dc, TRANSPARENT as i32);
        }
        let previous = unsafe { SelectObject(dc, font) };
        match row {
            PickerRow::Note { found, .. } => {
                draw_runs(
                    dc,
                    &mut text,
                    &found.name,
                    &found.name_hits,
                    font,
                    bold,
                    foreground,
                );
                if !found.folder.is_empty() {
                    text.left += padding;
                    draw_runs(
                        dc,
                        &mut text,
                        &found.folder,
                        &found.folder_hits,
                        font,
                        bold,
                        muted,
                    );
                }
            }
            PickerRow::Notice(label) => draw_runs(dc, &mut text, label, &[], font, bold, muted),
            other => {
                let label = self
                    .picker
                    .as_ref()
                    .map(|picker| picker_row_label(picker, other))
                    .unwrap_or_default();
                draw_runs(dc, &mut text, &label, &[], font, bold, foreground);
            }
        }
        unsafe {
            SelectObject(dc, previous);
        }
    }

    /// `base` in bold, made on first use and again when the list's font changes (a DPI change).
    /// A list with no font yet uses the default GUI font's metrics.
    fn bold_font(&self, base: HFONT) -> HFONT {
        let (made_for, bold) = self.bold.get();
        if made_for == base && !bold.is_null() {
            return bold;
        }
        if !bold.is_null() {
            unsafe {
                DeleteObject(bold);
            }
        }
        let source = if base.is_null() {
            unsafe { GetStockObject(DEFAULT_GUI_FONT) }
        } else {
            base
        };
        let mut font = LOGFONTW::default();
        let read = unsafe {
            GetObjectW(
                source,
                std::mem::size_of::<LOGFONTW>() as i32,
                (&mut font as *mut LOGFONTW).cast(),
            )
        };
        let bold = if read == 0 {
            std::ptr::null_mut()
        } else {
            font.lfWeight = FW_BOLD as i32;
            unsafe { CreateFontIndirectW(&font) }
        };
        self.bold.set((base, bold));
        bold
    }

    /// What the empty query field shows: the quick-open hint, nothing in other modes.
    pub(crate) fn placeholder(&self) -> Option<&'static str> {
        self.picker
            .as_ref()
            .filter(|picker| picker.kind == PickerKind::QuickOpen)
            .map(|_| QUICK_OPEN_PLACEHOLDER)
    }

    /// `WM_PAINT` for the empty query field while it has a placeholder: the hint in the muted
    /// color where typed text starts. False for any other control or mode, which paints normally.
    pub(crate) fn paint_placeholder(&self, edit: HWND) -> bool {
        if edit != self.query_edit {
            return false;
        }
        let Some(placeholder) = self.placeholder() else {
            return false;
        };
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(edit, &mut paint) };
        if dc.is_null() {
            return true;
        }
        unsafe {
            let mut client = RECT::default();
            GetClientRect(edit, &mut client);
            fill(dc, client, self.colors.editor_background);
            let font = SendMessageW(edit, WM_GETFONT, 0, 0);
            let previous = (font != 0).then(|| SelectObject(dc, font as _));
            // Typed text starts after the Edit's left margin (the low word).
            client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, self.colors.muted_foreground);
            let mut text = placeholder.encode_utf16().collect::<Vec<_>>();
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                text.len() as i32,
                &mut client,
                DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            if let Some(previous) = previous {
                SelectObject(dc, previous);
            }
            EndPaint(edit, &paint);
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn list_text(&self, index: usize) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{LB_GETTEXT, LB_GETTEXTLEN};
        let length = unsafe { SendMessageW(self.list, LB_GETTEXTLEN, index, 0) };
        let Ok(length) = usize::try_from(length) else {
            return String::new();
        };
        let mut buffer = vec![0u16; length + 1];
        let copied =
            unsafe { SendMessageW(self.list, LB_GETTEXT, index, buffer.as_mut_ptr() as LPARAM) };
        buffer.truncate(usize::try_from(copied).unwrap_or(0));
        String::from_utf16_lossy(&buffer)
    }

    #[cfg(test)]
    pub(crate) fn hint_repaints(&self) -> usize {
        self.hint_repaints
    }

    #[cfg(test)]
    pub(crate) fn has_bold_font(&self) -> bool {
        !self.bold.get().1.is_null()
    }

    #[cfg(test)]
    pub(crate) fn query_hwnd(&self) -> HWND {
        self.query_edit
    }

    #[cfg(test)]
    pub(crate) fn shown(&self) -> &[PaletteEntry] {
        &self.shown
    }

    #[cfg(test)]
    pub(crate) fn shown_picker_rows(&self) -> &[PickerRow] {
        &self.picker_rows
    }

    #[cfg(test)]
    pub(crate) fn selected_row(&self) -> Option<usize> {
        usize::try_from(unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) }).ok()
    }

    #[cfg(test)]
    pub(crate) fn panel_hwnd(&self) -> HWND {
        self.panel
    }
}

/// `text` cut into runs of chars that are all hits or all not, in order. `hits` are ascending
/// char indices (quick_open's), never byte offsets.
fn hit_runs<'a>(text: &'a str, hits: &[usize]) -> Vec<(&'a str, bool)> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (index, (byte, _)) in text.char_indices().enumerate() {
        let hit = hits.binary_search(&index).is_ok();
        match current {
            Some(previous) if previous == hit => {}
            Some(previous) => {
                runs.push((&text[start..byte], previous));
                start = byte;
                current = Some(hit);
            }
            None => current = Some(hit),
        }
    }
    if let Some(last) = current {
        runs.push((&text[start..], last));
    }
    runs
}

/// Draws `text` from `rect.left` in `color`, the chars at `hits` in `bold` and the rest in
/// `regular`, and moves `rect.left` past what it drew. The run that reaches `rect.right` ends in
/// an ellipsis, and nothing is drawn after it.
fn draw_runs(
    dc: HDC,
    rect: &mut RECT,
    text: &str,
    hits: &[usize],
    regular: HFONT,
    bold: HFONT,
    color: u32,
) {
    let flags = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT;
    unsafe {
        SetTextColor(dc, color);
    }
    for (run, hit) in hit_runs(text, hits) {
        if rect.left >= rect.right {
            return;
        }
        let font = if hit && !bold.is_null() {
            bold
        } else {
            regular
        };
        let mut wide = run.encode_utf16().collect::<Vec<_>>();
        let mut measured = *rect;
        unsafe {
            if !font.is_null() {
                SelectObject(dc, font);
            }
            DrawTextW(
                dc,
                wide.as_mut_ptr(),
                wide.len() as i32,
                &mut measured,
                flags | DT_CALCRECT,
            );
            DrawTextW(
                dc,
                wide.as_mut_ptr(),
                wide.len() as i32,
                &mut *rect,
                flags | DT_END_ELLIPSIS,
            );
        }
        rect.left += measured.right - measured.left;
    }
}

impl Drop for CommandPalette {
    fn drop(&mut self) {
        let (_, bold) = self.bold.get();
        unsafe {
            DeleteObject(self.field_brush);
            DeleteObject(self.list_brush);
            if !bold.is_null() {
                DeleteObject(bold);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaletteControl {
    Query,
    List,
}

struct PaletteHook {
    parent: HWND,
    control: PaletteControl,
}

const PALETTE_HOOK_ID: usize = 0x4650_4350;

fn install_hook(hwnd: HWND, parent: HWND, control: PaletteControl) -> crate::Result<()> {
    let data = Rc::into_raw(Rc::new(PaletteHook { parent, control })) as usize;
    if unsafe { SetWindowSubclass(hwnd, Some(palette_control_proc), PALETTE_HOOK_ID, data) } == 0 {
        unsafe {
            drop(Rc::from_raw(data as *const PaletteHook));
        }
        return Err(last_error());
    }
    Ok(())
}

unsafe extern "system" fn palette_control_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> isize {
    let raw = ref_data as *const PaletteHook;
    unsafe {
        Rc::increment_strong_count(raw);
    }
    let hook = unsafe { Rc::from_raw(raw) };
    let parent = hook.parent;
    match (hook.control, message) {
        (_, WM_NCDESTROY) => unsafe {
            RemoveWindowSubclass(hwnd, Some(palette_control_proc), PALETTE_HOOK_ID);
            Rc::decrement_strong_count(raw);
        },
        (PaletteControl::Query, WM_KEYDOWN) => {
            let key = wparam as u16;
            let modified = [VK_CONTROL, VK_MENU, VK_SHIFT]
                .into_iter()
                .any(|modifier| unsafe { GetKeyState(i32::from(modifier)) } < 0);
            let step = match key {
                VK_UP => Some(-1),
                VK_DOWN => Some(1),
                VK_PRIOR => Some(-(VISIBLE_ROWS as isize)),
                VK_NEXT => Some(VISIBLE_ROWS as isize),
                _ => None,
            };
            if let Some(step) = step.filter(|_| !modified) {
                super::main_window::move_command_palette_selection(parent, step);
                return 0;
            }
            if key == VK_RETURN {
                super::main_window::run_command_palette_selection(parent);
                return 0;
            }
            if key == VK_ESCAPE {
                super::main_window::close_command_palette(parent, true);
                return 0;
            }
            // Ctrl+W closes the palette here, not the tab behind it (quick-open spec §4);
            // `main_window::translate_accelerator` leaves the key to this hook.
            if key == u16::from(b'W')
                && unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0
                && unsafe { GetKeyState(i32::from(VK_MENU)) } >= 0
            {
                super::main_window::close_command_palette(parent, true);
                return 0;
            }
        }
        // A single-line Edit beeps at Enter, Escape and Ctrl+W characters; all three were
        // handled on key down.
        (PaletteControl::Query, WM_CHAR) if matches!(wparam as u16, 0x0d | 0x1b | 0x17) => {
            return 0;
        }
        (PaletteControl::Query, WM_KILLFOCUS) => {
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            if !super::main_window::command_palette_owns(parent, wparam as HWND) {
                super::main_window::close_command_palette(parent, false);
            }
            return result;
        }
        // The list never keeps the focus: typing stays in the field while rows are clicked.
        (PaletteControl::List, WM_SETFOCUS) => {
            super::main_window::focus_command_palette(parent);
            return 0;
        }
        (PaletteControl::List, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) => {
            if super::main_window::select_command_palette_row(parent, lparam) {
                super::main_window::run_command_palette_selection(parent);
            }
            return 0;
        }
        // The empty field shows quick open's hint (EM_SETCUEBANNER needs ComCtl32 v6).
        (PaletteControl::Query, WM_PAINT)
            if unsafe { GetWindowTextLengthW(hwnd) } == 0
                && super::main_window::paint_palette_placeholder(parent, hwnd) =>
        {
            return 0;
        }
        _ => {}
    }
    // The Edit repaints only the text it changes; the hint must go (or come back) whole.
    let edits_text = hook.control == PaletteControl::Query
        && matches!(
            message,
            WM_CHAR
                | WM_KEYDOWN
                | WM_PASTE
                | WM_CUT
                | WM_CLEAR
                | WM_UNDO
                | WM_SETTEXT
                | EM_UNDO
                | EM_REPLACESEL
        );
    let was_empty = edits_text && unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if edits_text && was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        ENTRIES, PanelLayout, Picker, PickerKind, PickerRow, SETTINGS_COMMANDS, filter_entries,
        hit_runs, match_rank, picker_row_label, picker_rows, shortcut_text,
    };
    use crate::window::commands::CommandId;

    #[test]
    fn hit_runs_cut_at_char_positions_not_bytes() {
        // Break caught (review focus 2): hits used as byte offsets, so "Über Straße" bolds "S"
        // and "t" one char late, or a run split inside a multi-byte char (a panic).
        assert_eq!(
            hit_runs("Über Straße", &[5, 6]),
            [("Über ", false), ("St", true), ("raße", false)]
        );
        assert_eq!(hit_runs("abc", &[0, 1, 2]), [("abc", true)]);
        assert_eq!(hit_runs("abc", &[]), [("abc", false)]);
        assert!(hit_runs("", &[]).is_empty());
    }

    #[test]
    fn every_settings_command_has_exactly_one_palette_entry() {
        // Break caught: a Settings button entry with no palette row, which the filtered palette
        // could never show, or a settings list that lets non-settings commands through.
        for command in SETTINGS_COMMANDS {
            let listed = ENTRIES
                .iter()
                .filter(|entry| entry.command == *command)
                .count();
            assert_eq!(listed, 1, "{command:?}");
        }
        let listed = filter_entries("", |command| SETTINGS_COMMANDS.contains(&command));
        assert_eq!(listed.len(), SETTINGS_COMMANDS.len());
        assert!(listed.iter().all(|entry| entry.command != CommandId::Save));
        assert!(
            listed
                .iter()
                .any(|entry| entry.command == CommandId::ThemeCatppuccinMocha)
        );
    }

    fn labels(query: &str) -> Vec<&'static str> {
        filter_entries(query, |_| true)
            .into_iter()
            .map(|entry| entry.label)
            .collect()
    }

    #[test]
    fn go_to_note_is_listed_once_with_ctrl_p() {
        // Break caught: Ctrl+P working but the palette never offering it, or its row showing no
        // shortcut (quick-open spec §3.1).
        assert_eq!(labels("go to note")[0], "Go to note\u{2026}");
        assert_eq!(
            shortcut_text(CommandId::QuickOpen).as_deref(),
            Some("Ctrl+P")
        );
        assert_eq!(ENTRIES.len(), 74);
    }

    #[test]
    fn new_folder_is_listed_under_notebook_without_a_shortcut() {
        // Break caught: the palette never offering New folder, or showing a shortcut it doesn't
        // have (spec §6).
        assert_eq!(labels("new folder")[0], "Notebook: New folder\u{2026}");
        assert_eq!(shortcut_text(CommandId::NoteNewFolder), None);
    }

    #[test]
    fn new_note_is_listed_right_before_new_folder_without_a_shortcut() {
        // Break caught: the palette never offering New note, listing it away from New folder,
        // or showing Ctrl+N (which opens an untitled tab) beside it (inline naming spec §8).
        assert_eq!(
            labels("notebook: new note")[0],
            "Notebook: New note\u{2026}"
        );
        assert_eq!(shortcut_text(CommandId::NoteNew), None);
        let position = |command| ENTRIES.iter().position(|entry| entry.command == command);
        assert_eq!(
            position(CommandId::NoteNew).map(|index| index + 1),
            position(CommandId::NoteNewFolder)
        );
    }

    #[test]
    fn close_tab_is_listed_with_ctrl_w() {
        // Break caught: the palette's Close tab row still showing no shortcut after Ctrl+W.
        assert_eq!(labels("close tab")[0], "File: Close tab");
        assert_eq!(
            shortcut_text(CommandId::CloseTab).as_deref(),
            Some("Ctrl+W")
        );
    }

    #[test]
    fn every_command_except_tab_positions_and_the_palette_is_listed_once() {
        // Break caught: a command added to the menus and shortcuts that the palette never offers.
        for value in 100..200u16 {
            let Ok(command) = CommandId::try_from(value) else {
                continue;
            };
            let listed = ENTRIES
                .iter()
                .filter(|entry| entry.command == command)
                .count();
            let expected = usize::from(
                command.tab_index().is_none()
                    && command != CommandId::CommandPalette
                    && command != CommandId::MarkdownPreviewCycle
                    && command != CommandId::FocusNextPane
                    && command != CommandId::FocusPreviousPane,
            );
            assert_eq!(listed, expected, "{command:?}");
        }
    }

    #[test]
    fn the_sidebar_commands_are_listed_with_their_shortcuts() {
        assert_eq!(labels("sidebar")[0], "View: Toggle sidebar");
        assert_eq!(labels("show search")[0], "View: Show search");
        assert_eq!(
            shortcut_text(CommandId::ToggleSidebar).as_deref(),
            Some("Ctrl+B")
        );
        assert_eq!(
            shortcut_text(CommandId::ShowNotebookView).as_deref(),
            Some("Ctrl+Shift+E")
        );
        // Break caught: the palette row still reading Ctrl+K after the shortcut moved.
        assert_eq!(
            shortcut_text(CommandId::ShowSearchView).as_deref(),
            Some("Ctrl+Shift+F")
        );
        assert_eq!(shortcut_text(CommandId::ShowFavoritesView), None);
    }

    #[test]
    fn the_search_option_rows_are_listed_without_shortcuts() {
        // Break caught: a toggle the palette never offers, or one showing a shortcut it doesn't
        // have (the Alt keys work only inside the Search box and the find bar).
        assert_eq!(labels("toggle match case")[0], "Search: Toggle match case");
        assert_eq!(labels("whole word")[0], "Search: Toggle whole word");
        assert_eq!(
            labels("regular expression")[0],
            "Search: Toggle regular expression"
        );
        for command in [
            CommandId::SearchToggleCase,
            CommandId::SearchToggleWholeWord,
            CommandId::SearchToggleRegex,
        ] {
            assert_eq!(shortcut_text(command), None, "{command:?}");
        }
    }

    #[test]
    fn replace_in_notes_is_listed_with_its_shortcut() {
        // Break caught: the palette never offering 3b's replace, or its row showing Ctrl+H, the
        // find bar's Replace.
        assert_eq!(labels("replace in notes")[0], "Search: Replace in notes");
        assert_eq!(
            shortcut_text(CommandId::ReplaceInNotes).as_deref(),
            Some("Ctrl+Shift+H")
        );
        assert_eq!(shortcut_text(CommandId::Replace).as_deref(), Some("Ctrl+H"));
    }

    #[test]
    fn an_empty_query_lists_every_available_command_in_catalog_order() {
        assert_eq!(labels("").len(), ENTRIES.len());
        assert_eq!(labels("   ")[0], ENTRIES[0].label);
        let without_documents = filter_entries("", |command| !command.needs_document());
        assert!(
            without_documents
                .iter()
                .all(|entry| !entry.command.needs_document())
        );
        assert!(
            without_documents
                .iter()
                .any(|entry| entry.command == CommandId::Open)
        );
        assert!(
            !without_documents
                .iter()
                .any(|entry| entry.command == CommandId::Save)
        );
    }

    #[test]
    fn matches_are_case_insensitive_and_ranked_prefix_word_substring_then_scattered() {
        assert_eq!(match_rank("FILE: s", "File: Save"), Some(0));
        assert_eq!(match_rank("zoom in", "View: Zoom in"), Some(1));
        assert_eq!(match_rank("oom", "View: Zoom in"), Some(2));
        assert_eq!(match_rank("vzi", "View: Zoom in"), Some(3));
        assert_eq!(match_rank("xyz", "View: Zoom in"), None);
        // Break caught: a scattered match outranking the command whose word the user typed.
        let found = labels("save");
        assert_eq!(found[..2], ["File: Save", "File: Save as..."]);
        assert_eq!(labels("json")[0], "JSON: Format document");
        assert_eq!(labels("zoom")[0], "View: Zoom in");
    }

    #[test]
    fn the_panel_centers_the_query_text_and_ends_with_the_list_on_its_bottom_border() {
        // Break caught: query text stuck to the top of its box, or a list that overhangs (or stops
        // short of) the panel's border.
        let layout = PanelLayout::calculate(560, 96, 16, 3);
        let field_middle = (layout.field.top + layout.field.bottom) / 2;
        let edit_middle = (layout.edit.top + layout.edit.bottom) / 2;
        assert!((field_middle - edit_middle).abs() <= 1);
        assert!(layout.edit.left > layout.field.left && layout.edit.right < layout.field.right);
        let list = layout.list.unwrap();
        assert_eq!(list.bottom - list.top, 3 * 26);
        assert_eq!((list.left, list.right), (1, 559));
        assert_eq!(layout.height, list.bottom + 1);

        let many = PanelLayout::calculate(560, 144, 24, 40);
        assert_eq!(many.list.map(|list| list.bottom - list.top), Some(12 * 39));

        let empty = PanelLayout::calculate(560, 96, 16, 0);
        assert!(empty.list.is_none());
        assert_eq!(empty.height, empty.field.bottom + 6);
    }

    #[test]
    fn shortcuts_are_spelled_from_the_accelerator_table() {
        assert_eq!(shortcut_text(CommandId::Save).as_deref(), Some("Ctrl+S"));
        assert_eq!(
            shortcut_text(CommandId::SaveAs).as_deref(),
            Some("Ctrl+Shift+S")
        );
        assert_eq!(
            shortcut_text(CommandId::NextTab).as_deref(),
            Some("Ctrl+Tab")
        );
        assert_eq!(shortcut_text(CommandId::ZoomIn).as_deref(), Some("Ctrl++"));
        assert_eq!(shortcut_text(CommandId::ZoomOut).as_deref(), Some("Ctrl+-"));
        assert_eq!(
            shortcut_text(CommandId::CommandPalette).as_deref(),
            Some("Ctrl+Shift+P")
        );
        // Break caught: Format JSON's row still showing Ctrl+Shift+F, now Search's shortcut.
        assert_eq!(
            shortcut_text(CommandId::FormatJson).as_deref(),
            Some("Shift+Alt+F")
        );
        assert_eq!(shortcut_text(CommandId::FindNext).as_deref(), Some("F3"));
        assert_eq!(
            shortcut_text(CommandId::FindPrevious).as_deref(),
            Some("Shift+F3")
        );
        assert_eq!(shortcut_text(CommandId::Copy), None);
    }

    fn picker(create: Option<&'static str>) -> Picker {
        Picker {
            kind: PickerKind::RecentFolder,
            items: vec!["idea".into(), "reference".into(), "todo".into()],
            create,
        }
    }

    #[test]
    fn picker_rows_filter_items_like_commands_and_offer_to_create_a_new_name() {
        // Break caught: typing a new name leaving nothing to press Enter on, or offering to
        // create a name that already exists under another case.
        let with_create = picker(Some("Create"));
        assert_eq!(
            picker_rows(&with_create, ""),
            vec![PickerRow::Item(0), PickerRow::Item(1), PickerRow::Item(2)]
        );
        assert_eq!(
            picker_rows(&with_create, "ref"),
            vec![PickerRow::Item(1), PickerRow::Create("ref".into())]
        );
        assert_eq!(picker_rows(&with_create, "TODO"), vec![PickerRow::Item(2)]);
        assert_eq!(picker_rows(&picker(None), "zzz"), vec![]);
        assert_eq!(
            picker_row_label(&with_create, &PickerRow::Create("urgent".into())),
            "Create \u{201c}urgent\u{201d}"
        );
        assert_eq!(picker_row_label(&with_create, &PickerRow::Item(0)), "idea");
    }

    #[test]
    fn markdown_preview_cycles_with_ctrl_shift_v_and_lists_three_palette_entries() {
        assert_eq!(
            shortcut_text(CommandId::MarkdownPreviewCycle).as_deref(),
            Some("Ctrl+Shift+V")
        );
        let labels = filter_entries("markdown preview", |_| true)
            .into_iter()
            .map(|entry| entry.command)
            .collect::<Vec<_>>();
        for command in [
            CommandId::MarkdownPreviewSide,
            CommandId::MarkdownPreviewFull,
            CommandId::MarkdownPreviewClose,
        ] {
            assert!(labels.contains(&command), "{command:?}");
        }
        assert!(
            filter_entries("markdown preview", |command| !command.is_markdown_preview()).is_empty()
        );
    }
}
