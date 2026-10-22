//! Text selection in the preview (2026-10-22 preview selection spec): the text runs each block
//! draws, in reading order; points between their UTF-16 units; and what a selection copies, as
//! plain text and as HTML. Win32-free: layout records the same runs with their text layouts, and
//! copying reads the model, so it lays nothing out.

use crate::preview::model::{
    AlertKind, Block, BlockKind, InlineStyle, OBJECT_REPLACEMENT, RichText,
};
use std::fmt::Write as _;

/// A place between two UTF-16 units of a run. Points order as the document reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPoint {
    /// The top-level block's index in the document.
    pub block: usize,
    /// The run's index among the block's runs (`text_runs`).
    pub run: u32,
    /// UTF-16 offset within the run.
    pub offset: u32,
}

impl TextPoint {
    pub const fn new(block: usize, run: u32, offset: u32) -> Self {
        Self { block, run, offset }
    }
}

/// The text between where a selection started (`anchor`) and where it was extended to (`focus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: TextPoint,
    pub focus: TextPoint,
}

impl Selection {
    pub const fn caret(point: TextPoint) -> Self {
        Self {
            anchor: point,
            focus: point,
        }
    }

    /// The selection's start and end in document order.
    pub fn ordered(&self) -> (TextPoint, TextPoint) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// The selected part of run `run` of block `block`, `len` units long, as drawn: `None` when
    /// none of it is selected.
    pub fn run_range(&self, block: usize, run: u32, len: u32) -> Option<(u32, u32)> {
        self.piece(block, run, len).filter(|(from, to)| from < to)
    }

    /// The part of a run between the selection's ends, empty when the run lies inside the
    /// selection but has no text. `None` outside it.
    fn piece(&self, block: usize, run: u32, len: u32) -> Option<(u32, u32)> {
        let (start, end) = self.ordered();
        let at = (block, run);
        if at < (start.block, start.run) || at > (end.block, end.run) {
            return None;
        }
        let from = if at == (start.block, start.run) {
            start.offset.min(len)
        } else {
            0
        };
        let to = if at == (end.block, end.run) {
            end.offset.min(len)
        } else {
            len
        };
        Some((from, to.max(from)))
    }
}

/// A run's text: rich text, or a code block's verbatim text.
#[derive(Clone, Copy, Debug)]
pub enum RunText<'a> {
    Rich(&'a RichText),
    Code(&'a str),
}

impl RunText<'_> {
    pub fn units(&self) -> Vec<u16> {
        match self {
            Self::Rich(rich) => rich.text.encode_utf16().collect(),
            Self::Code(code) => code.encode_utf16().collect(),
        }
    }

    pub fn utf16_len(&self) -> u32 {
        match self {
            Self::Rich(rich) => rich.utf16_len,
            Self::Code(code) => code.encode_utf16().count() as u32,
        }
    }
}

/// What a run is, for the shape its copy takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Paragraph,
    Heading(u8),
    Code,
    /// A table cell; row 0 is the header row.
    Cell {
        row: usize,
        column: usize,
    },
    /// A `<details>` summary.
    Summary,
}

/// A container a run sits in, outermost first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    Quote,
    Alert(AlertKind),
    Div,
    Details,
    /// `Some(start)` for a numbered list.
    List(Option<u64>),
    Item {
        number: Option<u64>,
        task: Option<bool>,
    },
    Table,
}

/// A frame and an id that tells it from another frame of the same kind in the block.
pub type FrameRef = (u32, Frame);

#[derive(Clone, Debug)]
pub struct Run<'a> {
    pub text: RunText<'a>,
    pub kind: RunKind,
    pub frames: Vec<FrameRef>,
    /// The first run of its innermost list item, where the item's marker goes.
    pub item_start: bool,
}

static EMPTY_CELL: RichText = RichText {
    text: String::new(),
    utf16_len: 0,
    spans: Vec::new(),
    images: Vec::new(),
};

/// Block `kind`'s runs in reading order: the text layout draws, without list markers, alert
/// titles or disclosure triangles. `open(index, attribute)` says whether the block's `index`-th
/// `<details>` section (document order, as layout and the outline count them) is open; a
/// collapsed section contributes its summary only.
pub fn text_runs<'a>(
    kind: &'a BlockKind,
    open: &mut dyn FnMut(usize, bool) -> bool,
) -> Vec<Run<'a>> {
    let mut walk = Walk {
        runs: Vec::new(),
        frames: Vec::new(),
        next_id: 0,
        next_details: 0,
        item_start: false,
        open,
    };
    walk.block(kind, true);
    walk.runs
}

struct Walk<'a, 'o> {
    runs: Vec<Run<'a>>,
    frames: Vec<FrameRef>,
    next_id: u32,
    next_details: usize,
    item_start: bool,
    open: &'o mut dyn FnMut(usize, bool) -> bool,
}

impl<'a> Walk<'a, '_> {
    fn push(&mut self, text: RunText<'a>, kind: RunKind) {
        self.runs.push(Run {
            text,
            kind,
            frames: self.frames.clone(),
            item_start: std::mem::take(&mut self.item_start),
        });
    }

    fn enter(&mut self, frame: Frame) {
        self.frames.push((self.next_id, frame));
        self.next_id += 1;
    }

    fn children(&mut self, frame: Frame, blocks: &'a [BlockKind], visible: bool) {
        self.enter(frame);
        for block in blocks {
            self.block(block, visible);
        }
        self.frames.pop();
    }

    /// `visible` is false inside a collapsed section: nothing is emitted, but sections are
    /// still counted so later ones keep their indices.
    fn block(&mut self, kind: &'a BlockKind, visible: bool) {
        match kind {
            BlockKind::Paragraph { text, .. } => {
                if visible {
                    self.push(RunText::Rich(text), RunKind::Paragraph);
                }
            }
            BlockKind::Heading { level, text, .. } => {
                if visible {
                    self.push(RunText::Rich(text), RunKind::Heading(*level));
                }
            }
            BlockKind::Code { text, .. } => {
                if visible {
                    self.push(RunText::Code(text), RunKind::Code);
                }
            }
            BlockKind::Rule => {}
            BlockKind::Quote(blocks) => self.children(Frame::Quote, blocks, visible),
            BlockKind::Alert { kind, blocks } => {
                self.children(Frame::Alert(*kind), blocks, visible);
            }
            BlockKind::Container { children, .. } => self.children(Frame::Div, children, visible),
            BlockKind::List { start, items } => {
                self.enter(Frame::List(*start));
                for (index, item) in items.iter().enumerate() {
                    self.enter(Frame::Item {
                        number: start.map(|first| first + index as u64),
                        task: item.task,
                    });
                    self.item_start = visible;
                    for block in &item.blocks {
                        self.block(block, visible);
                    }
                    self.item_start = false;
                    self.frames.pop();
                }
                self.frames.pop();
            }
            BlockKind::Table {
                alignments,
                head,
                rows,
            } => {
                if !visible {
                    return;
                }
                // Layout draws every column of every row, an empty cell where a row is short.
                let columns = alignments
                    .len()
                    .max(head.len())
                    .max(rows.iter().map(Vec::len).max().unwrap_or(0));
                self.enter(Frame::Table);
                let all_rows =
                    std::iter::once(head.as_slice()).chain(rows.iter().map(Vec::as_slice));
                for (row, cells) in all_rows.enumerate() {
                    for column in 0..columns {
                        let cell = cells.get(column).unwrap_or(&EMPTY_CELL);
                        self.push(RunText::Rich(cell), RunKind::Cell { row, column });
                    }
                }
                self.frames.pop();
            }
            BlockKind::Details {
                open,
                summary,
                children,
            } => {
                let index = self.next_details;
                self.next_details += 1;
                let opened = visible && (self.open)(index, *open);
                self.enter(Frame::Details);
                if visible {
                    self.push(RunText::Rich(summary), RunKind::Summary);
                }
                for block in children {
                    self.block(block, opened);
                }
                self.frames.pop();
            }
        }
    }
}

/// The UTF-16 range of the word, run of spaces, or single other character at `offset` in
/// `units`, as a double click selects it. Word characters are letters, digits and `_`, as in
/// the Live view.
pub fn word_range(units: &[u16], offset: u32) -> (u32, u32) {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let chars = char::decode_utf16(units.iter().copied())
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect::<Vec<_>>();
    if chars.is_empty() {
        return (0, 0);
    }
    let starts = chars
        .iter()
        .scan(0_u32, |position, c| {
            let start = *position;
            *position += c.len_utf16() as u32;
            Some(start)
        })
        .collect::<Vec<_>>();
    // The character after the point, or the last one at the end of the run.
    let at = starts
        .iter()
        .rposition(|start| *start <= offset)
        .unwrap_or(0)
        .min(chars.len() - 1);
    let wanted = class(chars[at]);
    let (mut first, mut last) = (at, at);
    if wanted != Class::Other {
        while first > 0 && class(chars[first - 1]) == wanted {
            first -= 1;
        }
        while last + 1 < chars.len() && class(chars[last + 1]) == wanted {
            last += 1;
        }
    }
    (starts[first], starts[last] + chars[last].len_utf16() as u32)
}

/// What a selection puts on the clipboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clip {
    pub plain: String,
    /// An HTML fragment, without the CF_HTML header (`cf_html` adds it).
    pub html: String,
}

/// A selected part of a run.
struct Piece<'a> {
    block: usize,
    run: Run<'a>,
    from: u32,
    to: u32,
}

/// The selected text of `blocks` as plain text and HTML. `open(block, index, attribute)` says
/// whether a block's `index`-th section is open, as `text_runs` asks.
pub fn copy(
    blocks: &[Block],
    selection: &Selection,
    open: &mut dyn FnMut(usize, usize, bool) -> bool,
) -> Clip {
    let (start, end) = selection.ordered();
    let mut pieces = Vec::new();
    for (block, item) in blocks
        .iter()
        .enumerate()
        .take(end.block.saturating_add(1))
        .skip(start.block)
    {
        let runs = text_runs(&item.kind, &mut |index, attribute| {
            open(block, index, attribute)
        });
        for (index, run) in runs.into_iter().enumerate() {
            let index = index as u32;
            let Some((from, to)) = selection.piece(block, index, run.text.utf16_len()) else {
                continue;
            };
            // An end that only touches a run (a drag ending at its start) does not copy it, but
            // an empty table cell between the ends keeps its place.
            let edge = (block, index) == (start.block, start.run)
                || (block, index) == (end.block, end.run);
            if from == to && edge && run.text.utf16_len() > 0 {
                continue;
            }
            pieces.push(Piece {
                block,
                run,
                from,
                to,
            });
        }
    }
    Clip {
        plain: plain_text(&pieces),
        html: html(&pieces),
    }
}

fn innermost(frames: &[FrameRef], wanted: impl Fn(&Frame) -> bool) -> Option<&FrameRef> {
    frames.iter().rev().find(|(_, frame)| wanted(frame))
}

fn is_item(frame: &Frame) -> bool {
    matches!(frame, Frame::Item { .. })
}

fn is_quote(frame: &Frame) -> bool {
    matches!(frame, Frame::Quote | Frame::Alert(_))
}

/// `units[from..to]` as text, with each inline image replaced by `image(alt)`.
fn segment(
    rich_images: Option<&RichText>,
    units: &[u16],
    from: u32,
    to: u32,
    image: &dyn Fn(&str) -> String,
) -> String {
    let text = String::from_utf16_lossy(&units[from as usize..to as usize]);
    match rich_images {
        Some(rich) if text.contains(OBJECT_REPLACEMENT) => {
            let mut out = String::new();
            let mut position = from;
            for c in text.chars() {
                if c == OBJECT_REPLACEMENT
                    && let Some(found) = rich
                        .images
                        .iter()
                        .find(|candidate| candidate.position == position)
                {
                    out.push_str(&image(&found.image.alt));
                } else if c != OBJECT_REPLACEMENT {
                    out.push(c);
                }
                position += c.len_utf16() as u32;
            }
            out
        }
        _ => text,
    }
}

fn plain_text(pieces: &[Piece<'_>]) -> String {
    let mut out = String::new();
    let mut previous: Option<&Piece<'_>> = None;
    for piece in pieces {
        let frames = &piece.run.frames;
        let quotes = frames.iter().filter(|(_, frame)| is_quote(frame)).count();
        let items = frames.iter().filter(|(_, frame)| is_item(frame)).count();
        let quote_prefix = "> ".repeat(quotes);
        if let Some(previous) = previous {
            let before = &previous.run.frames;
            let same_block = previous.block == piece.block;
            let table = innermost(frames, |frame| *frame == Frame::Table);
            let separator =
                match (previous.run.kind, piece.run.kind) {
                    (RunKind::Cell { row: a, .. }, RunKind::Cell { row: b, .. })
                        if same_block
                            && table == innermost(before, |frame| *frame == Frame::Table) =>
                    {
                        if a == b { "\t" } else { "\n" }
                    }
                    _ if same_block
                        && frames.iter().any(|frame| {
                            matches!(frame.1, Frame::List(_)) && before.contains(frame)
                        }) =>
                    {
                        "\n"
                    }
                    _ => "\n\n",
                };
            if separator == "\n\n" {
                // The blank line keeps the quote marks the two sides share.
                let shared = if same_block {
                    frames
                        .iter()
                        .zip(before.iter())
                        .take_while(|(a, b)| a == b)
                        .filter(|(frame, _)| is_quote(&frame.1))
                        .count()
                } else {
                    0
                };
                out.push('\n');
                out.push_str(">".repeat(shared).as_str());
                out.push('\n');
            } else {
                out.push_str(separator);
            }
        }
        let new_line = previous.is_none_or(|previous| {
            !matches!(
                (previous.run.kind, piece.run.kind),
                (RunKind::Cell { row: a, .. }, RunKind::Cell { row: b, .. }) if a == b && previous.block == piece.block
            )
        });
        let indent = "  ".repeat(items.saturating_sub(1));
        let marker = match innermost(frames, is_item) {
            Some((_, Frame::Item { number, task })) if piece.run.item_start && piece.from == 0 => {
                let mut marker = match number {
                    Some(number) => format!("{number}. "),
                    None => "- ".to_owned(),
                };
                match task {
                    Some(true) => marker.push_str("[x] "),
                    Some(false) => marker.push_str("[ ] "),
                    None => {}
                }
                marker
            }
            Some(_) => "  ".to_owned(),
            None => String::new(),
        };
        let first_prefix = format!("{quote_prefix}{indent}{marker}");
        let continuation = format!("{quote_prefix}{indent}{}", " ".repeat(marker.len().min(2)));
        let units = piece.run.text.units();
        let text = match piece.run.text {
            RunText::Rich(rich) => segment(Some(rich), &units, piece.from, piece.to, &|alt| {
                alt.to_owned()
            }),
            RunText::Code(_) => segment(None, &units, piece.from, piece.to, &|alt| alt.to_owned()),
        };
        let text = if matches!(piece.run.kind, RunKind::Cell { .. }) {
            text.replace(['\n', '\t'], " ")
        } else {
            text
        };
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                out.push('\n');
                out.push_str(&continuation);
            } else if new_line {
                out.push_str(&first_prefix);
            }
            out.push_str(line);
        }
        previous = Some(piece);
    }
    out
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn open_frame(out: &mut String, frame: &Frame) {
    match frame {
        Frame::Quote => out.push_str("<blockquote>"),
        Frame::Alert(kind) => {
            let _ = write!(out, "<blockquote><p><strong>{}</strong></p>", kind.title());
        }
        Frame::Div => out.push_str("<div>"),
        Frame::Details => out.push_str("<details open>"),
        Frame::List(Some(start)) if *start != 1 => {
            let _ = write!(out, "<ol start=\"{start}\">");
        }
        Frame::List(Some(_)) => out.push_str("<ol>"),
        Frame::List(None) => out.push_str("<ul>"),
        Frame::Item { .. } => out.push_str("<li>"),
        Frame::Table => out.push_str("<table>"),
    }
}

fn close_frame(out: &mut String, frame: &Frame) {
    out.push_str(match frame {
        Frame::Quote | Frame::Alert(_) => "</blockquote>",
        Frame::Div => "</div>",
        Frame::Details => "</details>",
        Frame::List(Some(_)) => "</ol>",
        Frame::List(None) => "</ul>",
        Frame::Item { .. } => "</li>",
        Frame::Table => "</table>",
    });
}

/// The HTML tag an inline style opens, or `None` for one with no element (a tag chip).
fn style_open(style: &InlineStyle) -> Option<String> {
    Some(match style {
        InlineStyle::Strong => "<strong>".to_owned(),
        InlineStyle::Emphasis => "<em>".to_owned(),
        InlineStyle::Strikethrough => "<del>".to_owned(),
        InlineStyle::Code => "<code>".to_owned(),
        InlineStyle::Link(dest) => format!("<a href=\"{}\">", escape(dest)),
        InlineStyle::Underline => "<u>".to_owned(),
        InlineStyle::Subscript => "<sub>".to_owned(),
        InlineStyle::Superscript => "<sup>".to_owned(),
        InlineStyle::Mark => "<mark>".to_owned(),
        InlineStyle::Small => "<small>".to_owned(),
        InlineStyle::Keyboard => "<kbd>".to_owned(),
        InlineStyle::Tag(_) => return None,
    })
}

fn style_close(style: &InlineStyle) -> &'static str {
    match style {
        InlineStyle::Strong => "</strong>",
        InlineStyle::Emphasis => "</em>",
        InlineStyle::Strikethrough => "</del>",
        InlineStyle::Code => "</code>",
        InlineStyle::Link(_) => "</a>",
        InlineStyle::Underline => "</u>",
        InlineStyle::Subscript => "</sub>",
        InlineStyle::Superscript => "</sup>",
        InlineStyle::Mark => "</mark>",
        InlineStyle::Small => "</small>",
        InlineStyle::Keyboard => "</kbd>",
        InlineStyle::Tag(_) => "",
    }
}

/// `rich[from..to]` as HTML: the spans that overlap it become elements, reopened where spans
/// cross.
fn inline_html(rich: &RichText, from: u32, to: u32) -> String {
    let units = rich.text.encode_utf16().collect::<Vec<_>>();
    let mut cuts = vec![from, to];
    for span in &rich.spans {
        cuts.extend([span.range.start, span.range.end]);
    }
    cuts.retain(|cut| (from..=to).contains(cut));
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    for window in cuts.windows(2) {
        let (a, b) = (window[0], window[1]);
        let active = rich
            .spans
            .iter()
            .enumerate()
            .filter(|(_, span)| span.range.start <= a && span.range.end >= b)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let keep = open.iter().zip(&active).take_while(|(a, b)| a == b).count();
        for &index in open[keep..].iter().rev() {
            out.push_str(style_close(&rich.spans[index].style));
        }
        open.truncate(keep);
        for &index in &active[keep..] {
            if let Some(tag) = style_open(&rich.spans[index].style) {
                out.push_str(&tag);
            }
            open.push(index);
        }
        out.push_str(&escape(&segment(Some(rich), &units, a, b, &|alt| {
            alt.to_owned()
        })));
    }
    for &index in open.iter().rev() {
        out.push_str(style_close(&rich.spans[index].style));
    }
    out
}

fn html(pieces: &[Piece<'_>]) -> String {
    let mut out = String::new();
    let mut stack: Vec<FrameRef> = Vec::new();
    let mut previous_block = None;
    // The open table row, (table id, row index); row 0 sits in `<thead>`, the rest in `<tbody>`.
    let mut row: Option<(u32, usize)> = None;
    let close_row = |out: &mut String, row: &mut Option<(u32, usize)>| {
        if let Some((_, index)) = row.take() {
            out.push_str(if index == 0 {
                "</tr></thead>"
            } else {
                "</tr></tbody>"
            });
        }
    };
    for piece in pieces {
        let target: &[FrameRef] = if previous_block == Some(piece.block) {
            &piece.run.frames
        } else {
            &[]
        };
        let keep = stack
            .iter()
            .zip(target.iter())
            .take_while(|(a, b)| a == b)
            .count();
        while stack.len() > keep {
            let frame = stack.pop().expect("longer than keep");
            if frame.1 == Frame::Table {
                close_row(&mut out, &mut row);
            }
            close_frame(&mut out, &frame.1);
        }
        for frame in &piece.run.frames[stack.len()..] {
            open_frame(&mut out, &frame.1);
            stack.push(*frame);
        }
        previous_block = Some(piece.block);
        match (piece.run.kind, piece.run.text) {
            (RunKind::Cell { row: index, .. }, RunText::Rich(rich)) => {
                let table = innermost(&piece.run.frames, |frame| *frame == Frame::Table)
                    .map_or(0, |(id, _)| *id);
                match row {
                    Some(open) if open == (table, index) => {}
                    // The next row of the same table: leave the header for the body once.
                    Some((_, previous)) => {
                        out.push_str(if previous == 0 && index != 0 {
                            "</tr></thead><tbody><tr>"
                        } else {
                            "</tr><tr>"
                        });
                        row = Some((table, index));
                    }
                    None => {
                        out.push_str(if index == 0 {
                            "<thead><tr>"
                        } else {
                            "<tbody><tr>"
                        });
                        row = Some((table, index));
                    }
                }
                let tag = if index == 0 { "th" } else { "td" };
                let _ = write!(
                    out,
                    "<{tag}>{}</{tag}>",
                    inline_html(rich, piece.from, piece.to)
                );
            }
            (RunKind::Code, RunText::Code(code)) => {
                let units = code.encode_utf16().collect::<Vec<_>>();
                let text = String::from_utf16_lossy(&units[piece.from as usize..piece.to as usize]);
                let _ = write!(out, "<pre><code>{}</code></pre>", escape(&text));
            }
            (kind, RunText::Rich(rich)) => {
                let tag = match kind {
                    RunKind::Heading(level) => format!("h{}", level.clamp(1, 6)),
                    RunKind::Summary => "summary".to_owned(),
                    _ => "p".to_owned(),
                };
                let mut body = String::new();
                if piece.run.item_start
                    && piece.from == 0
                    && let Some((
                        _,
                        Frame::Item {
                            task: Some(checked),
                            ..
                        },
                    )) = innermost(&piece.run.frames, is_item)
                {
                    body.push_str(if *checked {
                        "<input type=\"checkbox\" disabled checked> "
                    } else {
                        "<input type=\"checkbox\" disabled> "
                    });
                }
                body.push_str(&inline_html(rich, piece.from, piece.to));
                let _ = write!(out, "<{tag}>{body}</{tag}>");
            }
            (_, RunText::Code(code)) => out.push_str(&escape(code)),
        }
    }
    while let Some(frame) = stack.pop() {
        if frame.1 == Frame::Table {
            close_row(&mut out, &mut row);
        }
        close_frame(&mut out, &frame.1);
    }
    out
}

/// `fragment` wrapped in the `HTML Format` clipboard header, whose offsets count UTF-8 bytes.
pub fn cf_html(fragment: &str) -> String {
    const PREFIX: &str = "<html><body>\r\n<!--StartFragment-->";
    const SUFFIX: &str = "<!--EndFragment-->\r\n</body></html>";
    let header = |numbers: [usize; 4]| {
        format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            numbers[0], numbers[1], numbers[2], numbers[3]
        )
    };
    // Every number has ten digits, so the header's length does not depend on them.
    let start_html = header([0; 4]).len();
    let start_fragment = start_html + PREFIX.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + SUFFIX.len();
    format!(
        "{}{PREFIX}{fragment}{SUFFIX}",
        header([start_html, end_html, start_fragment, end_fragment])
    )
}

#[cfg(test)]
mod tests;
