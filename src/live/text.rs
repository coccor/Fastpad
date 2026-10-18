//! What the Live view reads from the document and how it sets the selection. The document is the
//! tab's Scintilla document (`EditorText`); tests use `MemoryText`. Positions are UTF-8 byte
//! offsets, as Scintilla's are. Edits never go through here: typing is forwarded to Scintilla
//! (phase 1 plan, deviation 2).

use crate::editor::Editor;
use std::borrow::Cow;
use std::ops::Range;

pub trait LiveText {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Lines as Scintilla counts them: a final line ending starts an empty last line.
    fn line_count(&self) -> usize;
    fn line_of(&self, position: usize) -> usize;
    /// Where `line` starts; the document length past the last line.
    fn line_start(&self, line: usize) -> usize;
    /// Where `line`'s text ends, before its line ending.
    fn line_end(&self, line: usize) -> usize;
    /// `line`'s text without its line ending; invalid UTF-8 is replaced.
    fn line(&self, line: usize) -> Cow<'_, str>;
    /// The main selection as (anchor, caret).
    fn selection(&self) -> (usize, usize);
    fn set_selection(&self, anchor: usize, caret: usize);
    /// The word around `position`; empty when there is none.
    fn word_at(&self, position: usize) -> Range<usize>;
}

/// The tab's Scintilla document. Errors (a destroyed editor) read as an empty document: there is
/// nothing left to show.
pub struct EditorText<'a>(pub &'a Editor);

impl LiveText for EditorText<'_> {
    fn len(&self) -> usize {
        self.0.length().unwrap_or(0)
    }

    fn line_count(&self) -> usize {
        self.0.line_count().unwrap_or(1).max(1)
    }

    fn line_of(&self, position: usize) -> usize {
        self.0.line_from_position(position).unwrap_or(0)
    }

    fn line_start(&self, line: usize) -> usize {
        self.0.line_start(line).unwrap_or(0)
    }

    fn line_end(&self, line: usize) -> usize {
        if line >= self.line_count() {
            return self.len();
        }
        self.0.line_end(line).unwrap_or(0)
    }

    fn line(&self, line: usize) -> Cow<'_, str> {
        let range = self.line_start(line)..self.line_end(line);
        match self.0.range_bytes(range) {
            Ok(bytes) => String::from_utf8_lossy(bytes),
            Err(_) => Cow::Borrowed(""),
        }
    }

    fn selection(&self) -> (usize, usize) {
        self.0.anchor_and_caret().unwrap_or((0, 0))
    }

    fn set_selection(&self, anchor: usize, caret: usize) {
        let _ = self.0.set_selection(Range {
            start: anchor,
            end: caret,
        });
    }

    fn word_at(&self, position: usize) -> Range<usize> {
        self.0.word_range_at(position).unwrap_or(position..position)
    }
}

/// A document in memory, for tests of the pure parts.
#[cfg(test)]
pub struct MemoryText {
    text: String,
    /// Start of each line, and where its text ends before the line ending.
    lines: Vec<(usize, usize)>,
    selection: std::cell::Cell<(usize, usize)>,
}

#[cfg(test)]
impl MemoryText {
    pub fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut at = 0;
        while at < bytes.len() {
            match bytes[at] {
                b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                    lines.push((start, at));
                    at += 2;
                    start = at;
                }
                b'\r' | b'\n' => {
                    lines.push((start, at));
                    at += 1;
                    start = at;
                }
                _ => at += 1,
            }
        }
        lines.push((start, bytes.len()));
        Self {
            text: text.to_owned(),
            lines,
            selection: std::cell::Cell::new((0, 0)),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
impl LiveText for MemoryText {
    fn len(&self) -> usize {
        self.text.len()
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line_of(&self, position: usize) -> usize {
        self.lines
            .partition_point(|&(start, _)| start <= position)
            .saturating_sub(1)
    }

    fn line_start(&self, line: usize) -> usize {
        self.lines
            .get(line)
            .map_or(self.text.len(), |&(start, _)| start)
    }

    fn line_end(&self, line: usize) -> usize {
        self.lines
            .get(line)
            .map_or(self.text.len(), |&(_, end)| end)
    }

    fn line(&self, line: usize) -> Cow<'_, str> {
        Cow::Borrowed(&self.text[self.line_start(line)..self.line_end(line)])
    }

    fn selection(&self) -> (usize, usize) {
        self.selection.get()
    }

    fn set_selection(&self, anchor: usize, caret: usize) {
        self.selection.set((anchor, caret));
    }

    fn word_at(&self, position: usize) -> Range<usize> {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = self.text[..position]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word(c))
            .last()
            .map_or(position, |(at, _)| at);
        let end = self.text[position..]
            .char_indices()
            .find(|&(_, c)| !is_word(c))
            .map_or(self.text.len(), |(at, _)| position + at);
        start..end
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveText, MemoryText};

    #[test]
    fn memory_text_counts_lines_like_scintilla() {
        let text = MemoryText::new("one\r\ntwo\nthree\rfour\n");
        assert_eq!(text.line_count(), 5);
        assert_eq!(text.line(0), "one");
        assert_eq!(text.line(1), "two");
        assert_eq!(text.line(2), "three");
        assert_eq!(text.line(3), "four");
        assert_eq!(text.line(4), "");
        assert_eq!(text.line_start(1), 5);
        assert_eq!(text.line_end(0), 3);
        assert_eq!(text.line_start(9), text.len());
    }

    #[test]
    fn memory_text_maps_positions_to_lines() {
        let text = MemoryText::new("ab\r\ncd");
        assert_eq!(text.line_of(0), 0);
        assert_eq!(text.line_of(2), 0);
        assert_eq!(text.line_of(3), 0);
        assert_eq!(text.line_of(4), 1);
        assert_eq!(text.line_of(6), 1);
    }

    #[test]
    fn memory_text_finds_words() {
        let text = MemoryText::new("one two_3 x");
        assert_eq!(text.word_at(5), 4..9);
        assert_eq!(text.word_at(4), 4..9);
        assert_eq!(text.word_at(9), 4..9);
        assert_eq!(text.word_at(0), 0..3);
    }

    #[test]
    fn memory_text_keeps_the_selection_direction() {
        let text = MemoryText::new("abc");
        text.set_selection(3, 1);
        assert_eq!(text.selection(), (3, 1));
    }
}
