//! One source line's map between its UTF-8 byte offsets (Scintilla positions, relative to the
//! line start) and the UTF-16 positions of the string DirectWrite lays out. Phase 2 hides markup
//! on rendered lines, so the map is built from pieces that are shown, hidden, or (phase 3) shown
//! as one inline object: a table pipe becomes a spacer that moves the next cell to its column.

use std::ops::Range;

/// How a piece of a line's source shows in its layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Show {
    Text,
    Hidden,
    /// One U+FFFC unit, whatever the piece's length (even zero), for an inline object.
    Object,
}

/// The object replacement character an object piece shows as.
const OBJECT: u16 = 0xFFFC;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OffsetMap {
    /// The string handed to DirectWrite: the shown pieces, in order.
    text: Vec<u16>,
    /// The byte offset of each UTF-16 unit; both units of a surrogate pair hold the pair's start.
    units: Vec<usize>,
    /// The byte just past the character each UTF-16 unit belongs to.
    ends: Vec<usize>,
    /// Hidden byte ranges (and those of non-empty objects), each with the UTF-16 position it
    /// collapses to.
    hidden: Vec<(Range<usize>, u32)>,
    /// The UTF-16 position of each object piece.
    objects: Vec<u32>,
    byte_len: usize,
}

impl OffsetMap {
    /// A line shown in full.
    pub fn plain(line: &str) -> Self {
        Self::new(line, [(0..line.len(), Show::Text)])
    }

    /// A line made of `pieces`, byte ranges of `line` in order and covering it, each shown,
    /// hidden or shown as an object. Ranges must fall on character boundaries. A byte inside an
    /// object maps to the object's position; the object's unit maps back to its range's start,
    /// and a trailing hit on it to its end.
    pub fn new(line: &str, pieces: impl IntoIterator<Item = (Range<usize>, Show)>) -> Self {
        let mut map = Self {
            byte_len: line.len(),
            ..Self::default()
        };
        for (range, show) in pieces {
            let position = map.text.len() as u32;
            match show {
                Show::Object => {
                    if !range.is_empty() {
                        map.hidden.push((range.clone(), position));
                    }
                    map.objects.push(position);
                    map.text.push(OBJECT);
                    map.units.push(range.start);
                    map.ends.push(range.end);
                    continue;
                }
                _ if range.is_empty() => continue,
                Show::Hidden => {
                    map.hidden.push((range, position));
                    continue;
                }
                Show::Text => {}
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

    /// The UTF-16 position of each object piece, in order.
    pub fn objects(&self) -> &[u32] {
        &self.objects
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
    use super::{OffsetMap, Show};

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
        OffsetMap::new(
            "**bold**",
            [
                (0..2, Show::Hidden),
                (2..6, Show::Text),
                (6..8, Show::Hidden),
            ],
        )
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
        let map = OffsetMap::new(
            "[ab](u)",
            [
                (0..1, Show::Hidden),
                (1..3, Show::Text),
                (3..7, Show::Hidden),
            ],
        );
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
        let map = OffsetMap::new("==", [(0..2, Show::Hidden)]);
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
                (0..2, Show::Text),
                (2..4, Show::Hidden),
                (4..5, Show::Text),
                (5..7, Show::Hidden),
                (7..9, Show::Text),
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
        let map = OffsetMap::new(
            line,
            [
                (0..2, Show::Hidden),
                (2..6, Show::Text),
                (6..8, Show::Hidden),
            ],
        );
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

    /// `| ab | c |` rendered as a table row: pipes are objects, cell padding is hidden.
    fn row() -> OffsetMap {
        OffsetMap::new(
            "| ab | c |",
            [
                (0..1, Show::Object),
                (1..2, Show::Hidden),
                (2..4, Show::Text),
                (4..5, Show::Hidden),
                (5..6, Show::Object),
                (6..7, Show::Hidden),
                (7..8, Show::Text),
                (8..9, Show::Hidden),
                (9..10, Show::Object),
            ],
        )
    }

    #[test]
    fn an_object_piece_is_one_unit() {
        let map = row();
        assert_eq!(
            map.layout_text(),
            [
                0xFFFC,
                u16::from(b'a'),
                u16::from(b'b'),
                0xFFFC,
                u16::from(b'c'),
                0xFFFC
            ]
        );
        assert_eq!(map.objects(), &[0, 3, 5]);
        assert_eq!(map.to_utf16(0), 0);
        assert_eq!(map.to_utf16(2), 1);
        assert_eq!(map.to_utf16(5), 3);
        assert_eq!(map.to_utf16(7), 4);
        assert_eq!(map.to_utf16(10), 6);
    }

    #[test]
    fn an_empty_object_maps_to_its_position() {
        // `a | b` without outer pipes: empty objects open and close the row.
        let map = OffsetMap::new(
            "a | b",
            [
                (0..0, Show::Object),
                (0..1, Show::Text),
                (1..2, Show::Hidden),
                (2..3, Show::Object),
                (3..4, Show::Hidden),
                (4..5, Show::Text),
                (5..5, Show::Object),
            ],
        );
        assert_eq!(map.layout_text().len(), 5);
        assert_eq!(map.objects(), &[0, 2, 4]);
        assert_eq!(map.to_utf16(0), 0);
        assert_eq!(map.to_byte(0), 0);
        assert_eq!(map.to_byte(1), 0);
        assert_eq!(map.to_byte(4), 5);
        assert_eq!(map.to_byte_after(1), 0);
    }

    #[test]
    fn hits_on_an_object_land_on_its_edges() {
        let map = row();
        assert_eq!(map.to_byte(3), 5);
        assert_eq!(map.to_byte_after(4), 6);
        assert_eq!(map.to_byte_after(3), 4);
        assert_eq!(map.to_utf16_range(2..4), 1..3);
    }
}
