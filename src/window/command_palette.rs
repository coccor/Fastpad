//! Ctrl+Shift+P command palette: a filter field over a list of every command the window can run,
//! overlaid at the top of the editor. A small painted panel hosts a native `Edit` and an
//! owner-drawn `ListBox` in the editor's palette colors; the main window owns showing, layout,
//! and running the chosen command.

use crate::library::quick_open::QuickMatch;
use crate::platform::{last_error, wide_null};
use crate::window::commands::CommandId;
use crate::window::design::metrics::{CONTROL_RADIUS, ROW_INSET_X, ROW_INSET_Y, scale};
use crate::window::design::round::{Corners, fill_bordered, fill_rounded, radius_for};
use crate::window::design::text_scale::scale_text;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, create_panel, fill, inset, text_height};
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
    VK_SHIFT, VK_UP,
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
pub(crate) const ENTRIES: [PaletteEntry; 112] = [
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
    entry("Language: Plain Text", CommandId::LanguagePlainText),
    entry("Language: Bash", CommandId::LanguageBash),
    entry("Language: Batch", CommandId::LanguageBatch),
    entry("Language: C", CommandId::LanguageC),
    entry("Language: C#", CommandId::LanguageCSharp),
    entry("Language: C++", CommandId::LanguageCpp),
    entry("Language: CSS", CommandId::LanguageCss),
    entry("Language: Env", CommandId::LanguageEnv),
    entry("Language: HTML", CommandId::LanguageHtml),
    entry("Language: INI", CommandId::LanguageIni),
    entry("Language: JavaScript", CommandId::LanguageJavaScript),
    entry("Language: JSON", CommandId::LanguageJson),
    entry("Language: Markdown", CommandId::LanguageMarkdown),
    entry("Language: PowerShell", CommandId::LanguagePowerShell),
    entry("Language: Properties", CommandId::LanguageProperties),
    entry("Language: Python", CommandId::LanguagePython),
    entry("Language: Rust", CommandId::LanguageRust),
    entry("Language: SQL", CommandId::LanguageSql),
    entry("Language: SVG", CommandId::LanguageSvg),
    entry("Language: TOML", CommandId::LanguageToml),
    entry("Language: TypeScript", CommandId::LanguageTypeScript),
    entry("Language: XML", CommandId::LanguageXml),
    entry("Language: YAML", CommandId::LanguageYaml),
    entry(
        "Markdown Preview: Side by Side",
        CommandId::MarkdownPreviewSide,
    ),
    entry("Markdown Preview: Full", CommandId::MarkdownPreviewFull),
    entry("Close Markdown Preview", CommandId::MarkdownPreviewClose),
    entry("View: Next tab", CommandId::NextTab),
    entry("View: Previous tab", CommandId::PreviousTab),
    entry("View: Toggle sidebar", CommandId::ToggleSidebar),
    entry("View: Split Editor Right", CommandId::SplitRight),
    entry("View: Split Editor Down", CommandId::SplitDown),
    entry("View: Close Editor Group", CommandId::CloseGroup),
    entry(
        "View: Move Editor into Next Group",
        CommandId::MoveTabToNextGroup,
    ),
    entry(
        "View: Move Editor into Previous Group",
        CommandId::MoveTabToPreviousGroup,
    ),
    entry("View: Show notebook", CommandId::ShowNotebookView),
    entry("View: Show search", CommandId::ShowSearchView),
    entry("View: Show favorites", CommandId::ShowFavoritesView),
    entry("View: Zoom in", CommandId::ZoomIn),
    entry("View: Zoom out", CommandId::ZoomOut),
    entry("View: Reset zoom", CommandId::ZoomReset),
    entry("View: Toggle word wrap", CommandId::ToggleWordWrap),
    entry("View: Toggle line numbers", CommandId::ToggleLineNumbers),
    entry("View: Increase font size", CommandId::FontSizeIncrease),
    entry("View: Decrease font size", CommandId::FontSizeDecrease),
    entry("View: Reset font size", CommandId::FontSizeReset),
    entry("Theme: System", CommandId::ThemeSystem),
    entry("Theme: Light", CommandId::ThemeLight),
    entry("Theme: Dark", CommandId::ThemeDark),
    entry(
        "Theme: Catppuccin Latte / Mocha (follows system)",
        CommandId::ThemeCatppuccin,
    ),
    entry("Theme: Catppuccin Latte", CommandId::ThemeCatppuccinLatte),
    entry("Theme: Catppuccin Frappe", CommandId::ThemeCatppuccinFrappe),
    entry(
        "Theme: Catppuccin Macchiato",
        CommandId::ThemeCatppuccinMacchiato,
    ),
    entry("Theme: Catppuccin Mocha", CommandId::ThemeCatppuccinMocha),
    entry(
        "Theme: Paper / Lamp (follows system)",
        CommandId::ThemePaperLamp,
    ),
    entry("Theme: Paper", CommandId::ThemePaper),
    entry("Theme: Lamp", CommandId::ThemeLamp),
    entry("File icons: Material", CommandId::FileIconsMaterial),
    entry("File icons: Minimal", CommandId::FileIconsMinimal),
    entry("File icons: Solid", CommandId::FileIconsSolid),
    entry("Editor: Tab width 2", CommandId::TabWidth2),
    entry("Editor: Tab width 4", CommandId::TabWidth4),
    entry("Editor: Tab width 8", CommandId::TabWidth8),
    entry(
        "Editor: Toggle indent with spaces",
        CommandId::ToggleInsertSpaces,
    ),
    entry(
        "Editor: Toggle show whitespace",
        CommandId::ToggleShowWhitespace,
    ),
    entry(
        "Editor: Toggle highlight current line",
        CommandId::ToggleHighlightCurrentLine,
    ),
    entry("View: Toggle always on top", CommandId::ToggleAlwaysOnTop),
    entry("View: Toggle code folding", CommandId::ToggleCodeFolding),
    entry("Edit: Fold all", CommandId::FoldAll),
    entry("Edit: Unfold all", CommandId::UnfoldAll),
    entry("File: Exit", CommandId::Exit),
    entry("Preferences: Open Settings", CommandId::OpenSettings),
    entry(
        "Preferences: Open Keyboard Shortcuts",
        CommandId::OpenKeyboardShortcuts,
    ),
    entry("Preferences: Edit fastpad.ini", CommandId::EditSettingsFile),
    entry("Help: About FastPad", CommandId::About),
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
    /// An empty query's open tab: the note shown in `group`, whose number the row shows while
    /// there are several groups (split editors spec §7).
    View {
        found: QuickMatch,
        group: crate::window::split_tree::GroupId,
        number: Option<usize>,
    },
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
    /// An open tab: the note, relative to the notebook, in `group`.
    View {
        path: PathBuf,
        group: crate::window::split_tree::GroupId,
    },
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
        PickerRow::View { found, number, .. } => {
            let mut label = if found.folder.is_empty() {
                found.name.clone()
            } else {
                format!("{}, in {}", found.name, found.folder)
            };
            if let Some(number) = number {
                label.push_str(&format!(", group {number}"));
            }
            label
        }
    }
}

const WIDTH_AT_96_DPI: i32 = 560;
const PADDING_AT_96_DPI: i32 = 6;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const ROW_HEIGHT_AT_96_DPI: i32 = 26;
const VISIBLE_ROWS: usize = 12;
const MARGIN_AT_96_DPI: i32 = 8;

/// The field box: rounded, in the editor's colors with an accent border, over the strip card.
fn paint_field(dc: HDC, field: RECT, colors: &Palette, dpi: u32) {
    unsafe {
        fill_bordered(
            dc,
            field,
            radius_for(colors, CONTROL_RADIUS, dpi),
            colors.editor_background,
            colors.selection_background,
            colors.strip_background,
        );
    }
}

/// A list row's background: the strip color, with the selected row's fill inset and rounded
/// (the whole row, square, under a high-contrast palette).
fn paint_row_background(dc: HDC, row: RECT, selected: bool, colors: &Palette, dpi: u32) {
    unsafe {
        fill(dc, row, colors.strip_background);
        if !selected {
            return;
        }
        if colors.high_contrast {
            fill(dc, row, colors.hover_background);
            return;
        }
        let fill_rect = RECT {
            left: row.left + scale(ROW_INSET_X, dpi),
            top: row.top + scale(ROW_INSET_Y, dpi),
            right: row.right - scale(ROW_INSET_X, dpi),
            bottom: row.bottom - scale(ROW_INSET_Y, dpi),
        };
        fill_rounded(
            dc,
            fill_rect,
            radius_for(colors, CONTROL_RADIUS, dpi),
            Corners::ALL,
            colors.hover_background,
            colors.strip_background,
        );
    }
}

/// Where the panel's parts sit, in panel client coordinates.
#[derive(Clone, Copy)]
struct PanelLayout {
    /// The DPI this layout was scaled for, which painting scales its radii and insets by too.
    dpi: u32,
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
        let field_height = scale_text(FIELD_HEIGHT_AT_96_DPI, dpi);
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
            bottom: field.bottom + padding + rows * scale_text(ROW_HEIGHT_AT_96_DPI, dpi),
        });
        // The list runs to the bottom border; the field alone keeps its padding below.
        let height = list.map_or(field.bottom + padding, |list| list.bottom + 1);
        Self {
            dpi,
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
    /// Each shown row's key text, from the window's keymap when the rows were set.
    shown_keys: Vec<Option<String>>,
    /// `Some` while the palette lists runtime items instead of commands.
    picker: Option<Picker>,
    /// The rows `picker` currently shows, filtered by the query.
    picker_rows: Vec<PickerRow>,
    /// The row `fill_list` selects in picker mode; `None` selects nothing.
    picker_selected: Option<usize>,
    visible: bool,
    colors: Palette,
    layout: Option<PanelLayout>,
    field_brush: HBRUSH,
    list_brush: HBRUSH,
    /// The list's font when the bold one was made, and the bold one; null until a quick-open
    /// row is first drawn.
    /// The bold font, with the list font, DPI and text-size factor it was made for.
    bold: Cell<(HFONT, (u32, u32), HFONT)>,
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
            shown_keys: Vec::new(),
            picker: None,
            picker_rows: Vec::new(),
            picker_selected: None,
            visible: false,
            colors,
            layout: None,
            field_brush: unsafe { CreateSolidBrush(colors.editor_background) },
            list_brush: unsafe { CreateSolidBrush(colors.strip_background) },
            bold: Cell::new((std::ptr::null_mut(), (0, 0), std::ptr::null_mut())),
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

    /// Records the rows to list and their keys; `fill_list` then puts them in the list box.
    pub(crate) fn set_entries(
        &mut self,
        entries: Vec<PaletteEntry>,
        keymap: &crate::window::keymap::Keymap,
    ) {
        self.shown_keys = entries
            .iter()
            .map(|entry| keymap.first_text(entry.command))
            .collect();
        self.shown = entries;
    }

    /// The key text shown on row `index`.
    #[cfg(test)]
    pub(crate) fn shown_shortcut(&self, index: usize) -> Option<&str> {
        self.shown_keys.get(index)?.as_deref()
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
                scale_text(ROW_HEIGHT_AT_96_DPI, dpi) as LPARAM,
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
            PickerRow::View { found, group, .. } => PickerChoice::View {
                path: found.path.clone(),
                group: *group,
            },
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
    /// in the editor's colors with a rounded accent outline around the borderless `Edit`.
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
                paint_field(dc, layout.field, &colors, layout.dpi);
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
            let shortcut = index
                .and_then(|index| self.shown_keys.get(index))
                .cloned()
                .flatten();
            (entry.label.to_owned(), shortcut)
        };
        let selected = item.itemState & ODS_SELECTED != 0;
        let colors = self.colors;
        let (foreground, muted) = if selected {
            (colors.hover_foreground, colors.hover_foreground)
        } else {
            (colors.editor_foreground, colors.muted_foreground)
        };
        let dpi = self.layout.map_or(0, |layout| layout.dpi);
        let dc = item.hDC;
        // Row text lines up with the query text above it; the list starts one pixel in.
        let padding = self.layout.map_or(8, |layout| layout.edit.left - 1);
        let mut text = RECT {
            left: item.rcItem.left + padding,
            right: item.rcItem.right - padding,
            ..item.rcItem
        };
        unsafe {
            paint_row_background(dc, item.rcItem, selected, &colors, dpi);
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
        let (foreground, muted) = if selected {
            (colors.hover_foreground, colors.hover_foreground)
        } else {
            (colors.editor_foreground, colors.muted_foreground)
        };
        let dpi = self.layout.map_or(0, |layout| layout.dpi);
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
            paint_row_background(dc, item.rcItem, selected, &colors, dpi);
            SetBkMode(dc, TRANSPARENT as i32);
        }
        let previous = unsafe { SelectObject(dc, font) };
        match row {
            PickerRow::Note { found, .. } | PickerRow::View { found, .. } => {
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
                if let PickerRow::View {
                    number: Some(number),
                    ..
                } = row
                {
                    text.left += padding;
                    let group = format!("Group {number}");
                    draw_runs(dc, &mut text, &group, &[], font, bold, muted);
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

    /// `base` in bold, made on first use and again when the list's font, the DPI or the text-size
    /// factor changes (a recycled handle value alone does not prove the font is the same).
    /// A list with no font yet uses the default GUI font's metrics.
    fn bold_font(&self, base: HFONT) -> HFONT {
        let key = (
            self.layout.as_ref().map_or(0, |layout| layout.dpi),
            crate::window::design::text_scale::factor(),
        );
        let (made_for, made_at, bold) = self.bold.get();
        if made_for == base && made_at == key && !bold.is_null() {
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
        self.bold.set((base, key, bold));
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
        !self.bold.get().2.is_null()
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
        let (_, _, bold) = self.bold.get();
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
mod tests;
