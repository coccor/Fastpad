//! Markdown writing helpers (live mode spec §8). Pure text → edit plans; the window layer
//! applies a plan as one undo step. Edits use pre-edit byte offsets; selections are post-edit.

// A plan's selections are ranges; a one-caret plan is legitimately a one-element Vec of them.
#![allow(clippy::single_range_in_vec_init)]

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

pub(crate) fn line_bounds(text: &str, at: usize) -> Range<usize> {
    let start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[at..].find('\n').map_or(text.len(), |newline| at + newline);
    let end = if end > start && text.as_bytes()[end - 1] == b'\r' { end - 1 } else { end };
    start..end
}

/// The line ending after `line_end`; for the last line the document's first one, or `fallback`
/// when the document has none.
pub(crate) fn line_ending(text: &str, line_end: usize, fallback: &'static str) -> &'static str {
    let rest = &text[line_end..];
    if rest.starts_with("\r\n") {
        "\r\n"
    } else if rest.starts_with('\n') {
        "\n"
    } else {
        match text.find('\n') {
            Some(at) if at > 0 && text.as_bytes()[at - 1] == b'\r' => "\r\n",
            Some(_) => "\n",
            None => fallback,
        }
    }
}

struct ListPrefix {
    indent: usize,
    bullet: Option<u8>,
    number: Option<(u64, u8)>,
    task: bool,
    /// Bytes from the line start to the item text.
    len: usize,
    /// The marker plus its space: how far a nested item indents.
    marker_width: usize,
}

fn list_prefix(line: &str) -> Option<ListPrefix> {
    let bytes = line.as_bytes();
    let indent = bytes.iter().take_while(|b| matches!(b, b' ' | b'\t')).count();
    let mut at = indent;
    let (bullet, number) = match *bytes.get(at)? {
        marker @ (b'-' | b'*' | b'+') => {
            at += 1;
            (Some(marker), None)
        }
        digit if digit.is_ascii_digit() => {
            let digits = bytes[at..].iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 {
                return None;
            }
            let value = line[at..at + digits].parse().ok()?;
            at += digits;
            let delimiter = *bytes.get(at)?;
            if delimiter != b'.' && delimiter != b')' {
                return None;
            }
            at += 1;
            (None, Some((value, delimiter)))
        }
        _ => return None,
    };
    match bytes.get(at) {
        Some(b' ') => at += 1,
        None => {}
        Some(_) => return None,
    }
    let marker_width = (at - indent).max(2);
    let rest = &line[at..];
    let task = ["[ ]", "[x]", "[X]"].iter().any(|box_| {
        rest.starts_with(box_) && (rest.len() == 3 || rest.as_bytes()[3] == b' ')
    });
    if task {
        at = (at + 4).min(line.len());
    }
    Some(ListPrefix { indent, bullet, number, task, len: at, marker_width })
}

/// `fallback` is the line ending for a document with none yet (the editor's EOL mode).
pub fn enter_in_list(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let prefix = list_prefix(&text[line.clone()])?;
    let prefix_end = line.start + prefix.len;
    if caret < prefix_end.min(line.end) {
        return None;
    }
    if text[prefix_end.min(line.end)..line.end].trim().is_empty() {
        return Some(EditPlan {
            edits: vec![TextEdit { range: line.clone(), text: String::new() }],
            selections: vec![line.start..line.start],
        });
    }
    let marker = match (prefix.bullet, prefix.number) {
        (Some(bullet), _) => char::from(bullet).to_string(),
        (None, Some((value, delimiter))) => format!("{}{}", value + 1, char::from(delimiter)),
        (None, None) => return None,
    };
    let inserted = format!(
        "{}{}{} {}",
        line_ending(text, line.end, fallback),
        &text[line.start..line.start + prefix.indent],
        marker,
        if prefix.task { "[ ] " } else { "" }
    );
    let after = caret + inserted.len();
    Some(EditPlan {
        edits: vec![TextEdit { range: caret..caret, text: inserted }],
        selections: vec![after..after],
    })
}

pub fn indent_list_item(text: &str, caret: usize, outdent: bool) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let prefix = list_prefix(&text[line.clone()])?;
    if !outdent {
        let pad = " ".repeat(prefix.marker_width);
        let after = caret + pad.len();
        return Some(EditPlan {
            edits: vec![TextEdit { range: line.start..line.start, text: pad }],
            selections: vec![after..after],
        });
    }
    let spaces = text[line.start..line.start + prefix.indent].bytes().take_while(|b| *b == b' ').count();
    let remove = spaces.min(prefix.marker_width);
    if remove == 0 {
        return None;
    }
    let after = caret.saturating_sub(remove).max(line.start);
    Some(EditPlan {
        edits: vec![TextEdit { range: line.start..line.start + remove, text: String::new() }],
        selections: vec![after..after],
    })
}

#[cfg(test)]
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

    fn enter(text: &str, caret: usize) -> Option<(String, usize)> {
        enter_in_list(text, caret, "\n").map(|plan| (plan.apply_to(text), plan.selections[0].start))
    }

    fn nest(text: &str, caret: usize, outdent: bool) -> Option<(String, usize)> {
        indent_list_item(text, caret, outdent)
            .map(|plan| (plan.apply_to(text), plan.selections[0].start))
    }

    #[test]
    fn enter_continues_bullets_numbers_and_tasks() {
        assert_eq!(enter("- a", 3), Some(("- a\n- ".into(), 6)));
        assert_eq!(enter("1. a", 4), Some(("1. a\n2. ".into(), 8)));
        assert_eq!(enter("3) a", 4), Some(("3) a\n4) ".into(), 8)));
        assert_eq!(enter("- [x] a", 7), Some(("- [x] a\n- [ ] ".into(), 14)));
        assert_eq!(enter("  * a", 5), Some(("  * a\n  * ".into(), 10)));
    }

    #[test]
    fn enter_mid_item_splits_it() {
        assert_eq!(enter("- ab", 3), Some(("- a\n- b".into(), 6)));
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        assert_eq!(enter("- a\n- ", 6), Some(("- a\n".into(), 4)));
        assert_eq!(enter("- a\n-", 5), Some(("- a\n".into(), 4)));
        assert_eq!(enter("- a\n- [ ] ", 10), Some(("- a\n".into(), 4)));
    }

    #[test]
    fn enter_outside_a_list_or_inside_the_marker_is_left_alone() {
        assert_eq!(enter("text", 4), None);
        assert_eq!(enter("**b**", 5), None);
        assert_eq!(enter("- a", 1), None);
    }

    #[test]
    fn continuation_uses_the_documents_line_ending() {
        assert_eq!(enter("- a\r\nb", 3), Some(("- a\r\n- \r\nb".into(), 7)));
        let plan = enter_in_list("- a", 3, "\r\n").unwrap();
        assert_eq!(plan.apply_to("- a"), "- a\r\n- ", "a one-line document uses the fallback");
    }

    #[test]
    fn tab_nests_by_the_marker_width_and_shift_tab_un_nests() {
        assert_eq!(nest("- a", 3, false), Some(("  - a".into(), 5)));
        assert_eq!(nest("1. a", 4, false), Some(("   1. a".into(), 7)));
        assert_eq!(nest("  - a", 5, true), Some(("- a".into(), 3)));
        assert_eq!(nest("- a", 3, true), None);
        assert_eq!(nest("text", 2, false), None);
    }
}
