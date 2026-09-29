use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 9] {
    [
        style(c, SCE_BAT_DEFAULT, Role::Text),
        style(c, SCE_BAT_COMMENT, Role::Comment),
        style(c, SCE_BAT_AFTER_LABEL, Role::Comment),
        style(c, SCE_BAT_WORD, Role::Keyword),
        style(c, SCE_BAT_LABEL, Role::Function).bold(),
        style(c, SCE_BAT_HIDE, Role::Operator),
        style(c, SCE_BAT_OPERATOR, Role::Operator),
        style(c, SCE_BAT_COMMAND, Role::Function),
        style(c, SCE_BAT_IDENTIFIER, Role::Variable),
    ]
}

static STYLES: [[LexerStyle; 9]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
