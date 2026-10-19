//! One source line's map between its UTF-8 byte offsets (Scintilla positions, relative to the
//! line start) and the UTF-16 positions of the string DirectWrite lays out. Phase 2 hides markup
//! on rendered lines, so the map is built from pieces that are either shown or hidden.

use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OffsetMap {
    /// The string handed to DirectWrite: the shown pieces, in order.
    text: Vec<u16>,
    /// The byte offset of each UTF-16 unit; both units of a surrogate pair hold the pair's start.
    units: Vec<usize>,
    /// The byte just past the character each UTF-16 unit belongs to.
    ends: Vec<usize>,
    /// Hidden byte ranges, each with the UTF-16 position it collapses to.
    hidden: Vec<(Range<usize>, u32)>,
    byte_len: usize,
}

impl OffsetMap {
    /// A line shown in full.
    pub fn plain(line: &str) -> Self {
        Self::new(line, [(0..line.len(), true)])
    }

    /// A line made of `pieces`, byte ranges of `line` in order and covering it, each shown or
    /// hidden. Ranges must fall on character boundaries.
    pub fn new(line: &str, pieces: impl IntoIterator<Item = (Range<usize>, bool)>) -> Self {
        let mut map = Self {
            byte_len: line.len(),
            ..Self::default()
        };
        for (range, shown) in pieces {
            if range.is_empty() {
                continue;
            }
            if !shown {
                map.hidden.push((range, map.text.len() as u32));
                continue;
            }
            for (offset, ch) in line[range.clone()].char_indices() {
                let mut buffer = [0u16; 2];
                for unit in ch.encode_utf16(&mut buffer) {
                    map.text.push(*unit);
                    map.units.push(range.start + offset);
                    map.ends.push(range.start + offset + ch.len_utf8());
                }
            }
        }
        map
    }

    /// The string DirectWrite lays out.
    pub fn layout_text(&self) -> &[u16] {
        &self.text
    }

    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    /// The UTF-16 position of byte `byte`. A byte inside a character rounds down to its start,
    /// a hidden byte goes to where its range collapses, and the line end (or past it) maps to the
    /// end of the layout text.
    pub fn to_utf16(&self, byte: usize) -> u32 {
        if byte >= self.byte_len {
            return self.text.len() as u32;
        }
        if let Some((_, position)) = self.hidden.iter().find(|(range, _)| range.contains(&byte)) {
            return *position;
        }
        let after = self.units.partition_point(|&start| start <= byte);
        let Some(&start) = after.checked_sub(1).and_then(|last| self.units.get(last)) else {
            return 0;
        };
        self.units.partition_point(|&unit| unit < start) as u32
    }

    /// The byte offset of UTF-16 position `utf16`. A low surrogate rounds down to its pair's
    /// start; the end of the layout text (or past it) maps to the line end.
    pub fn to_byte(&self, utf16: u32) -> usize {
        self.units
            .get(utf16 as usize)
            .copied()
            .unwrap_or(self.byte_len)
    }

    /// The byte just past the character whose last UTF-16 unit is `utf16 - 1`: where a trailing
    /// hit on that character lands, before any hidden range that follows it. 0 for 0.
    pub fn to_byte_after(&self, utf16: u32) -> usize {
        let Some(last) = (utf16 as usize).checked_sub(1) else {
            return 0;
        };
        self.ends
            .get(last)
            .copied()
            .unwrap_or_else(|| self.ends.last().copied().unwrap_or(0))
    }

    /// The UTF-16 range showing bytes `range`, with hidden bytes collapsed.
    pub fn to_utf16_range(&self, range: Range<usize>) -> Range<u32> {
        let start = self.to_utf16(range.start);
        start..self.to_utf16(range.end).max(start)
    }
}

#[cfg(test)]
mod tests {
    use super::OffsetMap;

    fn round_trip(line: &str) {
        let map = OffsetMap::plain(line);
        for (byte, _) in line.char_indices().chain([(line.len(), ' ')]) {
            assert_eq!(map.to_byte(map.to_utf16(byte)), byte, "{line:?} at {byte}");
        }
    }

    #[test]
    fn ascii_maps_one_to_one() {
        let map = OffsetMap::plain("abc");
        assert_eq!(map.layout_text(), "abc".encode_utf16().collect::<Vec<_>>());
        assert_eq!(
            (0..=3).map(|b| map.to_utf16(b)).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            (0..=3).map(|u| map.to_byte(u)).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }

    #[test]
    fn two_and_three_byte_chars_map_to_one_unit() {
        let map = OffsetMap::plain("é—x");
        assert_eq!(map.layout_text().len(), 3);
        assert_eq!(map.to_utf16(2), 1);
        assert_eq!(map.to_utf16(5), 2);
        assert_eq!(map.to_byte(1), 2);
        assert_eq!(map.to_byte(2), 5);
        round_trip("é—x");
    }

    #[test]
    fn astral_chars_map_to_a_surrogate_pair() {
        let map = OffsetMap::plain("a😀b");
        assert_eq!(map.layout_text().len(), 4);
        assert_eq!(map.to_utf16(1), 1);
        assert_eq!(map.to_utf16(5), 3);
        assert_eq!(map.to_byte(3), 5);
        round_trip("a😀b");
    }

    #[test]
    fn mid_char_bytes_round_down() {
        let map = OffsetMap::plain("a😀b");
        assert_eq!(map.to_utf16(2), 1);
        assert_eq!(map.to_utf16(4), 1);
        let map = OffsetMap::plain("é");
        assert_eq!(map.to_utf16(1), 0);
    }

    #[test]
    fn low_surrogate_rounds_down() {
        let map = OffsetMap::plain("a😀b");
        assert_eq!(map.to_byte(2), 1);
    }

    #[test]
    fn combining_marks_are_their_own_positions() {
        // e + U+0301: DirectWrite keeps the caret off the mark; the map only needs to be exact.
        round_trip("e\u{301}x");
        let map = OffsetMap::plain("e\u{301}x");
        assert_eq!(map.to_utf16(3), 2);
    }

    #[test]
    fn end_of_line_maps_to_the_end() {
        let map = OffsetMap::plain("ab");
        assert_eq!(map.to_utf16(2), 2);
        assert_eq!(map.to_utf16(9), 2);
        assert_eq!(map.to_byte(2), 2);
        assert_eq!(map.to_byte(9), 2);
    }

    #[test]
    fn empty_line() {
        let map = OffsetMap::plain("");
        assert!(map.layout_text().is_empty());
        assert_eq!(map.to_utf16(0), 0);
        assert_eq!(map.to_byte(0), 0);
    }

    fn bold() -> OffsetMap {
        OffsetMap::new("**bold**", [(0..2, false), (2..6, true), (6..8, false)])
    }

    #[test]
    fn trailing_hit_on_the_last_bold_letter_stays_inside() {
        let map = bold();
        assert_eq!(map.to_byte_after(4), 6, "before the closing markers");
        assert_eq!(map.to_byte(4), 8, "a leading hit at the end is past them");
        assert_eq!(
            map.to_byte(0),
            2,
            "a leading hit on the first letter is after the opener"
        );
        assert_eq!(map.to_byte_after(0), 0);
    }

    #[test]
    fn trailing_hit_before_a_hidden_link_tail_stays_in_the_text() {
        let map = OffsetMap::new("[ab](u)", [(0..1, false), (1..3, true), (3..7, false)]);
        assert_eq!(map.to_byte_after(2), 3);
        assert_eq!(map.to_byte_after(1), 2);
    }

    #[test]
    fn trailing_hit_on_an_astral_char_lands_after_both_units() {
        let map = OffsetMap::plain("a😀b");
        assert_eq!(map.to_byte_after(3), 5);
        assert_eq!(
            map.to_byte_after(2),
            5,
            "the high surrogate's trailing hit too"
        );
        assert_eq!(map.to_byte_after(4), 6);
    }

    #[test]
    fn a_fully_hidden_line_maps_everything_to_zero_and_the_end() {
        let map = OffsetMap::new("==", [(0..2, false)]);
        assert!(map.layout_text().is_empty());
        assert_eq!(map.to_utf16(0), 0);
        assert_eq!(map.to_utf16(1), 0);
        assert_eq!(map.to_byte(0), 2);
        assert_eq!(map.to_byte_after(0), 0);
        assert_eq!(map.to_utf16_range(0..2), 0..0);
    }

    #[test]
    fn utf16_range_of_a_run_skips_hidden_bytes() {
        let map = OffsetMap::new(
            "a **b** c",
            [
                (0..2, true),
                (2..4, false),
                (4..5, true),
                (5..7, false),
                (7..9, true),
            ],
        );
        assert_eq!(map.to_utf16_range(4..5), 2..3);
        assert_eq!(map.to_utf16_range(0..9), 0..5);
        assert_eq!(map.to_utf16_range(2..7), 2..3);
    }

    #[test]
    fn hidden_pieces_collapse_to_one_position() {
        // "**bold**" with both markers hidden.
        let line = "**bold**";
        let map = OffsetMap::new(line, [(0..2, false), (2..6, true), (6..8, false)]);
        assert_eq!(map.layout_text(), "bold".encode_utf16().collect::<Vec<_>>());
        assert_eq!(map.to_utf16(0), 0);
        assert_eq!(map.to_utf16(1), 0);
        assert_eq!(map.to_utf16(2), 0);
        assert_eq!(map.to_utf16(5), 3);
        assert_eq!(map.to_utf16(6), 4);
        assert_eq!(map.to_utf16(7), 4);
        assert_eq!(map.to_byte(0), 2);
        assert_eq!(map.to_byte(4), 8);
    }
}
