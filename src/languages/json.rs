use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 14] {
    [
        style(c, SCE_JSON_DEFAULT, Role::Text),
        style(c, SCE_JSON_NUMBER, Role::Number),
        style(c, SCE_JSON_STRING, Role::String),
        style(c, SCE_JSON_STRINGEOL, Role::Error),
        style(c, SCE_JSON_PROPERTYNAME, Role::Key),
        style(c, SCE_JSON_ESCAPESEQUENCE, Role::Escape),
        style(c, SCE_JSON_LINECOMMENT, Role::Comment),
        style(c, SCE_JSON_BLOCKCOMMENT, Role::Comment),
        style(c, SCE_JSON_OPERATOR, Role::Operator),
        style(c, SCE_JSON_URI, Role::Link),
        style(c, SCE_JSON_COMPACTIRI, Role::Type),
        style(c, SCE_JSON_KEYWORD, Role::Keyword),
        style(c, SCE_JSON_LDKEYWORD, Role::Preprocessor),
        style(c, SCE_JSON_ERROR, Role::Error),
    ]
}

static STYLES: [[LexerStyle; 14]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}

#[cfg(test)]
mod tests {
    use super::styles;
    use crate::editor::scintilla_constants::{SCE_JSON_DEFAULT, SCE_JSON_STRING};
    use crate::languages::{rgb, syntax_colors};
    use crate::platform::theme::Theme;

    #[test]
    fn every_theme_table_styles_the_default_json_text() {
        // Break caught: a style table missing an entry for SCE_JSON_DEFAULT leaves ordinary JSON
        // text uncolored (whatever Scintilla's built-in default happens to be) after a theme
        // switch, instead of a deterministic FastPad color.
        for theme in Theme::ALL {
            let default = styles(theme)
                .iter()
                .find(|style| style.style == SCE_JSON_DEFAULT)
                .expect("every theme styles SCE_JSON_DEFAULT");
            assert_eq!(default.foreground, syntax_colors(theme).text);
            assert_eq!(default.background, syntax_colors(theme).background);
        }
    }

    #[test]
    fn light_and_dark_tables_keep_their_original_colors() {
        // Break caught: moving to per-theme tables must not repaint existing light/dark users.
        let string = |theme| {
            styles(theme)
                .iter()
                .find(|style| style.style == SCE_JSON_STRING)
                .map(|style| style.foreground)
        };
        assert_eq!(string(Theme::Light), Some(rgb(163, 21, 21)));
        assert_eq!(string(Theme::Dark), Some(rgb(206, 145, 120)));
    }
}
