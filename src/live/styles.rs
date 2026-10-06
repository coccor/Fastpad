//! Live Markdown's Scintilla style table (live mode spec §2, §6): hidden markup takes no width,
//! blanked markup keeps its width in the background colour for the painter to draw over, and a
//! revealed line swaps both for the visible marker style.

use super::spans::SpanKind;
use crate::languages::SyntaxColors;

pub const TEXT: u8 = 0;
pub const HIDDEN: u8 = 1;
pub const BLANK: u8 = 2;
pub const BOLD: u8 = 3;
pub const ITALIC: u8 = 4;
pub const BOLD_ITALIC: u8 = 5;
pub const INLINE_CODE: u8 = 6;
pub const CODE_BLOCK: u8 = 7;
pub const LINK: u8 = 8;
pub const HEADING_SMALL: u8 = 9;
pub const QUOTE: u8 = 10;
pub const DIM: u8 = 11;
pub const MARKER: u8 = 12;
pub const TABLE: u8 = 13;
pub const TABLE_HEADER: u8 = 14;
pub const SOURCE_HEADING: u8 = 15;
pub const TABLE_BLANK: u8 = 16;
pub const CODE_MARKER: u8 = 17;
/// Styles 32–39 are Scintilla's predefined styles; the annotation style sits above them.
pub const ANNOTATION: u8 = 40;
pub const STRIKE_INDICATOR: u32 = 20;

pub fn style_for(kind: SpanKind, revealed: bool) -> u8 {
    match (kind, revealed) {
        (SpanKind::Text, _) => TEXT,
        (SpanKind::Bold, _) => BOLD,
        (SpanKind::Italic, _) => ITALIC,
        (SpanKind::BoldItalic, _) => BOLD_ITALIC,
        (SpanKind::InlineCode, _) => INLINE_CODE,
        (SpanKind::CodeBlock, _) => CODE_BLOCK,
        (SpanKind::Link, _) => LINK,
        (SpanKind::Quote, _) => QUOTE,
        (SpanKind::Dim, _) => DIM,
        (SpanKind::Marker, _) => MARKER,
        (SpanKind::Hide | SpanKind::Blank | SpanKind::HeadingMarker(_), true) => MARKER,
        (SpanKind::Hide | SpanKind::HeadingMarker(_), false) => HIDDEN,
        (SpanKind::Blank, false) => BLANK,
        (SpanKind::TableCell, _) => TABLE,
        (SpanKind::TableHeader, _) => TABLE_HEADER,
        (SpanKind::TableBlank, true) => TABLE,
        (SpanKind::TableBlank, false) => TABLE_BLANK,
        (SpanKind::HeadingText(_), true) => SOURCE_HEADING,
        (SpanKind::HeadingText(level), false) if level <= 3 => BLANK,
        (SpanKind::HeadingText(_), false) => HEADING_SMALL,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyleDef {
    pub style: u8,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub visible: bool,
    pub eol_filled: bool,
    /// Editor (monospace) font instead of the prose font.
    pub mono: bool,
}

// Used by the window layer in a later task.
#[allow(dead_code)]
pub(crate) fn style_table(c: &SyntaxColors) -> Vec<StyleDef> {
    let plain = |style, foreground| StyleDef {
        style,
        foreground,
        background: c.background,
        bold: false,
        italic: false,
        underline: false,
        visible: true,
        eol_filled: false,
        mono: false,
    };
    vec![
        plain(TEXT, c.text),
        StyleDef { visible: false, ..plain(HIDDEN, c.text) },
        plain(BLANK, c.background),
        StyleDef { bold: true, ..plain(BOLD, c.emphasis) },
        StyleDef { italic: true, ..plain(ITALIC, c.emphasis) },
        StyleDef { bold: true, italic: true, ..plain(BOLD_ITALIC, c.emphasis) },
        StyleDef { background: c.code_background, mono: true, ..plain(INLINE_CODE, c.code) },
        StyleDef {
            background: c.code_background,
            mono: true,
            eol_filled: true,
            ..plain(CODE_BLOCK, c.code)
        },
        StyleDef { underline: true, ..plain(LINK, c.link) },
        StyleDef { bold: true, ..plain(HEADING_SMALL, c.heading) },
        StyleDef { italic: true, ..plain(QUOTE, c.comment) },
        plain(DIM, c.comment),
        plain(MARKER, c.operator),
        StyleDef { mono: true, ..plain(TABLE, c.text) },
        StyleDef { mono: true, bold: true, ..plain(TABLE_HEADER, c.text) },
        StyleDef { bold: true, ..plain(SOURCE_HEADING, c.heading) },
        StyleDef { mono: true, ..plain(TABLE_BLANK, c.background) },
        StyleDef { mono: true, ..plain(CODE_MARKER, c.operator) },
        plain(ANNOTATION, c.background),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::spans::SpanKind;

    #[test]
    fn rendered_and_revealed_styles() {
        assert_eq!(style_for(SpanKind::Hide, false), HIDDEN);
        assert_eq!(style_for(SpanKind::Hide, true), MARKER);
        assert_eq!(style_for(SpanKind::Blank, false), BLANK);
        assert_eq!(style_for(SpanKind::Blank, true), MARKER);
        assert_eq!(style_for(SpanKind::TableBlank, false), TABLE_BLANK);
        assert_eq!(style_for(SpanKind::TableBlank, true), TABLE);
        assert_eq!(style_for(SpanKind::HeadingText(1), false), BLANK);
        assert_eq!(style_for(SpanKind::HeadingText(4), false), HEADING_SMALL);
        assert_eq!(style_for(SpanKind::HeadingText(1), true), SOURCE_HEADING);
        assert_eq!(style_for(SpanKind::HeadingMarker(2), false), HIDDEN);
        assert_eq!(style_for(SpanKind::HeadingMarker(2), true), MARKER);
        assert_eq!(style_for(SpanKind::Bold, true), BOLD);
        assert_eq!(style_for(SpanKind::CodeBlock, false), CODE_BLOCK);
    }

    #[test]
    fn blank_styles_paint_text_in_the_background_colour() {
        let colors = crate::languages::syntax_colors(crate::platform::theme::Theme::Light);
        let table = style_table(&colors);
        let get = |style| table.iter().find(|def| def.style == style).unwrap();
        assert_eq!(get(BLANK).foreground, get(BLANK).background);
        assert!(get(TABLE_BLANK).mono);
        assert_eq!(get(TABLE_BLANK).foreground, get(TABLE_BLANK).background);
        assert!(!get(HIDDEN).visible);
        assert!(get(CODE_BLOCK).eol_filled && get(CODE_BLOCK).mono);
        assert!(get(LINK).underline);
        assert_eq!(get(ANNOTATION).background, colors.background);
    }
}
