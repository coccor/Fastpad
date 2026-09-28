use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 21] {
    [
        style(c, SCE_P_DEFAULT, Role::Text),
        style(c, SCE_P_IDENTIFIER, Role::Text),
        style(c, SCE_P_ATTRIBUTE, Role::Text),
        style(c, SCE_P_COMMENTLINE, Role::Comment),
        style(c, SCE_P_COMMENTBLOCK, Role::Comment),
        style(c, SCE_P_NUMBER, Role::Number),
        style(c, SCE_P_STRING, Role::String),
        style(c, SCE_P_CHARACTER, Role::String),
        style(c, SCE_P_TRIPLE, Role::String),
        style(c, SCE_P_TRIPLEDOUBLE, Role::String),
        style(c, SCE_P_FSTRING, Role::String),
        style(c, SCE_P_FCHARACTER, Role::String),
        style(c, SCE_P_FTRIPLE, Role::String),
        style(c, SCE_P_FTRIPLEDOUBLE, Role::String),
        style(c, SCE_P_WORD, Role::Keyword),
        style(c, SCE_P_CLASSNAME, Role::Type),
        style(c, SCE_P_DEFNAME, Role::Function),
        style(c, SCE_P_WORD2, Role::Function),
        style(c, SCE_P_OPERATOR, Role::Operator),
        style(c, SCE_P_STRINGEOL, Role::Error),
        style(c, SCE_P_DECORATOR, Role::Preprocessor),
    ]
}

static STYLES: [[LexerStyle; 21]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
