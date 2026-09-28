use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 14] {
    [
        style(c, SCE_SH_DEFAULT, Role::Text),
        style(c, SCE_SH_IDENTIFIER, Role::Text),
        style(c, SCE_SH_ERROR, Role::Error),
        style(c, SCE_SH_COMMENTLINE, Role::Comment),
        style(c, SCE_SH_NUMBER, Role::Number),
        style(c, SCE_SH_WORD, Role::Keyword),
        style(c, SCE_SH_STRING, Role::String),
        style(c, SCE_SH_CHARACTER, Role::String),
        style(c, SCE_SH_HERE_Q, Role::String),
        style(c, SCE_SH_OPERATOR, Role::Operator),
        style(c, SCE_SH_SCALAR, Role::Variable),
        style(c, SCE_SH_PARAM, Role::Variable),
        style(c, SCE_SH_BACKTICKS, Role::Function),
        style(c, SCE_SH_HERE_DELIM, Role::Preprocessor),
    ]
}

static STYLES: [[LexerStyle; 14]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
