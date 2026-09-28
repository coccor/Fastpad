//! Lexilla's `cpp` lexer, shared by C, C++, C#, JavaScript and TypeScript; they differ only in
//! keyword sets and lexer properties (see the registry).

use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 28] {
    [
        style(c, SCE_C_DEFAULT, Role::Text),
        style(c, SCE_C_IDENTIFIER, Role::Text),
        style(c, SCE_C_COMMENT, Role::Comment),
        style(c, SCE_C_COMMENTLINE, Role::Comment),
        style(c, SCE_C_COMMENTDOC, Role::Comment),
        style(c, SCE_C_COMMENTLINEDOC, Role::Comment),
        style(c, SCE_C_PREPROCESSORCOMMENT, Role::Comment),
        style(c, SCE_C_PREPROCESSORCOMMENTDOC, Role::Comment),
        style(c, SCE_C_NUMBER, Role::Number),
        style(c, SCE_C_UUID, Role::Number),
        style(c, SCE_C_USERLITERAL, Role::Number),
        style(c, SCE_C_WORD, Role::Keyword),
        style(c, SCE_C_TASKMARKER, Role::Keyword),
        style(c, SCE_C_WORD2, Role::Type),
        style(c, SCE_C_GLOBALCLASS, Role::Type),
        style(c, SCE_C_STRING, Role::String),
        style(c, SCE_C_CHARACTER, Role::String),
        style(c, SCE_C_VERBATIM, Role::String),
        style(c, SCE_C_STRINGRAW, Role::String),
        style(c, SCE_C_TRIPLEVERBATIM, Role::String),
        style(c, SCE_C_HASHQUOTEDSTRING, Role::String),
        style(c, SCE_C_PREPROCESSOR, Role::Preprocessor),
        style(c, SCE_C_COMMENTDOCKEYWORD, Role::Preprocessor),
        style(c, SCE_C_OPERATOR, Role::Operator),
        style(c, SCE_C_STRINGEOL, Role::Error),
        style(c, SCE_C_COMMENTDOCKEYWORDERROR, Role::Error),
        style(c, SCE_C_REGEX, Role::Escape),
        style(c, SCE_C_ESCAPESEQUENCE, Role::Escape),
    ]
}

static STYLES: [[LexerStyle; 28]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
