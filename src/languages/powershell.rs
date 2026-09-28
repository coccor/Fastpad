use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 17] {
    [
        style(c, SCE_POWERSHELL_DEFAULT, Role::Text),
        style(c, SCE_POWERSHELL_IDENTIFIER, Role::Text),
        style(c, SCE_POWERSHELL_COMMENT, Role::Comment),
        style(c, SCE_POWERSHELL_COMMENTSTREAM, Role::Comment),
        style(c, SCE_POWERSHELL_STRING, Role::String),
        style(c, SCE_POWERSHELL_CHARACTER, Role::String),
        style(c, SCE_POWERSHELL_HERE_STRING, Role::String),
        style(c, SCE_POWERSHELL_HERE_CHARACTER, Role::String),
        style(c, SCE_POWERSHELL_NUMBER, Role::Number),
        style(c, SCE_POWERSHELL_VARIABLE, Role::Variable),
        style(c, SCE_POWERSHELL_OPERATOR, Role::Operator),
        style(c, SCE_POWERSHELL_KEYWORD, Role::Keyword),
        style(c, SCE_POWERSHELL_CMDLET, Role::Function),
        style(c, SCE_POWERSHELL_ALIAS, Role::Function),
        style(c, SCE_POWERSHELL_FUNCTION, Role::Function),
        style(c, SCE_POWERSHELL_USER1, Role::Type),
        style(c, SCE_POWERSHELL_COMMENTDOCKEYWORD, Role::Preprocessor),
    ]
}

static STYLES: [[LexerStyle; 17]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
