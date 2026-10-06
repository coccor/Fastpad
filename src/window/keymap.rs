//! Keyboard shortcuts: key strokes, the commands' stable IDs, the default bindings, and the
//! user's overrides resolved into the bindings every surface reads (keyboard shortcuts spec §3).
//! Pure: no window handles.

use crate::window::commands::{CommandId, Scope};
use std::collections::BTreeMap;
use std::fmt;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_ADD, VK_BACK, VK_DECIMAL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F3,
    VK_F6, VK_F10, VK_F24, VK_HOME, VK_INSERT, VK_LEFT, VK_MULTIPLY, VK_NEXT, VK_NUMPAD0,
    VK_NUMPAD9, VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA,
    VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_SUBTRACT,
    VK_TAB, VK_UP,
};
#[cfg(test)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_LWIN, VK_MENU, VK_SHIFT};
use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};

/// One key with its modifiers: what a shortcut is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct KeyStroke {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub vk: u16,
}

/// Keys named by a word or by the character they type on a US layout (spec §3.1). Letters,
/// digits, F-keys and numpad digits are named by `key_name`'s ranges instead.
const NAMED_KEYS: [(u16, &str); 31] = [
    (VK_RETURN, "Enter"),
    (VK_ESCAPE, "Escape"),
    (VK_SPACE, "Space"),
    (VK_TAB, "Tab"),
    (VK_BACK, "Backspace"),
    (VK_DELETE, "Delete"),
    (VK_INSERT, "Insert"),
    (VK_HOME, "Home"),
    (VK_END, "End"),
    (VK_PRIOR, "PageUp"),
    (VK_NEXT, "PageDown"),
    (VK_UP, "Up"),
    (VK_DOWN, "Down"),
    (VK_LEFT, "Left"),
    (VK_RIGHT, "Right"),
    (VK_OEM_PLUS, "="),
    (VK_OEM_MINUS, "-"),
    (VK_OEM_COMMA, ","),
    (VK_OEM_PERIOD, "."),
    (VK_OEM_2, "/"),
    (VK_OEM_5, "\\"),
    (VK_OEM_1, ";"),
    (VK_OEM_7, "'"),
    (VK_OEM_4, "["),
    (VK_OEM_6, "]"),
    (VK_OEM_3, "`"),
    (VK_ADD, "NumpadAdd"),
    (VK_SUBTRACT, "NumpadSubtract"),
    (VK_MULTIPLY, "NumpadMultiply"),
    (VK_DIVIDE, "NumpadDivide"),
    (VK_DECIMAL, "NumpadDecimal"),
];

fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some(char::from(vk as u8).to_string()),
        VK_F1..=VK_F24 => Some(format!("F{}", vk - VK_F1 + 1)),
        VK_NUMPAD0..=VK_NUMPAD9 => Some(format!("Numpad{}", vk - VK_NUMPAD0)),
        _ => NAMED_KEYS
            .iter()
            .find(|(key, _)| *key == vk)
            .map(|(_, name)| (*name).to_owned()),
    }
}

fn key_from_name(name: &str) -> Option<u16> {
    let upper = name.to_ascii_uppercase();
    if let [byte] = upper.as_bytes()
        && byte.is_ascii_alphanumeric()
    {
        return Some(u16::from(*byte));
    }
    if let Some(number) = upper
        .strip_prefix('F')
        .and_then(|digits| digits.parse::<u16>().ok())
        && (1..=24).contains(&number)
        && !upper[1..].starts_with('0')
    {
        return Some(VK_F1 + number - 1);
    }
    if let Some(digit) = upper.strip_prefix("NUMPAD")
        && let [byte @ b'0'..=b'9'] = digit.as_bytes()
    {
        return Some(VK_NUMPAD0 + u16::from(byte - b'0'));
    }
    NAMED_KEYS
        .iter()
        .find(|(_, key)| key.eq_ignore_ascii_case(name))
        .map(|(vk, _)| *vk)
}

impl KeyStroke {
    pub(crate) const fn new(ctrl: bool, shift: bool, alt: bool, vk: u16) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            vk,
        }
    }

    /// The stroke a key press makes, or `None` for a modifier alone or a key with no name.
    pub(crate) fn from_key(vk: u16, ctrl: bool, shift: bool, alt: bool) -> Option<Self> {
        key_name(vk)?;
        Some(Self::new(ctrl, shift, alt, vk))
    }

    /// `Ctrl+Shift+S` and the like: case-insensitive, spaces around `+` ignored, modifiers in
    /// any order, the key last.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let parts = text.split('+').map(str::trim).collect::<Vec<_>>();
        let (key, modifiers) = parts.split_last()?;
        let mut stroke = Self::new(false, false, false, key_from_name(key)?);
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => stroke.ctrl = true,
                "shift" => stroke.shift = true,
                "alt" => stroke.alt = true,
                _ => return None,
            }
        }
        Some(stroke)
    }

    /// The modifiers then the key, each as a keycap shows it.
    pub(crate) fn parts(self) -> Vec<String> {
        let mut parts = Vec::with_capacity(4);
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
        ] {
            if on {
                parts.push(name.to_owned());
            }
        }
        parts.push(key_name(self.vk).unwrap_or_else(|| format!("{:#04x}", self.vk)));
        parts
    }

    pub(crate) fn text(self) -> String {
        self.parts().join("+")
    }

    /// `ACCEL::fVirt`'s modifier bits (without `FVIRTKEY`).
    pub(crate) fn accel_flags(self) -> u8 {
        let mut flags = 0;
        if self.ctrl {
            flags |= FCONTROL;
        }
        if self.shift {
            flags |= FSHIFT;
        }
        if self.alt {
            flags |= FALT;
        }
        flags
    }
}

impl fmt::Display for KeyStroke {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text())
    }
}

/// Every command's stable ID, used in `key.<id>=` lines (spec §3.2). Never rename one: a
/// renamed ID orphans the user's saved override.
pub(crate) const COMMAND_IDS: &[(CommandId, &str)] = &[
    (CommandId::New, "file.new"),
    (CommandId::Open, "file.open"),
    (CommandId::OpenFolder, "file.openNotebook"),
    (CommandId::OpenRecentFolder, "file.openRecentNotebook"),
    (CommandId::QuickOpen, "file.goToNote"),
    (CommandId::Save, "file.save"),
    (CommandId::SaveAs, "file.saveAs"),
    (CommandId::CloseTab, "file.closeTab"),
    (CommandId::CloseAllTabs, "file.closeAllTabs"),
    (CommandId::ToggleRestoreSession, "file.toggleRestoreSession"),
    (CommandId::Exit, "file.exit"),
    (CommandId::Undo, "edit.undo"),
    (CommandId::Redo, "edit.redo"),
    (CommandId::Cut, "edit.cut"),
    (CommandId::Copy, "edit.copy"),
    (CommandId::Paste, "edit.paste"),
    (CommandId::Find, "search.find"),
    (CommandId::Replace, "search.replace"),
    (CommandId::FindNext, "search.findNext"),
    (CommandId::FindPrevious, "search.findPrevious"),
    (CommandId::ReplaceInNotes, "search.replaceInNotes"),
    (CommandId::SearchToggleCase, "search.toggleMatchCase"),
    (CommandId::SearchToggleWholeWord, "search.toggleWholeWord"),
    (CommandId::SearchToggleRegex, "search.toggleRegex"),
    (CommandId::FormatJson, "json.format"),
    (CommandId::ValidateJson, "json.validate"),
    (CommandId::LanguagePlainText, "language.plainText"),
    (CommandId::LanguageBash, "language.bash"),
    (CommandId::LanguageBatch, "language.batch"),
    (CommandId::LanguageC, "language.c"),
    (CommandId::LanguageCSharp, "language.csharp"),
    (CommandId::LanguageCpp, "language.cpp"),
    (CommandId::LanguageCss, "language.css"),
    (CommandId::LanguageEnv, "language.env"),
    (CommandId::LanguageHtml, "language.html"),
    (CommandId::LanguageIni, "language.ini"),
    (CommandId::LanguageJavaScript, "language.javascript"),
    (CommandId::LanguageJson, "language.json"),
    (CommandId::LanguageMarkdown, "language.markdown"),
    (CommandId::LanguagePowerShell, "language.powershell"),
    (CommandId::LanguageProperties, "language.properties"),
    (CommandId::LanguagePython, "language.python"),
    (CommandId::LanguageRust, "language.rust"),
    (CommandId::LanguageSql, "language.sql"),
    (CommandId::LanguageSvg, "language.svg"),
    (CommandId::LanguageToml, "language.toml"),
    (CommandId::LanguageTypeScript, "language.typescript"),
    (CommandId::LanguageXml, "language.xml"),
    (CommandId::LanguageYaml, "language.yaml"),
    (CommandId::MarkdownPreviewCycle, "markdown.cyclePreview"),
    (CommandId::MarkdownPreviewSide, "markdown.previewSide"),
    (CommandId::MarkdownPreviewFull, "markdown.previewFull"),
    (CommandId::MarkdownPreviewClose, "markdown.closePreview"),
    (CommandId::NextTab, "view.nextTab"),
    (CommandId::PreviousTab, "view.previousTab"),
    (CommandId::SelectTab1, "view.selectTab1"),
    (CommandId::SelectTab2, "view.selectTab2"),
    (CommandId::SelectTab3, "view.selectTab3"),
    (CommandId::SelectTab4, "view.selectTab4"),
    (CommandId::SelectTab5, "view.selectTab5"),
    (CommandId::SelectTab6, "view.selectTab6"),
    (CommandId::SelectTab7, "view.selectTab7"),
    (CommandId::SelectTab8, "view.selectTab8"),
    (CommandId::SelectTab9, "view.selectTab9"),
    (CommandId::CommandPalette, "view.commandPalette"),
    (CommandId::ToggleSidebar, "view.toggleSidebar"),
    (CommandId::ShowNotebookView, "view.showNotebook"),
    (CommandId::ShowSearchView, "view.showSearch"),
    (CommandId::ShowFavoritesView, "view.showFavorites"),
    (CommandId::FocusNextPane, "view.focusNextPane"),
    (CommandId::FocusPreviousPane, "view.focusPreviousPane"),
    (CommandId::SplitRight, "view.splitRight"),
    (CommandId::SplitDown, "view.splitDown"),
    (CommandId::CloseGroup, "view.closeGroup"),
    (CommandId::FocusGroup1, "view.focusGroup1"),
    (CommandId::FocusGroup2, "view.focusGroup2"),
    (CommandId::FocusGroup3, "view.focusGroup3"),
    (CommandId::FocusGroup4, "view.focusGroup4"),
    (CommandId::FocusGroup5, "view.focusGroup5"),
    (CommandId::FocusGroup6, "view.focusGroup6"),
    (CommandId::FocusGroup7, "view.focusGroup7"),
    (CommandId::FocusGroup8, "view.focusGroup8"),
    (CommandId::FocusLastGroup, "view.focusLastGroup"),
    (CommandId::MoveTabToNextGroup, "view.moveToNextGroup"),
    (
        CommandId::MoveTabToPreviousGroup,
        "view.moveToPreviousGroup",
    ),
    (CommandId::ZoomIn, "view.zoomIn"),
    (CommandId::ZoomOut, "view.zoomOut"),
    (CommandId::ZoomReset, "view.zoomReset"),
    (CommandId::ToggleWordWrap, "view.toggleWordWrap"),
    (CommandId::ToggleLineNumbers, "view.toggleLineNumbers"),
    (CommandId::FontSizeIncrease, "view.fontSizeIncrease"),
    (CommandId::FontSizeDecrease, "view.fontSizeDecrease"),
    (CommandId::FontSizeReset, "view.fontSizeReset"),
    (CommandId::ThemeSystem, "theme.system"),
    (CommandId::ThemeLight, "theme.light"),
    (CommandId::ThemeDark, "theme.dark"),
    (CommandId::ThemeCatppuccin, "theme.catppuccin"),
    (CommandId::ThemeCatppuccinLatte, "theme.catppuccinLatte"),
    (CommandId::ThemeCatppuccinFrappe, "theme.catppuccinFrappe"),
    (
        CommandId::ThemeCatppuccinMacchiato,
        "theme.catppuccinMacchiato",
    ),
    (CommandId::ThemeCatppuccinMocha, "theme.catppuccinMocha"),
    (CommandId::ThemePaperLamp, "theme.paperLamp"),
    (CommandId::ThemePaper, "theme.paper"),
    (CommandId::ThemeLamp, "theme.lamp"),
    (CommandId::FileIconsMaterial, "fileIcons.material"),
    (CommandId::FileIconsMinimal, "fileIcons.minimal"),
    (CommandId::FileIconsSolid, "fileIcons.solid"),
    (CommandId::TabWidth2, "editor.tabWidth2"),
    (CommandId::TabWidth4, "editor.tabWidth4"),
    (CommandId::TabWidth8, "editor.tabWidth8"),
    (CommandId::ToggleInsertSpaces, "editor.toggleInsertSpaces"),
    (
        CommandId::ToggleShowWhitespace,
        "editor.toggleShowWhitespace",
    ),
    (
        CommandId::ToggleHighlightCurrentLine,
        "editor.toggleHighlightCurrentLine",
    ),
    (CommandId::ToggleAlwaysOnTop, "view.toggleAlwaysOnTop"),
    (CommandId::ToggleCodeFolding, "view.toggleCodeFolding"),
    (CommandId::FoldAll, "edit.foldAll"),
    (CommandId::UnfoldAll, "edit.unfoldAll"),
    (CommandId::ToggleNotesMode, "notes.toggleNotesMode"),
    (CommandId::ToggleFolderAutosave, "notes.toggleAutosave"),
    (CommandId::CloseNotebook, "notebook.close"),
    (CommandId::ToggleNotebookFavorite, "notebook.toggleFavorite"),
    (CommandId::NoteNew, "notebook.newNote"),
    (CommandId::NoteNewFolder, "notebook.newFolder"),
    (CommandId::NoteReloadFromDisk, "note.reloadFromDisk"),
    (CommandId::NoteKeepMine, "note.keepMine"),
    (CommandId::NoteTogglePin, "note.togglePin"),
    (CommandId::NoteMoveToNotebook, "note.moveToNotebook"),
    (CommandId::NoteRevealInExplorer, "note.revealInExplorer"),
    (CommandId::NoteRename, "note.rename"),
    (CommandId::NoteDelete, "note.delete"),
    (CommandId::OpenSettings, "preferences.openSettings"),
    (
        CommandId::OpenKeyboardShortcuts,
        "preferences.openKeyboardShortcuts",
    ),
    (CommandId::EditSettingsFile, "preferences.editSettingsFile"),
    (CommandId::MoveLinesUp, "edit.moveLinesUp"),
    (CommandId::MoveLinesDown, "edit.moveLinesDown"),
    (CommandId::CopyLinesUp, "edit.copyLinesUp"),
    (CommandId::CopyLinesDown, "edit.copyLinesDown"),
    (CommandId::DeleteLines, "edit.deleteLines"),
    (CommandId::InsertLineBelow, "edit.insertLineBelow"),
    (CommandId::InsertLineAbove, "edit.insertLineAbove"),
    (CommandId::IndentLines, "edit.indentLines"),
    (CommandId::OutdentLines, "edit.outdentLines"),
    (CommandId::ExpandLineSelection, "edit.expandLineSelection"),
    (CommandId::ToggleLineComment, "edit.toggleLineComment"),
    (CommandId::ToggleBlockComment, "edit.toggleBlockComment"),
    (CommandId::AddNextOccurrence, "edit.addNextOccurrence"),
    (CommandId::SelectAllOccurrences, "edit.selectAllOccurrences"),
    (CommandId::AddCursorAbove, "edit.addCursorAbove"),
    (CommandId::AddCursorBelow, "edit.addCursorBelow"),
    (CommandId::About, "help.about"),
    (CommandId::MarkdownToggleLive, "markdown.toggleLive"),
    (CommandId::MarkdownBold, "markdown.bold"),
    (CommandId::MarkdownItalic, "markdown.italic"),
    (CommandId::MarkdownCode, "markdown.code"),
    (CommandId::MarkdownLink, "markdown.link"),
];

pub(crate) fn command_id(command: CommandId) -> Option<&'static str> {
    COMMAND_IDS
        .iter()
        .find(|(candidate, _)| *candidate == command)
        .map(|(_, id)| *id)
}

pub(crate) fn command_for_id(id: &str) -> Option<CommandId> {
    COMMAND_IDS
        .iter()
        .find(|(_, candidate)| *candidate == id)
        .map(|(command, _)| *command)
}

const C: u8 = 1;
const S: u8 = 2;
const A: u8 = 4;

const fn key(modifiers: u8, vk: u16) -> KeyStroke {
    KeyStroke::new(
        modifiers & C != 0,
        modifiers & S != 0,
        modifiers & A != 0,
        vk,
    )
}

const fn ch(byte: u8) -> u16 {
    byte as u16
}

/// FastPad's shortcuts before any override, in precedence order (spec §3.3).
pub(crate) const DEFAULT_BINDINGS: [(KeyStroke, CommandId); 87] = [
    (key(C, ch(b'N')), CommandId::New),
    (key(C, ch(b'T')), CommandId::New),
    (key(C, ch(b'O')), CommandId::Open),
    (key(C | S, ch(b'O')), CommandId::OpenFolder),
    (key(C | S, ch(b'M')), CommandId::NoteMoveToNotebook),
    (key(C, ch(b'S')), CommandId::Save),
    (key(C | S, ch(b'S')), CommandId::SaveAs),
    (key(C, ch(b'W')), CommandId::CloseTab),
    (key(C, ch(b'F')), CommandId::Find),
    (key(C, ch(b'H')), CommandId::Replace),
    (key(C | S, ch(b'H')), CommandId::ReplaceInNotes),
    (key(0, VK_F3), CommandId::FindNext),
    (key(S, VK_F3), CommandId::FindPrevious),
    (key(C, ch(b'Z')), CommandId::Undo),
    (key(C, ch(b'Y')), CommandId::Redo),
    (key(C | S, ch(b'F')), CommandId::ShowSearchView),
    (key(S | A, ch(b'F')), CommandId::FormatJson),
    (key(C, VK_TAB), CommandId::NextTab),
    (key(C | S, VK_TAB), CommandId::PreviousTab),
    // Ctrl+digits focus editor groups; Alt+digits select tabs (split editors spec §6).
    (key(C, ch(b'1')), CommandId::FocusGroup1),
    (key(C, ch(b'2')), CommandId::FocusGroup2),
    (key(C, ch(b'3')), CommandId::FocusGroup3),
    (key(C, ch(b'4')), CommandId::FocusGroup4),
    (key(C, ch(b'5')), CommandId::FocusGroup5),
    (key(C, ch(b'6')), CommandId::FocusGroup6),
    (key(C, ch(b'7')), CommandId::FocusGroup7),
    (key(C, ch(b'8')), CommandId::FocusGroup8),
    (key(C, ch(b'9')), CommandId::FocusLastGroup),
    (key(C, VK_NUMPAD0 + 1), CommandId::FocusGroup1),
    (key(C, VK_NUMPAD0 + 2), CommandId::FocusGroup2),
    (key(C, VK_NUMPAD0 + 3), CommandId::FocusGroup3),
    (key(C, VK_NUMPAD0 + 4), CommandId::FocusGroup4),
    (key(C, VK_NUMPAD0 + 5), CommandId::FocusGroup5),
    (key(C, VK_NUMPAD0 + 6), CommandId::FocusGroup6),
    (key(C, VK_NUMPAD0 + 7), CommandId::FocusGroup7),
    (key(C, VK_NUMPAD0 + 8), CommandId::FocusGroup8),
    (key(C, VK_NUMPAD9), CommandId::FocusLastGroup),
    (key(A, ch(b'1')), CommandId::SelectTab1),
    (key(A, ch(b'2')), CommandId::SelectTab2),
    (key(A, ch(b'3')), CommandId::SelectTab3),
    (key(A, ch(b'4')), CommandId::SelectTab4),
    (key(A, ch(b'5')), CommandId::SelectTab5),
    (key(A, ch(b'6')), CommandId::SelectTab6),
    (key(A, ch(b'7')), CommandId::SelectTab7),
    (key(A, ch(b'8')), CommandId::SelectTab8),
    (key(A, ch(b'9')), CommandId::SelectTab9),
    (key(C | A, VK_RIGHT), CommandId::MoveTabToNextGroup),
    (key(C | A, VK_LEFT), CommandId::MoveTabToPreviousGroup),
    // "+" shares a key with "=" on most layouts, so Ctrl+Shift+= is Ctrl++ as typed.
    (key(C, VK_OEM_PLUS), CommandId::ZoomIn),
    (key(C | S, VK_OEM_PLUS), CommandId::ZoomIn),
    (key(C, VK_ADD), CommandId::ZoomIn),
    (key(C, VK_OEM_MINUS), CommandId::ZoomOut),
    (key(C, VK_SUBTRACT), CommandId::ZoomOut),
    (key(C, ch(b'0')), CommandId::ZoomReset),
    (key(C, VK_NUMPAD0), CommandId::ZoomReset),
    (key(C, ch(b'P')), CommandId::QuickOpen),
    (key(C | S, ch(b'P')), CommandId::CommandPalette),
    (key(C, VK_OEM_COMMA), CommandId::OpenSettings),
    (key(C | S, ch(b'V')), CommandId::MarkdownPreviewCycle),
    (key(C, ch(b'B')), CommandId::ToggleSidebar),
    (key(C | S, ch(b'E')), CommandId::ShowNotebookView),
    (key(A, ch(b'Z')), CommandId::ToggleWordWrap),
    (key(0, VK_F6), CommandId::FocusNextPane),
    (key(S, VK_F6), CommandId::FocusPreviousPane),
    // The backslash key on a US layout (split editors spec §6).
    (key(C, VK_OEM_5), CommandId::SplitRight),
    (key(C | S, VK_OEM_5), CommandId::SplitDown),
    // VS Code's editing keys (editing shortcuts spec §3).
    (key(A, VK_UP), CommandId::MoveLinesUp),
    (key(A, VK_DOWN), CommandId::MoveLinesDown),
    (key(S | A, VK_UP), CommandId::CopyLinesUp),
    (key(S | A, VK_DOWN), CommandId::CopyLinesDown),
    (key(C | S, ch(b'K')), CommandId::DeleteLines),
    (key(C, VK_RETURN), CommandId::InsertLineBelow),
    (key(C | S, VK_RETURN), CommandId::InsertLineAbove),
    (key(C, VK_OEM_6), CommandId::IndentLines),
    (key(C, VK_OEM_4), CommandId::OutdentLines),
    (key(C, ch(b'L')), CommandId::ExpandLineSelection),
    (key(C, VK_OEM_2), CommandId::ToggleLineComment),
    (key(S | A, ch(b'A')), CommandId::ToggleBlockComment),
    (key(C, ch(b'D')), CommandId::AddNextOccurrence),
    (key(C | S, ch(b'L')), CommandId::SelectAllOccurrences),
    (key(C | A, VK_UP), CommandId::AddCursorAbove),
    (key(C | A, VK_DOWN), CommandId::AddCursorBelow),
    (key(C | A, ch(b'V')), CommandId::MarkdownToggleLive),
    // Markdown-scoped (live mode spec §9): these never shadow a global binding.
    (key(C, ch(b'B')), CommandId::MarkdownBold),
    (key(C, ch(b'I')), CommandId::MarkdownItalic),
    (key(C, VK_OEM_3), CommandId::MarkdownCode),
    (key(C, ch(b'K')), CommandId::MarkdownLink),
];

pub(crate) fn default_keys(command: CommandId) -> Vec<KeyStroke> {
    DEFAULT_BINDINGS
        .iter()
        .filter(|(_, candidate)| *candidate == command)
        .map(|(stroke, _)| *stroke)
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Source {
    Default,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Binding {
    pub stroke: KeyStroke,
    pub command: CommandId,
    pub source: Source,
}

/// The defaults with the user's overrides applied: each overridden command has exactly the
/// user's keys (none when unbound), every other command its defaults. `bindings` is in
/// precedence order: user bindings (by command ID), then defaults (in table order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Keymap {
    overrides: Vec<(CommandId, Vec<KeyStroke>)>,
    bindings: Vec<Binding>,
}

pub(crate) const TYPING_REFUSAL: &str = "Needs Ctrl or Alt: it would stop typing.";

/// Whether `stroke` may be a shortcut: a key that types or edits text needs Ctrl or Alt, Alt
/// with a numpad digit types an Alt code, and F10 / Shift+F10 belong to the menus (spec §6.5).
pub(crate) fn bindable(stroke: KeyStroke) -> Result<(), &'static str> {
    let vk = stroke.vk;
    let types = matches!(
        vk,
        0x30..=0x39
            | 0x41..=0x5A
            | VK_NUMPAD0..=VK_NUMPAD9
            | VK_SPACE
            | VK_BACK
            | VK_DELETE
            | VK_TAB
            | VK_RETURN
            | VK_ESCAPE
            | VK_ADD
            | VK_SUBTRACT
            | VK_MULTIPLY
            | VK_DIVIDE
            | VK_DECIMAL
            | VK_OEM_PLUS
            | VK_OEM_MINUS
            | VK_OEM_COMMA
            | VK_OEM_PERIOD
            | VK_OEM_1
            | VK_OEM_2
            | VK_OEM_3
            | VK_OEM_4
            | VK_OEM_5
            | VK_OEM_6
            | VK_OEM_7
    );
    if types && !stroke.ctrl && !stroke.alt {
        return Err(TYPING_REFUSAL);
    }
    if stroke.alt && !stroke.ctrl && (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk) {
        return Err("Alt with a numpad digit types a character code.");
    }
    if vk == VK_F10 && !stroke.ctrl && !stroke.alt {
        return Err("F10 and Shift+F10 open the menus.");
    }
    Ok(())
}

/// `key.<id>`, the `fastpad.ini` key of `command`'s override.
pub(crate) fn ini_key(command: CommandId) -> Option<String> {
    command_id(command).map(|id| format!("key.{id}"))
}

/// The `fastpad.ini` value for `keys`: `Ctrl+S, Ctrl+T`, or empty when there are none.
pub(crate) fn ini_value(keys: &[KeyStroke]) -> String {
    keys.iter()
        .map(|stroke| stroke.text())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A `fastpad.ini` value's keys, trimmed, empties dropped. A comma right after `+` is the comma
/// key (`Ctrl+,`), not a separator.
fn split_keys(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for c in value.chars() {
        if c == ',' && !current.trim_end().ends_with('+') {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

impl Keymap {
    pub(crate) fn defaults() -> Self {
        Self::resolve(Vec::new())
    }

    fn resolve(mut overrides: Vec<(CommandId, Vec<KeyStroke>)>) -> Self {
        overrides.sort_by_key(|(command, _)| command_id(*command));
        let mut bindings = Vec::with_capacity(DEFAULT_BINDINGS.len());
        for (command, keys) in &overrides {
            bindings.extend(keys.iter().map(|stroke| Binding {
                stroke: *stroke,
                command: *command,
                source: Source::User,
            }));
        }
        for (stroke, command) in DEFAULT_BINDINGS {
            if !overrides
                .iter()
                .any(|(overridden, _)| *overridden == command)
            {
                bindings.push(Binding {
                    stroke,
                    command,
                    source: Source::Default,
                });
            }
        }
        Self {
            overrides,
            bindings,
        }
    }

    /// The keymap `fastpad.ini`'s `key.<id>=` lines make (`entries` maps id to value), and one
    /// warning per part it skipped (spec §4).
    pub(crate) fn from_ini(entries: &BTreeMap<String, String>) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut overrides = Vec::new();
        for (id, value) in entries {
            let Some(command) = command_for_id(id) else {
                warnings.push(format!("key.{id}: unknown command"));
                continue;
            };
            let mut keys = Vec::new();
            let mut any = false;
            for part in split_keys(value) {
                let part = part.as_str();
                any = true;
                match KeyStroke::parse(part).map(|stroke| (stroke, bindable(stroke))) {
                    Some((stroke, Ok(()))) => {
                        if !keys.contains(&stroke) {
                            keys.push(stroke);
                        }
                    }
                    Some((_, Err(reason))) => {
                        warnings.push(format!(
                            "key.{id}: \"{part}\" can't be a shortcut: {reason}"
                        ));
                    }
                    None => warnings.push(format!("key.{id}: unknown key \"{part}\"")),
                }
            }
            // A value with nothing usable keeps the defaults; an empty one unbinds.
            if keys.is_empty() && any {
                continue;
            }
            overrides.push((command, keys));
        }
        (Self::resolve(overrides), warnings)
    }

    pub(crate) fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    pub(crate) fn keys_of(&self, command: CommandId) -> Vec<KeyStroke> {
        self.bindings
            .iter()
            .filter(|binding| binding.command == command)
            .map(|binding| binding.stroke)
            .collect()
    }

    pub(crate) fn is_user(&self, command: CommandId) -> bool {
        self.overrides
            .iter()
            .any(|(overridden, _)| *overridden == command)
    }

    /// The global command `stroke` runs: the first global binding in precedence order. The
    /// accelerator table resolves keys in the app; `first_text` and the tests follow the same
    /// rule with this.
    pub(crate) fn command_for(&self, stroke: KeyStroke) -> Option<CommandId> {
        self.command_for_in(stroke, Scope::Global)
    }

    /// The command `stroke` runs among the bindings of `scope` (live mode spec §9).
    pub(crate) fn command_for_in(&self, stroke: KeyStroke, scope: Scope) -> Option<CommandId> {
        self.bindings
            .iter()
            .find(|binding| binding.stroke == stroke && binding.command.scope() == scope)
            .map(|binding| binding.command)
    }

    /// The commands other than `except` bound to `stroke` in `except`'s scope, each once, in
    /// precedence order. A Markdown key that matches a global one is not a conflict.
    pub(crate) fn conflicts(&self, stroke: KeyStroke, except: CommandId) -> Vec<CommandId> {
        let mut commands = Vec::new();
        for binding in &self.bindings {
            if binding.stroke == stroke
                && binding.command != except
                && binding.command.scope() == except.scope()
                && !commands.contains(&binding.command)
            {
                commands.push(binding.command);
            }
        }
        commands
    }

    /// The text menus and the palette show for `command`: its first key that runs it. A key a
    /// higher-precedence binding holds runs that command instead, so it is skipped.
    pub(crate) fn first_text(&self, command: CommandId) -> Option<String> {
        self.bindings
            .iter()
            .filter(|binding| binding.command == command)
            .map(|binding| binding.stroke)
            .find(|stroke| self.command_for_in(*stroke, command.scope()) == Some(command))
            .map(|stroke| stroke.text())
    }

    /// This keymap with `command` bound to exactly `keys` (repeats dropped). Keys equal to the
    /// defaults drop the override instead.
    pub(crate) fn with_keys(&self, command: CommandId, keys: Vec<KeyStroke>) -> Self {
        let mut unique = Vec::with_capacity(keys.len());
        for stroke in keys {
            if !unique.contains(&stroke) {
                unique.push(stroke);
            }
        }
        if unique == default_keys(command) {
            return self.without_override(command);
        }
        let mut overrides = self.without_override(command).overrides;
        overrides.push((command, unique));
        Self::resolve(overrides)
    }

    pub(crate) fn without_override(&self, command: CommandId) -> Self {
        Self::resolve(
            self.overrides
                .iter()
                .filter(|(overridden, _)| *overridden != command)
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_spell_like_vs_code_and_parse_back() {
        // Break caught: a stroke that formats one way and parses another, so a saved override
        // stops loading after one round trip through fastpad.ini.
        let cases = [
            (
                KeyStroke::new(true, false, false, u16::from(b'S')),
                "Ctrl+S",
            ),
            (
                KeyStroke::new(true, true, false, u16::from(b'S')),
                "Ctrl+Shift+S",
            ),
            (
                KeyStroke::new(false, true, true, u16::from(b'F')),
                "Shift+Alt+F",
            ),
            (KeyStroke::new(true, false, false, VK_OEM_PLUS), "Ctrl+="),
            (KeyStroke::new(true, false, false, VK_OEM_5), "Ctrl+\\"),
            (KeyStroke::new(true, false, false, VK_OEM_COMMA), "Ctrl+,"),
            (KeyStroke::new(false, false, false, VK_F3), "F3"),
            (KeyStroke::new(false, true, false, VK_F24), "Shift+F24"),
            (
                KeyStroke::new(true, false, false, VK_NUMPAD0),
                "Ctrl+Numpad0",
            ),
            (KeyStroke::new(true, false, false, VK_ADD), "Ctrl+NumpadAdd"),
            (
                KeyStroke::new(true, false, true, VK_RIGHT),
                "Ctrl+Alt+Right",
            ),
            (KeyStroke::new(true, false, false, VK_PRIOR), "Ctrl+PageUp"),
            (KeyStroke::new(true, false, false, VK_TAB), "Ctrl+Tab"),
        ];
        for (stroke, text) in cases {
            assert_eq!(stroke.text(), text);
            assert_eq!(stroke.to_string(), text);
            assert_eq!(KeyStroke::parse(text), Some(stroke), "{text}");
        }
    }

    #[test]
    fn every_named_key_round_trips() {
        // Break caught: a key in the name table that parses to a different virtual key.
        let mut keys: Vec<u16> = (u16::from(b'0')..=u16::from(b'9'))
            .chain(u16::from(b'A')..=u16::from(b'Z'))
            .chain(VK_F1..=VK_F24)
            .chain(VK_NUMPAD0..=VK_NUMPAD9)
            .collect();
        keys.extend(NAMED_KEYS.iter().map(|(vk, _)| *vk));
        for vk in keys {
            let stroke = KeyStroke::new(true, false, false, vk);
            assert_eq!(KeyStroke::parse(&stroke.text()), Some(stroke), "{vk:#x}");
        }
    }

    #[test]
    fn parsing_ignores_case_and_spaces_and_puts_modifiers_in_order() {
        // Break caught: a hand-written "alt + shift + ctrl + z" rejected, or saved back in the
        // user's order so the same stroke has two spellings.
        let stroke = KeyStroke::parse(" alt + shift + control + z ").unwrap();
        assert_eq!(stroke, KeyStroke::new(true, true, true, u16::from(b'Z')));
        assert_eq!(stroke.text(), "Ctrl+Shift+Alt+Z");
        assert_eq!(
            KeyStroke::parse("pageup"),
            Some(KeyStroke::new(false, false, false, VK_PRIOR))
        );
    }

    #[test]
    fn nonsense_does_not_parse() {
        // Break caught: a typo in fastpad.ini silently binding some other key.
        for text in [
            "", "Ctrl+", "+S", "Hyper+S", "Ctrl+Foo", "Numpad10", "Numpad09", "F0", "F25",
            "Ctrl+S+X", "Ctrl++",
        ] {
            assert_eq!(KeyStroke::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn modifier_keys_and_unnamed_keys_are_not_strokes() {
        // Break caught: pressing Ctrl alone in the recording box recorded "Ctrl+Ctrl".
        for vk in [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, 0xE5] {
            assert_eq!(KeyStroke::from_key(vk, true, false, false), None, "{vk:#x}");
        }
        assert_eq!(
            KeyStroke::from_key(u16::from(b'K'), false, false, true),
            Some(KeyStroke::new(false, false, true, u16::from(b'K')))
        );
    }

    #[test]
    fn accelerator_flags_carry_the_modifiers() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};
        assert_eq!(
            KeyStroke::new(true, true, true, u16::from(b'A')).accel_flags(),
            FCONTROL | FSHIFT | FALT
        );
        assert_eq!(KeyStroke::new(false, false, false, VK_F3).accel_flags(), 0);
    }

    use crate::window::commands::CommandId;
    use std::collections::BTreeMap;

    fn stroke(text: &str) -> KeyStroke {
        KeyStroke::parse(text).unwrap()
    }

    fn ini(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn every_command_has_one_unique_id() {
        // Break caught: a new command with no ID (it can't be rebound or saved), or two
        // commands sharing an ID so one override moves both.
        let mut seen = std::collections::HashSet::new();
        for value in 0..=u16::MAX {
            if let Ok(command) = CommandId::try_from(value) {
                let id = command_id(command).unwrap_or_else(|| panic!("{command:?} has no ID"));
                assert!(seen.insert(id), "duplicate ID {id}");
                assert_eq!(command_for_id(id), Some(command));
            }
        }
        assert_eq!(seen.len(), COMMAND_IDS.len());
        assert_eq!(command_for_id("File.Save"), None, "IDs are case-sensitive");
    }

    #[test]
    fn the_defaults_resolve_with_first_key_text() {
        // Break caught: a default key resolving to the wrong command, or menus hinting a
        // command's second key (or any key) where its first belongs.
        let keymap = Keymap::defaults();
        assert_eq!(keymap.command_for(stroke("Ctrl+S")), Some(CommandId::Save));
        assert_eq!(
            keymap.first_text(CommandId::Save).as_deref(),
            Some("Ctrl+S")
        );
        assert_eq!(
            keymap.first_text(CommandId::ZoomIn).as_deref(),
            Some("Ctrl+=")
        );
        assert_eq!(keymap.keys_of(CommandId::ZoomIn).len(), 3);
        assert_eq!(keymap.first_text(CommandId::About), None);
        assert!(!keymap.is_user(CommandId::Save));
        assert!(
            keymap
                .bindings()
                .iter()
                .all(|binding| binding.source == Source::Default)
        );
    }

    #[test]
    fn a_user_override_replaces_the_commands_defaults_and_beats_them_on_a_shared_key() {
        // Break caught: an override added next to the defaults instead of replacing them, or a
        // default winning the key the user just took.
        let (keymap, warnings) = Keymap::from_ini(&ini(&[("search.find", "Ctrl+S, F9")]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            keymap.keys_of(CommandId::Find),
            [stroke("Ctrl+S"), stroke("F9")]
        );
        assert!(keymap.is_user(CommandId::Find));
        assert_eq!(keymap.command_for(stroke("Ctrl+F")), None);
        assert_eq!(keymap.command_for(stroke("Ctrl+S")), Some(CommandId::Find));
        assert_eq!(
            keymap.conflicts(stroke("Ctrl+S"), CommandId::Find),
            [CommandId::Save]
        );
        assert_eq!(
            keymap.conflicts(stroke("Ctrl+S"), CommandId::Save),
            [CommandId::Find]
        );
        assert!(keymap.conflicts(stroke("F9"), CommandId::Find).is_empty());
    }

    #[test]
    fn two_user_bindings_on_one_key_resolve_by_command_id() {
        // Break caught: which of two clashing overrides wins changing from run to run.
        let (keymap, _) = Keymap::from_ini(&ini(&[("view.zoomIn", "F9"), ("edit.undo", "F9")]));
        assert_eq!(keymap.command_for(stroke("F9")), Some(CommandId::Undo));
        assert_eq!(
            keymap.conflicts(stroke("F9"), CommandId::Undo),
            [CommandId::ZoomIn]
        );
    }

    #[test]
    fn an_empty_value_unbinds_and_bad_parts_are_warned_and_skipped() {
        // Break caught: a typo dropping the whole fastpad.ini line, or an unknown command
        // silently ignored.
        let (keymap, warnings) = Keymap::from_ini(&ini(&[
            ("file.save", ""),
            ("file.open", "Ctrl+Foo, F9"),
            ("nope.command", "Ctrl+Q"),
            ("file.new", "Ctrl+Foo"),
        ]));
        assert!(keymap.keys_of(CommandId::Save).is_empty());
        assert!(keymap.is_user(CommandId::Save));
        assert_eq!(keymap.keys_of(CommandId::Open), [stroke("F9")]);
        // Nothing usable on the line: the defaults stay.
        assert_eq!(
            keymap.keys_of(CommandId::New),
            [stroke("Ctrl+N"), stroke("Ctrl+T")]
        );
        assert!(!keymap.is_user(CommandId::New));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("key.nope.command") && w.contains("unknown command"))
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("key.file.open") && w.contains("Ctrl+Foo"))
        );
    }

    #[test]
    fn from_ini_refuses_keys_that_would_stop_typing() {
        // Break caught: `key.file.save=A` in fastpad.ini binding plain A, so typing "a" saves.
        let (keymap, warnings) = Keymap::from_ini(&ini(&[("file.save", "A, Ctrl+Alt+S")]));
        assert_eq!(keymap.keys_of(CommandId::Save), [stroke("Ctrl+Alt+S")]);
        assert_eq!(keymap.command_for(stroke("A")), None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("key.file.save") && warnings[0].contains("\"A\""));
    }

    #[test]
    fn with_keys_equal_to_the_defaults_drops_the_override() {
        // Break caught: resetting by hand leaving `key.file.new=Ctrl+N, Ctrl+T` in fastpad.ini,
        // so a later change to the defaults never reaches the user.
        let changed = Keymap::defaults().with_keys(CommandId::New, vec![stroke("F9")]);
        assert!(changed.is_user(CommandId::New));
        let back = changed.with_keys(CommandId::New, vec![stroke("Ctrl+N"), stroke("Ctrl+T")]);
        assert!(!back.is_user(CommandId::New));
        assert_eq!(back, Keymap::defaults());
        assert_eq!(changed.without_override(CommandId::New), Keymap::defaults());
    }

    #[test]
    fn the_comma_key_survives_a_list_of_keys() {
        // Break caught: `key.preferences.openSettings=Ctrl+,, F9` split at the comma key's own
        // comma, so Settings loses Ctrl+, after one save.
        let keys = vec![stroke("Ctrl+,"), stroke("F9"), stroke("Shift+Alt+,")];
        let value = ini_value(&keys);
        assert_eq!(value, "Ctrl+,, F9, Shift+Alt+,");
        let (keymap, warnings) = Keymap::from_ini(&ini(&[("preferences.openSettings", &value)]));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(keymap.keys_of(CommandId::OpenSettings), keys);
    }

    #[test]
    fn first_text_skips_a_key_a_higher_binding_holds() {
        // Break caught: Find next still hinting F3 after the user gave F3 to Save As, so the
        // menu shows a key that saves instead of finding.
        let keymap = Keymap::defaults().with_keys(CommandId::SaveAs, vec![stroke("F3")]);
        assert_eq!(keymap.command_for(stroke("F3")), Some(CommandId::SaveAs));
        assert_eq!(keymap.first_text(CommandId::SaveAs).as_deref(), Some("F3"));
        assert_eq!(keymap.first_text(CommandId::FindNext), None);
        let keymap = keymap.with_keys(CommandId::FindNext, vec![stroke("F3"), stroke("F8")]);
        assert_eq!(
            keymap.first_text(CommandId::FindNext).as_deref(),
            Some("F8")
        );
    }

    #[test]
    fn with_keys_drops_repeats_and_keeps_order() {
        // Break caught: a repeated key saved twice (`F9, F8, F9`), or the order the user gave
        // lost, so the first key menus show changes.
        let keymap = Keymap::defaults().with_keys(
            CommandId::Save,
            vec![stroke("F9"), stroke("F8"), stroke("F9")],
        );
        assert_eq!(
            keymap.keys_of(CommandId::Save),
            [stroke("F9"), stroke("F8")]
        );
        assert_eq!(ini_value(&keymap.keys_of(CommandId::Save)), "F9, F8");
        assert_eq!(ini_key(CommandId::Save).as_deref(), Some("key.file.save"));
    }

    #[test]
    fn typing_keys_need_ctrl_or_alt() {
        // Break caught: plain letters, Space or Backspace accepted as shortcuts, so they stop
        // typing; or F-keys and arrows refused although they type nothing.
        for text in [
            "A",
            "Shift+A",
            "5",
            "Space",
            "Backspace",
            "Delete",
            "Shift+Delete",
            "Tab",
            "=",
            "Numpad5",
            "NumpadAdd",
            "Enter",
            "Escape",
        ] {
            assert_eq!(bindable(stroke(text)), Err(TYPING_REFUSAL), "{text}");
        }
        for text in [
            "Ctrl+A",
            "Alt+A",
            "F9",
            "Shift+F9",
            "Home",
            "Ctrl+Space",
            "Shift+PageDown",
            "Insert",
            "Ctrl+Numpad5",
        ] {
            assert_eq!(bindable(stroke(text)), Ok(()), "{text}");
        }
        assert!(bindable(stroke("Alt+Numpad0")).is_err(), "Alt codes");
        assert!(bindable(stroke("F10")).is_err(), "menu");
        assert!(bindable(stroke("Shift+F10")).is_err(), "context menu");
        assert_eq!(bindable(stroke("Ctrl+F10")), Ok(()));
    }

    #[test]
    fn markdown_bindings_do_not_shadow_global_ones() {
        // Break caught: Ctrl+B bolding in a .txt file, or Toggle Sidebar losing its key text.
        let keymap = Keymap::defaults();
        let ctrl_b = KeyStroke::parse("Ctrl+B").unwrap();
        assert_eq!(keymap.command_for(ctrl_b), Some(CommandId::ToggleSidebar));
        assert_eq!(
            keymap.command_for_in(ctrl_b, Scope::Markdown),
            Some(CommandId::MarkdownBold)
        );
        assert_eq!(
            keymap.first_text(CommandId::ToggleSidebar).as_deref(),
            Some("Ctrl+B")
        );
        assert_eq!(
            keymap.first_text(CommandId::MarkdownBold).as_deref(),
            Some("Ctrl+B")
        );
        assert!(keymap.conflicts(ctrl_b, CommandId::MarkdownBold).is_empty());
        assert!(
            keymap
                .conflicts(ctrl_b, CommandId::ToggleSidebar)
                .is_empty()
        );
    }

    #[test]
    fn live_markdown_toggles_with_ctrl_alt_v() {
        let keymap = Keymap::defaults();
        let stroke = KeyStroke::parse("Ctrl+Alt+V").unwrap();
        assert_eq!(
            keymap.command_for(stroke),
            Some(CommandId::MarkdownToggleLive)
        );
    }

    #[test]
    fn a_user_rebinding_keeps_the_commands_scope() {
        let entries = BTreeMap::from([("markdown.bold".to_owned(), "Ctrl+Shift+B".to_owned())]);
        let (keymap, warnings) = Keymap::from_ini(&entries);
        assert!(warnings.is_empty(), "{warnings:?}");
        let stroke = KeyStroke::parse("Ctrl+Shift+B").unwrap();
        assert_eq!(keymap.command_for(stroke), None);
        assert_eq!(
            keymap.command_for_in(stroke, Scope::Markdown),
            Some(CommandId::MarkdownBold)
        );
    }
}
