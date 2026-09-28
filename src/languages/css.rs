use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 24] {
    [
        style(c, SCE_CSS_DEFAULT, Role::Text),
        style(c, SCE_CSS_TAG, Role::Tag),
        style(c, SCE_CSS_CLASS, Role::Type),
        style(c, SCE_CSS_ID, Role::Type),
        style(c, SCE_CSS_PSEUDOCLASS, Role::Function),
        style(c, SCE_CSS_UNKNOWN_PSEUDOCLASS, Role::Function),
        style(c, SCE_CSS_PSEUDOELEMENT, Role::Function),
        style(c, SCE_CSS_EXTENDED_PSEUDOCLASS, Role::Function),
        style(c, SCE_CSS_EXTENDED_PSEUDOELEMENT, Role::Function),
        style(c, SCE_CSS_OPERATOR, Role::Operator),
        style(c, SCE_CSS_IDENTIFIER, Role::Key),
        style(c, SCE_CSS_UNKNOWN_IDENTIFIER, Role::Key),
        style(c, SCE_CSS_IDENTIFIER2, Role::Key),
        style(c, SCE_CSS_IDENTIFIER3, Role::Key),
        style(c, SCE_CSS_EXTENDED_IDENTIFIER, Role::Key),
        style(c, SCE_CSS_VALUE, Role::String),
        style(c, SCE_CSS_DOUBLESTRING, Role::String),
        style(c, SCE_CSS_SINGLESTRING, Role::String),
        style(c, SCE_CSS_COMMENT, Role::Comment),
        style(c, SCE_CSS_IMPORTANT, Role::Keyword),
        style(c, SCE_CSS_DIRECTIVE, Role::Keyword),
        style(c, SCE_CSS_GROUP_RULE, Role::Keyword),
        style(c, SCE_CSS_ATTRIBUTE, Role::Attribute),
        style(c, SCE_CSS_VARIABLE, Role::Variable),
    ]
}

static STYLES: [[LexerStyle; 24]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
