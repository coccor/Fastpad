//! Caret movement that depends on wrapped rows, which only the Live layout knows: Up/Down,
//! PgUp/PgDn and Home/End (phase 1 plan, Task 4). Left/Right and word movement are Scintilla's.
//! Positions are document byte offsets; y values are in the layout's document coordinates.

/// One wrapped row of a source line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub line: usize,
    /// Where the row's source line starts.
    pub line_start: usize,
    pub start: usize,
    /// Where the next row starts, or the line's end for its last row.
    pub end: usize,
    /// The last caret position shown on this row: `end` on a line's last row, the position before
    /// the row's last character otherwise (the wrap point belongs to the next row).
    pub caret_end: usize,
    pub top: f32,
    pub height: f32,
}

pub trait LineGeometry {
    /// The row showing `position`.
    fn row_at(&mut self, position: usize) -> Row;
    fn row_above(&mut self, row: &Row) -> Option<Row>;
    fn row_below(&mut self, row: &Row) -> Option<Row>;
    /// The caret's x at `position`.
    fn x_of(&mut self, position: usize) -> f32;
    /// The caret position nearest `x` on `row`, between `row.start` and `row.caret_end`.
    fn position_at_x(&mut self, row: &Row, x: f32) -> usize;
    /// Where the indentation of `row`'s line ends.
    fn indent_end(&mut self, row: &Row) -> usize;
    fn page_height(&self) -> f32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Up,
    Down,
    PageUp,
    PageDown,
    RowStart,
    RowEnd,
}

/// Where a movement left the caret, and the x that further vertical movement aims for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub caret: usize,
    pub goal_x: Option<f32>,
}

/// Moves `caret`. `goal_x` is the x a run of vertical movement keeps aiming for; the view clears
/// it whenever the caret moves any other way.
pub fn apply(
    geometry: &mut impl LineGeometry,
    caret: usize,
    goal_x: Option<f32>,
    movement: Move,
) -> Motion {
    let row = geometry.row_at(caret);
    match movement {
        Move::Up | Move::Down | Move::PageUp | Move::PageDown => {
            let goal = goal_x.unwrap_or_else(|| geometry.x_of(caret));
            let target = match movement {
                Move::Up => geometry.row_above(&row),
                Move::Down => geometry.row_below(&row),
                Move::PageUp => page(geometry, row, true),
                _ => page(geometry, row, false),
            };
            let caret = match target {
                Some(target) => geometry.position_at_x(&target, goal),
                None if matches!(movement, Move::Up | Move::PageUp) => 0,
                None => last_row(geometry, row).end,
            };
            Motion {
                caret,
                goal_x: Some(goal),
            }
        }
        Move::RowStart => {
            // Scintilla's VCHomeWrap: a wrapped row's start first, then the line's indentation,
            // which toggles with the line start.
            let caret = if row.start != row.line_start && caret != row.start {
                row.start
            } else {
                let indent = geometry.indent_end(&row);
                if caret == indent {
                    row.line_start
                } else {
                    indent
                }
            };
            Motion {
                caret,
                goal_x: None,
            }
        }
        Move::RowEnd => Motion {
            caret: row.caret_end,
            goal_x: None,
        },
    }
}

/// The row about a page above or below `row`; `None` when there is no row that way at all.
fn page(geometry: &mut impl LineGeometry, row: Row, up: bool) -> Option<Row> {
    let distance = geometry.page_height().max(row.height);
    let mut current = row;
    let mut moved = false;
    loop {
        let next = if up {
            geometry.row_above(&current)
        } else {
            geometry.row_below(&current)
        };
        let Some(next) = next else {
            return moved.then_some(current);
        };
        if (next.top - row.top).abs() > distance {
            return Some(current);
        }
        current = next;
        moved = true;
    }
}

fn last_row(geometry: &mut impl LineGeometry, row: Row) -> Row {
    let mut current = row;
    while let Some(next) = geometry.row_below(&current) {
        current = next;
    }
    current
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{LineGeometry, Move, Row, apply};
    use crate::live::text::{LiveText, MemoryText};

    /// A fixed-width font: every byte is 10 px wide, rows are 20 px tall and wrap every `width`
    /// bytes, breaking anywhere.
    pub(crate) struct FakeGeometry {
        pub text: MemoryText,
        pub width: usize,
        pub page: f32,
    }

    impl FakeGeometry {
        pub(crate) fn new(text: &str, width: usize) -> Self {
            Self {
                text: MemoryText::new(text),
                width,
                page: 60.0,
            }
        }

        fn rows_of(&self, line: usize) -> Vec<(usize, usize)> {
            let start = self.text.line_start(line);
            let end = self.text.line_end(line);
            let mut rows = Vec::new();
            let mut at = start;
            loop {
                let row_end = (at + self.width).min(end);
                rows.push((at, row_end));
                if row_end >= end {
                    return rows;
                }
                at = row_end;
            }
        }

        fn row(&self, line: usize, index: usize) -> Row {
            let before: usize = (0..line).map(|l| self.rows_of(l).len()).sum();
            let rows = self.rows_of(line);
            let (start, end) = rows[index];
            let last = index + 1 == rows.len();
            Row {
                line,
                line_start: self.text.line_start(line),
                start,
                end,
                caret_end: if last { end } else { end - 1 },
                top: (before + index) as f32 * 20.0,
                height: 20.0,
            }
        }
    }

    impl LineGeometry for FakeGeometry {
        fn row_at(&mut self, position: usize) -> Row {
            let line = self.text.line_of(position);
            let rows = self.rows_of(line);
            let index = rows
                .iter()
                .position(|&(start, end)| position >= start && position < end)
                .unwrap_or(rows.len() - 1);
            self.row(line, index)
        }

        fn row_above(&mut self, row: &Row) -> Option<Row> {
            let index = self
                .rows_of(row.line)
                .iter()
                .position(|r| r.0 == row.start)?;
            if index > 0 {
                return Some(self.row(row.line, index - 1));
            }
            let line = row.line.checked_sub(1)?;
            Some(self.row(line, self.rows_of(line).len() - 1))
        }

        fn row_below(&mut self, row: &Row) -> Option<Row> {
            let rows = self.rows_of(row.line);
            let index = rows.iter().position(|r| r.0 == row.start)?;
            if index + 1 < rows.len() {
                return Some(self.row(row.line, index + 1));
            }
            (row.line + 1 < self.text.line_count()).then(|| self.row(row.line + 1, 0))
        }

        fn x_of(&mut self, position: usize) -> f32 {
            let row = self.row_at(position);
            (position - row.start) as f32 * 10.0
        }

        fn position_at_x(&mut self, row: &Row, x: f32) -> usize {
            (row.start + (x / 10.0).round().max(0.0) as usize).min(row.caret_end)
        }

        fn indent_end(&mut self, row: &Row) -> usize {
            let line = self.text.line(row.line);
            row.line_start + (line.len() - line.trim_start_matches([' ', '\t']).len())
        }

        fn page_height(&self) -> f32 {
            self.page
        }
    }

    #[test]
    fn down_keeps_the_goal_column_across_a_short_row() {
        let mut geometry = FakeGeometry::new("abcdefgh\nab\nabcdefgh", 20);
        let first = apply(&mut geometry, 6, None, Move::Down);
        assert_eq!(first.caret, 11, "clamped to the short line's end");
        let second = apply(&mut geometry, first.caret, first.goal_x, Move::Down);
        assert_eq!(second.caret, 18, "back at column 6");
    }

    #[test]
    fn down_moves_between_wrapped_rows_of_one_line() {
        let mut geometry = FakeGeometry::new("abcdefghij", 4);
        assert_eq!(apply(&mut geometry, 1, None, Move::Down).caret, 5);
        assert_eq!(apply(&mut geometry, 5, None, Move::Down).caret, 9);
        assert_eq!(apply(&mut geometry, 9, None, Move::Up).caret, 5);
    }

    #[test]
    fn up_from_the_first_row_goes_to_the_start() {
        let mut geometry = FakeGeometry::new("abc\ndef", 20);
        let motion = apply(&mut geometry, 2, None, Move::Up);
        assert_eq!(motion.caret, 0);
        assert_eq!(motion.goal_x, Some(20.0));
    }

    #[test]
    fn down_from_the_last_row_goes_to_the_end() {
        let mut geometry = FakeGeometry::new("abc\ndef", 20);
        assert_eq!(apply(&mut geometry, 5, None, Move::Down).caret, 7);
    }

    #[test]
    fn page_down_moves_a_screenful() {
        let text = (0..10)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut geometry = FakeGeometry::new(&text, 20);
        // 60 px pages, 20 px rows: three rows down.
        let motion = apply(&mut geometry, 0, None, Move::PageDown);
        assert_eq!(geometry.text.line_of(motion.caret), 3);
        let back = apply(&mut geometry, motion.caret, motion.goal_x, Move::PageUp);
        assert_eq!(back.caret, 0);
    }

    #[test]
    fn page_down_near_the_end_stops_on_the_last_row() {
        let mut geometry = FakeGeometry::new("a\nb", 20);
        assert_eq!(apply(&mut geometry, 0, None, Move::PageDown).caret, 2);
        assert_eq!(apply(&mut geometry, 2, None, Move::PageDown).caret, 3);
    }

    #[test]
    fn home_toggles_between_indent_and_line_start() {
        let mut geometry = FakeGeometry::new("    text", 20);
        assert_eq!(apply(&mut geometry, 6, None, Move::RowStart).caret, 4);
        assert_eq!(apply(&mut geometry, 4, None, Move::RowStart).caret, 0);
        assert_eq!(apply(&mut geometry, 0, None, Move::RowStart).caret, 4);
        assert_eq!(apply(&mut geometry, 2, None, Move::RowStart).caret, 4);
    }

    #[test]
    fn home_on_a_wrapped_row_goes_to_the_row_start() {
        let mut geometry = FakeGeometry::new("  abcdefghij", 4);
        assert_eq!(apply(&mut geometry, 6, None, Move::RowStart).caret, 4);
        // Already at the row start: on to the line's own Home.
        assert_eq!(apply(&mut geometry, 4, None, Move::RowStart).caret, 2);
    }

    #[test]
    fn end_goes_to_the_row_end_not_the_line_end() {
        let mut geometry = FakeGeometry::new("abcdefghij\nx", 4);
        assert_eq!(apply(&mut geometry, 1, None, Move::RowEnd).caret, 3);
        assert_eq!(apply(&mut geometry, 9, None, Move::RowEnd).caret, 10);
    }

    #[test]
    fn other_moves_clear_the_goal() {
        let mut geometry = FakeGeometry::new("abc", 20);
        assert_eq!(
            apply(&mut geometry, 1, Some(30.0), Move::RowEnd).goal_x,
            None
        );
    }
}
