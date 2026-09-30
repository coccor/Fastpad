//! The text styles FastPad's chrome uses: a pixel height at 96 DPI, a weight and an italic flag
//! each, and the one function that turns a style into a GDI font. Icon fonts and the editor and
//! preview fonts are not part of the ramp.

use super::metrics::scale;
use crate::window::titlebar::create_ui_font;
use windows_sys::Win32::Graphics::Gdi::{FW_BOLD, FW_NORMAL, FW_SEMIBOLD, HFONT};

/// The chrome's text face. Segoe UI Variable with a fallback replaces it in a later step.
pub(crate) const UI_FACE: &str = "Segoe UI";

/// A named text style. Fonts for icons, the editor and the Markdown preview are not styles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextStyle {
    /// Title strip, tabs, sidebar row text and the Settings and About dialogs' text.
    Body,
    /// The preview tab's label and notices inside sidebar lists.
    BodyItalic,
    /// The match in a Search result's snippet.
    BodyBold,
    /// Sidebar header titles.
    PanelHeader,
    /// A dialog's section headings.
    Heading,
    /// A dialog's title.
    Title,
}

/// A style's pixel height at 96 DPI, GDI weight and italic flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Spec {
    pub(crate) px: i32,
    pub(crate) weight: i32,
    pub(crate) italic: bool,
}

impl TextStyle {
    pub(crate) const fn spec(self) -> Spec {
        let (px, weight, italic) = match self {
            Self::Body => (13, FW_NORMAL, false),
            Self::BodyItalic => (13, FW_NORMAL, true),
            Self::BodyBold => (13, FW_BOLD, false),
            Self::PanelHeader => (13, FW_SEMIBOLD, false),
            Self::Heading => (14, FW_SEMIBOLD, false),
            Self::Title => (18, FW_SEMIBOLD, false),
        };
        Spec {
            px,
            weight: weight as i32,
            italic,
        }
    }
}

/// A GDI font for `style` at `dpi`. The caller owns it and deletes it.
pub(crate) fn create(style: TextStyle, dpi: u32) -> HFONT {
    let spec = style.spec();
    create_ui_font(scale(spec.px, dpi), UI_FACE, spec.weight, spec.italic)
}

#[cfg(test)]
mod tests {
    use super::{TextStyle, create};
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, FW_BOLD, FW_NORMAL, FW_SEMIBOLD, GetObjectW, LOGFONTW,
    };

    #[test]
    fn every_style_has_the_size_and_weight_the_spec_gives_it() {
        // Break caught: a size or weight drifting from the spec. The four body-text styles are the
        // 13px trial; reverting it means setting Body, BodyItalic and BodyBold back to 12 and
        // PanelHeader back to 11 (semibold) in `spec` above.
        let spec = |style: TextStyle| {
            let s = style.spec();
            (s.px, s.weight, s.italic)
        };
        assert_eq!(spec(TextStyle::Body), (13, FW_NORMAL as i32, false)); // strip, tabs, sidebar, dialogs
        assert_eq!(spec(TextStyle::BodyItalic), (13, FW_NORMAL as i32, true)); // preview tab, notices
        assert_eq!(spec(TextStyle::BodyBold), (13, FW_BOLD as i32, false)); // search match
        assert_eq!(
            spec(TextStyle::PanelHeader),
            (13, FW_SEMIBOLD as i32, false)
        );
        assert_eq!(spec(TextStyle::Heading), (14, FW_SEMIBOLD as i32, false));
        assert_eq!(spec(TextStyle::Title), (18, FW_SEMIBOLD as i32, false));
    }

    #[test]
    fn fonts_are_created_at_unusual_dpis_and_scale_with_them() {
        // Break caught: a DPI of 0 (bad handle) or a scaled monitor producing a null font, or a
        // font whose height ignores the DPI.
        for (dpi, expected_height) in [(0_u32, 13), (96, 13), (144, 20), (192, 26)] {
            let font = create(TextStyle::Body, dpi);
            assert!(!font.is_null(), "dpi {dpi}");
            let mut log: LOGFONTW = unsafe { std::mem::zeroed() };
            let written = unsafe {
                GetObjectW(
                    font,
                    std::mem::size_of::<LOGFONTW>() as i32,
                    (&mut log as *mut LOGFONTW).cast(),
                )
            };
            assert!(written > 0, "dpi {dpi}");
            assert_eq!(log.lfHeight, -expected_height, "dpi {dpi}");
            unsafe { DeleteObject(font) };
        }
    }
}
