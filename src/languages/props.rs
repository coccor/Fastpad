//! Lexilla's `props` lexer, shared by INI, Properties and Env files.

use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 6] {
    [
        style(c, SCE_PROPS_DEFAULT, Role::Text),
        style(c, SCE_PROPS_COMMENT, Role::Comment),
        style(c, SCE_PROPS_SECTION, Role::Heading).bold(),
        style(c, SCE_PROPS_ASSIGNMENT, Role::Operator),
        style(c, SCE_PROPS_DEFVAL, Role::Variable),
        style(c, SCE_PROPS_KEY, Role::Key),
    ]
}

static STYLES: [[LexerStyle; 6]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
