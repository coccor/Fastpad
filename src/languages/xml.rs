//! XML, SVG and HTML (Lexilla's `xml` and `hypertext` lexers share the `SCE_H_*` styles; HTML's
//! embedded JavaScript uses `SCE_HJ_*`). Lexilla does not sub-lex `<style>` contents.

use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 45] {
    [
        style(c, SCE_H_DEFAULT, Role::Text),
        style(c, SCE_H_TAG, Role::Tag),
        style(c, SCE_H_TAGUNKNOWN, Role::Tag),
        style(c, SCE_H_TAGEND, Role::Tag),
        style(c, SCE_H_SCRIPT, Role::Tag),
        style(c, SCE_H_ATTRIBUTE, Role::Attribute),
        style(c, SCE_H_ATTRIBUTEUNKNOWN, Role::Attribute),
        style(c, SCE_H_NUMBER, Role::Number),
        style(c, SCE_H_DOUBLESTRING, Role::String),
        style(c, SCE_H_SINGLESTRING, Role::String),
        style(c, SCE_H_VALUE, Role::String),
        style(c, SCE_H_CDATA, Role::String),
        style(c, SCE_H_SGML_DOUBLESTRING, Role::String),
        style(c, SCE_H_SGML_SIMPLESTRING, Role::String),
        style(c, SCE_H_OTHER, Role::Operator),
        style(c, SCE_H_SGML_SPECIAL, Role::Operator),
        style(c, SCE_H_COMMENT, Role::Comment),
        style(c, SCE_H_XCCOMMENT, Role::Comment),
        style(c, SCE_H_SGML_COMMENT, Role::Comment),
        style(c, SCE_H_SGML_1ST_PARAM_COMMENT, Role::Comment),
        style(c, SCE_H_ENTITY, Role::Escape),
        style(c, SCE_H_SGML_ENTITY, Role::Escape),
        style(c, SCE_H_XMLSTART, Role::Preprocessor),
        style(c, SCE_H_XMLEND, Role::Preprocessor),
        style(c, SCE_H_QUESTION, Role::Preprocessor),
        style(c, SCE_H_SGML_DEFAULT, Role::Preprocessor),
        style(c, SCE_H_SGML_COMMAND, Role::Preprocessor),
        style(c, SCE_H_SGML_BLOCK_DEFAULT, Role::Preprocessor),
        style(c, SCE_H_ASP, Role::Preprocessor),
        style(c, SCE_H_ASPAT, Role::Preprocessor),
        style(c, SCE_H_SGML_1ST_PARAM, Role::Type),
        style(c, SCE_H_SGML_ERROR, Role::Error),
        style(c, SCE_HJ_START, Role::Text),
        style(c, SCE_HJ_DEFAULT, Role::Text),
        style(c, SCE_HJ_WORD, Role::Text),
        style(c, SCE_HJ_COMMENT, Role::Comment),
        style(c, SCE_HJ_COMMENTLINE, Role::Comment),
        style(c, SCE_HJ_COMMENTDOC, Role::Comment),
        style(c, SCE_HJ_NUMBER, Role::Number),
        style(c, SCE_HJ_KEYWORD, Role::Keyword),
        style(c, SCE_HJ_DOUBLESTRING, Role::String),
        style(c, SCE_HJ_SINGLESTRING, Role::String),
        style(c, SCE_HJ_TEMPLATELITERAL, Role::String),
        style(c, SCE_HJ_SYMBOLS, Role::Operator),
        style(c, SCE_HJ_STRINGEOL, Role::Error),
    ]
}

static STYLES: [[LexerStyle; 45]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}
