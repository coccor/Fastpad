use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 23] {
    [
        style(c, SCE_SQL_DEFAULT, Role::Text),
        style(c, SCE_SQL_IDENTIFIER, Role::Text),
        style(c, SCE_SQL_COMMENT, Role::Comment),
        style(c, SCE_SQL_COMMENTLINE, Role::Comment),
        style(c, SCE_SQL_COMMENTDOC, Role::Comment),
        style(c, SCE_SQL_COMMENTLINEDOC, Role::Comment),
        style(c, SCE_SQL_SQLPLUS_COMMENT, Role::Comment),
        style(c, SCE_SQL_NUMBER, Role::Number),
        style(c, SCE_SQL_WORD, Role::Keyword),
        style(c, SCE_SQL_SQLPLUS, Role::Keyword),
        style(c, SCE_SQL_SQLPLUS_PROMPT, Role::Keyword),
        style(c, SCE_SQL_STRING, Role::String),
        style(c, SCE_SQL_CHARACTER, Role::String),
        style(c, SCE_SQL_OPERATOR, Role::Operator),
        style(c, SCE_SQL_QOPERATOR, Role::Operator),
        style(c, SCE_SQL_WORD2, Role::Type),
        style(c, SCE_SQL_COMMENTDOCKEYWORD, Role::Preprocessor),
        style(c, SCE_SQL_COMMENTDOCKEYWORDERROR, Role::Error),
        style(c, SCE_SQL_USER1, Role::Function),
        style(c, SCE_SQL_USER2, Role::Function),
        style(c, SCE_SQL_USER3, Role::Function),
        style(c, SCE_SQL_USER4, Role::Function),
        style(c, SCE_SQL_QUOTEDIDENTIFIER, Role::Key),
    ]
}

static STYLES: [[LexerStyle; 23]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
