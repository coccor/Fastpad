#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandId {
    New = 100,
    Open,
    Save,
    SaveAs,
    CloseTab,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Find,
    Replace,
    ValidateJson,
    FormatJson,
    LanguagePlainText,
    LanguageJson,
    LanguageMarkdown,
    Exit,
    CloseAllTabs,
    NextTab,
    PreviousTab,
    SelectTab1,
    SelectTab2,
    SelectTab3,
    SelectTab4,
    SelectTab5,
    SelectTab6,
    SelectTab7,
    SelectTab8,
    SelectTab9,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    // 133 and 134 were the text direction commands.
    CommandPalette = 135,
    ThemeSystem,
    ThemeLight,
    ThemeDark,
    ToggleWordWrap,
    ToggleLineNumbers,
    FontSizeIncrease,
    FontSizeDecrease,
    FontSizeReset,
    TabWidth2,
    TabWidth4,
    TabWidth8,
    ThemeCatppuccin,
    ThemeCatppuccinLatte,
    ThemeCatppuccinFrappe,
    ThemeCatppuccinMacchiato,
    ThemeCatppuccinMocha,
    MarkdownPreviewCycle,
    MarkdownPreviewSide,
    MarkdownPreviewFull,
    MarkdownPreviewClose,
    ToggleRestoreSession,
    ToggleNotesMode,
    OpenFolder,
    OpenRecentFolder,
    ToggleFolderAutosave,
    NoteReloadFromDisk,
    NoteKeepMine,
    // 163 and 166-173 were the favorite, tag and notebook commands: retired, never reused.
    NoteTogglePin = 164,
    NoteMoveToNotebook = 165,
    NoteRename = 174,
    NoteDelete = 175,
    ToggleSidebar = 176,
    ShowNotebookView = 177,
    ShowSearchView = 178,
    ShowFavoritesView = 179,
    CloseNotebook = 180,
    ToggleNotebookFavorite = 181,
    NoteRevealInExplorer = 182,
    FocusNextPane = 183,
    FocusPreviousPane = 184,
    SearchToggleCase = 185,
    SearchToggleWholeWord = 186,
    SearchToggleRegex = 187,
    FindNext = 188,
    FindPrevious = 189,
    ReplaceInNotes = 190,
    QuickOpen = 191,
    NoteNewFolder = 192,
    NoteNew = 193,
    FileIconsMaterial = 194,
    FileIconsMinimal = 195,
    FileIconsSolid = 196,
    LanguageBash = 197,
    LanguageBatch = 198,
    LanguageC = 199,
    LanguageCSharp = 200,
    LanguageCpp = 201,
    LanguageCss = 202,
    LanguageEnv = 203,
    LanguageHtml = 204,
    LanguageIni = 205,
    LanguageJavaScript = 206,
    LanguagePowerShell = 207,
    LanguageProperties = 208,
    LanguagePython = 209,
    LanguageRust = 210,
    LanguageSql = 211,
    LanguageSvg = 212,
    LanguageToml = 213,
    LanguageTypeScript = 214,
    LanguageXml = 215,
    LanguageYaml = 216,
    SplitRight = 217,
    SplitDown = 218,
    CloseGroup = 219,
    FocusGroup1 = 220,
    FocusGroup2 = 221,
    FocusGroup3 = 222,
    FocusGroup4 = 223,
    FocusGroup5 = 224,
    FocusGroup6 = 225,
    FocusGroup7 = 226,
    FocusGroup8 = 227,
    FocusLastGroup = 228,
    MoveTabToNextGroup = 229,
    MoveTabToPreviousGroup = 230,
    About = 231,
    ToggleInsertSpaces = 232,
    ToggleShowWhitespace = 233,
    ToggleHighlightCurrentLine = 234,
    OpenSettings = 235,
    EditSettingsFile = 236,
    OpenKeyboardShortcuts = 237,
    ToggleAlwaysOnTop = 238,
    ThemePaperLamp = 239,
    ThemePaper = 240,
    ThemeLamp = 241,
    ToggleCodeFolding = 242,
    FoldAll = 243,
    UnfoldAll = 244,
    MoveLinesUp = 245,
    MoveLinesDown = 246,
    CopyLinesUp = 247,
    CopyLinesDown = 248,
    DeleteLines = 249,
    InsertLineBelow = 250,
    InsertLineAbove = 251,
    IndentLines = 252,
    OutdentLines = 253,
    ExpandLineSelection = 254,
    ToggleLineComment = 255,
    ToggleBlockComment = 256,
    AddNextOccurrence = 257,
    SelectAllOccurrences = 258,
    AddCursorAbove = 259,
    AddCursorBelow = 260,
}

/// The editing-shortcut commands (editing shortcuts spec §3): text commands whose keys work only
/// while an editor has the focus.
pub const EDITING_COMMANDS: [CommandId; 16] = [
    CommandId::MoveLinesUp,
    CommandId::MoveLinesDown,
    CommandId::CopyLinesUp,
    CommandId::CopyLinesDown,
    CommandId::DeleteLines,
    CommandId::InsertLineBelow,
    CommandId::InsertLineAbove,
    CommandId::IndentLines,
    CommandId::OutdentLines,
    CommandId::ExpandLineSelection,
    CommandId::ToggleLineComment,
    CommandId::ToggleBlockComment,
    CommandId::AddNextOccurrence,
    CommandId::SelectAllOccurrences,
    CommandId::AddCursorAbove,
    CommandId::AddCursorBelow,
];

/// Commands that read or change a tab's text; an image tab has none (image preview spec §5).
pub const TEXT_COMMANDS: [CommandId; 56] = [
    CommandId::MoveLinesUp,
    CommandId::MoveLinesDown,
    CommandId::CopyLinesUp,
    CommandId::CopyLinesDown,
    CommandId::DeleteLines,
    CommandId::InsertLineBelow,
    CommandId::InsertLineAbove,
    CommandId::IndentLines,
    CommandId::OutdentLines,
    CommandId::ExpandLineSelection,
    CommandId::ToggleLineComment,
    CommandId::ToggleBlockComment,
    CommandId::AddNextOccurrence,
    CommandId::SelectAllOccurrences,
    CommandId::AddCursorAbove,
    CommandId::AddCursorBelow,
    CommandId::FoldAll,
    CommandId::UnfoldAll,
    CommandId::Save,
    CommandId::SaveAs,
    CommandId::Undo,
    CommandId::Redo,
    CommandId::Cut,
    CommandId::Copy,
    CommandId::Paste,
    CommandId::Find,
    CommandId::FindNext,
    CommandId::FindPrevious,
    CommandId::Replace,
    CommandId::ValidateJson,
    CommandId::FormatJson,
    CommandId::LanguagePlainText,
    CommandId::LanguageJson,
    CommandId::LanguageMarkdown,
    CommandId::LanguageBash,
    CommandId::LanguageBatch,
    CommandId::LanguageC,
    CommandId::LanguageCSharp,
    CommandId::LanguageCpp,
    CommandId::LanguageCss,
    CommandId::LanguageEnv,
    CommandId::LanguageHtml,
    CommandId::LanguageIni,
    CommandId::LanguageJavaScript,
    CommandId::LanguagePowerShell,
    CommandId::LanguageProperties,
    CommandId::LanguagePython,
    CommandId::LanguageRust,
    CommandId::LanguageSql,
    CommandId::LanguageSvg,
    CommandId::LanguageToml,
    CommandId::LanguageTypeScript,
    CommandId::LanguageXml,
    CommandId::LanguageYaml,
    CommandId::NoteReloadFromDisk,
    CommandId::NoteKeepMine,
];

impl CommandId {
    /// Commands that need the active tab's text, and so do nothing on an image tab.
    pub fn needs_text(self) -> bool {
        TEXT_COMMANDS.contains(&self)
    }

    /// Editing-shortcut commands, whose keys stay with other controls when no editor has the
    /// focus (editing shortcuts spec §6).
    pub fn is_editing(self) -> bool {
        EDITING_COMMANDS.contains(&self)
    }

    /// Commands that act on the active document, and so do nothing while no tab is open.
    pub const fn needs_document(self) -> bool {
        !matches!(
            self,
            Self::New
                | Self::Open
                | Self::Exit
                | Self::CloseAllTabs
                | Self::CommandPalette
                | Self::ThemeSystem
                | Self::ThemeLight
                | Self::ThemeDark
                | Self::ToggleWordWrap
                | Self::ToggleLineNumbers
                | Self::FontSizeIncrease
                | Self::FontSizeDecrease
                | Self::FontSizeReset
                | Self::TabWidth2
                | Self::TabWidth4
                | Self::TabWidth8
                | Self::ThemeCatppuccin
                | Self::ThemeCatppuccinLatte
                | Self::ThemeCatppuccinFrappe
                | Self::ThemeCatppuccinMacchiato
                | Self::ThemeCatppuccinMocha
                | Self::ThemePaperLamp
                | Self::ThemePaper
                | Self::ThemeLamp
                | Self::FileIconsMaterial
                | Self::FileIconsMinimal
                | Self::FileIconsSolid
                | Self::ToggleRestoreSession
                | Self::ToggleNotesMode
                | Self::OpenFolder
                | Self::OpenRecentFolder
                | Self::ToggleFolderAutosave
                | Self::ToggleSidebar
                | Self::ShowNotebookView
                | Self::ShowSearchView
                | Self::ShowFavoritesView
                | Self::CloseNotebook
                | Self::ToggleNotebookFavorite
                | Self::FocusNextPane
                | Self::FocusPreviousPane
                | Self::SearchToggleCase
                | Self::SearchToggleWholeWord
                | Self::SearchToggleRegex
                | Self::ReplaceInNotes
                | Self::QuickOpen
                | Self::NoteNewFolder
                | Self::NoteNew
                | Self::SplitRight
                | Self::SplitDown
                | Self::CloseGroup
                | Self::FocusGroup1
                | Self::FocusGroup2
                | Self::FocusGroup3
                | Self::FocusGroup4
                | Self::FocusGroup5
                | Self::FocusGroup6
                | Self::FocusGroup7
                | Self::FocusGroup8
                | Self::FocusLastGroup
                | Self::About
                | Self::ToggleInsertSpaces
                | Self::ToggleShowWhitespace
                | Self::ToggleHighlightCurrentLine
                | Self::OpenSettings
                | Self::EditSettingsFile
                | Self::OpenKeyboardShortcuts
                | Self::ToggleAlwaysOnTop
                | Self::ToggleCodeFolding
        )
    }

    pub const fn is_markdown_preview(self) -> bool {
        matches!(
            self,
            Self::MarkdownPreviewCycle
                | Self::MarkdownPreviewSide
                | Self::MarkdownPreviewFull
                | Self::MarkdownPreviewClose
        )
    }

    /// Commands that act on the sidebar, which exists only in notes mode.
    pub const fn is_sidebar(self) -> bool {
        matches!(
            self,
            Self::ToggleSidebar
                | Self::ShowNotebookView
                | Self::ShowSearchView
                | Self::ShowFavoritesView
                | Self::SearchToggleCase
                | Self::SearchToggleWholeWord
                | Self::SearchToggleRegex
                | Self::ReplaceInNotes
        )
    }

    /// The Search view option a `SearchToggle*` command flips.
    pub const fn search_option(self) -> Option<crate::search::SearchOption> {
        match self {
            Self::SearchToggleCase => Some(crate::search::SearchOption::Case),
            Self::SearchToggleWholeWord => Some(crate::search::SearchOption::WholeWord),
            Self::SearchToggleRegex => Some(crate::search::SearchOption::Regex),
            _ => None,
        }
    }

    /// The language a `Language*` command switches the active tab to.
    pub const fn language(self) -> Option<crate::document::Language> {
        use crate::document::Language as L;
        Some(match self {
            Self::LanguagePlainText => L::PlainText,
            Self::LanguageJson => L::Json,
            Self::LanguageMarkdown => L::Markdown,
            Self::LanguageBash => L::Bash,
            Self::LanguageBatch => L::Batch,
            Self::LanguageC => L::C,
            Self::LanguageCSharp => L::CSharp,
            Self::LanguageCpp => L::Cpp,
            Self::LanguageCss => L::Css,
            Self::LanguageEnv => L::Env,
            Self::LanguageHtml => L::Html,
            Self::LanguageIni => L::Ini,
            Self::LanguageJavaScript => L::JavaScript,
            Self::LanguagePowerShell => L::PowerShell,
            Self::LanguageProperties => L::Properties,
            Self::LanguagePython => L::Python,
            Self::LanguageRust => L::Rust,
            Self::LanguageSql => L::Sql,
            Self::LanguageSvg => L::Svg,
            Self::LanguageToml => L::Toml,
            Self::LanguageTypeScript => L::TypeScript,
            Self::LanguageXml => L::Xml,
            Self::LanguageYaml => L::Yaml,
            _ => return None,
        })
    }

    /// The command that switches the active tab to `language`.
    pub const fn for_language(language: crate::document::Language) -> Self {
        use crate::document::Language as L;
        match language {
            L::PlainText => Self::LanguagePlainText,
            L::Json => Self::LanguageJson,
            L::Markdown => Self::LanguageMarkdown,
            L::Bash => Self::LanguageBash,
            L::Batch => Self::LanguageBatch,
            L::C => Self::LanguageC,
            L::CSharp => Self::LanguageCSharp,
            L::Cpp => Self::LanguageCpp,
            L::Css => Self::LanguageCss,
            L::Env => Self::LanguageEnv,
            L::Html => Self::LanguageHtml,
            L::Ini => Self::LanguageIni,
            L::JavaScript => Self::LanguageJavaScript,
            L::PowerShell => Self::LanguagePowerShell,
            L::Properties => Self::LanguageProperties,
            L::Python => Self::LanguagePython,
            L::Rust => Self::LanguageRust,
            L::Sql => Self::LanguageSql,
            L::Svg => Self::LanguageSvg,
            L::Toml => Self::LanguageToml,
            L::TypeScript => Self::LanguageTypeScript,
            L::Xml => Self::LanguageXml,
            L::Yaml => Self::LanguageYaml,
        }
    }

    /// The zero-based tab a `SelectTabN` command activates.
    pub const fn tab_index(self) -> Option<usize> {
        let first = Self::SelectTab1 as u16;
        let value = self as u16;
        if value >= first && value <= Self::SelectTab9 as u16 {
            Some((value - first) as usize)
        } else {
            None
        }
    }
}

impl CommandId {
    /// The zero-based group a `FocusGroupN` command focuses, in layout order; `usize::MAX` for
    /// Focus Last Group.
    pub const fn group_index(self) -> Option<usize> {
        let first = Self::FocusGroup1 as u16;
        let value = self as u16;
        if value >= first && value <= Self::FocusGroup8 as u16 {
            Some((value - first) as usize)
        } else if value == Self::FocusLastGroup as u16 {
            Some(usize::MAX)
        } else {
            None
        }
    }
}

impl TryFrom<u16> for CommandId {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        const COMMANDS: [CommandId; 150] = [
            CommandId::MoveLinesUp,
            CommandId::MoveLinesDown,
            CommandId::CopyLinesUp,
            CommandId::CopyLinesDown,
            CommandId::DeleteLines,
            CommandId::InsertLineBelow,
            CommandId::InsertLineAbove,
            CommandId::IndentLines,
            CommandId::OutdentLines,
            CommandId::ExpandLineSelection,
            CommandId::ToggleLineComment,
            CommandId::ToggleBlockComment,
            CommandId::AddNextOccurrence,
            CommandId::SelectAllOccurrences,
            CommandId::AddCursorAbove,
            CommandId::AddCursorBelow,
            CommandId::New,
            CommandId::Open,
            CommandId::Save,
            CommandId::SaveAs,
            CommandId::CloseTab,
            CommandId::Undo,
            CommandId::Redo,
            CommandId::Cut,
            CommandId::Copy,
            CommandId::Paste,
            CommandId::Find,
            CommandId::Replace,
            CommandId::ValidateJson,
            CommandId::FormatJson,
            CommandId::LanguagePlainText,
            CommandId::LanguageJson,
            CommandId::LanguageMarkdown,
            CommandId::Exit,
            CommandId::CloseAllTabs,
            CommandId::NextTab,
            CommandId::PreviousTab,
            CommandId::SelectTab1,
            CommandId::SelectTab2,
            CommandId::SelectTab3,
            CommandId::SelectTab4,
            CommandId::SelectTab5,
            CommandId::SelectTab6,
            CommandId::SelectTab7,
            CommandId::SelectTab8,
            CommandId::SelectTab9,
            CommandId::ZoomIn,
            CommandId::ZoomOut,
            CommandId::ZoomReset,
            CommandId::CommandPalette,
            CommandId::ThemeSystem,
            CommandId::ThemeLight,
            CommandId::ThemeDark,
            CommandId::ToggleWordWrap,
            CommandId::ToggleLineNumbers,
            CommandId::FontSizeIncrease,
            CommandId::FontSizeDecrease,
            CommandId::FontSizeReset,
            CommandId::TabWidth2,
            CommandId::TabWidth4,
            CommandId::TabWidth8,
            CommandId::ThemeCatppuccin,
            CommandId::ThemeCatppuccinLatte,
            CommandId::ThemeCatppuccinFrappe,
            CommandId::ThemeCatppuccinMacchiato,
            CommandId::ThemeCatppuccinMocha,
            CommandId::MarkdownPreviewCycle,
            CommandId::MarkdownPreviewSide,
            CommandId::MarkdownPreviewFull,
            CommandId::MarkdownPreviewClose,
            CommandId::ToggleRestoreSession,
            CommandId::ToggleNotesMode,
            CommandId::OpenFolder,
            CommandId::OpenRecentFolder,
            CommandId::ToggleFolderAutosave,
            CommandId::NoteReloadFromDisk,
            CommandId::NoteKeepMine,
            CommandId::NoteTogglePin,
            CommandId::NoteMoveToNotebook,
            CommandId::NoteRename,
            CommandId::NoteDelete,
            CommandId::ToggleSidebar,
            CommandId::ShowNotebookView,
            CommandId::ShowSearchView,
            CommandId::ShowFavoritesView,
            CommandId::CloseNotebook,
            CommandId::ToggleNotebookFavorite,
            CommandId::NoteRevealInExplorer,
            CommandId::FocusNextPane,
            CommandId::FocusPreviousPane,
            CommandId::SearchToggleCase,
            CommandId::SearchToggleWholeWord,
            CommandId::SearchToggleRegex,
            CommandId::FindNext,
            CommandId::FindPrevious,
            CommandId::ReplaceInNotes,
            CommandId::QuickOpen,
            CommandId::NoteNewFolder,
            CommandId::NoteNew,
            CommandId::FileIconsMaterial,
            CommandId::FileIconsMinimal,
            CommandId::FileIconsSolid,
            CommandId::LanguageBash,
            CommandId::LanguageBatch,
            CommandId::LanguageC,
            CommandId::LanguageCSharp,
            CommandId::LanguageCpp,
            CommandId::LanguageCss,
            CommandId::LanguageEnv,
            CommandId::LanguageHtml,
            CommandId::LanguageIni,
            CommandId::LanguageJavaScript,
            CommandId::LanguagePowerShell,
            CommandId::LanguageProperties,
            CommandId::LanguagePython,
            CommandId::LanguageRust,
            CommandId::LanguageSql,
            CommandId::LanguageSvg,
            CommandId::LanguageToml,
            CommandId::LanguageTypeScript,
            CommandId::LanguageXml,
            CommandId::LanguageYaml,
            CommandId::SplitRight,
            CommandId::SplitDown,
            CommandId::CloseGroup,
            CommandId::FocusGroup1,
            CommandId::FocusGroup2,
            CommandId::FocusGroup3,
            CommandId::FocusGroup4,
            CommandId::FocusGroup5,
            CommandId::FocusGroup6,
            CommandId::FocusGroup7,
            CommandId::FocusGroup8,
            CommandId::FocusLastGroup,
            CommandId::MoveTabToNextGroup,
            CommandId::MoveTabToPreviousGroup,
            CommandId::About,
            CommandId::ToggleInsertSpaces,
            CommandId::ToggleShowWhitespace,
            CommandId::ToggleHighlightCurrentLine,
            CommandId::OpenSettings,
            CommandId::EditSettingsFile,
            CommandId::OpenKeyboardShortcuts,
            CommandId::ToggleAlwaysOnTop,
            CommandId::ThemePaperLamp,
            CommandId::ThemePaper,
            CommandId::ThemeLamp,
            CommandId::ToggleCodeFolding,
            CommandId::FoldAll,
            CommandId::UnfoldAll,
        ];
        COMMANDS
            .into_iter()
            .find(|command| *command as u16 == value)
            .ok_or(())
    }
}

pub(crate) fn choose_open_path(
    owner: windows_sys::Win32::Foundation::HWND,
) -> crate::Result<Option<std::path::PathBuf>> {
    crate::platform::dialogs::show_open_dialog(owner)
}

pub(crate) fn choose_folder_path(
    owner: windows_sys::Win32::Foundation::HWND,
) -> crate::Result<Option<std::path::PathBuf>> {
    crate::platform::dialogs::show_folder_dialog(owner)
}

pub(crate) fn choose_save_path(
    owner: windows_sys::Win32::Foundation::HWND,
    suggested_name: &str,
    folder: Option<&std::path::Path>,
) -> crate::Result<Option<std::path::PathBuf>> {
    crate::platform::dialogs::show_save_dialog(owner, suggested_name, folder)
}

#[cfg(test)]
mod tests {
    use super::CommandId;

    #[test]
    fn the_display_toggles_are_232_to_234_and_238_and_need_no_document() {
        // Break caught: a toggle renumbered onto another command, or greyed out while no tab is
        // open (settings dialog spec §4.3).
        for (value, command) in [
            (232, CommandId::ToggleInsertSpaces),
            (233, CommandId::ToggleShowWhitespace),
            (234, CommandId::ToggleHighlightCurrentLine),
            (238, CommandId::ToggleAlwaysOnTop),
        ] {
            assert_eq!(command as u16, value);
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(!command.needs_document(), "{command:?}");
            assert!(!command.needs_text(), "{command:?}");
            assert!(!command.is_sidebar(), "{command:?}");
        }
    }

    #[test]
    fn settings_commands_are_235_to_237_and_need_no_document() {
        // Break caught: Ctrl+, renumbered onto another command, or Settings greyed out while no
        // tab is open.
        for (value, command) in [
            (235, CommandId::OpenSettings),
            (236, CommandId::EditSettingsFile),
            (237, CommandId::OpenKeyboardShortcuts),
        ] {
            assert_eq!(command as u16, value);
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(!command.needs_document(), "{command:?}");
            assert!(!command.needs_text(), "{command:?}");
            assert!(!command.is_sidebar(), "{command:?}");
        }
    }

    #[test]
    fn native_command_values_are_stable_and_round_trip() {
        assert_eq!(CommandId::New as u16, 100);
        assert_eq!(CommandId::Exit as u16, 117);
        assert_eq!(CommandId::SplitRight as u16, 217);
        assert_eq!(CommandId::MoveTabToPreviousGroup as u16, 230);
        assert_eq!(CommandId::try_from(220), Ok(CommandId::FocusGroup1));
        assert_eq!(CommandId::FocusGroup8.group_index(), Some(7));
        assert_eq!(CommandId::FocusLastGroup.group_index(), Some(usize::MAX));
        assert!(!CommandId::SplitRight.needs_document());
        assert!(CommandId::MoveTabToNextGroup.needs_document());
        assert_eq!(CommandId::try_from(103), Ok(CommandId::SaveAs));
        assert!(CommandId::try_from(99).is_err());
        assert_eq!(CommandId::try_from(118), Ok(CommandId::CloseAllTabs));
        assert!(!CommandId::New.needs_document());
        assert!(CommandId::Paste.needs_document());
        assert_eq!(CommandId::try_from(132), Ok(CommandId::ZoomReset));
        assert!(CommandId::try_from(133).is_err());
        assert!(CommandId::try_from(134).is_err());
        assert_eq!(CommandId::try_from(135), Ok(CommandId::CommandPalette));
        assert!(!CommandId::CommandPalette.needs_document());
        assert_eq!(CommandId::try_from(146), Ok(CommandId::TabWidth8));
        assert_eq!(
            CommandId::try_from(151),
            Ok(CommandId::ThemeCatppuccinMocha)
        );
        assert!(!CommandId::ToggleWordWrap.needs_document());
        assert_eq!(
            CommandId::try_from(156),
            Ok(CommandId::ToggleRestoreSession)
        );
        assert!(!CommandId::ToggleRestoreSession.needs_document());
        assert_eq!(CommandId::try_from(157), Ok(CommandId::ToggleNotesMode));
        assert!(!CommandId::ToggleNotesMode.needs_document());
        assert_eq!(CommandId::try_from(158), Ok(CommandId::OpenFolder));
        assert_eq!(CommandId::try_from(159), Ok(CommandId::OpenRecentFolder));
        assert!(!CommandId::OpenFolder.needs_document());
        assert!(!CommandId::OpenRecentFolder.needs_document());
        assert_eq!(CommandId::try_from(183), Ok(CommandId::FocusNextPane));
        assert_eq!(CommandId::try_from(184), Ok(CommandId::FocusPreviousPane));
        assert!(!CommandId::FocusNextPane.needs_document());
        assert_eq!(
            CommandId::try_from(160),
            Ok(CommandId::ToggleFolderAutosave)
        );
        assert_eq!(CommandId::try_from(161), Ok(CommandId::NoteReloadFromDisk));
        assert_eq!(CommandId::try_from(162), Ok(CommandId::NoteKeepMine));
        assert!(!CommandId::ToggleFolderAutosave.needs_document());
        assert!(CommandId::NoteReloadFromDisk.needs_document());
        assert!(CommandId::NoteKeepMine.needs_document());
        // Break caught: a removed command's number reused, so a stale shortcut or a test
        // posting 163 runs something else.
        for retired in [163_u16, 166, 167, 168, 169, 170, 171, 172, 173] {
            assert!(CommandId::try_from(retired).is_err(), "{retired}");
        }
        assert_eq!(CommandId::try_from(164), Ok(CommandId::NoteTogglePin));
        assert_eq!(CommandId::try_from(165), Ok(CommandId::NoteMoveToNotebook));
        assert_eq!(CommandId::try_from(174), Ok(CommandId::NoteRename));
        assert_eq!(CommandId::try_from(175), Ok(CommandId::NoteDelete));
        assert!(CommandId::NoteTogglePin.needs_document());
        assert!(CommandId::NoteMoveToNotebook.needs_document());
        assert!(CommandId::NoteRename.needs_document());
        assert!(CommandId::NoteDelete.needs_document());
    }

    #[test]
    fn sidebar_commands_have_their_reserved_numbers_and_need_no_document() {
        // Break caught: a renumbered view command, which would break the accelerator table and
        // any WM_COMMAND an outside test posts by number.
        assert_eq!(CommandId::try_from(176), Ok(CommandId::ToggleSidebar));
        assert_eq!(CommandId::try_from(177), Ok(CommandId::ShowNotebookView));
        assert_eq!(CommandId::try_from(178), Ok(CommandId::ShowSearchView));
        assert_eq!(CommandId::try_from(179), Ok(CommandId::ShowFavoritesView));
        for command in [
            CommandId::ToggleSidebar,
            CommandId::ShowNotebookView,
            CommandId::ShowSearchView,
            CommandId::ShowFavoritesView,
        ] {
            assert!(!command.needs_document());
            assert!(command.is_sidebar());
        }
        assert!(!CommandId::Save.is_sidebar());
    }

    #[test]
    fn notebook_commands_have_stable_values_and_need_no_document() {
        // Break caught: Close notebook greyed out, or silently ignored, while no tab is open.
        assert_eq!(CommandId::try_from(180), Ok(CommandId::CloseNotebook));
        assert_eq!(
            CommandId::try_from(181),
            Ok(CommandId::ToggleNotebookFavorite)
        );
        assert!(!CommandId::CloseNotebook.needs_document());
        assert!(!CommandId::ToggleNotebookFavorite.needs_document());
    }

    #[test]
    fn reveal_in_explorer_has_a_stable_value() {
        assert_eq!(
            CommandId::try_from(182),
            Ok(CommandId::NoteRevealInExplorer)
        );
        assert!(CommandId::NoteRevealInExplorer.needs_document());
    }

    #[test]
    fn select_tab_commands_map_to_zero_based_indices() {
        assert_eq!(CommandId::SelectTab1.tab_index(), Some(0));
        assert_eq!(CommandId::SelectTab9.tab_index(), Some(8));
        assert_eq!(CommandId::NextTab.tab_index(), None);
        assert_eq!(CommandId::ZoomIn.tab_index(), None);
    }

    #[test]
    fn search_option_commands_have_their_reserved_numbers_and_are_sidebar_commands() {
        // Break caught: a toggle renumbered into another command's range, greyed out while no
        // tab is open, or left enabled with notes mode off, where there is no Search view.
        use crate::search::SearchOption;
        for (value, command, option) in [
            (185, CommandId::SearchToggleCase, SearchOption::Case),
            (
                186,
                CommandId::SearchToggleWholeWord,
                SearchOption::WholeWord,
            ),
            (187, CommandId::SearchToggleRegex, SearchOption::Regex),
        ] {
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(command.is_sidebar(), "{command:?}");
            assert!(!command.needs_document(), "{command:?}");
            assert_eq!(command.search_option(), Some(option));
        }
        assert_eq!(CommandId::ShowSearchView.search_option(), None);
        assert_eq!(CommandId::Find.search_option(), None);
    }

    #[test]
    fn find_next_and_previous_have_stable_values_and_need_a_document() {
        assert_eq!(CommandId::try_from(188), Ok(CommandId::FindNext));
        assert_eq!(CommandId::try_from(189), Ok(CommandId::FindPrevious));
        assert!(CommandId::FindNext.needs_document());
        assert!(!CommandId::FindNext.is_sidebar());
    }

    #[test]
    fn replace_in_notes_is_190_a_sidebar_command_and_needs_no_document() {
        // Break caught: 3b's first command renumbered onto another command's value, greyed out
        // while no tab is open, or left running with notes mode off, where there is no Search
        // view (spec §5).
        assert_eq!(CommandId::ReplaceInNotes as u16, 190);
        assert_eq!(CommandId::try_from(190), Ok(CommandId::ReplaceInNotes));
        assert!(CommandId::ReplaceInNotes.is_sidebar());
        assert!(!CommandId::ReplaceInNotes.needs_document());
        assert_eq!(CommandId::ReplaceInNotes.search_option(), None);
    }

    #[test]
    fn quick_open_is_191_and_needs_no_document() {
        // Break caught: Ctrl+P renumbered onto another command, greyed out while no tab is open
        // (when opening a note matters most), or hidden with notes mode off.
        assert_eq!(CommandId::QuickOpen as u16, 191);
        assert_eq!(CommandId::try_from(191), Ok(CommandId::QuickOpen));
        assert!(!CommandId::QuickOpen.needs_document());
        assert!(!CommandId::QuickOpen.is_sidebar());
    }

    #[test]
    fn new_folder_is_192_and_neither_needs_a_document_nor_the_sidebar() {
        // Break caught: New folder renumbered onto another command, greyed out while no tab is
        // open, or treated as a sidebar command (notebook folders spec §6).
        assert_eq!(CommandId::NoteNewFolder as u16, 192);
        assert_eq!(CommandId::try_from(192), Ok(CommandId::NoteNewFolder));
        assert!(!CommandId::NoteNewFolder.needs_document());
        assert!(!CommandId::NoteNewFolder.is_sidebar());
    }

    #[test]
    fn new_note_is_193_and_neither_needs_a_document_nor_the_sidebar() {
        // Break caught: a renumbered command breaking the menus, or New note hidden while no
        // tab is open, when it is most wanted (inline naming spec §8).
        assert_eq!(CommandId::NoteNew as u16, 193);
        assert_eq!(CommandId::try_from(193), Ok(CommandId::NoteNew));
        assert!(!CommandId::NoteNew.needs_document());
        assert!(!CommandId::NoteNew.is_sidebar());
    }

    #[test]
    fn about_is_231_and_runs_without_a_document() {
        // Break caught: About renumbered onto another command, or greyed out while no tab is open
        // or an image tab is active.
        assert_eq!(CommandId::About as u16, 231);
        assert_eq!(CommandId::try_from(231), Ok(CommandId::About));
        assert!(!CommandId::About.needs_document());
        assert!(!CommandId::About.needs_text());
        assert!(!CommandId::About.is_sidebar());
    }

    #[test]
    fn language_commands_round_trip_and_keep_their_numbers() {
        // Break caught: a renumbered language command, or one that maps to the wrong language.
        use crate::document::Language;
        assert_eq!(CommandId::LanguagePlainText as u16, 114);
        assert_eq!(CommandId::LanguageJson as u16, 115);
        assert_eq!(CommandId::LanguageMarkdown as u16, 116);
        assert_eq!(CommandId::LanguageBash as u16, 197);
        assert_eq!(CommandId::LanguageYaml as u16, 216);
        for row in crate::languages::LANGUAGES.iter() {
            let command = CommandId::for_language(row.language);
            assert_eq!(command.language(), Some(row.language));
            assert_eq!(CommandId::try_from(command as u16), Ok(command));
        }
        assert_eq!(CommandId::Save.language(), None);
        assert_eq!(
            CommandId::for_language(Language::Xml),
            CommandId::LanguageXml
        );
    }

    #[test]
    fn every_language_command_needs_text() {
        // Break caught: a language command left enabled on an image tab.
        for row in crate::languages::LANGUAGES.iter() {
            assert!(
                CommandId::for_language(row.language).needs_text(),
                "{}",
                row.name
            );
        }
    }

    #[test]
    fn markdown_preview_commands_have_stable_values() {
        assert_eq!(
            CommandId::try_from(152),
            Ok(CommandId::MarkdownPreviewCycle)
        );
        assert_eq!(CommandId::try_from(153), Ok(CommandId::MarkdownPreviewSide));
        assert_eq!(CommandId::try_from(154), Ok(CommandId::MarkdownPreviewFull));
        assert_eq!(
            CommandId::try_from(155),
            Ok(CommandId::MarkdownPreviewClose)
        );
        assert!(CommandId::MarkdownPreviewSide.needs_document());
        assert!(CommandId::MarkdownPreviewClose.is_markdown_preview());
        assert!(!CommandId::Save.is_markdown_preview());
    }
}
