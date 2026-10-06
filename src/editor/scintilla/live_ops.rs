//! `Editor`'s operations for Live Markdown (live mode spec §7): container-lexer styling runs,
//! style attributes the language tables never set, annotation lines that reserve height, the
//! strikethrough indicator, and the geometry the decoration painter needs.

use super::*;
use crate::editor::scintilla_constants::{
    ANNOTATION_HIDDEN, ANNOTATION_STANDARD, INDIC_STRIKE, SC_EOL_CR, SC_EOL_LF, SCI_ADDSELECTION,
    SCI_ANNOTATIONCLEARALL, SCI_ANNOTATIONGETLINES, SCI_ANNOTATIONSETSTYLE, SCI_ANNOTATIONSETTEXT,
    SCI_ANNOTATIONSETVISIBLE, SCI_COLOURISE, SCI_GETCURRENTPOS, SCI_GETENDSTYLED, SCI_GETEOLMODE,
    SCI_GETSTYLEAT, SCI_INDICATORCLEARRANGE, SCI_INDICATORFILLRANGE, SCI_INDICSETFORE,
    SCI_INDICSETSTYLE, SCI_LINESONSCREEN, SCI_POINTXFROMPOSITION, SCI_POINTYFROMPOSITION,
    SCI_POSITIONFROMPOINT, SCI_SETINDICATORCURRENT, SCI_SETSELECTION, SCI_SETSTYLING,
    SCI_STARTSTYLING, SCI_STYLESETEOLFILLED, SCI_STYLESETUNDERLINE, SCI_STYLESETVISIBLE,
    SCI_TEXTHEIGHT, SCI_WRAPCOUNT,
};
use std::ops::Range;

impl Editor {
    fn live_send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// Replaces the selections; each range's start is the anchor and its end the caret.
    pub fn set_selections(&self, selections: &[Range<usize>]) -> Result<()> {
        let Some((first, rest)) = selections.split_first() else {
            return Ok(());
        };
        self.live_send(SCI_SETSELECTION, first.end, first.start as isize)?;
        for selection in rest {
            self.live_send(SCI_ADDSELECTION, selection.end, selection.start as isize)?;
        }
        Ok(())
    }

    /// Applies a Markdown helper's plan as one undo step (live mode spec §8).
    pub fn apply_plan(&self, plan: &crate::editor::markdown_edit::EditPlan) -> Result<()> {
        self.begin_undo_action();
        let result = plan
            .edits
            .iter()
            .rev()
            .try_for_each(|edit| {
                self.replace_target(edit.range.clone(), &edit.text)
                    .map(drop)
            })
            .and_then(|()| self.set_selections(&plan.selections));
        self.end_undo_action();
        result
    }

    /// The main selection's caret.
    pub fn caret(&self) -> Result<usize> {
        Ok(self.live_send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize)
    }

    /// The line ending Enter inserts.
    pub fn eol(&self) -> Result<&'static str> {
        Ok(match self.live_send(SCI_GETEOLMODE, 0, 0)? as u32 {
            SC_EOL_LF => "\n",
            SC_EOL_CR => "\r",
            _ => "\r\n",
        })
    }

    pub fn set_style_visible(&self, style: u32, visible: bool) -> Result<()> {
        self.live_send(SCI_STYLESETVISIBLE, style as usize, isize::from(visible))
            .map(drop)
    }

    pub fn set_style_eol_filled(&self, style: u32, filled: bool) -> Result<()> {
        self.live_send(SCI_STYLESETEOLFILLED, style as usize, isize::from(filled))
            .map(drop)
    }

    pub fn set_style_underline(&self, style: u32, underline: bool) -> Result<()> {
        self.live_send(
            SCI_STYLESETUNDERLINE,
            style as usize,
            isize::from(underline),
        )
        .map(drop)
    }

    pub fn end_styled(&self) -> Result<usize> {
        Ok(self.live_send(SCI_GETENDSTYLED, 0, 0)?.max(0) as usize)
    }

    /// Styles `runs` (byte length, style) back to back from `start`.
    pub fn apply_styling(&self, start: usize, runs: &[(usize, u8)]) -> Result<()> {
        self.live_send(SCI_STARTSTYLING, start, 0)?;
        for &(length, style) in runs {
            self.live_send(SCI_SETSTYLING, length, isize::from(style))?;
        }
        Ok(())
    }

    pub fn style_at(&self, position: usize) -> Result<u8> {
        Ok(self.live_send(SCI_GETSTYLEAT, position, 0)? as u8)
    }

    /// Reserves `count` blank display lines under `line` (0 removes them).
    pub fn set_annotation_lines(&self, line: usize, count: usize, style: u32) -> Result<()> {
        if count == 0 {
            return self.live_send(SCI_ANNOTATIONSETTEXT, line, 0).map(drop);
        }
        // Scintilla shows one annotation line per text line; a space keeps each line non-empty.
        let text = vec![" "; count].join("\n");
        let text = std::ffi::CString::new(text).expect("spaces and newlines only");
        self.live_send(SCI_ANNOTATIONSETTEXT, line, text.as_ptr() as isize)?;
        self.live_send(SCI_ANNOTATIONSETSTYLE, line, style as isize)
            .map(drop)
    }

    pub fn annotation_lines(&self, line: usize) -> Result<usize> {
        Ok(self.live_send(SCI_ANNOTATIONGETLINES, line, 0)?.max(0) as usize)
    }

    pub fn show_annotations(&self, visible: bool) -> Result<()> {
        let mode = if visible {
            ANNOTATION_STANDARD
        } else {
            ANNOTATION_HIDDEN
        };
        self.live_send(SCI_ANNOTATIONSETVISIBLE, mode as usize, 0)
            .map(drop)
    }

    pub fn clear_annotations(&self) -> Result<()> {
        self.live_send(SCI_ANNOTATIONCLEARALL, 0, 0).map(drop)
    }

    pub fn text_height(&self) -> Result<i32> {
        Ok(self.live_send(SCI_TEXTHEIGHT, 0, 0)? as i32)
    }

    pub fn wrap_count(&self, line: usize) -> Result<usize> {
        Ok(self.live_send(SCI_WRAPCOUNT, line, 0)?.max(1) as usize)
    }

    pub fn lines_on_screen(&self) -> Result<usize> {
        Ok(self.live_send(SCI_LINESONSCREEN, 0, 0)?.max(0) as usize)
    }

    pub fn point_of(&self, position: usize) -> Result<(i32, i32)> {
        let x = self.live_send(SCI_POINTXFROMPOSITION, 0, position as isize)? as i32;
        let y = self.live_send(SCI_POINTYFROMPOSITION, 0, position as isize)? as i32;
        Ok((x, y))
    }

    pub fn position_at(&self, x: i32, y: i32) -> Result<usize> {
        Ok(self
            .live_send(SCI_POSITIONFROMPOINT, x as usize, y as isize)?
            .max(0) as usize)
    }

    pub fn define_strike_indicator(&self, indicator: u32, colour: u32) -> Result<()> {
        self.live_send(SCI_INDICSETSTYLE, indicator as usize, INDIC_STRIKE as isize)?;
        self.live_send(SCI_INDICSETFORE, indicator as usize, colour as isize)
            .map(drop)
    }

    pub fn set_indicator(&self, indicator: u32, range: Range<usize>, on: bool) -> Result<()> {
        self.live_send(SCI_SETINDICATORCURRENT, indicator as usize, 0)?;
        let message = if on {
            SCI_INDICATORFILLRANGE
        } else {
            SCI_INDICATORCLEARRANGE
        };
        self.live_send(message, range.start, (range.end - range.start) as isize)
            .map(drop)
    }

    pub fn colourise(&self, range: Range<usize>) -> Result<()> {
        self.live_send(SCI_COLOURISE, range.start, range.end as isize)
            .map(drop)
    }
}
