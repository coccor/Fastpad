//! Markdown writing helpers (live mode spec §8). Pure text → edit plans; the window layer
//! applies a plan as one undo step. Edits use pre-edit byte offsets; selections are post-edit.

use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EditPlan {
    pub edits: Vec<TextEdit>,
    pub selections: Vec<Range<usize>>,
}

impl EditPlan {
    pub fn apply_to(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 16);
        let mut at = 0;
        for edit in &self.edits {
            out.push_str(&text[at..edit.range.start]);
            out.push_str(&edit.text);
            at = edit.range.end;
        }
        out.push_str(&text[at..]);
        out
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(at, |(index, _)| index);
    let end = text[at..]
        .char_indices()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(at, |(index, c)| at + index + c.len_utf8());
    (start < end).then_some(start..end)
}

/// Length of the run of `byte` ending at `end` (backwards) or starting at `start` (forwards).
fn run_before(text: &str, end: usize, byte: u8) -> usize {
    text.as_bytes()[..end].iter().rev().take_while(|b| **b == byte).count()
}

fn run_after(text: &str, start: usize, byte: u8) -> usize {
    text.as_bytes()[start..].iter().take_while(|b| **b == byte).count()
}

/// A marker run of `run` characters can drop a `width`-character marker: exactly that marker,
/// or the three-character bold-italic run for `*` / `**`.
fn run_matches(run: usize, width: usize) -> bool {
    run == width || (run == 3 && width < 3)
}

fn normalized(selection: &Range<usize>) -> Range<usize> {
    selection.start.min(selection.end)..selection.start.max(selection.end)
}

fn shift(position: usize, delta: isize) -> usize {
    (position as isize + delta) as usize
}

pub fn toggle_marker(text: &str, selections: &[Range<usize>], marker: &str) -> EditPlan {
    let width = marker.len();
    let byte = marker.as_bytes()[0];
    let mut order: Vec<usize> = (0..selections.len()).collect();
    order.sort_by_key(|index| normalized(&selections[*index]).start);
    let mut plan = EditPlan { edits: Vec::new(), selections: vec![0..0; selections.len()] };
    let mut delta: isize = 0;
    let insert = |plan: &mut EditPlan, at: usize, text: &str| {
        plan.edits.push(TextEdit { range: at..at, text: text.to_owned() });
    };
    let remove = |plan: &mut EditPlan, range: Range<usize>| {
        plan.edits.push(TextEdit { range, text: String::new() });
    };
    for index in order {
        let selection = normalized(&selections[index]);
        let caret = selection.is_empty().then_some(selection.start);
        let target = match caret {
            Some(at) => word_at(text, at).unwrap_or(at..at),
            None => selection.clone(),
        };
        if target.is_empty() {
            insert(&mut plan, target.start, &marker.repeat(2));
            let at = shift(target.start, delta) + width;
            plan.selections[index] = at..at;
            delta += 2 * width as isize;
            continue;
        }
        let inner = &text[target.clone()];
        let inside = inner.len() >= 2 * width
            && run_matches(run_after(inner, 0, byte), width)
            && run_matches(run_before(inner, inner.len(), byte), width);
        let around = target.start >= width
            && run_matches(run_before(text, target.start, byte), width)
            && run_matches(run_after(text, target.end, byte), width);
        if inside {
            remove(&mut plan, target.start..target.start + width);
            remove(&mut plan, target.end - width..target.end);
            let start = shift(target.start, delta);
            plan.selections[index] = match caret {
                Some(at) => {
                    let at = shift(at, delta).saturating_sub(width).max(start);
                    at..at
                }
                None => start..start + inner.len() - 2 * width,
            };
            delta -= 2 * width as isize;
        } else if around {
            remove(&mut plan, target.start - width..target.start);
            remove(&mut plan, target.end..target.end + width);
            plan.selections[index] = match caret {
                Some(at) => {
                    let at = shift(at, delta) - width;
                    at..at
                }
                None => shift(target.start, delta) - width..shift(target.end, delta) - width,
            };
            delta -= 2 * width as isize;
        } else {
            insert(&mut plan, target.start, marker);
            insert(&mut plan, target.end, marker);
            plan.selections[index] = match caret {
                Some(at) => {
                    let at = shift(at, delta) + width;
                    at..at
                }
                None => shift(target.start, delta) + width..shift(target.end, delta) + width,
            };
            delta += 2 * width as isize;
        }
    }
    plan.edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    plan
}

pub fn insert_link(_text: &str, selections: &[Range<usize>]) -> EditPlan {
    let mut order: Vec<usize> = (0..selections.len()).collect();
    order.sort_by_key(|index| normalized(&selections[*index]).start);
    let mut plan = EditPlan { edits: Vec::new(), selections: vec![0..0; selections.len()] };
    let mut delta: isize = 0;
    for index in order {
        let selection = normalized(&selections[index]);
        if selection.is_empty() {
            plan.edits.push(TextEdit { range: selection.clone(), text: "[]()".into() });
            let at = shift(selection.start, delta) + 1;
            plan.selections[index] = at..at;
        } else {
            plan.edits.push(TextEdit { range: selection.start..selection.start, text: "[".into() });
            plan.edits.push(TextEdit { range: selection.end..selection.end, text: "]()".into() });
            let at = shift(selection.end, delta) + 3;
            plan.selections[index] = at..at;
        }
        delta += 4;
    }
    plan
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn run(text: &str, selections: &[Range<usize>], marker: &str) -> (String, Vec<Range<usize>>) {
        let plan = toggle_marker(text, selections, marker);
        (plan.apply_to(text), plan.selections)
    }

    #[test]
    fn wraps_a_selection_and_keeps_it_selected() {
        assert_eq!(run("a b c", &[2..3], "**"), ("a **b** c".into(), vec![4..5]));
    }

    #[test]
    fn unwraps_when_the_markers_are_inside_the_selection() {
        assert_eq!(run("a **b** c", &[2..7], "**"), ("a b c".into(), vec![2..3]));
    }

    #[test]
    fn unwraps_when_the_markers_surround_the_selection() {
        assert_eq!(run("a **b** c", &[4..5], "**"), ("a b c".into(), vec![2..3]));
    }

    #[test]
    fn an_empty_selection_toggles_the_word_under_the_caret() {
        assert_eq!(run("hello world", &[2..2], "**"), ("**hello** world".into(), vec![4..4]));
        assert_eq!(run("**hello** world", &[4..4], "**"), ("hello world".into(), vec![2..2]));
    }

    #[test]
    fn an_empty_selection_outside_a_word_inserts_a_pair() {
        assert_eq!(run("a  b", &[2..2], "**"), ("a **** b".into(), vec![4..4]));
    }

    #[test]
    fn italic_on_bold_text_wraps_instead_of_eating_a_bold_star() {
        assert_eq!(run("**b**", &[2..3], "*"), ("***b***".into(), vec![3..4]));
    }

    #[test]
    fn bold_off_bold_italic_leaves_italic() {
        assert_eq!(run("***b***", &[3..4], "**"), ("*b*".into(), vec![1..2]));
    }

    #[test]
    fn every_caret_of_a_multi_cursor_is_toggled() {
        assert_eq!(run("a b", &[0..0, 2..2], "**"), ("**a** **b**".into(), vec![2..2, 8..8]));
    }

    #[test]
    fn inline_code_uses_backticks() {
        assert_eq!(run("x y", &[2..3], "`"), ("x `y`".into(), vec![3..4]));
    }

    #[test]
    fn a_reversed_selection_is_handled() {
        let reversed = Range { start: 3, end: 2 };
        assert_eq!(run("a b c", &[reversed], "**"), ("a **b** c".into(), vec![4..5]));
    }

    #[test]
    fn link_wraps_the_selection_and_puts_the_caret_in_the_parentheses() {
        let plan = insert_link("see docs", &[4..8]);
        assert_eq!(plan.apply_to("see docs"), "see [docs]()");
        assert_eq!(plan.selections, vec![11..11]);
    }

    #[test]
    fn link_without_a_selection_puts_the_caret_in_the_brackets() {
        let plan = insert_link("a ", &[2..2]);
        assert_eq!(plan.apply_to("a "), "a []()");
        assert_eq!(plan.selections, vec![3..3]);
    }
}
