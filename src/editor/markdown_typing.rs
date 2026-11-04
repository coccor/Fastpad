//! Markdown typing speed (2026-11-04): what a typed bracket, backtick or emphasis marker does, a
//! URL pasted over text, Backspace in an empty pair, Home on a list item, Enter after a code fence
//! or in a block quote, the heading and list commands, table rows and columns, and the queries the
//! `[[`, `#tag` and `/` lists complete. Pure text → edit plans, like `markdown_edit`: edits use
//! pre-edit byte offsets, selections are post-edit.

// A plan's selections are ranges; a one-caret plan is legitimately a one-element Vec of them.
#![allow(clippy::single_range_in_vec_init)]

use super::markdown_edit::{
    EditPlan, ListPrefix, TextEdit, fence_line, format_table, is_word_char, leading, line_bounds,
    line_ending, line_ranges, list_prefix, next_cell, normalized, split_cells, table_at,
};
use std::ops::Range;

fn insert(at: usize, text: impl Into<String>) -> TextEdit {
    TextEdit {
        range: at..at,
        text: text.into(),
    }
}

fn caret_plan(edits: Vec<TextEdit>, caret: usize) -> EditPlan {
    EditPlan {
        edits,
        selections: vec![caret..caret],
    }
}

// ---------------------------------------------------------------------------------------------
// Typing

/// What typing `ch` does with one `selection` (normalized or not):
/// - over a one-line selection, `*`, `_`, `~` and `` ` `` wrap it in the marker and `[`, `(` in
///   the pair, keeping the text selected (so `*` twice makes it bold);
/// - with a caret, `(`, `[` and `` ` `` add their closer when nothing but space or closing
///   punctuation follows, and `)`, `]`, `` ` `` step over the closer an opener on the line left
///   open.
///
/// `None` lets the editor insert the character itself.
pub fn type_char(text: &str, selection: Range<usize>, ch: char) -> Option<EditPlan> {
    let selection = normalized(&selection);
    if !selection.is_empty() {
        let close = match ch {
            '*' | '_' | '~' | '`' => ch,
            '[' => ']',
            '(' => ')',
            _ => return None,
        };
        if text[selection.clone()].contains('\n') {
            return None;
        }
        return Some(EditPlan {
            edits: vec![insert(selection.start, ch), insert(selection.end, close)],
            selections: vec![selection.start + 1..selection.end + 1],
        });
    }
    let caret = selection.start;
    let line = line_bounds(text, caret);
    let before = &text[line.start..caret];
    let next = text[caret..line.end].chars().next();
    let previous = before.chars().next_back();
    let free_after = next.is_none_or(|c| {
        c.is_whitespace() || matches!(c, ')' | ']' | '}' | '.' | ',' | ';' | ':' | '!' | '?')
    });
    let ticks_before = before.matches('`').count();
    let step_over = caret_plan(Vec::new(), caret + 1);
    match ch {
        ')' | ']'
            if next == Some(ch) && unmatched(before, if ch == ')' { '(' } else { '[' }, ch) =>
        {
            Some(step_over)
        }
        '`' if next == Some('`') && ticks_before % 2 == 1 => Some(step_over),
        '(' | '[' if free_after => Some(caret_plan(
            vec![insert(
                caret,
                format!("{ch}{}", if ch == '(' { ')' } else { ']' }),
            )],
            caret + 1,
        )),
        '`' if free_after
            && ticks_before.is_multiple_of(2)
            && previous.is_none_or(|c| c != '`' && !is_word_char(c)) =>
        {
            Some(caret_plan(vec![insert(caret, "``")], caret + 1))
        }
        _ => None,
    }
}

/// Whether `before` opens more `open` than it closes with `close`.
fn unmatched(before: &str, open: char, close: char) -> bool {
    before.matches(open).count() > before.matches(close).count()
}

/// Backspace with the caret between an empty `()`, `[]` or `` `` `` removes both.
pub fn backspace_pair(text: &str, caret: usize) -> Option<EditPlan> {
    let previous = text[..caret].chars().next_back()?;
    let next = text[caret..].chars().next()?;
    matches!((previous, next), ('(', ')') | ('[', ']') | ('`', '`')).then(|| {
        caret_plan(
            vec![TextEdit {
                range: caret - 1..caret + 1,
                text: String::new(),
            }],
            caret - 1,
        )
    })
}

/// Whether `text` is one URL: a scheme FastPad links (`http`, `https`, `mailto`, `ftp`, `file`)
/// and no whitespace.
pub fn is_url(text: &str) -> bool {
    let text = text.trim();
    !text.contains(char::is_whitespace)
        && ["http://", "https://", "mailto:", "ftp://", "file://"]
            .iter()
            .any(|scheme| {
                text.len() > scheme.len()
                    && text.is_char_boundary(scheme.len())
                    && text[..scheme.len()].eq_ignore_ascii_case(scheme)
            })
}

/// Pasting the URL `clip` over a one-line selection that isn't a URL itself makes it a link:
/// `[selection](url)`. The caret ends after the link.
pub fn link_from_paste(text: &str, selection: Range<usize>, clip: &str) -> Option<EditPlan> {
    let selection = normalized(&selection);
    let selected = &text[selection.clone()];
    if selection.is_empty() || selected.contains('\n') || is_url(selected) || !is_url(clip) {
        return None;
    }
    let url = clip.trim();
    let url = if url.contains(['(', ')']) {
        format!("<{url}>")
    } else {
        url.to_owned()
    };
    let link = format!("[{selected}]({url})");
    let caret = selection.start + link.len();
    Some(caret_plan(
        vec![TextEdit {
            range: selection,
            text: link,
        }],
        caret,
    ))
}

// ---------------------------------------------------------------------------------------------
// Line prefixes: block quotes, headings, where a line's text starts

/// The block quote markers starting `line`, at most `max_levels` of them: their bytes (the space
/// after each `>` included) and how many there are. Up to three spaces may come before a `>`.
pub(crate) fn quote_prefix_upto(line: &str, max_levels: usize) -> (usize, usize) {
    let bytes = line.as_bytes();
    let (mut at, mut levels) = (0, 0);
    while levels < max_levels {
        let spaces = bytes[at..].iter().take_while(|b| **b == b' ').count();
        if spaces > 3 || bytes.get(at + spaces) != Some(&b'>') {
            break;
        }
        at += spaces + 1;
        if bytes.get(at) == Some(&b' ') {
            at += 1;
        }
        levels += 1;
    }
    (at, levels)
}

pub(crate) fn quote_prefix(line: &str) -> (usize, usize) {
    quote_prefix_upto(line, usize::MAX)
}

/// An ATX heading's marker on `line`: the bytes up to its text and its level.
pub(crate) fn heading_prefix(line: &str) -> Option<(usize, usize)> {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let hashes = line[indent..].bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = indent + hashes;
    match line.as_bytes().get(after) {
        None => Some((after, hashes)),
        Some(b' ' | b'\t') => {
            let spaces = line[after..]
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            Some((after + spaces, hashes))
        }
        _ => None,
    }
}

/// Where the text of `line` starts after its quote markers, list marker (and task box) or heading
/// marker.
pub(crate) fn content_start(line: &str) -> usize {
    let (quote, _) = quote_prefix(line);
    let rest = &line[quote..];
    if let Some(item) = list_prefix(rest) {
        return quote + item.len.min(rest.len());
    }
    if let Some((heading, _)) = heading_prefix(rest) {
        return quote + heading;
    }
    quote
}

/// Home on a quote, list item or heading line: to where its text starts, or from there to the line
/// start. `None` on other lines (the editor's own Home).
pub fn home_target(text: &str, caret: usize) -> Option<usize> {
    let line = line_bounds(text, caret);
    let content = line.start + content_start(&text[line.clone()]);
    (content != line.start).then_some(if caret == content {
        line.start
    } else {
        content
    })
}

// ---------------------------------------------------------------------------------------------
// Enter

/// Enter at the end of a line opening a code fence that nothing closes: the closing fence comes
/// too, with the caret on the empty line between.
pub fn enter_after_fence(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    if caret != line.end {
        return None;
    }
    let content = &text[line.clone()];
    let (fence, run, _) = fence_line(content)?;
    // The fences below, paired from this one: still open at the end means it has no closer (or
    // took one from a block below, which then has none).
    let mut open = Some((fence, run));
    let mut line_end = line.end;
    while let Some(newline) = text[line_end..].find('\n') {
        let below = line_bounds(text, line_end + newline + 1);
        line_end = below.end;
        if let Some((other, length, bare)) = fence_line(&text[below]) {
            open = match open {
                Some((open_fence, open_run))
                    if other == open_fence && length >= open_run && bare =>
                {
                    None
                }
                Some(open) => Some(open),
                None => Some((other, length)),
            };
        }
    }
    open?;
    let indent = &content[..leading(content).0];
    let fence_text = &content[indent.len()..indent.len() + run];
    let eol = line_ending(text, line.end, fallback);
    let at = caret + eol.len() + indent.len();
    Some(caret_plan(
        vec![insert(
            caret,
            format!("{eol}{indent}{eol}{indent}{fence_text}"),
        )],
        at,
    ))
}

/// The marker a list item continues with on the next line: a bullet as it is, a number one up.
fn next_marker(item: &ListPrefix) -> Option<String> {
    Some(match (item.bullet, item.number) {
        (Some(bullet), _) => char::from(bullet).to_string(),
        (None, Some((value, delimiter))) => format!("{}{}", value + 1, char::from(delimiter)),
        (None, None) => return None,
    })
}

/// Enter in a block quote: the next line continues it, with its list item when the quote holds
/// one. On an empty quote line the quote loses a level (and ends after its last); on an empty item
/// inside one the item's marker goes, the quote stays.
pub fn enter_in_quote(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let content = &text[line.clone()];
    let (len, levels) = quote_prefix(content);
    if levels == 0 || caret < line.start + len {
        return None;
    }
    let rest = &content[len..];
    let eol = line_ending(text, line.end, fallback);
    let mut quote = content[..len].to_owned();
    if !quote.ends_with(' ') {
        quote.push(' ');
    }
    let replace_line = |with: String| {
        let caret = line.start + with.len();
        caret_plan(
            vec![TextEdit {
                range: line.clone(),
                text: with,
            }],
            caret,
        )
    };
    if let Some(item) = list_prefix(rest) {
        let item_end = line.start + len + item.len.min(rest.len());
        if caret < item_end {
            return None;
        }
        if rest[item.len.min(rest.len())..].trim().is_empty() {
            return Some(replace_line(quote));
        }
        let inserted = format!(
            "{eol}{quote}{}{} {}",
            &rest[..item.indent_len],
            next_marker(&item)?,
            if item.task { "[ ] " } else { "" }
        );
        return Some(caret_plan(
            vec![insert(caret, inserted.clone())],
            caret + inserted.len(),
        ));
    }
    if rest.trim().is_empty() {
        let (outer, _) = quote_prefix_upto(content, levels - 1);
        return Some(replace_line(content[..outer].to_owned()));
    }
    let inserted = format!("{eol}{quote}");
    Some(caret_plan(
        vec![insert(caret, inserted.clone())],
        caret + inserted.len(),
    ))
}

// ---------------------------------------------------------------------------------------------
// The heading and list commands

/// A line's prefix rewritten: `old` bytes from `start` become `new`.
struct Rewrite {
    start: usize,
    old: usize,
    new: String,
}

/// The lines `selections` touch, in order, each once. A selection ending at a line's start
/// doesn't touch that line.
fn touched_lines(text: &str, selections: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut lines: Vec<Range<usize>> = selections
        .iter()
        .flat_map(|selection| line_ranges(text, normalized(selection)))
        .collect();
    lines.sort_by_key(|line| line.start);
    lines.dedup_by_key(|line| line.start);
    lines
}

/// The plan for `rewrites` (ascending, one per line). A caret, or a selection's end, inside or
/// right after a rewritten prefix lands after the new one; a selection's start there stays in
/// front of it, so selected lines stay selected whole. Any other position moves with the text.
fn rewrite_plan(selections: &[Range<usize>], rewrites: Vec<Rewrite>) -> EditPlan {
    let map = |position: usize, before_prefix: bool| {
        let mut delta: isize = 0;
        for rewrite in &rewrites {
            if position > rewrite.start + rewrite.old {
                delta += rewrite.new.len() as isize - rewrite.old as isize;
            } else if position >= rewrite.start {
                let start = (rewrite.start as isize + delta) as usize;
                return if before_prefix {
                    start
                } else {
                    start + rewrite.new.len()
                };
            } else {
                break;
            }
        }
        (position as isize + delta) as usize
    };
    let selections = selections
        .iter()
        .map(|selection| {
            let selection = normalized(selection);
            map(selection.start, !selection.is_empty())..map(selection.end, false)
        })
        .collect();
    EditPlan {
        edits: rewrites
            .into_iter()
            .map(|rewrite| TextEdit {
                range: rewrite.start..rewrite.start + rewrite.old,
                text: rewrite.new,
            })
            .collect(),
        selections,
    }
}

/// The lines a line command changes: every touched line, blank ones too when there is only one.
fn command_lines(text: &str, selections: &[Range<usize>]) -> Vec<Range<usize>> {
    let lines = touched_lines(text, selections);
    if lines.len() == 1 {
        return lines;
    }
    lines
        .into_iter()
        .filter(|line| !text[line.clone()].trim().is_empty())
        .collect()
}

/// Makes the touched lines headings of `level` (1–6), or paragraphs for 0; lines already all at
/// `level` become paragraphs. A quote's markers stay in front.
pub fn set_heading(text: &str, selections: &[Range<usize>], level: usize) -> EditPlan {
    let lines = command_lines(text, selections);
    let level_of = |line: &Range<usize>| {
        let content = &text[line.clone()];
        heading_prefix(&content[quote_prefix(content).0..]).map(|(_, level)| level)
    };
    let toggle_off = level > 0 && lines.iter().all(|line| level_of(line) == Some(level));
    let new = if level == 0 || toggle_off {
        String::new()
    } else {
        format!("{} ", "#".repeat(level))
    };
    let rewrites = lines
        .iter()
        .filter_map(|line| {
            let content = &text[line.clone()];
            let quote = quote_prefix(content).0;
            let rest = &content[quote..];
            let old = heading_prefix(rest).map_or(0, |(len, _)| len);
            (rest[..old] != new).then(|| Rewrite {
                start: line.start + quote,
                old,
                new: new.clone(),
            })
        })
        .collect();
    rewrite_plan(selections, rewrites)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListKind {
    Bullet,
    Numbered,
    Task,
}

/// A touched line as a list command sees it: after its quote markers, its item if it is one.
struct ListLine<'a> {
    start: usize,
    rest: &'a str,
    item: Option<ListPrefix>,
}

impl ListLine<'_> {
    /// The task box with the space after it (`[ ] `), or `""`.
    fn task_box(&self) -> &str {
        match &self.item {
            Some(item) if item.task => {
                let prefix = &self.rest[..item.len.min(self.rest.len())];
                prefix.rfind('[').map_or("", |at| &prefix[at..])
            }
            _ => "",
        }
    }

    fn indent(&self) -> &str {
        &self.rest[..leading(self.rest).0]
    }

    fn prefix_len(&self) -> usize {
        self.item
            .as_ref()
            .map_or(self.indent().len(), |item| item.len.min(self.rest.len()))
    }

    fn checked(&self) -> bool {
        let task = self.task_box();
        task.starts_with("[x]") || task.starts_with("[X]")
    }
}

/// Ctrl+Shift+8 (`Bullet`), Ctrl+Shift+7 (`Numbered`), Ctrl+Shift+9 (`Task`) on the touched lines:
/// - lines already all of that kind lose their markers (tasks instead tick, or untick when every
///   one is ticked);
/// - otherwise each becomes an item of that kind, keeping its indent and task box; numbers count
///   from 1, or on from a numbered item right above a single line.
pub fn toggle_list(text: &str, selections: &[Range<usize>], kind: ListKind) -> EditPlan {
    let lines: Vec<ListLine<'_>> = command_lines(text, selections)
        .into_iter()
        .map(|line| {
            let content = &text[line.clone()];
            let quote = quote_prefix(content).0;
            let rest = &content[quote..];
            ListLine {
                start: line.start + quote,
                rest,
                item: list_prefix(rest),
            }
        })
        .collect();
    let all = |wanted: &dyn Fn(&ListLine<'_>) -> bool| lines.iter().all(wanted);
    let is_task = |line: &ListLine<'_>| line.item.as_ref().is_some_and(|item| item.task);
    let rewrite = |line: &ListLine<'_>, new: String| {
        let old = line.prefix_len();
        (line.rest[..old] != new).then_some(Rewrite {
            start: line.start,
            old,
            new,
        })
    };
    let rewrites: Vec<Rewrite> = match kind {
        ListKind::Task if all(&is_task) => {
            let tick = !all(&|line| line.checked());
            lines
                .iter()
                .filter_map(|line| {
                    let old = line.prefix_len();
                    let prefix = &line.rest[..old];
                    let at = prefix.rfind('[')?;
                    let new = format!(
                        "{}[{}]{}",
                        &prefix[..at],
                        if tick { 'x' } else { ' ' },
                        &prefix[at + 3..]
                    );
                    rewrite(line, new)
                })
                .collect()
        }
        ListKind::Task => lines
            .iter()
            .filter(|line| !is_task(line))
            .filter_map(|line| {
                let new = match &line.item {
                    Some(_) => {
                        let mut prefix = line.rest[..line.prefix_len()].to_owned();
                        if !prefix.ends_with(' ') {
                            prefix.push(' ');
                        }
                        prefix + "[ ] "
                    }
                    None => format!("{}- [ ] ", line.indent()),
                };
                rewrite(line, new)
            })
            .collect(),
        ListKind::Bullet | ListKind::Numbered => {
            let numbered = kind == ListKind::Numbered;
            let of_kind = |line: &ListLine<'_>| {
                line.item.as_ref().is_some_and(|item| {
                    if numbered {
                        item.number.is_some()
                    } else {
                        item.bullet.is_some()
                    }
                })
            };
            if all(&of_kind) {
                lines
                    .iter()
                    .filter_map(|line| rewrite(line, line.indent().to_owned()))
                    .collect()
            } else {
                let mut number = first_number(text, &lines);
                lines
                    .iter()
                    .filter_map(|line| {
                        let marker = if numbered {
                            number += 1;
                            format!("{}. ", number - 1)
                        } else {
                            "- ".to_owned()
                        };
                        rewrite(
                            line,
                            format!("{}{marker}{}", line.indent(), line.task_box()),
                        )
                    })
                    .collect()
            }
        }
    };
    rewrite_plan(selections, rewrites)
}

/// Where a new numbered list over `lines` starts counting: on from a numbered item at the same
/// indent on the line above a single line, else 1.
fn first_number(text: &str, lines: &[ListLine<'_>]) -> u64 {
    let [line] = lines else {
        return 1;
    };
    let line_start = line_bounds(text, line.start).start;
    if line_start == 0 {
        return 1;
    }
    let above = &text[line_bounds(text, line_start - 1)];
    let above = &above[quote_prefix(above).0..];
    match list_prefix(above) {
        Some(item) if item.indent == leading(line.rest).1 => {
            item.number.map_or(1, |(value, _)| value + 1)
        }
        _ => 1,
    }
}

// ---------------------------------------------------------------------------------------------
// Tables

/// Tab in the last cell of a table's last row adds an empty row below and moves into its first
/// cell.
pub fn append_row(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan> {
    let table = table_at(text, caret)?;
    let lines = line_ranges(text, table);
    let last = lines.last()?.clone();
    if lines.len() < 3 || caret < last.start || caret > last.end {
        return None;
    }
    if next_cell(text, caret, false).is_some() {
        return None;
    }
    let columns = split_cells(&text[lines[0].clone()]).len().max(1);
    let row_text = &text[last.clone()];
    let indent = &row_text[..leading(row_text).0];
    let eol = line_ending(text, last.end, fallback);
    let row = format!("{eol}{indent}|{}", "  |".repeat(columns));
    let at = last.end + eol.len() + indent.len() + 2;
    Some(caret_plan(vec![insert(last.end, row)], at))
}

const NEW_TABLE: [&str; 3] = [
    "| Column 1 | Column 2 | Column 3 |",
    "| -------- | -------- | -------- |",
    "|          |          |          |",
];

/// The text of a new three-column table with its line endings, and where "Column 1" sits in it.
pub fn new_table(eol: &str) -> (String, Range<usize>) {
    (NEW_TABLE.join(eol), 2..10)
}

/// Markdown: Insert table: a three-column table on the caret's line when it is blank, else after
/// it, set off by blank lines. "Column 1" is selected, so typing names the first column and Tab
/// moves on.
pub fn insert_table(text: &str, caret: usize, fallback: &'static str) -> EditPlan {
    let line = line_bounds(text, caret);
    let eol = line_ending(text, line.end, fallback);
    let (table, header) = new_table(eol);
    let blank = text[line.clone()].trim().is_empty();
    let (range, before) = if blank {
        (line.clone(), String::new())
    } else {
        (line.end..line.end, format!("{eol}{eol}"))
    };
    // A paragraph right below would read as a row: a blank line ends the table.
    let next_blank = text[line.end..]
        .find('\n')
        .map(|newline| line_bounds(text, line.end + newline + 1))
        .is_none_or(|next| text[next].trim().is_empty());
    let after = if next_blank { "" } else { eol };
    let start = range.start + before.len();
    EditPlan {
        edits: vec![TextEdit {
            range,
            text: format!("{before}{table}{after}"),
        }],
        selections: vec![start + header.start..start + header.end],
    }
}

/// Markdown: Add table column (`add`) inserts an empty column after the caret's; Delete table
/// column removes the caret's (not a table's last). The table is re-padded and the caret goes to
/// the new column, or the one now in the deleted one's place.
pub fn table_column(text: &str, caret: usize, add: bool) -> Option<EditPlan> {
    let table = table_at(text, caret)?;
    let lines = line_ranges(text, table.clone());
    let eol = if text[table.clone()].contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let first = &text[lines[0].clone()];
    let indent = &first[..leading(first).0];
    let row = lines
        .iter()
        .position(|line| line.start <= caret && caret <= line.end)?;
    let segments = split_cells(&text[lines[row].clone()]);
    let offset = caret - lines[row].start;
    let column = segments
        .iter()
        .position(|(segment, _)| offset <= segment.end)
        .unwrap_or(segments.len().saturating_sub(1));
    let mut rows: Vec<Vec<String>> = lines
        .iter()
        .map(|line| {
            split_cells(&text[line.clone()])
                .into_iter()
                .map(|(_, cell)| cell.trim().to_owned())
                .collect()
        })
        .collect();
    let width = rows.iter().map(Vec::len).max()?;
    if !add && width <= 1 {
        return None;
    }
    let target = if add { column + 1 } else { column };
    for (index, cells) in rows.iter_mut().enumerate() {
        cells.resize(
            width,
            if index == 1 {
                "---".into()
            } else {
                String::new()
            },
        );
        if add {
            cells.insert(
                target,
                if index == 1 {
                    "---".into()
                } else {
                    String::new()
                },
            );
        } else {
            cells.remove(target);
        }
    }
    let raw = rows
        .iter()
        .map(|cells| format!("{indent}| {} |", cells.join(" | ")))
        .collect::<Vec<_>>()
        .join(eol);
    let new = format_table(&raw).unwrap_or(raw);
    let new_lines = line_ranges(&new, 0..new.len());
    let line = new_lines.get(row)?.clone();
    let cells = split_cells(&new[line.clone()]);
    let (segment, raw_cell) = cells.get(target.min(cells.len().saturating_sub(1)))?;
    let lead = raw_cell.len() - raw_cell.trim_start().len();
    let at = table.start + line.start + segment.start + lead.min(1);
    Some(EditPlan {
        edits: vec![TextEdit {
            range: table,
            text: new,
        }],
        selections: vec![at..at],
    })
}

// ---------------------------------------------------------------------------------------------
// Completion queries

/// The query of a `[[` link being typed: where it starts in `before` (the line up to the caret).
/// `None` once the link is closed or has a `|` alias or a `#` heading.
pub fn wiki_query(before: &str) -> Option<usize> {
    let open = before.rfind("[[")?;
    let query = &before[open + 2..];
    (!query.contains([']', '[', '|', '#'])).then_some(open + 2)
}

/// The query of a `#tag` being typed: where it starts in `before` (after the `#`). The `#` starts
/// the line or follows whitespace or `(`, and at least one tag character follows it.
pub fn tag_query(before: &str) -> Option<usize> {
    let hash = before.rfind('#')?;
    let query = &before[hash + 1..];
    let previous = before[..hash].chars().next_back();
    (!query.is_empty()
        && query.chars().all(crate::library::facts::is_tag_char)
        && previous.is_none_or(|c| c.is_whitespace() || c == '('))
    .then_some(hash + 1)
}

/// The query of a `/` command being typed: where the `/` is in `before`. Only quote markers and
/// spaces may come before it, and only letters and digits after.
pub fn slash_query(before: &str) -> Option<usize> {
    let slash = before.rfind('/')?;
    let head = &before[..slash];
    let query = &before[slash + 1..];
    (head.chars().all(|c| c == ' ' || c == '\t' || c == '>')
        && query.chars().all(char::is_alphanumeric))
    .then_some(slash)
}

/// The `/` commands, in the order the list shows them: label and other words that find it.
pub const SLASH_COMMANDS: [(&str, &str); 18] = [
    ("Heading 1", "h1 title"),
    ("Heading 2", "h2 subtitle"),
    ("Heading 3", "h3"),
    ("Bulleted list", "ul bullet unordered"),
    ("Numbered list", "ol ordered number"),
    ("Task list", "todo checkbox check"),
    ("Quote", "blockquote"),
    ("Code block", "fence pre"),
    ("Table", "grid"),
    ("Divider", "hr rule line separator"),
    ("Link", "url"),
    ("Image", "img picture"),
    ("Note link", "wikilink wiki"),
    ("Note callout", "alert info"),
    ("Tip callout", "alert hint"),
    ("Warning callout", "alert caution"),
    ("Date", "today"),
    ("Time", "now"),
];

/// How well `query` (lowercase) finds `label`: by the label's start, a word's start, anywhere in
/// it, or its letters in order; `None` when not at all.
fn find_score(label: &str, words: &str, query: &str) -> Option<u8> {
    let label = label.to_lowercase();
    let words = words.to_lowercase();
    if label.starts_with(query) {
        return Some(0);
    }
    if label
        .split([' ', '/', '-', '_'])
        .chain(words.split(' '))
        .any(|word| word.starts_with(query))
    {
        return Some(1);
    }
    if label.contains(query) {
        return Some(2);
    }
    let mut letters = label.chars().filter(|c| *c != ' ');
    query
        .chars()
        .all(|wanted| letters.any(|c| c == wanted))
        .then_some(3)
}

/// The `/` commands `query` finds, best first.
pub fn rank_slash(query: &str) -> Vec<&'static str> {
    let query = query.to_lowercase();
    let mut found: Vec<(u8, usize, &'static str)> = SLASH_COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, (label, words))| {
            find_score(label, words, &query).map(|score| (score, index, *label))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, label)| label).collect()
}

/// The names `query` finds, best first (case ignored), at most `limit`; `names` keep their order
/// within a rank.
pub fn rank_names(names: &[String], query: &str, limit: usize) -> Vec<String> {
    let query = query.to_lowercase();
    let mut found: Vec<(u8, usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| find_score(name, "", &query).map(|score| (score, index)))
        .collect();
    found.sort();
    found
        .into_iter()
        .take(limit)
        .map(|(_, index)| names[index].clone())
        .collect()
}

/// What a `/` command writes, and the selection in it afterwards (relative to its start).
/// `after_text`: the line above has text, so a divider needs a blank line first (`---` under a
/// paragraph would underline it as a heading).
pub fn slash_snippet(
    label: &str,
    eol: &str,
    date: &str,
    time: &str,
    after_text: bool,
) -> Option<(String, Range<usize>)> {
    let caret_after = |text: String| {
        let end = text.len();
        Some((text, end..end))
    };
    match label {
        "Heading 1" => caret_after("# ".into()),
        "Heading 2" => caret_after("## ".into()),
        "Heading 3" => caret_after("### ".into()),
        "Bulleted list" => caret_after("- ".into()),
        "Numbered list" => caret_after("1. ".into()),
        "Task list" => caret_after("- [ ] ".into()),
        "Quote" => caret_after("> ".into()),
        "Code block" => Some((format!("```{eol}{eol}```"), 3..3)),
        "Table" => Some(new_table(eol)),
        "Divider" => caret_after(format!("{}---{eol}", if after_text { eol } else { "" })),
        "Link" => Some(("[]()".into(), 1..1)),
        "Image" => Some(("![]()".into(), 4..4)),
        "Note link" => Some(("[[]]".into(), 2..2)),
        "Note callout" => caret_after(format!("> [!NOTE]{eol}> ")),
        "Tip callout" => caret_after(format!("> [!TIP]{eol}> ")),
        "Warning callout" => caret_after(format!("> [!WARNING]{eol}> ")),
        "Date" => caret_after(date.to_owned()),
        "Time" => caret_after(time.to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
