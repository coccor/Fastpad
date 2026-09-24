//! The single owner of FastPad's UI colors: title strip, tabs, caption buttons, status line, and the
//! editor's base/selection/caret-line colors. High contrast always uses system colors.
//!
//! Every themed palette is compiled-in static data, so resolving one is an array index.

use super::file_icons::IconColor;
use crate::catppuccin::{self, Flavor};
use crate::languages::rgb;
use crate::platform::theme::{SystemTheme, Theme};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, GetSysColor,
};

/// Every color is a Windows `COLORREF` (`0x00BBGGRR`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    pub strip_background: u32,
    pub editor_background: u32,
    pub editor_foreground: u32,
    pub muted_foreground: u32,
    pub hover_foreground: u32,
    pub hover_background: u32,
    pub pressed_background: u32,
    pub close_hover_background: u32,
    pub close_hover_foreground: u32,
    pub close_pressed_background: u32,
    pub selection_background: u32,
    pub inactive_selection_background: u32,
    /// Selected-text foreground. `None` leaves Scintilla's own syntax colors showing through, which
    /// is what the themed palettes want; high contrast must force the system pair to stay legible.
    pub selection_foreground: Option<u32>,
    pub caret_line_background: u32,
    /// Line numbers in the editor gutter, on `editor_background`.
    pub line_number_foreground: u32,
    /// Resting caption-button glyphs and status-line text on `strip_background`.
    pub strip_foreground: u32,
    /// Error text on `editor_background` or the panel, such as a regex error in the Search view.
    pub error_foreground: u32,
    /// Whether the frame and editor scrollbars should request the dark system styling.
    pub dark_frame: bool,
    /// The system's high-contrast colors: only system color pairs may be drawn, never a blend.
    pub high_contrast: bool,
}

const CLOSE_HOVER: u32 = rgb(0xC4, 0x2B, 0x1C);
const CLOSE_PRESSED: u32 = rgb(0x94, 0x1F, 0x15);
const WHITE: u32 = rgb(255, 255, 255);

const LIGHT: Palette = Palette {
    strip_background: rgb(243, 243, 243),
    editor_background: WHITE,
    editor_foreground: rgb(0, 0, 0),
    muted_foreground: rgb(96, 96, 96),
    hover_foreground: rgb(0, 0, 0),
    hover_background: rgb(224, 224, 224),
    pressed_background: rgb(204, 204, 204),
    close_hover_background: CLOSE_HOVER,
    close_hover_foreground: WHITE,
    close_pressed_background: CLOSE_PRESSED,
    selection_background: rgb(173, 214, 255),
    inactive_selection_background: rgb(229, 235, 241),
    selection_foreground: None,
    caret_line_background: rgb(245, 247, 250),
    line_number_foreground: rgb(110, 118, 129),
    strip_foreground: rgb(32, 32, 32),
    error_foreground: rgb(0xA1, 0x26, 0x0D),
    dark_frame: false,
    high_contrast: false,
};

const DARK: Palette = Palette {
    strip_background: rgb(37, 37, 38),
    editor_background: rgb(30, 30, 30),
    editor_foreground: rgb(212, 212, 212),
    muted_foreground: rgb(150, 150, 150),
    hover_foreground: rgb(240, 240, 240),
    hover_background: rgb(55, 55, 58),
    pressed_background: rgb(72, 72, 76),
    close_hover_background: CLOSE_HOVER,
    close_hover_foreground: WHITE,
    close_pressed_background: CLOSE_PRESSED,
    selection_background: rgb(38, 79, 120),
    inactive_selection_background: rgb(58, 61, 65),
    selection_foreground: None,
    caret_line_background: rgb(40, 40, 40),
    line_number_foreground: rgb(133, 133, 133),
    strip_foreground: rgb(212, 212, 212),
    error_foreground: rgb(0xF4, 0x87, 0x71),
    dark_frame: true,
    high_contrast: false,
};

/// Maps a Catppuccin flavor onto FastPad's UI roles per the Catppuccin style guide: `mantle`
/// chrome around a `base` editor, `overlay2` selection at ~25% opacity, `text` caret line at ~10%.
const fn catppuccin(flavor: &Flavor, dark: bool) -> Palette {
    Palette {
        strip_background: flavor.mantle,
        editor_background: flavor.base,
        editor_foreground: flavor.text,
        muted_foreground: flavor.subtext0,
        hover_foreground: flavor.text,
        hover_background: flavor.surface0,
        pressed_background: flavor.surface1,
        close_hover_background: flavor.red,
        close_hover_foreground: flavor.crust,
        close_pressed_background: flavor.maroon,
        selection_background: catppuccin::blend(flavor.overlay2, flavor.base, 64),
        inactive_selection_background: catppuccin::blend(flavor.overlay2, flavor.base, 38),
        selection_foreground: None,
        caret_line_background: catppuccin::blend(flavor.text, flavor.base, 26),
        line_number_foreground: flavor.overlay1,
        strip_foreground: flavor.text,
        error_foreground: flavor.red,
        dark_frame: dark,
        high_contrast: false,
    }
}

/// Indexed by `Theme as usize`; order must match `Theme::ALL`.
static PALETTES: [Palette; Theme::COUNT] = [
    LIGHT,
    DARK,
    catppuccin(&catppuccin::LATTE, false),
    catppuccin(&catppuccin::FRAPPE, true),
    catppuccin(&catppuccin::MACCHIATO, true),
    catppuccin(&catppuccin::MOCHA, true),
];

/// The Notebook view's file-type icon colours (notebook folders spec §5.2): six Catppuccin
/// roles, from the theme's own flavour, Latte's for Light and Mocha's for Dark. High contrast
/// uses the muted system colour for all of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIcons {
    pub blue: u32,
    pub yellow: u32,
    pub peach: u32,
    pub green: u32,
    pub maroon: u32,
    pub overlay2: u32,
}

const fn file_icons(flavor: &Flavor) -> FileIcons {
    FileIcons {
        blue: flavor.blue,
        yellow: flavor.yellow,
        peach: flavor.peach,
        green: flavor.green,
        maroon: flavor.maroon,
        overlay2: flavor.overlay2,
    }
}

/// Indexed by `Theme as usize`, like `PALETTES`.
static FILE_ICONS: [FileIcons; Theme::COUNT] = [
    file_icons(&catppuccin::LATTE),
    file_icons(&catppuccin::MOCHA),
    file_icons(&catppuccin::LATTE),
    file_icons(&catppuccin::FRAPPE),
    file_icons(&catppuccin::MACCHIATO),
    file_icons(&catppuccin::MOCHA),
];

impl FileIcons {
    /// The neutral first-paint palette's icons (Light's).
    pub const fn neutral() -> Self {
        file_icons(&catppuccin::LATTE)
    }

    pub fn for_theme(theme: Theme, high_contrast: bool) -> Self {
        if high_contrast {
            let muted = Palette::for_theme(theme, true).muted_foreground;
            Self {
                blue: muted,
                yellow: muted,
                peach: muted,
                green: muted,
                maroon: muted,
                overlay2: muted,
            }
        } else {
            FILE_ICONS[theme as usize]
        }
    }

    /// `None` means chrome has not been built yet: the neutral icons, with no theme queries.
    pub fn for_cached_theme(
        theme: Option<SystemTheme>,
        preference: crate::config::ThemePreference,
    ) -> Self {
        theme.map_or_else(Self::neutral, |theme| {
            Self::for_theme(theme.effective_theme(preference), theme.high_contrast)
        })
    }

    pub(crate) const fn color(&self, role: IconColor) -> u32 {
        match role {
            IconColor::Blue => self.blue,
            IconColor::Yellow => self.yellow,
            IconColor::Peach => self.peach,
            IconColor::Green => self.green,
            IconColor::Maroon => self.maroon,
            IconColor::Overlay2 => self.overlay2,
        }
    }
}

impl Palette {
    /// The compiled palette used before `WM_FASTPAD_BUILD_CHROME`, so first paint makes no theme
    /// queries.
    pub const fn neutral() -> Self {
        LIGHT
    }

    pub fn for_theme(theme: Theme, high_contrast: bool) -> Self {
        if high_contrast {
            Self::high_contrast()
        } else {
            PALETTES[theme as usize]
        }
    }

    /// `None` means chrome has not been built yet: the neutral palette, with no theme queries.
    pub fn for_cached_theme(
        theme: Option<SystemTheme>,
        preference: crate::config::ThemePreference,
    ) -> Self {
        theme.map_or_else(Self::neutral, |theme| {
            Self::for_theme(theme.effective_theme(preference), theme.high_contrast)
        })
    }

    pub const fn active_tab_background(&self) -> u32 {
        self.editor_background
    }

    /// The side panel's background: halfway between the strip and the editor, channel by channel.
    pub const fn panel_background(&self) -> u32 {
        ((self.strip_background >> 1) & 0x007f_7f7f) + ((self.editor_background >> 1) & 0x007f_7f7f)
    }

    fn high_contrast() -> Self {
        let color = |index| unsafe { GetSysColor(index) };
        let window = color(COLOR_WINDOW);
        let text = color(COLOR_WINDOWTEXT);
        let highlight = color(COLOR_HIGHLIGHT);
        let highlight_text = color(COLOR_HIGHLIGHTTEXT);
        Self {
            strip_background: color(COLOR_BTNFACE),
            editor_background: window,
            editor_foreground: text,
            muted_foreground: color(COLOR_BTNTEXT),
            hover_foreground: highlight_text,
            hover_background: highlight,
            pressed_background: highlight,
            close_hover_background: highlight,
            close_hover_foreground: highlight_text,
            close_pressed_background: highlight,
            selection_background: highlight,
            inactive_selection_background: highlight,
            selection_foreground: Some(highlight_text),
            caret_line_background: window,
            line_number_foreground: text,
            strip_foreground: color(COLOR_BTNTEXT),
            error_foreground: text,
            dark_frame: false,
            high_contrast: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Palette;
    use crate::catppuccin;
    use crate::languages::{rgb, syntax_colors};
    use crate::platform::theme::Theme;
    use windows_sys::Win32::Graphics::Gdi::{
        COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
    };

    #[test]
    fn every_theme_has_a_distinct_palette_and_neutral_is_light() {
        // Break caught: a strip that ignores the theme paints the same light chrome over a dark
        // editor, and a neutral first paint that is not the light palette flashes a third look.
        for (index, theme) in Theme::ALL.into_iter().enumerate() {
            let palette = Palette::for_theme(theme, false);
            for other in &Theme::ALL[index + 1..] {
                let other = Palette::for_theme(*other, false);
                assert_ne!(palette.strip_background, other.strip_background);
                assert_ne!(palette.editor_background, other.editor_background);
            }
            assert_eq!(palette.dark_frame, theme.is_dark());
            // Themed palettes leave selected text to the lexer colors; only high contrast forces it.
            assert_eq!(palette.selection_foreground, None);
            // Break caught: selection or caret line blending into the editor background.
            assert_ne!(palette.selection_background, palette.editor_background);
            assert_ne!(palette.caret_line_background, palette.editor_background);
            assert_ne!(palette.selection_background, palette.caret_line_background);
        }
        assert_eq!(Palette::neutral(), Palette::for_theme(Theme::Light, false));
    }

    #[test]
    fn catppuccin_palettes_use_the_flavor_swatches() {
        let mocha = Palette::for_theme(Theme::CatppuccinMocha, false);
        assert_eq!(mocha.editor_background, catppuccin::MOCHA.base);
        assert_eq!(mocha.strip_background, catppuccin::MOCHA.mantle);
        assert_eq!(mocha.editor_foreground, catppuccin::MOCHA.text);
        let latte = Palette::for_theme(Theme::CatppuccinLatte, false);
        assert_eq!(latte.editor_background, catppuccin::LATTE.base);
        assert!(!latte.dark_frame);
    }

    #[test]
    fn active_tab_background_is_the_editor_background() {
        for high_contrast in [false, true] {
            for theme in Theme::ALL {
                let palette = Palette::for_theme(theme, high_contrast);
                assert_eq!(palette.active_tab_background(), palette.editor_background);
            }
        }
    }

    #[test]
    fn editor_backgrounds_match_the_lexer_style_tables() {
        // Break caught: a palette editor background that drifts from the JSON/Markdown style tables
        // leaves highlighted text on a different background than the active tab.
        for theme in Theme::ALL {
            assert_eq!(
                Palette::for_theme(theme, false).editor_background,
                syntax_colors(theme).background
            );
        }
        assert_eq!(
            Palette::for_theme(Theme::Dark, false).editor_background,
            rgb(30, 30, 30)
        );
        assert_eq!(
            Palette::for_theme(Theme::Light, false).editor_background,
            rgb(255, 255, 255)
        );
    }

    #[test]
    fn high_contrast_uses_system_colors_regardless_of_theme() {
        let palette = Palette::for_theme(Theme::Dark, true);
        for theme in Theme::ALL {
            assert_eq!(palette, Palette::for_theme(theme, true));
        }
        unsafe {
            assert_eq!(palette.editor_background, GetSysColor(COLOR_WINDOW));
            assert_eq!(palette.editor_foreground, GetSysColor(COLOR_WINDOWTEXT));
            assert_eq!(palette.selection_background, GetSysColor(COLOR_HIGHLIGHT));
            assert_eq!(palette.hover_background, GetSysColor(COLOR_HIGHLIGHT));
            assert_eq!(palette.close_hover_background, GetSysColor(COLOR_HIGHLIGHT));
            assert_eq!(
                palette.close_hover_foreground,
                GetSysColor(COLOR_HIGHLIGHTTEXT)
            );
            // Break caught: selected text keeping its lexer color on the system highlight
            // background is unreadable in high contrast.
            assert_eq!(
                palette.selection_foreground,
                Some(GetSysColor(COLOR_HIGHLIGHTTEXT))
            );
        }
        assert!(!palette.dark_frame);
    }

    #[test]
    fn close_hover_is_red_with_a_white_glyph_in_the_system_themes() {
        for theme in [Theme::Light, Theme::Dark] {
            let palette = Palette::for_theme(theme, false);
            assert_eq!(palette.close_hover_background, rgb(0xC4, 0x2B, 0x1C));
            assert_eq!(palette.close_hover_foreground, rgb(255, 255, 255));
        }
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            assert_ne!(palette.muted_foreground, palette.hover_foreground);
            assert_ne!(
                palette.close_hover_background,
                palette.close_hover_foreground
            );
        }
    }

    #[test]
    fn the_error_color_stands_apart_from_the_text_and_the_background() {
        // Break caught: a regex error in the Search view that reads like the note count, or
        // vanishes into the background.
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            assert_ne!(
                palette.error_foreground, palette.editor_background,
                "{theme:?}"
            );
            assert_ne!(
                palette.error_foreground, palette.editor_foreground,
                "{theme:?}"
            );
            assert_ne!(
                palette.error_foreground, palette.muted_foreground,
                "{theme:?}"
            );
        }
        assert_eq!(
            Palette::for_theme(Theme::CatppuccinMocha, true).error_foreground,
            unsafe { GetSysColor(COLOR_WINDOWTEXT) },
            "high contrast keeps the system text color"
        );
    }

    /// WCAG relative luminance of a `COLORREF`.
    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let value = f64::from((color >> shift) & 0xFF) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(0) + 0.7152 * channel(8) + 0.0722 * channel(16)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn file_icon_colours_come_from_the_themes_flavour_and_high_contrast_mutes_them() {
        // Break caught: the Light theme drawing Mocha's pastel icons on white, a Catppuccin theme
        // using another flavour's swatches, or coloured icons in high contrast (spec §5.2).
        use super::FileIcons;
        let flavors = [
            (Theme::Light, catppuccin::LATTE),
            (Theme::Dark, catppuccin::MOCHA),
            (Theme::CatppuccinLatte, catppuccin::LATTE),
            (Theme::CatppuccinFrappe, catppuccin::FRAPPE),
            (Theme::CatppuccinMacchiato, catppuccin::MACCHIATO),
            (Theme::CatppuccinMocha, catppuccin::MOCHA),
        ];
        for (theme, flavor) in flavors {
            let icons = FileIcons::for_theme(theme, false);
            assert_eq!(
                (
                    icons.blue,
                    icons.yellow,
                    icons.peach,
                    icons.green,
                    icons.maroon,
                    icons.overlay2
                ),
                (
                    flavor.blue,
                    flavor.yellow,
                    flavor.peach,
                    flavor.green,
                    flavor.maroon,
                    flavor.overlay2
                ),
                "{theme:?}"
            );
            let muted = Palette::for_theme(theme, true).muted_foreground;
            let system = FileIcons::for_theme(theme, true);
            assert!(
                [
                    system.blue,
                    system.yellow,
                    system.peach,
                    system.green,
                    system.maroon,
                    system.overlay2
                ]
                .iter()
                .all(|&color| color == muted)
            );
        }
        assert_eq!(
            FileIcons::neutral(),
            FileIcons::for_theme(Theme::Light, false)
        );
    }

    #[test]
    fn file_icon_colours_stay_visible_on_selected_and_hovered_rows() {
        // Break caught: an icon colour that disappears into the selection or hover highlight,
        // where spec §5.2 keeps it. The weakest pair, Latte yellow on the Light theme's
        // selection, is about 1.7:1.
        use super::FileIcons;
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            let icons = FileIcons::for_theme(theme, false);
            for color in [
                icons.blue,
                icons.yellow,
                icons.peach,
                icons.green,
                icons.maroon,
                icons.overlay2,
            ] {
                for background in [
                    palette.selection_background,
                    palette.inactive_selection_background,
                    palette.hover_background,
                    palette.panel_background(),
                ] {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= 1.5,
                        "{theme:?}: {color:06x} on {background:06x} is {ratio:.2}:1"
                    );
                }
            }
        }
    }
}
