use crate::editor::scintilla_constants::*;
use crate::languages::{LexerStyle, Role, SyntaxColors, code, per_theme_styles, style};
use crate::platform::theme::Theme;

const fn table(c: &SyntaxColors) -> [LexerStyle; 22] {
    [
        style(c, SCE_MARKDOWN_DEFAULT, Role::Text),
        style(c, SCE_MARKDOWN_LINE_BEGIN, Role::Text),
        style(c, SCE_MARKDOWN_PRECHAR, Role::Text),
        style(c, SCE_MARKDOWN_STRONG1, Role::Emphasis).bold(),
        style(c, SCE_MARKDOWN_STRONG2, Role::Emphasis).bold(),
        style(c, SCE_MARKDOWN_EM1, Role::Emphasis).italic(),
        style(c, SCE_MARKDOWN_EM2, Role::Emphasis).italic(),
        style(c, SCE_MARKDOWN_HEADER1, Role::Heading).bold(),
        style(c, SCE_MARKDOWN_HEADER2, Role::Heading).bold(),
        style(c, SCE_MARKDOWN_HEADER3, Role::Heading).bold(),
        style(c, SCE_MARKDOWN_HEADER4, Role::Heading).bold(),
        style(c, SCE_MARKDOWN_HEADER5, Role::Heading).bold(),
        style(c, SCE_MARKDOWN_HEADER6, Role::Heading).bold(),
        style(c, SCE_MARKDOWN_ULIST_ITEM, Role::Operator),
        style(c, SCE_MARKDOWN_OLIST_ITEM, Role::Operator),
        style(c, SCE_MARKDOWN_HRULE, Role::Operator),
        style(c, SCE_MARKDOWN_BLOCKQUOTE, Role::Comment),
        style(c, SCE_MARKDOWN_STRIKEOUT, Role::Comment),
        style(c, SCE_MARKDOWN_LINK, Role::Link),
        code(c, SCE_MARKDOWN_CODE),
        code(c, SCE_MARKDOWN_CODE2),
        code(c, SCE_MARKDOWN_CODEBK),
    ]
}

static STYLES: [[LexerStyle; 22]; Theme::COUNT] = per_theme_styles!(table);

pub(crate) fn styles(theme: Theme) -> &'static [LexerStyle] {
    &STYLES[theme as usize]
}

#[cfg(test)]
mod tests {
    use super::styles;
    use crate::editor::scintilla_constants::{SCE_MARKDOWN_CODE, SCE_MARKDOWN_DEFAULT};
    use crate::languages::{rgb, syntax_colors};
    use crate::platform::theme::Theme;

    #[test]
    fn every_theme_table_styles_the_default_markdown_text() {
        // Break caught: a style table missing an entry for SCE_MARKDOWN_DEFAULT leaves ordinary
        // Markdown text uncolored after a theme switch, instead of a deterministic FastPad color.
        for theme in Theme::ALL {
            let default = styles(theme)
                .iter()
                .find(|style| style.style == SCE_MARKDOWN_DEFAULT)
                .expect("every theme styles SCE_MARKDOWN_DEFAULT");
            assert_eq!(default.foreground, syntax_colors(theme).text);
        }
    }

    #[test]
    fn light_and_dark_tables_keep_their_original_colors() {
        // Break caught: moving to per-theme tables must not repaint existing light/dark users.
        let code = |theme| {
            styles(theme)
                .iter()
                .find(|style| style.style == SCE_MARKDOWN_CODE)
                .map(|style| (style.foreground, style.background))
        };
        assert_eq!(
            code(Theme::Light),
            Some((rgb(110, 65, 15), rgb(246, 248, 250)))
        );
        assert_eq!(
            code(Theme::Dark),
            Some((rgb(215, 186, 125), rgb(45, 45, 45)))
        );
    }
}
