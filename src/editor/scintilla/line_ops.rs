//! `Editor`'s editing-shortcut operations (editing shortcuts spec): line editing, comments and
//! multiple carets, plus the one-time setup that turns VS Code's editing model on.

use super::*;
use crate::editor::comment::{self, CommentSyntax};
use crate::editor::scintilla_constants::{
    SC_EOL_CR, SC_EOL_LF, SC_MULTIPASTE_EACH, SCI_ADDSELECTION, SCI_ASSIGNCMDKEY,
    SCI_CHARLEFTRECTEXTEND, SCI_CHARRIGHTRECTEXTEND, SCI_CLEARCMDKEY, SCI_GETEOLMODE,
    SCI_GETLINEENDPOSITION, SCI_GETSELECTIONNCARET, SCI_GETSELECTIONNEND, SCI_GETSELECTIONNSTART,
    SCI_GETSELECTIONS, SCI_LINEDOWNRECTEXTEND, SCI_LINEUPRECTEXTEND, SCI_POSITIONFROMLINE,
    SCI_SETADDITIONALSELECTIONTYPING, SCI_SETMULTIPASTE, SCI_SETMULTIPLESELECTION, SCK_DOWN,
    SCK_LEFT, SCK_RIGHT, SCK_UP, SCMOD_ALT, SCMOD_CTRL, SCMOD_SHIFT,
};
use crate::editor::scintilla_constants::{
    SCFIND_MATCHCASE, SCFIND_WHOLEWORD, SCI_MULTIPLESELECTADDEACH, SCI_MULTIPLESELECTADDNEXT,
    SCI_SETSEARCHFLAGS, SCI_TARGETWHOLEDOCUMENT,
};
use crate::editor::scintilla_constants::{
    SCI_FINDCOLUMN, SCI_GETANCHOR, SCI_GETCOLUMN, SCI_GETCURRENTPOS, SCI_GETINDENT,
    SCI_GETLINEINDENTATION, SCI_GETLINEINDENTPOSITION, SCI_GETTABWIDTH, SCI_MOVESELECTEDLINESDOWN,
    SCI_MOVESELECTEDLINESUP, SCI_SETLINEINDENTATION, SCI_SETSEL,
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

    /// The touched lines of the main selection swap with the line above or below (spec §3).
    pub fn move_lines(&self, up: bool) -> Result<()> {
        let message = if up {
            SCI_MOVESELECTEDLINESUP
        } else {
            SCI_MOVESELECTEDLINESDOWN
        };
        self.begin_undo_action();
        let result = self.send(message, 0, 0);
        self.end_undo_action();
        result.map(drop)
    }

    /// Duplicates the main selection's touched lines; the selection ends on the lower copy when
    /// copying down, the upper one when copying up (spec §3).
    pub fn copy_lines(&self, down: bool) -> Result<()> {
        let anchor = self.send(SCI_GETANCHOR, 0, 0)?.max(0) as usize;
        let caret = self.send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        let lines = self.touched_lines(anchor.min(caret)..anchor.max(caret))?;
        let start = self.line_start(*lines.start())?;
        let end = self.line_end(*lines.end())?;
        let text = String::from_utf8_lossy(self.range_bytes(start..end)?).into_owned();
        let eol = self.line_ending(*lines.end())?;
        let (at, inserted) = if down {
            (end, format!("{eol}{text}"))
        } else {
            (start, format!("{text}{eol}"))
        };
        self.begin_undo_action();
        let result = self.replace_target(at..at, &inserted);
        self.end_undo_action();
        result?;
        let shift = if down { inserted.len() } else { 0 };
        self.send(SCI_SETSEL, anchor + shift, (caret + shift) as isize)
            .map(drop)
    }

    /// Deletes every line any selection touches, line ends included; one caret stays, on the
    /// line that took the main caret's line's place, in the same column (spec §3).
    pub fn delete_lines(&self) -> Result<()> {
        let caret = self.send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        let caret_line = self.line_from_position(caret)?;
        let column = self.send(SCI_GETCOLUMN, caret, 0)?;
        let runs = self.touched_runs()?;
        self.begin_undo_action();
        let result = runs.iter().rev().try_for_each(|run| {
            let range = if run.end() + 1 < self.line_count()? {
                self.line_start(*run.start())?..self.line_start(run.end() + 1)?
            } else if *run.start() > 0 {
                self.line_end(run.start() - 1)?..self.length()?
            } else {
                0..self.length()?
            };
            self.replace_target(range, "").map(drop)
        });
        self.end_undo_action();
        result?;
        let removed_above: usize = runs
            .iter()
            .filter(|run| *run.end() < caret_line)
            .map(|run| run.end() - run.start() + 1)
            .sum();
        let base = runs
            .iter()
            .find(|run| run.contains(&caret_line))
            .map_or(caret_line, |run| *run.start());
        let line = (base - removed_above).min(self.line_count()?.saturating_sub(1));
        let position = self.send(SCI_FINDCOLUMN, line, column)?.max(0) as usize;
        self.send(SCI_SETSEL, position, position as isize).map(drop)
    }

    /// A new line below or above the main caret's line, with that line's indentation; the caret
    /// moves to it (spec §3).
    pub fn insert_line(&self, below: bool) -> Result<()> {
        let caret = self.send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        let line = self.line_from_position(caret)?;
        let indentation = self.send(SCI_GETLINEINDENTATION, line, 0)?;
        let eol = self.line_ending(line)?;
        let (at, new_line) = if below {
            (self.line_end(line)?, line + 1)
        } else {
            (self.line_start(line)?, line)
        };
        self.begin_undo_action();
        let result = self
            .replace_target(at..at, &eol)
            .and_then(|_| self.send(SCI_SETLINEINDENTATION, new_line, indentation));
        self.end_undo_action();
        result?;
        let position = self.send(SCI_GETLINEINDENTPOSITION, new_line, 0)?.max(0) as usize;
        self.send(SCI_SETSEL, position, position as isize).map(drop)
    }

    /// Indents (or outdents) every touched line to the next (or previous) indent stop, whatever
    /// the selection. Indenting skips empty lines (spec §3).
    pub fn indent_lines(&self, outdent: bool) -> Result<()> {
        let width = match self.send(SCI_GETINDENT, 0, 0)? {
            0 => self.send(SCI_GETTABWIDTH, 0, 0)?,
            width => width,
        }
        .max(1);
        let runs = self.touched_runs()?;
        self.begin_undo_action();
        let result = runs.iter().flat_map(Clone::clone).try_for_each(|line| {
            let current = self.send(SCI_GETLINEINDENTATION, line, 0)?;
            let next = if outdent {
                if current == 0 {
                    return Ok(());
                }
                (current - 1) / width * width
            } else {
                if self.line_end(line)? == self.line_start(line)? {
                    return Ok(());
                }
                (current / width + 1) * width
            };
            self.send(SCI_SETLINEINDENTATION, line, next).map(drop)
        });
        self.end_undo_action();
        result
    }

    /// Selects the main selection's lines whole, line end included; again, one more line
    /// (spec §3).
    pub fn expand_line_selection(&self) -> Result<()> {
        let selection = self.selection()?;
        let start = self.line_start(self.line_from_position(selection.start)?)?;
        let end = self.line_start(self.line_from_position(selection.end)? + 1)?;
        self.send(SCI_SETSEL, start, end as isize).map(drop)
    }

    /// Toggles line comments on every touched line, one run at a time; the selections follow
    /// their text (spec §4.2).
    pub fn toggle_line_comment(&self, syntax: CommentSyntax) -> Result<()> {
        let mut edits = Vec::new();
        for run in self.touched_runs()? {
            let texts = run
                .clone()
                .map(|line| self.line_text(line))
                .collect::<Result<Vec<_>>>()?;
            let lines: Vec<&str> = texts.iter().map(String::as_str).collect();
            for edit in comment::toggle_line(&lines, syntax) {
                let at = self.line_start(run.start() + edit.line)? + edit.column;
                edits.push((at..at + edit.remove, edit.insert));
            }
        }
        self.replace_ranges_with(&edits).map(drop)
    }

    /// Wraps or unwraps the main selection in the block pair; the result is selected, or the
    /// caret goes between the markers of an empty pair (spec §4.3).
    pub fn toggle_block_comment(&self, syntax: CommentSyntax) -> Result<()> {
        let selection = self.selection()?;
        let selected = String::from_utf8_lossy(self.range_bytes(selection.clone())?).into_owned();
        let Some(toggle) = comment::toggle_block(&selected, syntax) else {
            return Ok(());
        };
        self.begin_undo_action();
        let result = self.replace_target(selection.clone(), &toggle.replacement);
        self.end_undo_action();
        result?;
        let start = selection.start;
        match toggle.caret {
            Some(offset) => self.set_selection(start + offset..start + offset),
            None => self.set_selection(start..start + toggle.replacement.len()),
        }
    }

    /// Ctrl+D: the word at an empty caret, then the next match as a new selection (spec §3).
    pub fn add_next_occurrence(&self) -> Result<()> {
        self.add_occurrences(SCI_MULTIPLESELECTADDNEXT)
    }

    /// Ctrl+Shift+L: a selection on every match (spec §3).
    pub fn select_all_occurrences(&self) -> Result<()> {
        self.add_occurrences(SCI_MULTIPLESELECTADDEACH)
    }

    /// Case-sensitive; whole-word only when the run started from an empty caret, as VS Code.
    /// At an empty caret Scintilla only selects the word, so Select all occurrences then goes
    /// on to add every match.
    fn add_occurrences(&self, message: u32) -> Result<()> {
        let endpoint = &self.endpoint;
        if self.send(SCI_GETSELECTIONS, 0, 0)? <= 1 {
            let selection = self.selection()?;
            if selection.is_empty() {
                self.send(message, 0, 0)?;
                let word = self.selection()?;
                endpoint.occurrence_word.set(Some((word.start, word.end)));
                endpoint.occurrence_whole_word.set(true);
                if message == SCI_MULTIPLESELECTADDNEXT || word.is_empty() {
                    return Ok(());
                }
            } else {
                let started_at_caret =
                    endpoint.occurrence_word.get() == Some((selection.start, selection.end));
                endpoint.occurrence_whole_word.set(started_at_caret);
            }
        }
        let whole_word = if endpoint.occurrence_whole_word.get() {
            SCFIND_WHOLEWORD
        } else {
            0
        };
        self.send(
            SCI_SETSEARCHFLAGS,
            (SCFIND_MATCHCASE | whole_word) as usize,
            0,
        )?;
        self.send(SCI_TARGETWHOLEDOCUMENT, 0, 0)?;
        self.send(message, 0, 0).map(drop)
    }

    /// A caret on the line above the topmost caret (or below the bottommost), in the same
    /// visual column, clamped to that line's end (spec §3).
    pub fn add_cursor(&self, above: bool) -> Result<()> {
        let carets = self
            .carets()?
            .into_iter()
            .map(|caret| Ok((self.line_from_position(caret)?, caret)))
            .collect::<Result<Vec<_>>>()?;
        let edge = if above {
            carets.iter().min()
        } else {
            carets.iter().max()
        };
        let Some(&(line, caret)) = edge else {
            return Ok(());
        };
        let target = if above {
            match line.checked_sub(1) {
                Some(target) => target,
                None => return Ok(()),
            }
        } else if line + 1 < self.line_count()? {
            line + 1
        } else {
            return Ok(());
        };
        let column = self.send(SCI_GETCOLUMN, caret, 0)?;
        let position = self.send(SCI_FINDCOLUMN, target, column)?;
        self.send(SCI_ADDSELECTION, position.max(0) as usize, position)
            .map(drop)
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
