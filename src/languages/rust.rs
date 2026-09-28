use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 26] {
    [
        style(c, SCE_RUST_DEFAULT, Role::Text),
        style(c, SCE_RUST_IDENTIFIER, Role::Text),
        style(c, SCE_RUST_COMMENTBLOCK, Role::Comment),
        style(c, SCE_RUST_COMMENTLINE, Role::Comment),
        style(c, SCE_RUST_COMMENTBLOCKDOC, Role::Comment),
        style(c, SCE_RUST_COMMENTLINEDOC, Role::Comment),
        style(c, SCE_RUST_NUMBER, Role::Number),
        style(c, SCE_RUST_WORD, Role::Keyword),
        style(c, SCE_RUST_WORD3, Role::Keyword),
        style(c, SCE_RUST_WORD4, Role::Keyword),
        style(c, SCE_RUST_WORD5, Role::Keyword),
        style(c, SCE_RUST_WORD6, Role::Keyword),
        style(c, SCE_RUST_WORD7, Role::Keyword),
        style(c, SCE_RUST_WORD2, Role::Type),
        style(c, SCE_RUST_STRING, Role::String),
        style(c, SCE_RUST_STRINGR, Role::String),
        style(c, SCE_RUST_CHARACTER, Role::String),
        style(c, SCE_RUST_BYTESTRING, Role::String),
        style(c, SCE_RUST_BYTESTRINGR, Role::String),
        style(c, SCE_RUST_BYTECHARACTER, Role::String),
        style(c, SCE_RUST_CSTRING, Role::String),
        style(c, SCE_RUST_CSTRINGR, Role::String),
        style(c, SCE_RUST_OPERATOR, Role::Operator),
        style(c, SCE_RUST_LIFETIME, Role::Variable),
        style(c, SCE_RUST_MACRO, Role::Function),
        style(c, SCE_RUST_LEXERROR, Role::Error),
    ]
}

static STYLES: [[LexerStyle; 26]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
