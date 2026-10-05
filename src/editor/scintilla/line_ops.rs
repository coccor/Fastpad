//! `Editor`'s editing-shortcut operations (editing shortcuts spec): line editing, comments and
//! multiple carets, plus the one-time setup that turns VS Code's editing model on.

use super::*;
use crate::editor::scintilla_constants::{
    SC_EOL_CR, SC_EOL_LF, SC_MULTIPASTE_EACH, SCI_ADDSELECTION, SCI_ASSIGNCMDKEY,
    SCI_CHARLEFTRECTEXTEND, SCI_CHARRIGHTRECTEXTEND, SCI_CLEARCMDKEY, SCI_GETEOLMODE,
    SCI_GETLINEENDPOSITION, SCI_GETSELECTIONNCARET, SCI_GETSELECTIONNEND, SCI_GETSELECTIONNSTART,
    SCI_GETSELECTIONS, SCI_LINEDOWNRECTEXTEND, SCI_LINEUPRECTEXTEND, SCI_POSITIONFROMLINE,
    SCI_SETADDITIONALSELECTIONTYPING, SCI_SETMULTIPASTE, SCI_SETMULTIPLESELECTION, SCK_DOWN,
    SCK_LEFT, SCK_RIGHT, SCK_UP, SCMOD_ALT, SCMOD_CTRL, SCMOD_SHIFT,
};
use std::ops::RangeInclusive;

/// A Scintilla key definition: the key in the low word, `SCMOD_*` modifiers in the high word.
const fn key_definition(key: u32, modifiers: u32) -> usize {
    (key | (modifiers << 16)) as usize
}

const SHIFT_ALT: u32 = SCMOD_SHIFT | SCMOD_ALT;
const CTRL_SHIFT_ALT: u32 = SCMOD_CTRL | SCMOD_SHIFT | SCMOD_ALT;

/// Scintilla's defaults on keys FastPad's commands now own (spec §5).
const CLEARED_KEYS: [usize; 11] = [
    key_definition(b'D' as u32, SCMOD_CTRL),
    key_definition(b'L' as u32, SCMOD_CTRL),
    key_definition(b'L' as u32, SCMOD_CTRL | SCMOD_SHIFT),
    key_definition(b'T' as u32, SCMOD_CTRL),
    key_definition(b'T' as u32, SCMOD_CTRL | SCMOD_SHIFT),
    key_definition(b'[' as u32, SCMOD_CTRL),
    key_definition(b']' as u32, SCMOD_CTRL),
    key_definition(SCK_UP, SHIFT_ALT),
    key_definition(SCK_DOWN, SHIFT_ALT),
    key_definition(SCK_LEFT, SHIFT_ALT),
    key_definition(SCK_RIGHT, SHIFT_ALT),
];

/// Column selection by keyboard, on VS Code's keys (spec §5).
const RECTANGLE_KEYS: [(usize, u32); 4] = [
    (key_definition(SCK_UP, CTRL_SHIFT_ALT), SCI_LINEUPRECTEXTEND),
    (
        key_definition(SCK_DOWN, CTRL_SHIFT_ALT),
        SCI_LINEDOWNRECTEXTEND,
    ),
    (
        key_definition(SCK_LEFT, CTRL_SHIFT_ALT),
        SCI_CHARLEFTRECTEXTEND,
    ),
    (
        key_definition(SCK_RIGHT, CTRL_SHIFT_ALT),
        SCI_CHARRIGHTRECTEXTEND,
    ),
];

/// Sorted, merged runs of lines from `spans` (each `first..=last`).
fn merge_runs(mut spans: Vec<RangeInclusive<usize>>) -> Vec<RangeInclusive<usize>> {
    spans.sort_by_key(|span| *span.start());
    let mut runs: Vec<RangeInclusive<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match runs.last_mut() {
            Some(last) if *span.start() <= last.end() + 1 => {
                *last = *last.start()..=(*last.end()).max(*span.end());
            }
            _ => runs.push(span),
        }
    }
    runs
}

impl Editor {
    fn send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// Several carets that all type and paste, and Scintilla's own keys moved off the strokes
    /// FastPad's commands own (spec §2, §5).
    pub(crate) fn configure_editing(&self) -> Result<()> {
        self.send(SCI_SETMULTIPLESELECTION, 1, 0)?;
        self.send(SCI_SETADDITIONALSELECTIONTYPING, 1, 0)?;
        self.send(SCI_SETMULTIPASTE, SC_MULTIPASTE_EACH as usize, 0)?;
        for key in CLEARED_KEYS {
            self.send(SCI_CLEARCMDKEY, key, 0)?;
        }
        for (key, command) in RECTANGLE_KEYS {
            self.send(SCI_ASSIGNCMDKEY, key, command as isize)?;
        }
        Ok(())
    }

    /// Every selection, the main one included, in Scintilla's order.
    pub(crate) fn selections(&self) -> Result<Vec<Range<usize>>> {
        let count = self.send(SCI_GETSELECTIONS, 0, 0)?.max(1) as usize;
        (0..count)
            .map(|n| {
                let start = self.send(SCI_GETSELECTIONNSTART, n, 0)?.max(0) as usize;
                let end = self.send(SCI_GETSELECTIONNEND, n, 0)?.max(0) as usize;
                Ok(start..end)
            })
            .collect()
    }

    /// Every selection's caret, in Scintilla's order.
    pub(crate) fn carets(&self) -> Result<Vec<usize>> {
        let count = self.send(SCI_GETSELECTIONS, 0, 0)?.max(1) as usize;
        (0..count)
            .map(|n| Ok(self.send(SCI_GETSELECTIONNCARET, n, 0)?.max(0) as usize))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn add_selection_for_test(&self, caret: usize) {
        self.send(SCI_ADDSELECTION, caret, caret as isize).unwrap();
    }

    /// Where `line` starts; the document's length past the last line.
    fn line_start(&self, line: usize) -> Result<usize> {
        if line >= self.line_count()? {
            return self.length();
        }
        Ok(self.send(SCI_POSITIONFROMLINE, line, 0)?.max(0) as usize)
    }

    /// Where `line`'s text ends, before its line end.
    fn line_end(&self, line: usize) -> Result<usize> {
        Ok(self.send(SCI_GETLINEENDPOSITION, line, 0)?.max(0) as usize)
    }

    /// The lines `range` touches. A non-empty range ending at a line's start leaves that line
    /// out, as VS Code does.
    fn touched_lines(&self, range: Range<usize>) -> Result<RangeInclusive<usize>> {
        let first = self.line_from_position(range.start)?;
        let mut last = self.line_from_position(range.end)?;
        if last > first && range.end == self.line_start(last)? {
            last -= 1;
        }
        Ok(first..=last)
    }

    /// The lines every selection touches, as sorted, merged runs.
    fn touched_runs(&self) -> Result<Vec<RangeInclusive<usize>>> {
        let spans = self
            .selections()?
            .into_iter()
            .map(|range| self.touched_lines(range))
            .collect::<Result<Vec<_>>>()?;
        Ok(merge_runs(spans))
    }

    /// The line end `line` has, else the one before it, else the document's end-of-line mode:
    /// new lines match the document rather than the platform.
    fn line_ending(&self, line: usize) -> Result<String> {
        let count = self.line_count()?;
        for candidate in [line, line.saturating_sub(1)] {
            if candidate + 1 < count {
                let range = self.line_end(candidate)?..self.line_start(candidate + 1)?;
                return Ok(String::from_utf8_lossy(self.range_bytes(range)?).into_owned());
            }
        }
        Ok(match self.send(SCI_GETEOLMODE, 0, 0)? as u32 {
            SC_EOL_LF => "\n",
            SC_EOL_CR => "\r",
            _ => "\r\n",
        }
        .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::merge_runs;

    #[test]
    fn runs_merge_when_they_overlap_or_touch() {
        assert_eq!(
            merge_runs(vec![5..=6, 0..=1, 2..=2, 9..=9]),
            vec![0..=2, 5..=6, 9..=9]
        );
        assert_eq!(merge_runs(vec![3..=8, 4..=5]), vec![3..=8]);
    }
}
