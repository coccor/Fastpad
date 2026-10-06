//! Markdown writing helpers (live mode spec §8). Pure text → edit plans; the window layer
//! applies a plan as one undo step. Edits use pre-edit byte offsets; selections are post-edit.

// A plan's selections are ranges; a one-caret plan is legitimately a one-element Vec of them.
#![allow(clippy::single_range_in_vec_init)]

use std::ops::Range;

use crate::live::spans::pipe_positions;

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
    text.as_bytes()[..end]
        .iter()
        .rev()
        .take_while(|b| **b == byte)
        .count()
}

fn run_after(text: &str, start: usize, byte: u8) -> usize {
    text.as_bytes()[start..]
        .iter()
        .take_while(|b| **b == byte)
        .count()
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

/// Two edit ranges clash when they overlap; insertions at one position never clash.
fn edits_clash(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

pub fn toggle_marker(text: &str, selections: &[Range<usize>], marker: &str) -> EditPlan {
    enum Kind {
        Pair,
        Inside,
        Around,
        Wrap,
    }
    let width = marker.len();
    let byte = marker.as_bytes()[0];
    // (selection index, selection, target) in document order of the targets.
    let mut items: Vec<(usize, Range<usize>, Range<usize>)> = selections
        .iter()
        .enumerate()
        .map(|(index, selection)| {
            let selection = normalized(selection);
            let target = if selection.is_empty() {
                word_at(text, selection.start).unwrap_or(selection.clone())
            } else {
                selection.clone()
            };
            (index, selection, target)
        })
        .collect();
    items.sort_by_key(|(index, selection, target)| {
        (
            target.start,
            target.end,
            selection.start,
            selection.end,
            *index,
        )
    });
    let mut plan = EditPlan {
        edits: Vec::new(),
        selections: vec![0..0; selections.len()],
    };
    let mut delta: isize = 0;
    // Targets already planned, and the selections that clashed with them.
    let mut handled: Vec<Range<usize>> = Vec::new();
    let mut dropped: Vec<(usize, Range<usize>)> = Vec::new();
    for (index, selection, target) in items {
        let caret = selection.is_empty().then_some(selection.start);
        let inner = &text[target.clone()];
        let kind = if target.is_empty() {
            Kind::Pair
        } else if inner.len() >= 2 * width
            && run_matches(run_after(inner, 0, byte), width)
            && run_matches(run_before(inner, inner.len(), byte), width)
        {
            Kind::Inside
        } else if target.start >= width
            && run_matches(run_before(text, target.start, byte), width)
            && run_matches(run_after(text, target.end, byte), width)
        {
            Kind::Around
        } else {
            Kind::Wrap
        };
        let insert = |at: usize, text: String| TextEdit {
            range: at..at,
            text,
        };
        let remove = |range: Range<usize>| TextEdit {
            range,
            text: String::new(),
        };
        let edits = match kind {
            Kind::Pair => vec![insert(target.start, marker.repeat(2))],
            Kind::Inside => vec![
                remove(target.start..target.start + width),
                remove(target.end - width..target.end),
            ],
            Kind::Around => vec![
                remove(target.start - width..target.start),
                remove(target.end..target.end + width),
            ],
            Kind::Wrap => {
                vec![
                    insert(target.start, marker.to_owned()),
                    insert(target.end, marker.to_owned()),
                ]
            }
        };
        // Selections on one word, or on words that share a marker run, share one toggle.
        let clashes = handled
            .iter()
            .any(|done| *done == target || (target.start < done.end && done.start < target.end))
            || edits.iter().any(|new| {
                plan.edits
                    .iter()
                    .any(|old| edits_clash(&new.range, &old.range))
            });
        if clashes {
            dropped.push((index, selection));
            continue;
        }
        plan.edits.extend(edits);
        let result = match kind {
            Kind::Pair => {
                let at = shift(target.start, delta) + width;
                at..at
            }
            Kind::Inside => {
                let start = shift(target.start, delta);
                match caret {
                    Some(at) => {
                        let at = shift(at, delta).saturating_sub(width).max(start);
                        at..at
                    }
                    None => start..start + inner.len() - 2 * width,
                }
            }
            Kind::Around => match caret {
                Some(at) => {
                    let at = shift(at, delta) - width;
                    at..at
                }
                None => shift(target.start, delta) - width..shift(target.end, delta) - width,
            },
            Kind::Wrap => match caret {
                Some(at) => {
                    let at = shift(at, delta) + width;
                    at..at
                }
                None => shift(target.start, delta) + width..shift(target.end, delta) + width,
            },
        };
        delta += match kind {
            Kind::Pair | Kind::Wrap => 2 * width as isize,
            Kind::Inside | Kind::Around => -2 * (width as isize),
        };
        plan.selections[index] = result;
        handled.push(target);
    }
    plan.edits
        .sort_by_key(|edit| (edit.range.start, edit.range.end));
    // A dropped selection keeps its own place, moved by the edits that were planned.
    for (index, selection) in dropped {
        let start = map_position(&plan.edits, selection.start);
        plan.selections[index] = start..map_position(&plan.edits, selection.end);
    }
    plan
}

/// Where a pre-edit position lands: shifted by the edits wholly before it; an insertion at the
/// position itself stays after it, and a position inside a removal moves to its start.
fn map_position(edits: &[TextEdit], position: usize) -> usize {
    let mut mapped = position as isize;
    for edit in edits {
        let before = if edit.range.is_empty() {
            edit.range.start < position
        } else {
            edit.range.end <= position
        };
        if before {
            mapped += edit.text.len() as isize - edit.range.len() as isize;
        } else if edit.range.start < position {
            mapped -= (position - edit.range.start) as isize;
        }
    }
    mapped as usize
}

/// Two link spans clash when equal, overlapping, or when a caret touches a selection.
fn spans_clash(a: &Range<usize>, b: &Range<usize>) -> bool {
    let touches = |caret: &Range<usize>, span: &Range<usize>| {
        caret.is_empty() && span.start <= caret.start && caret.start <= span.end
    };
    a == b || (a.start < b.end && b.start < a.end) || touches(a, b) || touches(b, a)
}

pub fn insert_link(_text: &str, selections: &[Range<usize>]) -> EditPlan {
    let mut order: Vec<(usize, Range<usize>)> = selections
        .iter()
        .enumerate()
        .map(|(index, selection)| (index, normalized(selection)))
        .collect();
    order.sort_by_key(|(index, selection)| (selection.start, selection.end, *index));
    let mut plan = EditPlan {
        edits: Vec::new(),
        selections: vec![0..0; selections.len()],
    };
    let mut delta: isize = 0;
    let mut handled: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for (index, selection) in order {
        if let Some((_, result)) = handled
            .iter()
            .find(|(done, _)| spans_clash(done, &selection))
        {
            plan.selections[index] = result.clone();
            continue;
        }
        let at = if selection.is_empty() {
            plan.edits.push(TextEdit {
                range: selection.clone(),
                text: "[]()".into(),
            });
            shift(selection.start, delta) + 1
        } else {
            plan.edits.push(TextEdit {
                range: selection.start..selection.start,
                text: "[".into(),
            });
            plan.edits.push(TextEdit {
                range: selection.end..selection.end,
                text: "]()".into(),
            });
            shift(selection.end, delta) + 3
        };
        delta += 4;
        plan.selections[index] = at..at;
        handled.push((selection, at..at));
    }
    plan.edits
        .sort_by_key(|edit| (edit.range.start, edit.range.end));
    plan
}

pub(crate) fn line_bounds(text: &str, at: usize) -> Range<usize> {
    let start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[at..]
        .find('\n')
        .map_or(text.len(), |newline| at + newline);
    let end = if end > start && text.as_bytes()[end - 1] == b'\r' {
        end - 1
    } else {
        end
    };
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

/// The visual width of leading whitespace: a tab advances to the next multiple of 4.
fn columns(whitespace: &str) -> usize {
    whitespace.bytes().fold(0, |column, byte| {
        if byte == 9 {
            column + 4 - column % 4
        } else {
            column + 1
        }
    })
}

struct ListPrefix {
    /// Visual columns of the leading whitespace.
    indent: usize,
    /// Bytes of the leading whitespace.
    indent_len: usize,
    bullet: Option<u8>,
    number: Option<(u64, u8)>,
    task: bool,
    /// Bytes from the line start to the item text.
    len: usize,
    /// The visual column where the item's content starts (CommonMark: 1-4 spaces after the
    /// marker count, otherwise one); a child item is indented to at least this column.
    content: usize,
}

/// Three or more of the same `-`, `*` or `_`, spaces and tabs between them allowed.
fn is_thematic_break(line: &str) -> bool {
    let mut marks = line.bytes().filter(|b| !matches!(b, b' ' | b'\t'));
    let Some(first @ (b'-' | b'*' | b'_')) = marks.next() else {
        return false;
    };
    let mut count = 1;
    for mark in marks {
        if mark != first {
            return false;
        }
        count += 1;
    }
    count >= 3
}

fn list_prefix(line: &str) -> Option<ListPrefix> {
    if is_thematic_break(line) {
        return None;
    }
    let bytes = line.as_bytes();
    let indent_len = bytes.iter().take_while(|b| matches!(b, 32 | 9)).count();
    let indent = columns(&line[..indent_len]);
    let mut at = indent_len;
    let (bullet, number) = match *bytes.get(at)? {
        marker @ (b'-' | b'*' | b'+') => {
            at += 1;
            (Some(marker), None)
        }
        digit if digit.is_ascii_digit() => {
            let digits = bytes[at..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
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
    let marker_end = at;
    match bytes.get(at) {
        Some(b' ') => at += 1,
        None => {}
        Some(_) => return None,
    }
    let spaces = bytes[marker_end..]
        .iter()
        .take_while(|b| **b == b' ')
        .count();
    let gap = if (1..=4).contains(&spaces) && marker_end + spaces < line.len() {
        spaces
    } else {
        1
    };
    let content = indent + (marker_end - indent_len) + gap;
    let rest = &line[at..];
    let task = ["[ ]", "[x]", "[X]"]
        .iter()
        .any(|box_| rest.starts_with(box_) && (rest.len() == 3 || rest.as_bytes()[3] == b' '));
    if task {
        at = (at + 4).min(line.len());
    }
    Some(ListPrefix {
        indent,
        indent_len,
        bullet,
        number,
        task,
        len: at,
        content,
    })
}

/// `fallback` is the line ending for a document with none yet (the editor's EOL mode).
pub fn enter_in_list(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let prefix = list_prefix(&text[line.clone()])?;
    let prefix_end = line.start + prefix.len;
    if caret < prefix_end.min(line.end) {
        return None;
    }
    // A bare `-` right under a paragraph line is a setext underline, not an empty item.
    let bare = line.len() == prefix.len && !prefix.task && !text[line.clone()].ends_with(' ');
    if bare && line.start > 0 {
        let previous = line_bounds(text, line.start - 1);
        if !text[previous.clone()].trim().is_empty() && list_prefix(&text[previous]).is_none() {
            return None;
        }
    }
    if text[prefix_end.min(line.end)..line.end].trim().is_empty() {
        return Some(EditPlan {
            edits: vec![TextEdit {
                range: line.clone(),
                text: String::new(),
            }],
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
        &text[line.start..line.start + prefix.indent_len],
        marker,
        if prefix.task { "[ ] " } else { "" }
    );
    let after = caret + inserted.len();
    Some(EditPlan {
        edits: vec![TextEdit {
            range: caret..caret,
            text: inserted,
        }],
        selections: vec![after..after],
    })
}

/// An item as CommonMark nests it: indent and content start in visual columns, plus the marker
/// kind (bullet character or ordered delimiter); a different kind starts a new list.
#[derive(Clone, Copy)]
struct Item {
    indent: usize,
    content: usize,
    kind: u8,
}

impl Item {
    fn of(prefix: &ListPrefix) -> Self {
        let kind = prefix
            .bullet
            .or(prefix.number.map(|(_, delimiter)| delimiter))
            .unwrap_or(0);
        Self {
            indent: prefix.indent,
            content: prefix.content,
            kind,
        }
    }
}

/// Where the item on `line` sits in its list: its parent and its previous sibling (the nearest
/// earlier item at the same level in the same list).
struct Nesting {
    own: Item,
    /// The marker is 4+ columns past the containing item's content (or the margin), so
    /// CommonMark reads the line as text or code, not as an item.
    too_deep: bool,
    parent: Option<Item>,
    previous: Option<Item>,
    /// The previous sibling's last child list's item at this point, if any: what the line
    /// continues once nested.
    nephew: Option<Item>,
}

/// How many items of `open` (outermost first) contain a line indented to `indent`.
fn containing(open: &[Item], indent: usize) -> usize {
    open.iter()
        .rposition(|item| item.content <= indent)
        .map_or(0, |at| at + 1)
}

fn nesting(text: &str, line: Range<usize>) -> Option<Nesting> {
    // Start at an unindented item, or after an unindented paragraph line that follows a blank
    // line: no list is open there. Otherwise from the document start.
    let mut start = line.start;
    while start > 0 {
        let above = line_bounds(text, start - 1);
        let content = &text[above.clone()];
        let unindented = !content.is_empty() && !content.starts_with([' ', '\t']);
        if unindented && list_prefix(content).is_some() {
            start = above.start;
            break;
        }
        let after_blank =
            above.start == 0 || text[line_bounds(text, above.start - 1)].trim().is_empty();
        if unindented && after_blank {
            break;
        }
        start = above.start;
    }
    let mut open: Vec<Item> = Vec::new();
    let mut after_blank = true;
    let mut at = start;
    while at < line.start {
        let current = line_bounds(text, at);
        at = text[current.end..]
            .find('\n')
            .map_or(text.len(), |newline| current.end + newline + 1);
        let content = &text[current];
        if content.trim().is_empty() {
            after_blank = true;
            continue;
        }
        let indent = leading(content).1;
        let keep = containing(&open, indent);
        let base = keep.checked_sub(1).map_or(0, |parent| open[parent].content);
        match list_prefix(content) {
            Some(prefix) if prefix.indent < base + 4 => {
                open.truncate(keep);
                open.push(Item::of(&prefix));
            }
            _ if indent == 0 && breaks_lists(content) => {
                open.clear();
            }
            // A paragraph after a blank line closes the items that do not contain it; without
            // one it is a lazy continuation.
            _ if after_blank => open.truncate(keep),
            _ => {}
        }
        after_blank = false;
    }
    let own = Item::of(&list_prefix(&text[line])?);
    let keep = containing(&open, own.indent);
    let parent = keep.checked_sub(1).map(|parent| open[parent]);
    let too_deep = own.indent >= parent.map_or(0, |parent| parent.content) + 4;
    let previous = open
        .get(keep)
        .copied()
        .filter(|previous| previous.kind == own.kind);
    Some(Nesting {
        own,
        too_deep,
        parent,
        previous,
        nephew: open.get(keep + 1).copied(),
    })
}

/// Tab nests the item under its previous sibling (indent = the sibling's content column); on a
/// list's first item it is consumed without an edit, since CommonMark cannot nest that item.
/// Shift+Tab moves the item to its parent's indent, or a top-level item to the margin; a marker
/// indented too deep to be an item comes back to the shallowest item position there. An
/// ordered item that ends up starting a new list is renumbered to 1: only `1.` may start a list
/// right under paragraph text. The item's children move with it; after a Shift+Tab its later
/// siblings become its children.
pub fn indent_list_item(text: &str, caret: usize, outdent: bool) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let prefix = list_prefix(&text[line.clone()])?;
    let nesting = nesting(text, line.clone())?;
    let (new_indent, starts_a_list) = if nesting.too_deep {
        if !outdent {
            return None;
        }
        (
            nesting.parent.map_or(0, |parent| parent.content),
            nesting.previous.is_none(),
        )
    } else if outdent {
        if prefix.indent == 0 {
            return None;
        }
        match nesting.parent {
            Some(parent) => (parent.indent, parent.kind != nesting.own.kind),
            None => (0, false),
        }
    } else {
        let Some(previous) = nesting.previous else {
            return Some(EditPlan {
                edits: Vec::new(),
                selections: vec![caret..caret],
            });
        };
        (
            previous.content,
            nesting
                .nephew
                .is_none_or(|nephew| nephew.kind != nesting.own.kind),
        )
    };
    // The leading whitespace is replaced by spaces, so a tab never ends up after spaces.
    let mut replaced = prefix.indent_len;
    let mut new_prefix = " ".repeat(new_indent);
    if starts_a_list && prefix.number.is_some_and(|(value, _)| value != 1) {
        replaced += text[line.start + replaced..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        new_prefix.push('1');
    }
    let after = if caret >= line.start + replaced {
        caret + new_prefix.len() - replaced
    } else {
        line.start + new_indent
    };
    // The item's block (lines indented to its content, blank lines inside it) moves with it, by
    // the change of its content column, so its children keep their place under it.
    let renumber_shrink = (replaced - prefix.indent_len) - (new_prefix.len() - new_indent);
    let new_content = prefix.content - prefix.indent + new_indent - renumber_shrink;
    let mut edits = vec![TextEdit {
        range: line.start..line.start + replaced,
        text: new_prefix,
    }];
    let block_end = shift_lines(text, line.end, prefix.content, new_content, &mut edits);
    if outdent && !nesting.too_deep {
        match nesting.parent {
            // The rest of the parent's block (the item's later siblings) becomes the item's
            // children: lined up with its content instead of the parent's.
            Some(parent) => {
                let mut rest = block_end;
                // The first of them starts a new child list unless it continues the item's own
                // last child list; an ordered one is then renumbered to 1.
                let mut last_child: Option<Item> = None;
                for below in text[line.end..block_end].split('\n').skip(1) {
                    let Some(child) = list_prefix(below.trim_end_matches('\r')) else {
                        continue;
                    };
                    let child = Item::of(&child);
                    if last_child.is_none_or(|last| child.indent < last.content) {
                        last_child = Some(child);
                    }
                }
                let first = next_line(text, block_end, |content| !content.trim().is_empty());
                if let Some(first) = first
                    && let Some(sibling) = list_prefix(&text[first.clone()])
                    && sibling.indent >= parent.content
                    && sibling.number.is_some_and(|(value, _)| value != 1)
                    && last_child.is_none_or(|last| last.kind != Item::of(&sibling).kind)
                {
                    let digits = text[first.start + sibling.indent_len..]
                        .bytes()
                        .take_while(u8::is_ascii_digit)
                        .count();
                    let indent = sibling.indent - parent.content + new_content;
                    edits.push(TextEdit {
                        range: first.start..first.start + sibling.indent_len + digits,
                        text: format!("{}1", " ".repeat(indent)),
                    });
                    let content = sibling.content - sibling.indent + indent - (digits - 1);
                    rest = shift_lines(text, first.end, sibling.content, content, &mut edits);
                }
                shift_lines(text, rest, parent.content, new_content, &mut edits);
            }
            // At the top level the depth does not change; refuse when the next sibling would
            // end up inside the item.
            None => {
                let next = next_line(text, block_end, |content| !content.trim().is_empty());
                if next.is_some_and(|next| {
                    list_prefix(&text[next]).is_some_and(|item| item.indent >= new_content)
                }) {
                    return Some(EditPlan {
                        edits: Vec::new(),
                        selections: vec![caret..caret],
                    });
                }
            }
        }
    }
    Some(EditPlan {
        edits,
        selections: vec![after..after],
    })
}

/// The first line after the line ending at `line_end` that satisfies `wanted`.
fn next_line(
    text: &str,
    mut line_end: usize,
    wanted: impl Fn(&str) -> bool,
) -> Option<Range<usize>> {
    while let Some(newline) = text[line_end..].find('\n') {
        let below = line_bounds(text, line_end + newline + 1);
        if wanted(&text[below.clone()]) {
            return Some(below);
        }
        line_end = below.end;
    }
    None
}

/// Re-indents the lines after the line ending at `line_end` that are indented to `column` or
/// deeper (blank lines between them included) so that `column` lands on `new_column`, keeping
/// their indents relative to each other. Stops before the first non-blank line indented less;
/// returns the end of the last line it took.
fn shift_lines(
    text: &str,
    mut line_end: usize,
    column: usize,
    new_column: usize,
    edits: &mut Vec<TextEdit>,
) -> usize {
    let mut taken = line_end;
    while let Some(newline) = text[line_end..].find('\n') {
        let below = line_bounds(text, line_end + newline + 1);
        line_end = below.end;
        let content = &text[below.clone()];
        if content.trim().is_empty() {
            continue;
        }
        let (lead_len, lead) = leading(content);
        if lead < column {
            break;
        }
        taken = below.end;
        let pad = " ".repeat(lead - column + new_column);
        if content[..lead_len] != pad {
            edits.push(TextEdit {
                range: below.start..below.start + lead_len,
                text: pad,
            });
        }
    }
    taken
}

/// A line's leading whitespace: its length in bytes and its width in visual columns.
fn leading(line: &str) -> (usize, usize) {
    let len = line.len() - line.trim_start_matches([' ', '\t']).len();
    (len, columns(&line[..len]))
}

/// A column-0 line that ends every open list: an ATX heading, a block quote, a code fence or a
/// thematic break.
fn breaks_lists(line: &str) -> bool {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    let heading = (1..=6).contains(&hashes)
        && matches!(line.as_bytes().get(hashes), None | Some(b' ' | b'\t'));
    heading
        || line.starts_with('>')
        || line.starts_with("```")
        || line.starts_with("~~~")
        || is_thematic_break(line)
}

fn is_delimiter_row(line: &str) -> bool {
    let cells = split_cells(line);
    !cells.is_empty()
        && cells.iter().all(|(_, cell)| {
            let cell = cell.trim();
            let body = cell.trim_start_matches(':').trim_end_matches(':');
            !body.is_empty() && body.bytes().all(|b| b == b'-')
        })
}

/// A table line's cells: (range within the line, raw text), the outer pipes optional.
fn split_cells(line: &str) -> Vec<(Range<usize>, &str)> {
    let pipes = pipe_positions(line);
    if pipes.is_empty() {
        return Vec::new();
    }
    let trimmed_start = line.len() - line.trim_start().len();
    let trimmed_end = line.trim_end().len();
    let mut bounds: Vec<usize> = Vec::new();
    if pipes[0] != trimmed_start {
        bounds.push(trimmed_start);
    } else {
        bounds.push(pipes[0] + 1);
    }
    for pipe in &pipes {
        if *pipe != trimmed_start && *pipe + 1 != trimmed_end {
            bounds.push(*pipe);
            bounds.push(*pipe + 1);
        }
    }
    let last = *pipes.last().expect("non-empty");
    bounds.push(if last + 1 == trimmed_end {
        last
    } else {
        trimmed_end
    });
    bounds
        .chunks(2)
        .filter(|pair| pair.len() == 2 && pair[0] <= pair[1])
        .map(|pair| (pair[0]..pair[1], &line[pair[0]..pair[1]]))
        .collect()
}

fn table_lines(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    crate::live::spans::line_ranges(text, range)
}

/// The table containing `at`: whole lines, no final line ending.
pub fn table_at(text: &str, at: usize) -> Option<Range<usize>> {
    let is_row = |line: &Range<usize>| {
        let content = &text[line.clone()];
        !content.trim().is_empty() && !pipe_positions(content).is_empty()
    };
    let here = line_bounds(text, at);
    if !is_row(&here) {
        return None;
    }
    let mut first = here.clone();
    while first.start > 0 {
        let previous = line_bounds(text, first.start - 1);
        if !is_row(&previous) {
            break;
        }
        first = previous;
    }
    let mut last = here;
    loop {
        let next_start = text[last.end..]
            .find('\n')
            .map(|newline| last.end + newline + 1);
        let Some(next_start) = next_start.filter(|start| *start < text.len()) else {
            break;
        };
        let next = line_bounds(text, next_start);
        if !is_row(&next) {
            break;
        }
        last = next;
    }
    let lines = table_lines(text, first.start..last.end);
    (lines.len() >= 2 && is_delimiter_row(&text[lines[1].clone()])).then_some(first.start..last.end)
}

#[derive(Clone, Copy)]
enum Align {
    None,
    Left,
    Right,
    Center,
}

/// The table re-padded into aligned columns; `None` when it already is.
pub fn format_table(table: &str) -> Option<String> {
    let ending = if table.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = table
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|line| {
            split_cells(line)
                .into_iter()
                .map(|(_, cell)| cell.trim().to_owned())
                .collect()
        })
        .collect();
    let columns = rows.iter().map(Vec::len).max()?;
    let aligns: Vec<Align> = (0..columns)
        .map(|column| {
            let cell = rows
                .get(1)
                .and_then(|row| row.get(column))
                .map_or("", String::as_str);
            match (cell.starts_with(':'), cell.ends_with(':') && cell.len() > 1) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                (false, false) => Align::None,
            }
        })
        .collect();
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .enumerate()
                .filter(|(index, _)| *index != 1)
                .filter_map(|(_, row)| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
                .max(3)
        })
        .collect();
    let rendered: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let cells: Vec<String> = (0..columns)
                .map(|column| {
                    let width = widths[column];
                    if index == 1 {
                        return match aligns[column] {
                            Align::None => "-".repeat(width),
                            Align::Left => format!(":{}", "-".repeat(width - 1)),
                            Align::Right => format!("{}:", "-".repeat(width - 1)),
                            Align::Center => format!(":{}:", "-".repeat(width - 2)),
                        };
                    }
                    let cell = row.get(column).map_or("", String::as_str);
                    let pad = width - cell.chars().count();
                    match aligns[column] {
                        Align::Right => format!("{}{cell}", " ".repeat(pad)),
                        _ => format!("{cell}{}", " ".repeat(pad)),
                    }
                })
                .collect();
            format!("| {} |", cells.join(" | "))
        })
        .collect();
    let indent = &lines[0][..lines[0].len() - lines[0].trim_start().len()];
    let out = rendered
        .iter()
        .map(|row| format!("{indent}{row}"))
        .collect::<Vec<_>>()
        .join(ending);
    (out != table).then_some(out)
}

/// The content of the next (or previous) cell of the table around `caret`, delimiter row skipped.
pub fn next_cell(text: &str, caret: usize, back: bool) -> Option<Range<usize>> {
    let table = table_at(text, caret)?;
    // (line start, segment, trimmed content) of every cell outside the delimiter row.
    let mut cells: Vec<(usize, Range<usize>, Range<usize>)> = Vec::new();
    let mut caret_line = None;
    for (index, line) in table_lines(text, table).into_iter().enumerate() {
        if line.start <= caret && caret <= line.end {
            caret_line = Some(line.start);
        }
        if index == 1 {
            continue;
        }
        for (segment, raw) in split_cells(&text[line.clone()]) {
            let lead = raw.len() - raw.trim_start().len();
            let content_start = line.start + segment.start + lead;
            let content = content_start..content_start + raw.trim().len();
            cells.push((
                line.start,
                line.start + segment.start..line.start + segment.end,
                content,
            ));
        }
    }
    let current = cells
        .iter()
        .position(|(_, segment, _)| segment.start <= caret && caret <= segment.end)
        .or_else(|| {
            // Before the first pipe is the first cell, after the last pipe the last one.
            let on_line =
                |(line, _, _): &(usize, Range<usize>, Range<usize>)| Some(*line) == caret_line;
            let first = cells.iter().position(on_line)?;
            let last = cells.iter().rposition(on_line)?;
            Some(if caret < cells[first].1.start {
                first
            } else {
                last
            })
        })?;
    let target = if back {
        current.checked_sub(1)?
    } else {
        current + 1
    };
    cells.get(target).map(|(_, _, content)| content.clone())
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
        assert_eq!(
            run("a b c", &[2..3], "**"),
            ("a **b** c".into(), vec![4..5])
        );
    }

    #[test]
    fn unwraps_when_the_markers_are_inside_the_selection() {
        assert_eq!(
            run("a **b** c", &[2..7], "**"),
            ("a b c".into(), vec![2..3])
        );
    }

    #[test]
    fn unwraps_when_the_markers_surround_the_selection() {
        assert_eq!(
            run("a **b** c", &[4..5], "**"),
            ("a b c".into(), vec![2..3])
        );
    }

    #[test]
    fn an_empty_selection_toggles_the_word_under_the_caret() {
        assert_eq!(
            run("hello world", &[2..2], "**"),
            ("**hello** world".into(), vec![4..4])
        );
        assert_eq!(
            run("**hello** world", &[4..4], "**"),
            ("hello world".into(), vec![2..2])
        );
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
        assert_eq!(
            run("a b", &[0..0, 2..2], "**"),
            ("**a** **b**".into(), vec![2..2, 8..8])
        );
    }

    #[test]
    fn inline_code_uses_backticks() {
        assert_eq!(run("x y", &[2..3], "`"), ("x `y`".into(), vec![3..4]));
    }

    #[test]
    fn a_reversed_selection_is_handled() {
        let reversed = Range { start: 3, end: 2 };
        assert_eq!(
            run("a b c", &[reversed], "**"),
            ("a **b** c".into(), vec![4..5])
        );
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
        assert_eq!(
            plan.apply_to("- a"),
            "- a\r\n- ",
            "a one-line document uses the fallback"
        );
    }

    #[test]
    fn tab_nests_under_the_previous_sibling_and_shift_tab_un_nests() {
        assert_eq!(nest("- a\n- b", 7, false), Some(("- a\n  - b".into(), 9)));
        assert_eq!(
            nest("1. a\n2. b", 9, false),
            Some(("1. a\n   1. b".into(), 12))
        );
        assert_eq!(nest("  - a", 5, true), Some(("- a".into(), 3)));
        assert_eq!(nest("- a", 3, true), None);
        assert_eq!(nest("text", 2, false), None);
    }

    #[test]
    fn table_at_needs_a_delimiter_row() {
        let text = "x\n\n|a|b|\n|-|-|\n|c|d|\n\ny";
        let start = text.find("|a").unwrap();
        let end = text.find("|\n\ny").unwrap() + 1;
        assert_eq!(table_at(text, start + 1), Some(start..end));
        assert_eq!(table_at("|a|b|\n|c|d|", 1), None);
        assert_eq!(table_at(text, 0), None);
    }

    #[test]
    fn format_pads_columns_with_a_minimum_width_of_three() {
        assert_eq!(
            format_table("|a|bb|\n|-|-|\n|ccc|d|").as_deref(),
            Some("| a   | bb  |\n| --- | --- |\n| ccc | d   |")
        );
    }

    #[test]
    fn an_aligned_table_is_left_alone() {
        assert_eq!(
            format_table("| a   | bb  |\n| --- | --- |\n| ccc | d   |"),
            None
        );
    }

    #[test]
    fn alignment_colons_are_kept_and_right_columns_pad_left() {
        assert_eq!(
            format_table("|a|b|\n|:-|-:|\n|c|d|").as_deref(),
            Some("| a   |   b |\n| :-- | --: |\n| c   |   d |")
        );
    }

    #[test]
    fn padding_counts_characters_not_bytes() {
        assert_eq!(
            format_table("|é|b|\n|-|-|").as_deref(),
            Some("| é   | b   |\n| --- | --- |")
        );
    }

    #[test]
    fn crlf_tables_stay_crlf_and_short_rows_get_empty_cells() {
        assert_eq!(
            format_table("|a|b|\r\n|-|-|\r\n|c|").as_deref(),
            Some("| a   | b   |\r\n| --- | --- |\r\n| c   |     |")
        );
    }

    #[test]
    fn escaped_pipes_stay_in_their_cell() {
        assert_eq!(
            format_table("|a\\|b|c|\n|-|-|").as_deref(),
            Some("| a\\|b | c   |\n| ---- | --- |")
        );
    }

    #[test]
    fn tab_walks_cells_skipping_the_delimiter_row() {
        let text = "| a | b |\n| - | - |\n| c | d |";
        let at = |s: &str| text.find(s).unwrap();
        assert_eq!(next_cell(text, at("a"), false), Some(at("b")..at("b") + 1));
        assert_eq!(next_cell(text, at("b"), false), Some(at("c")..at("c") + 1));
        assert_eq!(next_cell(text, at("c"), true), Some(at("b")..at("b") + 1));
        assert_eq!(next_cell(text, at("d"), false), None);
    }

    #[test]
    fn tab_from_the_last_cell_does_not_enter_a_following_table() {
        let separated = "| a | b |\n| - | - |\n| c | d |\n\n| e | f |\n| - | - |\n| g | h |\n";
        let at = |text: &str, s: &str| text.find(s).unwrap();
        assert_eq!(next_cell(separated, at(separated, "d"), false), None);
        assert_eq!(next_cell(separated, at(separated, "e"), true), None);
        let with_text = "| a | b |\n| - | - |\n| c | d |\ntext\n| e | f |\n| - | - |\n| g | h |";
        assert_eq!(next_cell(with_text, at(with_text, "d"), false), None);
        assert_eq!(
            table_at(with_text, at(with_text, "d")),
            Some(0..with_text.find("\ntext").unwrap())
        );
    }

    #[test]
    fn cells_are_found_in_crlf_tables() {
        let text = "| a | b |\r\n| - | - |\r\n| c | d |\r\nafter";
        let at = |s: &str| text.find(s).unwrap();
        assert_eq!(next_cell(text, at("b"), false), Some(at("c")..at("c") + 1));
        assert_eq!(next_cell(text, at("d"), false), None);
        assert_eq!(
            table_at(text, at("c")),
            Some(0..text.find("\r\nafter").unwrap())
        );
    }

    #[test]
    fn carets_in_one_word_toggle_it_once() {
        assert_eq!(
            run("**hello**", &[3..3, 5..5], "**"),
            ("hello".into(), vec![1..1, 3..3])
        );
        assert_eq!(
            run("hello", &[1..1, 3..3], "**"),
            ("**hello**".into(), vec![3..3, 5..5])
        );
        assert_eq!(
            run("ab cd", &[0..4, 2..5], "**").0,
            "**ab c**d",
            "the overlapping selection is dropped, not toggled again"
        );
    }

    #[test]
    fn a_multi_byte_word_is_toggled_whole() {
        assert_eq!(run("é b", &[0..0], "**"), ("**é** b".into(), vec![2..2]));
        assert_eq!(
            run("héllo", &[3..3], "**"),
            ("**héllo**".into(), vec![5..5])
        );
    }

    #[test]
    fn thematic_breaks_are_not_list_items() {
        assert_eq!(enter("* * *", 5), None);
        assert_eq!(enter("- - -", 2), None);
        assert_eq!(nest("- - -", 5, false), None);
        assert_eq!(nest("***", 3, false), None);
    }

    #[test]
    fn a_setext_underline_is_not_an_empty_item() {
        assert_eq!(enter("Title\n-", 7), None);
    }

    #[test]
    fn ending_a_list_keeps_crlf() {
        assert_eq!(enter("- a\r\n- ", 7), Some(("- a\r\n".into(), 5)));
    }

    #[test]
    fn nesting_follows_the_parent_items_content_column() {
        assert_eq!(
            nest("1. a\n1. b", 9, false),
            Some(("1. a\n   1. b".into(), 12))
        );
        assert_eq!(
            nest("1. a\n   2. b", 12, true),
            Some(("1. a\n2. b".into(), 9)),
            "same list as the parent"
        );
        assert_eq!(
            nest("- a\n\n  2. b", 11, true),
            Some(("- a\n\n1. b".into(), 9)),
            "a new list starts at 1"
        );
        assert_eq!(
            nest("-   a\n- b", 9, false),
            Some(("-   a\n    - b".into(), 13)),
            "content after 3 spaces"
        );
        assert_eq!(
            nest("- a\n  - b\n  - c", 15, false),
            Some(("- a\n  - b\n    - c".into(), 17))
        );
        assert_eq!(nest("\t- a", 4, true), Some(("- a".into(), 3)));
    }

    #[test]
    fn tab_works_from_before_the_first_pipe_and_after_the_last() {
        let text = "| a | b |\n| - | - |\n| c | d |";
        let at = |s: &str| text.find(s).unwrap();
        assert_eq!(next_cell(text, 0, false), Some(at("b")..at("b") + 1));
        assert_eq!(next_cell(text, 9, false), Some(at("c")..at("c") + 1));
        assert_eq!(next_cell(text, 9, true), Some(at("a")..at("a") + 1));
        assert_eq!(next_cell(text, 0, true), None);
    }

    #[test]
    fn an_indented_table_keeps_its_indent() {
        assert_eq!(
            format_table("  |a|b|\n  |-|-|").as_deref(),
            Some("  | a   | b   |\n  | --- | --- |")
        );
    }

    fn assert_well_formed(text: &str, plan: &EditPlan, context: &str) {
        let mut at = 0;
        for edit in &plan.edits {
            assert!(
                edit.range.start >= at,
                "{context}: edits overlap or are unsorted: {plan:?}"
            );
            assert!(
                edit.range.start <= edit.range.end && edit.range.end <= text.len(),
                "{context}"
            );
            assert!(
                text.is_char_boundary(edit.range.start) && text.is_char_boundary(edit.range.end)
            );
            at = edit.range.end;
        }
        let out = plan.apply_to(text);
        for selection in &plan.selections {
            assert!(
                selection.start <= selection.end && selection.end <= out.len(),
                "{context}: {plan:?}"
            );
            assert!(
                out.is_char_boundary(selection.start) && out.is_char_boundary(selection.end),
                "{context}"
            );
        }
    }

    #[test]
    fn words_sharing_a_marker_run_toggle_once() {
        let carets = toggle_marker("**a**b**", &[2..2, 5..5], "**");
        assert_well_formed("**a**b**", &carets, "carets");
        assert_eq!(carets.apply_to("**a**b**"), "ab**");
        assert_eq!(carets.selections, vec![0..0, 1..1]);
        let selected = toggle_marker("**a**b**", &[2..3, 5..6], "**");
        assert_well_formed("**a**b**", &selected, "selections");
        assert_eq!(selected.apply_to("**a**b**"), "ab**");
        assert_eq!(selected.selections, vec![0..1, 1..2]);
    }

    #[test]
    fn touching_targets_give_selections_independent_of_input_order() {
        for selections in [[1..3, 1..1], [1..1, 1..3]] {
            let plan = toggle_marker("x  y", &selections, "**");
            assert_eq!(plan.apply_to("x  y"), "**x****  **y");
            let caret = selections.iter().position(Range::is_empty).unwrap();
            assert_eq!(plan.selections[caret], 3..3);
            assert_eq!(plan.selections[1 - caret], 7..9);
        }
    }

    #[test]
    fn overlapping_links_share_one_insertion() {
        for (text, selections, expected, caret) in [
            ("abcd", vec![0..4, 2..2], "[abcd]()", 7),
            ("abcde", vec![0..4, 2..5], "[abcd]()e", 7),
            ("abcd", vec![0..4, 0..4], "[abcd]()", 7),
            ("abcd", vec![0..4, 0..0], "[]()abcd", 1),
        ] {
            let plan = insert_link(text, &selections);
            assert_well_formed(text, &plan, text);
            assert_eq!(plan.apply_to(text), expected);
            assert_eq!(plan.selections, vec![caret..caret; 2]);
        }
    }

    #[test]
    fn tab_on_the_first_item_of_a_list_is_consumed_without_an_edit() {
        let consumed = |text: &str, caret: usize| {
            let plan = indent_list_item(text, caret, false).unwrap();
            plan.edits.is_empty() && plan.selections == vec![caret..caret]
        };
        assert!(consumed("- a", 3));
        assert!(consumed("1. a", 4));
        assert!(consumed("- a\n  - b", 9), "first item of a sublist");
        assert!(
            consumed("1. a\n- b", 8),
            "a different marker starts a new list"
        );
        assert!(
            consumed("- a\n\nText\n\n- b", 14),
            "a paragraph after a blank line ends the list"
        );
    }

    #[test]
    fn tab_on_a_nested_item_stays_a_list_item() {
        let text = "- a\n\t- b\n\t\t- c";
        assert_eq!(
            nest(text, text.len(), false),
            Some((text.into(), text.len())),
            "c is b's first child"
        );
        let text = "- a\n\t- b\n\t\t\t- c";
        assert_eq!(
            nest(text, text.len(), false),
            None,
            "c is 4+ columns past b's content: text, not an item"
        );
        assert_eq!(
            nest(text, text.len(), true),
            Some(("- a\n\t- b\n      - c".into(), text.len() + 3))
        );
        assert_eq!(nest("\t- a", 4, false), None, "an indented code block");
        let text = "- a\n\t- b\n\t- c";
        assert_eq!(
            nest(text, text.len(), false),
            Some(("- a\n\t- b\n      - c".into(), text.len() + 5))
        );
        assert_eq!(
            nest("- a\n\t- b", 8, false),
            Some(("- a\n\t- b".into(), 8)),
            "b is the first child"
        );
        let text = "- a\n  - b\n  - c";
        assert_eq!(
            nest(text, text.len(), false),
            Some(("- a\n  - b\n    - c".into(), text.len() + 2))
        );
        let nested = "- a\n  - b\n    - c";
        assert_eq!(
            nest(nested, nested.len(), false),
            Some((nested.into(), nested.len())),
            "no sibling left"
        );
    }

    #[test]
    fn a_tab_indented_item_un_nests_to_a_space_indented_parent() {
        let text = "  - a\n\t- b";
        assert_eq!(
            nest(text, text.len(), true),
            Some(("  - a\n  - b".into(), text.len() + 1))
        );
        assert_eq!(
            nest("- a\nlazy\n  - b", 14, true),
            Some(("- a\nlazy\n- b".into(), 12))
        );
    }

    /// Each label's list nesting depth (0 when the label does not start a list item) and
    /// whether its item is the first of its list.
    fn item_depths(text: &str) -> std::collections::HashMap<String, (usize, bool)> {
        use pulldown_cmark::{Event, Parser, Tag, TagEnd};
        let mut depths = std::collections::HashMap::new();
        let (mut depth, mut fresh, mut first) = (0, false, false);
        for event in Parser::new_ext(text, crate::preview::model::PARSE_OPTIONS) {
            match event {
                Event::Start(Tag::List(_)) => first = true,
                Event::Start(Tag::Item) => {
                    depth += 1;
                    fresh = true;
                }
                Event::End(TagEnd::Item) => {
                    depth -= 1;
                    first = false;
                }
                Event::Text(label) => {
                    depths.insert(
                        label.to_string(),
                        if fresh { (depth, first) } else { (0, false) },
                    );
                    fresh = false;
                }
                _ => {}
            }
        }
        depths
    }

    #[test]
    fn a_moved_item_takes_its_children_along() {
        let text = "- a\n- b\n  - c";
        assert_eq!(
            nest(text, 7, false),
            Some(("- a\n  - b\n    - c".into(), 9))
        );
        let text = "- a\n\t- b\n\t\t- c";
        assert_eq!(nest(text, 8, true), Some(("- a\n- b\n    - c".into(), 7)));
        let text = "- a\r\n- b\r\n\r\n  more\r\n  - c\r\n- d";
        assert_eq!(
            nest(text, 8, false),
            Some(("- a\r\n  - b\r\n\r\n    more\r\n    - c\r\n- d".into(), 10)),
            "blank lines inside the block, CRLF; the next sibling stays"
        );
        assert_eq!(
            nest("1. a\n10. b\n    - c", 9, true),
            None,
            "top level, nothing to un-nest"
        );
        assert_eq!(
            nest("- a\n  - b\n  - c", 9, true),
            Some(("- a\n- b\n  - c".into(), 7)),
            "c becomes b's child"
        );
        assert_eq!(
            nest("- a\n   1. b\n   2. c", 11, true),
            Some(("- a\n1. b\n    1. c".into(), 8)),
            "c starts b's child list, keeping its extra space"
        );
        assert_eq!(nest("  - a\n- b", 5, true), Some(("- a\n- b".into(), 3)));
        assert_eq!(
            nest("  - a\n  - b", 5, true),
            Some(("  - a\n  - b".into(), 5)),
            "b would end up inside a"
        );
        let text = "1. a\n\n   10. b\n       - c";
        assert_eq!(
            nest(text, 14, true),
            Some(("1. a\n\n10. b\n    - c".into(), 11)),
            "the children follow the content column"
        );
        let text = "- a\n\n  10. b\n      - c";
        assert_eq!(
            nest(text, 12, true),
            Some(("- a\n\n1. b\n   - c".into(), 9)),
            "renumbered to 1: the children follow the narrower marker"
        );
    }

    #[test]
    fn only_real_headings_quotes_and_fences_end_a_list() {
        assert_eq!(
            nest("- a\n#tag\n- b", 12, false),
            Some(("- a\n#tag\n  - b".into(), 14))
        );
        let consumed = |text: &str| {
            indent_list_item(text, text.len(), false)
                .unwrap()
                .edits
                .is_empty()
        };
        assert!(consumed("- a\n# Title\n- b"));
        assert!(consumed("- a\n> quote\n- b"));
        assert!(consumed("- a\n```\n- b"));
    }

    #[test]
    fn tab_and_shift_tab_change_the_parsed_depth_by_one() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(state >> 33).unwrap() % bound
        };
        let label = |index: usize| char::from(b'a' + u8::try_from(index).unwrap()).to_string();
        for _ in 0..1500 {
            // A list's indent is its parent's content column (0 at the top) plus 0-1 spaces,
            // written with spaces or with tabs; ordered items count up from 1 within their list.
            // Some items are preceded by a blank line.
            let mut lines: Vec<String> = Vec::new();
            let mut item_lines = Vec::new();
            let mut open: Vec<(usize, &str, usize)> = Vec::new(); // (content column, marker, number)
            for index in 0..2 + next(7) {
                let depth = if index == 0 {
                    1
                } else {
                    1 + next((open.len() + 1).min(3))
                };
                let base = if depth == 1 { 0 } else { open[depth - 2].0 };
                let marker = ["-", "*", "1."][next(3)];
                let number = if open.len() >= depth && open[depth - 1].1 == marker {
                    open[depth - 1].2 + 1
                } else {
                    1
                };
                open.truncate(depth - 1);
                let column = base + next(2);
                let lead = if next(2) == 0 {
                    " ".repeat(column)
                } else {
                    format!("{}{}", "\t".repeat(column / 4), " ".repeat(column % 4))
                };
                let marker_text = if marker == "1." {
                    format!("{number}.")
                } else {
                    marker.to_string()
                };
                open.push((column + marker_text.len() + 1, marker, number));
                if index > 0 && next(6) == 0 {
                    lines.push(String::new());
                }
                item_lines.push(lines.len());
                lines.push(format!("{lead}{marker_text} {}", label(index)));
            }
            let text = lines.join("\n");
            let before = item_depths(&text);
            let depths: Vec<usize> = (0..item_lines.len())
                .map(|index| before[&label(index)].0)
                .collect();
            // After an edit to item `index` whose own depth went to `moved`: its descendants keep
            // their depth relative to it, every other item keeps its depth.
            let check = |out: &str, index: usize, moved: usize, what: &str| {
                let after = item_depths(out);
                let after: Vec<usize> = (0..depths.len())
                    .map(|other| after.get(&label(other)).map_or(0, |depth| depth.0))
                    .collect();
                assert_eq!(
                    after[index],
                    moved,
                    "{what} on {}: {text:?} -> {out:?}",
                    label(index)
                );
                let block_end = (index + 1..depths.len())
                    .find(|other| depths[*other] <= depths[index])
                    .unwrap_or(depths.len());
                for other in 0..depths.len() {
                    let expected = if other == index {
                        moved
                    } else if (index + 1..block_end).contains(&other) {
                        depths[other] + moved - depths[index]
                    } else {
                        depths[other]
                    };
                    assert_eq!(
                        after[other],
                        expected,
                        "{what} on {}, item {}: {text:?} -> {out:?}",
                        label(index),
                        label(other)
                    );
                }
            };
            for (index, line) in item_lines.iter().enumerate() {
                let (depth, first) = before[&label(index)];
                assert!(depth >= 1, "{text:?}: {} should be an item", label(index));
                let caret = lines[..=*line]
                    .iter()
                    .map(|line| line.len() + 1)
                    .sum::<usize>()
                    - 1;

                let plan = indent_list_item(&text, caret, false).unwrap();
                assert_well_formed(&text, &plan, &text);
                if plan.edits.is_empty() {
                    assert!(
                        first,
                        "Tab on {} in {text:?} was a no-op but it has a previous sibling",
                        label(index)
                    );
                    assert_eq!(plan.selections, vec![caret..caret]);
                } else {
                    check(&plan.apply_to(&text), index, depth + 1, "Tab");
                }

                match indent_list_item(&text, caret, true) {
                    None => assert_eq!(
                        depth,
                        1,
                        "Shift+Tab on {} in {text:?} did nothing",
                        label(index)
                    ),
                    Some(plan) => {
                        assert_well_formed(&text, &plan, &text);
                        check(
                            &plan.apply_to(&text),
                            index,
                            (depth - 1).max(1),
                            "Shift+Tab",
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn random_selections_always_give_well_formed_plans() {
        let pieces = ["a", "b", " ", "*", "`", "é", "_", "**"];
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(state >> 33).unwrap() % bound
        };
        for _ in 0..3000 {
            let text: String = (0..1 + next(8))
                .map(|_| pieces[next(pieces.len())])
                .collect();
            let boundaries: Vec<usize> = (0..=text.len())
                .filter(|at| text.is_char_boundary(*at))
                .collect();
            let selections: Vec<Range<usize>> = (0..1 + next(3))
                .map(|_| boundaries[next(boundaries.len())]..boundaries[next(boundaries.len())])
                .collect();
            for marker in ["**", "*", "`"] {
                let plan = toggle_marker(&text, &selections, marker);
                assert_well_formed(&text, &plan, &format!("{text:?} {selections:?} {marker}"));
            }
            let plan = insert_link(&text, &selections);
            assert_well_formed(&text, &plan, &format!("link {text:?} {selections:?}"));
        }
    }
}
