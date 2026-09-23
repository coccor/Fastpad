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
    TextLeftToRight,
    TextRightToLeft,
    CommandPalette,
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
}

impl CommandId {
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
                | Self::ToggleRestoreSession
                | Self::ToggleNotesMode
                | Self::OpenFolder
                | Self::OpenRecentFolder
                | Self::ToggleFolderAutosave
                | Self::ToggleSidebar
                | Self::ShowNotebookView
                | Self::ShowSearchView
                | Self::ShowFavoritesView
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
        )
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

impl TryFrom<u16> for CommandId {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        const COMMANDS: [CommandId; 71] = [
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
            CommandId::TextLeftToRight,
            CommandId::TextRightToLeft,
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
    fn native_command_values_are_stable_and_round_trip() {
        assert_eq!(CommandId::New as u16, 100);
        assert_eq!(CommandId::Exit as u16, 117);
        assert_eq!(CommandId::try_from(103), Ok(CommandId::SaveAs));
        assert!(CommandId::try_from(99).is_err());
        assert_eq!(CommandId::try_from(118), Ok(CommandId::CloseAllTabs));
        assert!(!CommandId::New.needs_document());
        assert!(CommandId::Paste.needs_document());
        assert_eq!(CommandId::try_from(134), Ok(CommandId::TextRightToLeft));
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
    fn select_tab_commands_map_to_zero_based_indices() {
        assert_eq!(CommandId::SelectTab1.tab_index(), Some(0));
        assert_eq!(CommandId::SelectTab9.tab_index(), Some(8));
        assert_eq!(CommandId::NextTab.tab_index(), None);
        assert_eq!(CommandId::ZoomIn.tab_index(), None);
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
