use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 16] {
    [
        style(c, SCE_TOML_DEFAULT, Role::Text),
        style(c, SCE_TOML_IDENTIFIER, Role::Text),
        style(c, SCE_TOML_COMMENT, Role::Comment),
        style(c, SCE_TOML_KEYWORD, Role::Keyword),
        style(c, SCE_TOML_NUMBER, Role::Number),
        style(c, SCE_TOML_DATETIME, Role::Number),
        style(c, SCE_TOML_TABLE, Role::Heading).bold(),
        style(c, SCE_TOML_KEY, Role::Key),
        style(c, SCE_TOML_ERROR, Role::Error),
        style(c, SCE_TOML_STRINGEOL, Role::Error),
        style(c, SCE_TOML_OPERATOR, Role::Operator),
        style(c, SCE_TOML_STRING_SQ, Role::String),
        style(c, SCE_TOML_STRING_DQ, Role::String),
        style(c, SCE_TOML_TRIPLE_STRING_SQ, Role::String),
        style(c, SCE_TOML_TRIPLE_STRING_DQ, Role::String),
        style(c, SCE_TOML_ESCAPECHAR, Role::Escape),
    ]
}

static STYLES: [[LexerStyle; 16]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
