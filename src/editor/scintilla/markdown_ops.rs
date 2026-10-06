//! `Editor`'s operations for the Markdown writing helpers (2026-10-06 Markdown design spec §8):
//! several selections at once, a helper's edit plan as one undo step, the caret and the line
//! ending Enter inserts.

use super::*;
use crate::editor::scintilla_constants::{
    SC_EOL_CR, SC_EOL_LF, SCI_ADDSELECTION, SCI_GETCURRENTPOS, SCI_GETEOLMODE, SCI_SETSELECTION,
};
use std::ops::Range;

impl Editor {
    fn markdown_send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// Replaces the selections; each range's start is the anchor and its end the caret.
    pub fn set_selections(&self, selections: &[Range<usize>]) -> Result<()> {
        let Some((first, rest)) = selections.split_first() else {
            return Ok(());
        };
        self.markdown_send(SCI_SETSELECTION, first.end, first.start as isize)?;
        for selection in rest {
            self.markdown_send(SCI_ADDSELECTION, selection.end, selection.start as isize)?;
        }
        Ok(())
    }

    /// Applies a Markdown helper's plan as one undo step (spec §8).
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
        Ok(self.markdown_send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize)
    }

    /// The line ending Enter inserts.
    pub fn eol(&self) -> Result<&'static str> {
        Ok(match self.markdown_send(SCI_GETEOLMODE, 0, 0)? as u32 {
            SC_EOL_LF => "\n",
            SC_EOL_CR => "\r",
            _ => "\r\n",
        })
    }
}
