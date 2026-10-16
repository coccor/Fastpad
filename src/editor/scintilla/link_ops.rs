//! `Editor`'s operations for links between notes (second brain phase 1 spec §7.3, §7.4): the
//! underline under links on screen, where a point is in the text, and the `[[` completion list.

use super::*;
use crate::editor::scintilla_constants::{
    INDIC_PLAIN, SC_ORDER_PERFORMSORT, SCI_AUTOCACTIVE, SCI_AUTOCSETCANCELATSTART,
    SCI_AUTOCSETIGNORECASE, SCI_AUTOCSETORDER, SCI_AUTOCSETSEPARATOR, SCI_AUTOCSHOW,
    SCI_INDICATORCLEARRANGE, SCI_INDICATORFILLRANGE, SCI_INDICSETFORE, SCI_INDICSETSTYLE,
    SCI_INDICSETUNDER, SCI_LINESONSCREEN, SCI_POSITIONFROMPOINT, SCI_SETINDICATORCURRENT,
};
use std::ops::Range;

/// The indicator links are drawn with. Lexers use 0 to 7; containers own 8 and up.
const LINK_INDICATOR: usize = 8;
/// Separates completion items, which may hold spaces.
const COMPLETION_SEPARATOR: u8 = b'\n';

impl Editor {
    fn link_send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// Sets the link underline's color (a COLORREF). The indicator draws a plain underline under
    /// the text, so a link reads like one without hiding the source.
    pub(crate) fn set_link_color(&self, color: u32) -> Result<()> {
        self.link_send(SCI_INDICSETSTYLE, LINK_INDICATOR, INDIC_PLAIN as isize)?;
        self.link_send(SCI_INDICSETFORE, LINK_INDICATOR, color as isize)?;
        self.link_send(SCI_INDICSETUNDER, LINK_INDICATOR, 1)?;
        Ok(())
    }

    /// The document lines on screen, first to last (exclusive), as far as the document goes.
    pub(crate) fn visible_lines(&self) -> Result<Range<usize>> {
        let first = self.doc_line_from_visible(self.first_visible_line()?)?;
        let count = self.link_send(SCI_LINESONSCREEN, 0, 0)?.max(0) as usize;
        let last = self.line_count()?.min(first + count + 1);
        Ok(first..last.max(first))
    }

    /// Replaces the link underlines within `area` (byte range) with `links`.
    pub(crate) fn mark_links(&self, area: Range<usize>, links: &[Range<usize>]) -> Result<()> {
        self.link_send(SCI_SETINDICATORCURRENT, LINK_INDICATOR, 0)?;
        self.link_send(
            SCI_INDICATORCLEARRANGE,
            area.start,
            (area.end - area.start) as isize,
        )?;
        for link in links {
            self.link_send(
                SCI_INDICATORFILLRANGE,
                link.start,
                (link.end - link.start) as isize,
            )?;
        }
        Ok(())
    }

    /// The text position at client point `x`, `y`, or `None` off the text.
    pub(crate) fn position_at(&self, x: i32, y: i32) -> Option<usize> {
        let position = self
            .link_send(SCI_POSITIONFROMPOINT, x as usize, y as isize)
            .ok()?;
        (position >= 0).then_some(position as usize)
    }

    /// Opens the completion list with `items`, as if `typed` bytes before the caret were already
    /// part of the choice. Scintilla filters it as the user types, ignoring case, and replaces the
    /// typed text with the choice.
    pub(crate) fn show_completion(&self, typed: usize, items: &[String]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let separator = char::from(COMPLETION_SEPARATOR);
        let list: String = items
            .iter()
            .map(|item| item.replace(separator, " "))
            .collect::<Vec<_>>()
            .join(&separator.to_string());
        let list = std::ffi::CString::new(list)
            .map_err(|_| FastPadError::Invariant("a completion item holds a NUL"))?;
        self.link_send(SCI_AUTOCSETSEPARATOR, usize::from(COMPLETION_SEPARATOR), 0)?;
        self.link_send(SCI_AUTOCSETIGNORECASE, 1, 0)?;
        self.link_send(SCI_AUTOCSETORDER, SC_ORDER_PERFORMSORT as usize, 0)?;
        // Backspacing over the `[[` closes the list.
        self.link_send(SCI_AUTOCSETCANCELATSTART, 1, 0)?;
        self.link_send(SCI_AUTOCSHOW, typed, list.as_ptr() as isize)?;
        Ok(())
    }

    /// Whether the completion list is open.
    pub(crate) fn completion_active(&self) -> bool {
        self.link_send(SCI_AUTOCACTIVE, 0, 0)
            .is_ok_and(|active| active != 0)
    }
}
