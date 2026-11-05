//! `Editor`'s find highlights: every match of the find bar's query is shaded, and the current
//! one gets a stronger, outlined box on top, so it stands out from the rest.

use super::*;
use crate::editor::scintilla_constants::{
    INDIC_ROUNDBOX, INDIC_STRAIGHTBOX, SCI_INDICATORCLEARRANGE, SCI_INDICATORFILLRANGE,
    SCI_INDICSETALPHA, SCI_INDICSETFORE, SCI_INDICSETOUTLINEALPHA, SCI_INDICSETSTYLE,
    SCI_INDICSETUNDER, SCI_SETINDICATORCURRENT,
};
use std::ops::Range;

/// Every match. Containers own indicators 8 and up; links use 8.
const FIND_MATCH_INDICATOR: usize = 9;
/// The match the selection is, drawn over `FIND_MATCH_INDICATOR`.
const FIND_CURRENT_INDICATOR: usize = 10;

/// Fill opacity (0 to 255) of the other matches: a tint that leaves the text easy to read.
const MATCH_FILL_ALPHA: isize = 60;
const MATCH_OUTLINE_ALPHA: isize = 0;
/// The current match is filled more and outlined fully.
const CURRENT_FILL_ALPHA: isize = 130;
const CURRENT_OUTLINE_ALPHA: isize = 255;

impl Editor {
    fn find_send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// Sets the find highlights' color (a COLORREF). Both boxes are drawn under the text, so the
    /// text stays legible, and over the selection background, so the current match is visible
    /// while it is selected.
    pub(crate) fn set_find_highlight_color(&self, color: u32) -> Result<()> {
        for (indicator, style, fill, outline) in [
            (
                FIND_MATCH_INDICATOR,
                INDIC_STRAIGHTBOX,
                MATCH_FILL_ALPHA,
                MATCH_OUTLINE_ALPHA,
            ),
            (
                FIND_CURRENT_INDICATOR,
                INDIC_ROUNDBOX,
                CURRENT_FILL_ALPHA,
                CURRENT_OUTLINE_ALPHA,
            ),
        ] {
            self.find_send(SCI_INDICSETSTYLE, indicator, style as isize)?;
            self.find_send(SCI_INDICSETFORE, indicator, color as isize)?;
            self.find_send(SCI_INDICSETALPHA, indicator, fill)?;
            self.find_send(SCI_INDICSETOUTLINEALPHA, indicator, outline)?;
            self.find_send(SCI_INDICSETUNDER, indicator, 1)?;
        }
        Ok(())
    }

    /// Replaces the document's find highlights with `matches` (byte ranges), and marks `current`
    /// as the current match.
    pub(crate) fn mark_find_matches(
        &self,
        matches: &[Range<usize>],
        current: Option<Range<usize>>,
    ) -> Result<()> {
        self.fill_indicator(FIND_MATCH_INDICATOR, matches)?;
        self.mark_find_current(current)
    }

    /// Moves the current-match box to `current`, or removes it. The other matches' shading stays.
    pub(crate) fn mark_find_current(&self, current: Option<Range<usize>>) -> Result<()> {
        self.fill_indicator(FIND_CURRENT_INDICATOR, current.as_slice())
    }

    /// Removes every find highlight from the document.
    pub(crate) fn clear_find_matches(&self) -> Result<()> {
        self.mark_find_matches(&[], None)
    }

    /// Clears `indicator` over the whole document and fills it over `ranges`. Empty ranges are
    /// skipped.
    fn fill_indicator(&self, indicator: usize, ranges: &[Range<usize>]) -> Result<()> {
        let length = self.length()?;
        self.find_send(SCI_SETINDICATORCURRENT, indicator, 0)?;
        self.find_send(SCI_INDICATORCLEARRANGE, 0, length as isize)?;
        for range in ranges {
            let end = range.end.min(length);
            if end > range.start {
                self.find_send(
                    SCI_INDICATORFILLRANGE,
                    range.start,
                    (end - range.start) as isize,
                )?;
            }
        }
        Ok(())
    }
}
