//! Markdown preview: a block model, incremental reparsing, and a lazily created Direct2D view.
//! Nothing here runs until the user opens a preview.

pub mod accessible;
pub mod colors;
pub mod dwrite;
pub mod heights;
pub mod html;
mod html_entities;
pub mod images;
pub mod incremental;
pub mod inline_object;
pub mod layout;
pub mod links;
pub mod model;
pub mod outline;
pub mod render;
pub mod svg;
pub mod view;

/// How a Markdown or SVG tab is shown (2026-10-22 view modes spec §1). Each tab has its own.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ViewMode {
    /// The Scintilla editor.
    #[default]
    Source,
    /// The Live Markdown view.
    Live,
    /// The editor with the preview beside it.
    Split,
    /// The preview alone, centred.
    Preview,
}

impl ViewMode {
    /// The cycle order, and the floating control's order.
    pub const ALL: [Self; 4] = [Self::Source, Self::Live, Self::Split, Self::Preview];
    /// The modes an SVG tab can show.
    pub const SVG: [Self; 3] = [Self::Source, Self::Split, Self::Preview];

    /// The `markdown_view=` value that parses back to this mode.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Live => "live",
            Self::Split => "split",
            Self::Preview => "preview",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| value.eq_ignore_ascii_case(mode.token()))
    }

    /// The name the floating control, its tooltip and screen readers give the mode.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Live => "Live",
            Self::Split => "Split",
            Self::Preview => "Preview",
        }
    }

    pub const fn shows_preview(self) -> bool {
        matches!(self, Self::Split | Self::Preview)
    }

    /// Whether the editor (or Live view) is hidden behind the preview.
    pub const fn hides_text(self) -> bool {
        matches!(self, Self::Preview)
    }

    /// The mode after `self` in the cycle, skipping modes `available` rejects. `self` itself when
    /// nothing else is available.
    pub fn next(self, available: impl Fn(Self) -> bool) -> Self {
        let start = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        (1..=Self::ALL.len())
            .map(|step| Self::ALL[(start + step) % Self::ALL.len()])
            .find(|mode| available(*mode))
            .unwrap_or(self)
    }
}

/// Documents larger than this reparse fully on a worker thread.
pub const WORKER_PARSE_THRESHOLD: usize = 1_048_576;
/// Documents larger than this pause live updates behind a "Refresh preview" bar.
pub const LIVE_UPDATE_LIMIT: usize = 10_485_760;
/// Idle time after the last edit before the preview updates.
pub const PREVIEW_UPDATE_DELAY_MS: u32 = 120;

#[cfg(test)]
mod tests {
    use super::ViewMode;

    #[test]
    fn the_cycle_visits_every_mode_and_skips_the_unavailable_ones() {
        let all = |_| true;
        assert_eq!(ViewMode::Source.next(all), ViewMode::Live);
        assert_eq!(ViewMode::Live.next(all), ViewMode::Split);
        assert_eq!(ViewMode::Split.next(all), ViewMode::Preview);
        assert_eq!(ViewMode::Preview.next(all), ViewMode::Source);
        // Break caught: an SVG tab cycling into Live and getting a "Markdown only" notice.
        let no_live = |mode| mode != ViewMode::Live;
        assert_eq!(ViewMode::Source.next(no_live), ViewMode::Split);
        assert_eq!(ViewMode::Source.next(|_| false), ViewMode::Source);
    }

    #[test]
    fn every_mode_writes_the_token_that_parses_back_to_it() {
        for mode in ViewMode::ALL {
            assert_eq!(ViewMode::parse(mode.token()), Some(mode));
        }
        assert_eq!(ViewMode::parse("LIVE"), Some(ViewMode::Live));
        assert_eq!(ViewMode::parse("full"), None);
    }
}
