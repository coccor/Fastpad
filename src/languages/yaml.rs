use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 10] {
    [
        style(c, SCE_YAML_DEFAULT, Role::Text),
        style(c, SCE_YAML_COMMENT, Role::Comment),
        style(c, SCE_YAML_IDENTIFIER, Role::Key),
        style(c, SCE_YAML_KEYWORD, Role::Keyword),
        style(c, SCE_YAML_NUMBER, Role::Number),
        style(c, SCE_YAML_REFERENCE, Role::Variable),
        style(c, SCE_YAML_DOCUMENT, Role::Preprocessor),
        style(c, SCE_YAML_TEXT, Role::String),
        style(c, SCE_YAML_ERROR, Role::Error),
        style(c, SCE_YAML_OPERATOR, Role::Operator),
    ]
}

static STYLES: [[LexerStyle; 10]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
