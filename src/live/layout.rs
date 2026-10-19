//! The Live view's text layout: one DirectWrite layout per source line, wrapped to the view's
//! width, laid out only when a line is about to be shown. Lines not laid out yet have an
//! estimated height in a `HeightIndex`, as the preview's blocks do. All y values are in document
//! coordinates (0 is the top of the first line); x values start at the text's left edge.
//!
//! Each line is laid out from its `LineStyle` (phase 2 plan, Task 5): hidden markup leaves the
//! layout string, styled runs set weights, styles, sizes and fonts, and a heading line gets its
//! own size and line spacing. Colours are kept per layout and bound to the painter's brushes when
//! it draws (`LineLayout::bind`), because brushes belong to a render target that can be recreated.
//!
//! A line inside quotes or list items is laid out at the text width less its containers' indent
//! and drawn at `x = indent` (phase 3 plan, Decision 1); every x this module reports or accepts
//! is in line coordinates, the indent included. The margin's quote bars, bullets, numbers and
//! checkboxes, a code block's background and a rule are `Decoration`s the painter draws.

use crate::Result;
use crate::live::lines::LineCache;
use crate::live::motion::{LineGeometry, Row};
use crate::live::offsets::OffsetMap;
use crate::live::spans::{Container, Marker};
use crate::live::styles::{BodyStyle, Effect, ImageInfo, LineStyle, Paint, StyledLines, effects};
use crate::preview::dwrite::{Graphics, hresult_error};
use crate::preview::heights::HeightIndex;
use crate::preview::inline_object::inline_box;
use crate::preview::layout::{CODE_SCALE, HEADING_SCALE};
use crate::preview::render::RectF;
use std::cell::Cell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_HIT_TEST_METRICS,
    DWRITE_LINE_METRICS, DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE, DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteTextFormat,
    IDWriteTextLayout,
};
use windows::core::{BOOL, PCWSTR};

/// The prose font and the code font, at the size the view draws them.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveFonts {
    pub family: String,
    pub weight: DWRITE_FONT_WEIGHT,
    /// The editor font, for inline code.
    pub code_family: String,
    pub code_weight: DWRITE_FONT_WEIGHT,
    /// Text size in DIPs, zoom included.
    pub size: f32,
    /// Height of one row in DIPs.
    pub line_height: f32,
    /// The editor's zoom as a factor (1.0 = none), for image sizes.
    pub zoom: f32,
}

impl LiveFonts {
    /// The preview's prose font settings (`preview_font`, `preview_font_size` in 96-DPI pixels,
    /// `preview_line_height` in tenths), scaled by Scintilla's zoom the way Scintilla scales the
    /// editor font: `zoom` points added to `editor_font_size` points.
    pub fn new(
        graphics: &Graphics,
        preview_font: &str,
        code_font: &str,
        preview_font_size: u16,
        line_height_tenths: u8,
        editor_font_size: u16,
        zoom: i32,
    ) -> Self {
        let resolved = graphics.resolve_family(preview_font);
        let code = graphics.resolve_family(code_font);
        let base = f32::from(editor_font_size.max(1));
        let scale = ((base + zoom as f32) / base).max(1.0 / base);
        let size = f32::from(preview_font_size.max(1)) * scale;
        Self {
            family: resolved.name,
            weight: resolved.weight,
            code_family: code.name,
            code_weight: code.weight,
            size,
            line_height: size * (f32::from(line_height_tenths.max(10)) / 10.0),
            zoom: scale,
        }
    }
}

/// One wrapped row of a laid-out line, in UTF-16 positions of the line's layout text.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RowMetrics {
    start: u32,
    length: u32,
    top: f32,
    height: f32,
}

/// Something the painter draws for a line besides its text, in line coordinates (x from the
/// text area's left edge, y from the line's top).
#[derive(Clone, Debug, PartialEq)]
pub enum Decoration {
    /// A filled rectangle; `right == f32::MAX` reaches the text area's right edge.
    Fill { rect: RectF, paint: Paint },
    /// A hairline; an x of `f32::MAX` is the text area's right edge.
    Line {
        from: (f32, f32),
        to: (f32, f32),
        paint: Paint,
    },
    /// A list bullet: filled at depth 1, a ring at depth 2, a square deeper.
    Bullet {
        center: (f32, f32),
        radius: f32,
        depth: u8,
    },
    /// An ordered marker's text, drawn right-aligned in `rect` on the first row's baseline.
    Number { rect: RectF, text: String },
    /// A task's box; `inner` is the line-relative byte between its brackets.
    Checkbox {
        rect: RectF,
        checked: bool,
        inner: usize,
    },
    /// A decoded image, drawn scaled into `rect`.
    Image { rect: RectF, path: PathBuf },
}

/// One laid-out source line.
pub struct LineLayout {
    pub layout: IDWriteTextLayout,
    pub offsets: OffsetMap,
    rows: Vec<RowMetrics>,
    pub height: f32,
    /// Where the layout is drawn: its containers' indent.
    pub indent: f32,
    pub decorations: Vec<Decoration>,
    /// The style it was laid out with: a line is laid out again only when this changes.
    pub style: LineStyle,
    /// Colour runs, lowest rank first, bound to brushes by `bind`.
    colors: Vec<(DWRITE_TEXT_RANGE, Paint)>,
    /// UTF-16 ranges filled behind the text (inline code), and their colour.
    pub backgrounds: Vec<(u32, u32, Paint)>,
    /// A table row's column widths when it was laid out (empty for any other line): the row is
    /// laid out again when its table's columns change.
    pub columns: Vec<f32>,
    /// The brush generation the colour runs were bound for; 0 = never.
    bound: Cell<u64>,
}

impl LineLayout {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Sets each colour run's drawing effect to its brush, unless already done for
    /// `generation` (brushes are recreated with the render target, and bump it).
    pub fn bind<'b>(
        &self,
        brush: impl Fn(Paint) -> Option<&'b ID2D1SolidColorBrush>,
        generation: u64,
    ) {
        if self.bound.get() == generation {
            return;
        }
        for (range, paint) in &self.colors {
            if let Some(brush) = brush(*paint) {
                let _ = unsafe { self.layout.SetDrawingEffect(brush, *range) };
            }
        }
        self.bound.set(generation);
    }

    /// The brush generation the colour runs are bound for.
    #[cfg(test)]
    pub fn bound(&self) -> u64 {
        self.bound.get()
    }

    /// The rectangles (line coordinates) covering UTF-16 `start..end`.
    pub fn range_rects(&self, start: u32, end: u32) -> Result<Vec<RectF>> {
        range_rects(&self.layout, start, end, self.indent, 0.0)
    }
}

pub struct LiveLayout {
    format: IDWriteTextFormat,
    /// The format's baseline within a row, for spacing heading rows to scale.
    baseline: f32,
    fonts: LiveFonts,
    width: f32,
    lines: LineCache<LineLayout>,
    heights: HeightIndex,
    /// The average character width, for height estimates.
    char_width: f32,
    /// The width of one digit, for the line-number gutter.
    digit_width: f32,
    /// Each table row's cell widths, by `row_key`; cleared with every reset (new fonts or width).
    cell_widths: HashMap<u64, Rc<[f32]>>,
    /// Lines laid out and table rows measured, for the tests that check what is redone.
    #[cfg(test)]
    pub(crate) counts: Cell<(usize, usize)>,
    /// Last, so the layouts and format above are released before it can unload DirectWrite.
    graphics: Rc<Graphics>,
}

/// How a table row or delimiter is laid out.
#[derive(Clone, Copy)]
enum Table<'a> {
    /// Not a table line, or one whose table could not be found.
    No,
    /// Measuring its cells: zero-width spacers, no wrapping.
    Measure,
    /// In its table's `columns`; `cells` are this row's cell widths.
    Columns {
        columns: &'a [f32],
        cells: &'a [f32],
    },
}

/// The cell-width cache key of a table row: its text, and whether it is the header.
fn row_key(source: &str, header: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    header.hash(&mut hasher);
    hasher.finish()
}

fn is_table(style: &LineStyle) -> bool {
    matches!(
        style.block.body,
        BodyStyle::TableRow { .. } | BodyStyle::TableDelimiter
    )
}

impl LiveLayout {
    pub fn new(graphics: Rc<Graphics>, fonts: LiveFonts, width: f32) -> Result<Self> {
        let (format, baseline) = text_format(&graphics, &fonts)?;
        let char_width = fonts.size * 0.5;
        let digit_width = digit_width(&graphics, &format)?;
        Ok(Self {
            graphics,
            format,
            baseline,
            fonts,
            width: width.max(1.0),
            lines: LineCache::default(),
            heights: HeightIndex::default(),
            char_width,
            digit_width,
            cell_widths: HashMap::new(),
            #[cfg(test)]
            counts: Cell::new((0, 0)),
        })
    }

    /// The width of one digit in the text font.
    pub fn digit_width(&self) -> f32 {
        self.digit_width
    }

    /// Line number `number`, right-aligned in a box `width` wide, on the text's rows: drawn at a
    /// line's top, it sits on that line's first row with the same baseline.
    pub fn number_layout(&self, number: usize, width: f32) -> Result<IDWriteTextLayout> {
        let digits = number.to_string().encode_utf16().collect::<Vec<_>>();
        let layout = unsafe {
            self.graphics
                .dwrite
                .CreateTextLayout(&digits, &self.format, width.max(1.0), f32::MAX)
        }
        .map_err(hresult_error)?;
        unsafe { layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING) }
            .map_err(hresult_error)?;
        Ok(layout)
    }

    pub fn fonts(&self) -> &LiveFonts {
        &self.fonts
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Drops every layout and estimates every line of `text` again.
    pub fn reset(&mut self, text: &impl StyledLines) {
        let count = text.line_count();
        self.lines.reset(count);
        self.cell_widths.clear();
        let estimates = (0..count)
            .map(|line| self.estimate(text, line))
            .collect::<Vec<_>>();
        self.heights.reset(estimates);
    }

    /// A new text width; every line is laid out again. The caller keeps its scroll anchor.
    pub fn set_width(&mut self, text: &impl StyledLines, width: f32) {
        let width = width.max(1.0);
        if width != self.width {
            self.width = width;
            self.reset(text);
        }
    }

    /// New fonts (zoom, settings, DPI); every line is laid out again.
    pub fn set_fonts(&mut self, text: &impl StyledLines, fonts: LiveFonts) -> Result<()> {
        if fonts != self.fonts {
            (self.format, self.baseline) = text_format(&self.graphics, &fonts)?;
            self.char_width = fonts.size * 0.5;
            self.digit_width = digit_width(&self.graphics, &self.format)?;
            self.fonts = fonts;
            self.reset(text);
        }
        Ok(())
    }

    /// An edit starting on `first_line` that added (`lines_delta > 0`) or removed lines. Returns
    /// the lines that must be laid out again.
    pub fn edit(
        &mut self,
        text: &impl StyledLines,
        first_line: usize,
        lines_delta: isize,
    ) -> Range<usize> {
        let old_len = self.heights.len();
        let changed = self.lines.apply(first_line, lines_delta);
        let tail = self.lines.len() - changed.end;
        let old = changed.start.min(old_len)
            ..old_len.saturating_sub(tail).max(changed.start.min(old_len));
        let estimates = changed
            .clone()
            .map(|line| self.estimate(text, line))
            .collect::<Vec<_>>();
        self.heights.splice(old, &estimates);
        changed
    }

    /// Drops the layout of each laid-out line in `lines` whose style no longer matches `text`;
    /// returns those lines. Their measured heights stay until they are laid out again.
    ///
    /// A table line in `lines` brings its whole table along: the table's columns are measured
    /// again (only rows whose text changed are), and every row laid out in other columns is
    /// dropped too.
    pub fn restyle(&mut self, text: &impl StyledLines, lines: Range<usize>) -> Vec<usize> {
        let end = lines.end.min(self.lines.len());
        let mut dropped = Vec::new();
        let mut tables: Vec<Range<usize>> = Vec::new();
        for line in lines.start.min(end)..end {
            let Some(laid) = self.lines.get(line) else {
                continue;
            };
            let style = text.line_style(line);
            let table = is_table(&style) || is_table(&laid.style) || !laid.columns.is_empty();
            if laid.style != style {
                self.lines.invalidate(line..line + 1);
                dropped.push(line);
            }
            if table
                && !tables.iter().any(|known| known.contains(&line))
                && let Some(lines) = text.table_lines(line)
            {
                tables.push(lines);
            }
        }
        for table in tables {
            // A failed measurement drops every row, so the next paint lays them out (and reports
            // the failure) again.
            let columns = self.table_columns(text, table.clone()).unwrap_or_default();
            for row in table {
                let moved = self
                    .lines
                    .get(row)
                    .is_some_and(|laid| !laid.columns.is_empty() && laid.columns != columns);
                if moved {
                    self.lines.invalidate(row..row + 1);
                    dropped.push(row);
                }
            }
        }
        dropped.sort_unstable();
        dropped.dedup();
        dropped
    }

    /// The column widths of the table on `lines`: each row's cells measured once per text.
    fn table_columns(&mut self, text: &impl StyledLines, lines: Range<usize>) -> Result<Vec<f32>> {
        let mut rows = Vec::with_capacity(lines.len());
        for line in lines.clone() {
            let source = text.line(line);
            let key = row_key(&source, line == lines.start);
            if let Some(widths) = self.cell_widths.get(&key) {
                rows.push(Rc::clone(widths));
                continue;
            }
            let style = text.rendered_style(line);
            let widths: Rc<[f32]> = if matches!(style.block.body, BodyStyle::TableRow { .. }) {
                #[cfg(test)]
                {
                    let (laid, measured) = self.counts.get();
                    self.counts.set((laid, measured + 1));
                }
                let laid = self.lay_out(&source, style, None, Table::Measure)?;
                tables::measure_row(&laid.layout, &laid.offsets)?.into()
            } else {
                // The delimiter row: no cells.
                Rc::from([])
            };
            // Typing in a cell keeps adding texts; never let the cache grow without bound.
            if self.cell_widths.len() >= 4096 {
                self.cell_widths.clear();
            }
            self.cell_widths.insert(key, Rc::clone(&widths));
            rows.push(widths);
        }
        Ok(tables::columns(
            rows.iter().map(|row| &row[..]),
            self.fonts.size * 0.5,
        ))
    }

    /// Lays out the lines in `lines` that are not laid out yet.
    pub fn ensure(&mut self, text: &impl StyledLines, lines: Range<usize>) -> Result<()> {
        let end = lines.end.min(self.lines.len());
        for line in lines.start.min(end)..end {
            if self.lines.get(line).is_none() {
                #[cfg(test)]
                {
                    let (laid, measured) = self.counts.get();
                    self.counts.set((laid + 1, measured));
                }
                let laid = self.lay_out_line(text, line)?;
                self.heights.set_measured(line, laid.height);
                self.lines.set(line, laid);
            }
        }
        Ok(())
    }

    /// The laid-out `line`, laying it out first if needed.
    pub fn line(&mut self, text: &impl StyledLines, line: usize) -> Result<&LineLayout> {
        self.ensure(text, line..line + 1)?;
        self.lines
            .get(line)
            .ok_or(crate::FastPadError::Invariant("Live line out of range"))
    }

    /// `line`'s layout if it is laid out.
    pub fn laid_line(&self, line: usize) -> Option<&LineLayout> {
        self.lines.get(line)
    }

    pub fn line_top(&mut self, line: usize) -> f32 {
        self.heights.top(line)
    }

    /// The line at `y`, clamped to the document.
    pub fn line_at_y(&mut self, y: f32) -> usize {
        self.heights.index_at(y.max(0.0))
    }

    pub fn total_height(&mut self) -> f32 {
        self.heights.total()
    }

    pub fn anchor(&mut self, scroll: f32) -> (usize, f32) {
        self.heights.anchor(scroll)
    }

    pub fn scroll_for_anchor(&mut self, anchor: (usize, f32)) -> f32 {
        self.heights.scroll_for_anchor(anchor)
    }

    /// Where the caret is drawn at `position`: a zero-width rectangle as tall as its row.
    pub fn caret_rect(&mut self, text: &impl StyledLines, position: usize) -> Result<RectF> {
        let line = text.line_of(position);
        let local = position - text.line_start(line);
        let top = self.line_top(line);
        let laid = self.line(text, line)?;
        let (x, y) = position_point(&laid.layout, laid.offsets.to_utf16(local))?;
        let row = row_at_y(&laid.rows, y);
        let x = laid.indent + x;
        Ok(RectF::new(x, top + row.top, x, top + row.top + row.height))
    }

    /// The caret position nearest the point (`x`, `y`), on the row under it.
    pub fn position_at(&mut self, text: &impl StyledLines, x: f32, y: f32) -> Result<usize> {
        let line = self.line_at_y(y);
        let top = self.line_top(line);
        let start = text.line_start(line);
        let laid = self.line(text, line)?;
        let row = row_at_y(&laid.rows, y - top);
        Ok(start + hit_row(laid, &row, x - laid.indent)?)
    }

    /// The rectangles that show `range` selected on the lines in `visible`, with a narrow box
    /// at the end of each line whose line ending is selected.
    pub fn selection_rects(
        &mut self,
        text: &impl StyledLines,
        range: Range<usize>,
        visible: Range<usize>,
    ) -> Result<Vec<RectF>> {
        let mut rects = Vec::new();
        if range.is_empty() {
            return Ok(rects);
        }
        let first = text.line_of(range.start).max(visible.start);
        let last = text.line_of(range.end).min(visible.end.saturating_sub(1));
        let eol_width = self.fonts.size * 0.3;
        for line in first..=last.max(first) {
            if line >= self.lines.len() || line >= visible.end {
                break;
            }
            let start = text.line_start(line);
            let end = text.line_end(line);
            let top = self.line_top(line);
            let laid = self.line(text, line)?;
            let from = laid.offsets.to_utf16(range.start.max(start) - start);
            let to = laid.offsets.to_utf16(range.end.min(end).max(start) - start);
            rects.extend(range_rects(&laid.layout, from, to, laid.indent, top)?);
            if range.end > end && line + 1 < text.line_count() {
                let (x, y) = position_point(&laid.layout, laid.offsets.layout_text().len() as u32)?;
                let x = laid.indent + x;
                let row = row_at_y(&laid.rows, y);
                rects.push(RectF::new(
                    x,
                    top + row.top,
                    x + eol_width,
                    top + row.top + row.height,
                ));
            }
        }
        Ok(rects)
    }

    /// The vertical-motion geometry over this layout and `text`.
    pub fn geometry<'a, T: StyledLines>(
        &'a mut self,
        text: &'a T,
        page_height: f32,
    ) -> Geometry<'a, T> {
        Geometry {
            layout: self,
            text,
            page_height,
        }
    }

    fn estimate(&self, text: &impl StyledLines, line: usize) -> f32 {
        let bytes = text.line_end(line).saturating_sub(text.line_start(line));
        let rows = ((bytes as f32 * self.char_width) / self.width)
            .ceil()
            .max(1.0);
        rows * self.fonts.line_height
    }

    /// Lays out `line` from its style, finding an image line's picture first.
    fn lay_out_line(&mut self, text: &impl StyledLines, line: usize) -> Result<LineLayout> {
        let style = text.line_style(line);
        let source = text.line(line);
        if is_table(&style)
            && let Some(table) = text.table_lines(line)
        {
            let header = line == table.start;
            let columns = self.table_columns(text, table)?;
            let cells = self
                .cell_widths
                .get(&row_key(&source, header))
                .cloned()
                .unwrap_or_else(|| Rc::from([]));
            let table = Table::Columns {
                columns: &columns,
                cells: &cells,
            };
            return self.lay_out(&source, style, None, table);
        }
        let image = match &style.block.body {
            BodyStyle::Image { dest, .. } => match text.image(dest) {
                ImageInfo::Ready { path, size } => Some((path, size)),
                ImageInfo::Pending(_) | ImageInfo::Unavailable => None,
            },
            _ => None,
        };
        self.lay_out(&source, style, image, Table::No)
    }

    /// The indent of a line inside `containers`, and where each container's margin starts.
    fn indents(&self, containers: &[Container]) -> (f32, Vec<f32>) {
        let mut x = 0.0;
        let mut starts = Vec::with_capacity(containers.len());
        for container in containers {
            starts.push(x);
            x += container_width(container, self.fonts.size);
        }
        (x, starts)
    }

    /// Lays out `line` as `style` says; `image` is an image line's decoded picture (its file and
    /// natural size), without which the line shows its alt text.
    fn lay_out(
        &self,
        line: &str,
        style: LineStyle,
        image: Option<(PathBuf, (u32, u32))>,
        table: Table<'_>,
    ) -> Result<LineLayout> {
        let mut shown = if image.is_none() && matches!(style.block.body, BodyStyle::Image { .. }) {
            style.image_fallback()
        } else {
            style.clone()
        };
        let em = self.fonts.size;
        let (indent, starts) = self.indents(&shown.block.containers);
        // A deep nest still leaves a few words a row.
        let text_width = (self.width - indent).max((em * 4.0).min(self.width));
        let columns = match table {
            Table::Columns { columns, .. } => columns.to_vec(),
            Table::No | Table::Measure => Vec::new(),
        };
        let grid = !columns.is_empty() && columns.iter().sum::<f32>() <= text_width;
        if matches!(table, Table::Columns { .. }) && !grid {
            // Too wide for the view (no horizontal scrolling): the row shows as its source.
            shown = LineStyle::plain(line.len());
        }
        let offsets = OffsetMap::new(line, shown.pieces.iter().cloned());
        let layout = unsafe {
            self.graphics.dwrite.CreateTextLayout(
                offsets.layout_text(),
                &self.format,
                text_width,
                f32::MAX,
            )
        }
        .map_err(hresult_error)?;
        let whole = DWRITE_TEXT_RANGE {
            startPosition: 0,
            length: offsets.layout_text().len() as u32,
        };
        let heading_scale = shown
            .heading
            .map(|level| HEADING_SCALE[usize::from(level).clamp(1, 6) - 1]);
        let size = self.fonts.size * heading_scale.unwrap_or(1.0);
        if let Some(scale) = heading_scale {
            unsafe {
                layout.SetFontSize(size, whole).map_err(hresult_error)?;
                layout
                    .SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, whole)
                    .map_err(hresult_error)?;
                // The format's uniform spacing would clip a large heading.
                layout
                    .SetLineSpacing(
                        DWRITE_LINE_SPACING_METHOD_UNIFORM,
                        self.fonts.line_height * scale,
                        self.baseline * scale,
                    )
                    .map_err(hresult_error)?;
            }
        }
        let code_family = crate::platform::wide_null(&self.fonts.code_family);
        if let BodyStyle::Code { fence, info } = shown.block.body {
            // A code block's line: the code font at the code size, on the prose rows, so the
            // line numbers beside it stay in step.
            unsafe {
                layout
                    .SetFontFamilyName(PCWSTR(code_family.as_ptr()), whole)
                    .and_then(|()| layout.SetFontSize(size * CODE_SCALE, whole))
                    .and_then(|()| layout.SetFontWeight(self.fonts.code_weight, whole))
                    .map_err(hresult_error)?;
                if fence && info {
                    layout
                        .SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)
                        .map_err(hresult_error)?;
                }
            }
        }
        let mut backgrounds = Vec::new();
        for (range, kind) in &shown.runs {
            let span = offsets.to_utf16_range(range.clone());
            if span.start >= span.end {
                continue;
            }
            let dw = DWRITE_TEXT_RANGE {
                startPosition: span.start,
                length: span.end - span.start,
            };
            for effect in effects(*kind, shown.revealed) {
                let applied = unsafe {
                    match effect {
                        Effect::Bold if shown.heading.is_none() => {
                            layout.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, dw)
                        }
                        Effect::Bold | Effect::Color(_) => Ok(()),
                        Effect::Italic => layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, dw),
                        Effect::Strike => layout.SetStrikethrough(true, dw),
                        Effect::Underline => layout.SetUnderline(true, dw),
                        Effect::CodeFont => layout
                            .SetFontFamilyName(PCWSTR(code_family.as_ptr()), dw)
                            .and_then(|()| layout.SetFontSize(size * CODE_SCALE, dw))
                            .and_then(|()| layout.SetFontWeight(self.fonts.code_weight, dw)),
                        Effect::Background(paint) => {
                            backgrounds.push((span.start, span.end, *paint));
                            Ok(())
                        }
                    }
                };
                applied.map_err(hresult_error)?;
            }
        }
        if matches!(shown.block.body, BodyStyle::TableRow { .. }) {
            let widths = match table {
                Table::Columns { columns, cells } => {
                    let aligns = match &shown.block.body {
                        BodyStyle::TableRow { aligns, .. } => aligns.clone(),
                        _ => Rc::from([]),
                    };
                    tables::spacers(cells, columns, &aligns, em * 0.5)
                }
                Table::No | Table::Measure => Vec::new(),
            };
            unsafe {
                layout
                    .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
                    .map_err(hresult_error)?;
                for (index, &object) in offsets.objects().iter().enumerate() {
                    let width = widths.get(index).copied().unwrap_or(0.0);
                    let range = DWRITE_TEXT_RANGE {
                        startPosition: object,
                        length: 1,
                    };
                    layout
                        .SetInlineObject(&inline_box(width, 1.0), range)
                        .map_err(hresult_error)?;
                }
            }
        }
        let colors = shown
            .colour_runs()
            .into_iter()
            .filter_map(|(range, paint)| {
                let span = offsets.to_utf16_range(range);
                (span.start < span.end).then_some((
                    DWRITE_TEXT_RANGE {
                        startPosition: span.start,
                        length: span.end - span.start,
                    },
                    paint,
                ))
            })
            .collect();
        let metrics = line_metrics(&layout)?;
        let baseline = metrics
            .first()
            .map_or(self.baseline, |first| first.baseline);
        let mut rows = Vec::new();
        let mut start = 0;
        let mut top = 0.0;
        for metrics in &metrics {
            rows.push(RowMetrics {
                start,
                length: metrics.length,
                top,
                height: metrics.height,
            });
            start += metrics.length;
            top += metrics.height;
        }
        if rows.is_empty() {
            rows.push(RowMetrics {
                start: 0,
                length: 0,
                top: 0.0,
                height: self.fonts.line_height,
            });
            top = self.fonts.line_height;
        }
        let mut height = top;
        let mut decorations = Vec::new();
        match &shown.block.body {
            BodyStyle::Code { .. } => decorations.push(Decoration::Fill {
                rect: RectF::new(indent, 0.0, f32::MAX, height),
                paint: Paint::CodeBackground,
            }),
            BodyStyle::Rule => decorations.push(Decoration::Line {
                from: (indent, height / 2.0),
                to: (f32::MAX, height / 2.0),
                paint: Paint::Rule,
            }),
            BodyStyle::TableDelimiter => height = self.fonts.line_height * 0.5,
            BodyStyle::Image { .. } => {
                if let Some((path, natural)) = image {
                    let (width, image_height) = image_size(natural, text_width, self.fonts.zoom);
                    let pad = em * 0.25;
                    height = image_height + 2.0 * pad;
                    decorations.push(Decoration::Image {
                        rect: RectF::new(indent, pad, indent + width, pad + image_height),
                        path,
                    });
                }
            }
            BodyStyle::Text | BodyStyle::TableRow { .. } => {}
        }
        if height != top {
            // One row the line's whole height, so hits and vertical motion land on it.
            rows.truncate(1);
            rows[0].height = height;
        }
        if grid {
            decorations.extend(grid_lines(&shown.block.body, &columns, indent, height));
        }
        let first = rows[0];
        // Where a marker sits: centred on the first row's lower-case letters.
        let marker_y = first.top + baseline - em * 0.32;
        for (container, x) in shown.block.containers.iter().zip(starts) {
            match container {
                Container::Quote => decorations.push(Decoration::Fill {
                    rect: RectF::new(x, 0.0, x + em * 0.25, height),
                    paint: Paint::QuoteBar,
                }),
                Container::Item {
                    depth,
                    marker: Some(marker),
                    ..
                } => decorations.push(match marker {
                    Marker::Bullet => Decoration::Bullet {
                        center: (x + em * 0.55, marker_y),
                        radius: em * 0.17,
                        depth: *depth,
                    },
                    Marker::Ordered(range) => Decoration::Number {
                        rect: RectF::new(
                            x,
                            first.top,
                            x + container_width(container, em) - em * 0.35,
                            first.top + first.height,
                        ),
                        text: line.get(range.clone()).unwrap_or_default().to_owned(),
                    },
                    Marker::Task { checked, inner } => {
                        let side = em * 0.8;
                        Decoration::Checkbox {
                            rect: RectF::new(
                                x + em * 0.2,
                                marker_y - side / 2.0,
                                x + em * 0.2 + side,
                                marker_y + side / 2.0,
                            ),
                            checked: *checked,
                            inner: *inner,
                        }
                    }
                }),
                Container::Item { marker: None, .. } => {}
            }
        }
        Ok(LineLayout {
            layout,
            offsets,
            rows,
            height,
            indent,
            decorations,
            style,
            colors,
            backgrounds,
            columns,
            bound: Cell::new(0),
        })
    }

    fn rows_of(&mut self, text: &impl StyledLines, line: usize) -> Result<Vec<Row>> {
        let top = self.line_top(line);
        let line_start = text.line_start(line);
        let line_end = text.line_end(line);
        let laid = self.line(text, line)?;
        let count = laid.rows.len();
        Ok(laid
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let last = index + 1 == count;
                let start = line_start + laid.offsets.to_byte(row.start);
                let end = if last {
                    line_end
                } else {
                    line_start + laid.offsets.to_byte(row.start + row.length)
                };
                let caret_end = if last || row.length == 0 {
                    end
                } else {
                    line_start + laid.offsets.to_byte(row.start + row.length - 1)
                };
                Row {
                    line,
                    line_start,
                    start,
                    end,
                    caret_end,
                    top: top + row.top,
                    height: row.height,
                }
            })
            .collect())
    }
}

/// `LineGeometry` over a `LiveLayout`, for `motion::apply`. Lines are laid out as motion reaches
/// them. A DirectWrite failure reads as the document start, which motion then clamps.
pub struct Geometry<'a, T: StyledLines> {
    layout: &'a mut LiveLayout,
    text: &'a T,
    page_height: f32,
}

impl<T: StyledLines> Geometry<'_, T> {
    fn rows(&mut self, line: usize) -> Vec<Row> {
        self.layout.rows_of(self.text, line).unwrap_or_default()
    }

    fn fallback(&self, line: usize) -> Row {
        let start = self.text.line_start(line);
        let end = self.text.line_end(line);
        Row {
            line,
            line_start: start,
            start,
            end,
            caret_end: end,
            top: 0.0,
            height: self.layout.fonts.line_height,
        }
    }
}

impl<T: StyledLines> LineGeometry for Geometry<'_, T> {
    fn row_at(&mut self, position: usize) -> Row {
        let line = self.text.line_of(position);
        let rows = self.rows(line);
        rows.iter()
            .find(|row| position >= row.start && position < row.end)
            .or(rows.last())
            .copied()
            .unwrap_or_else(|| self.fallback(line))
    }

    fn row_above(&mut self, row: &Row) -> Option<Row> {
        let rows = self.rows(row.line);
        let index = rows.iter().position(|r| r.start == row.start)?;
        if index > 0 {
            return Some(rows[index - 1]);
        }
        let line = row.line.checked_sub(1)?;
        self.rows(line).last().copied()
    }

    fn row_below(&mut self, row: &Row) -> Option<Row> {
        let rows = self.rows(row.line);
        let index = rows.iter().position(|r| r.start == row.start)?;
        if let Some(next) = rows.get(index + 1) {
            return Some(*next);
        }
        let line = row.line + 1;
        if line >= self.text.line_count() {
            return None;
        }
        self.rows(line).first().copied()
    }

    fn x_of(&mut self, position: usize) -> f32 {
        self.layout
            .caret_rect(self.text, position)
            .map_or(0.0, |rect| rect.left)
    }

    fn position_at_x(&mut self, row: &Row, x: f32) -> usize {
        self.layout
            .position_at(self.text, x, row.top + row.height / 2.0)
            .map_or(row.start, |position| {
                position.clamp(row.start, row.caret_end)
            })
    }

    fn indent_end(&mut self, row: &Row) -> usize {
        let line = self.text.line(row.line);
        row.line_start + (line.len() - line.trim_start_matches([' ', '\t']).len())
    }

    fn page_height(&self) -> f32 {
        self.page_height
    }
}

/// A table line's grid in `columns` from `indent`: the header's fill and top border, each row's
/// bottom border, and the column edges down every row and the delimiter.
fn grid_lines(body: &BodyStyle, columns: &[f32], indent: f32, height: f32) -> Vec<Decoration> {
    let right = indent + columns.iter().sum::<f32>();
    let line = |from: (f32, f32), to: (f32, f32)| Decoration::Line {
        from,
        to,
        paint: Paint::Rule,
    };
    let mut decorations = Vec::new();
    if let BodyStyle::TableRow { header, .. } = body {
        if *header {
            decorations.push(Decoration::Fill {
                rect: RectF::new(indent, 0.0, right, height),
                paint: Paint::CodeBackground,
            });
            decorations.push(line((indent, 0.0), (right, 0.0)));
        }
        decorations.push(line((indent, height), (right, height)));
    }
    let mut x = indent;
    decorations.push(line((x, 0.0), (x, height)));
    for width in columns {
        x += width;
        decorations.push(line((x, 0.0), (x, height)));
    }
    decorations
}

/// How much a container indents the lines inside it, for text size `em`: a quote's bar and gap,
/// or a list item's marker box (wider for numbers).
fn container_width(container: &Container, em: f32) -> f32 {
    match container {
        Container::Quote => em,
        Container::Item { ordered: true, .. } => em * 2.0,
        Container::Item { .. } => em * 1.5,
    }
}

/// An image's size in DIPs: its natural pixels at the editor's `zoom`, at most 300 DIPs (times
/// the zoom) tall and `max_width` wide, keeping its aspect ratio.
fn image_size((width, height): (u32, u32), max_width: f32, zoom: f32) -> (f32, f32) {
    let (mut width, mut height) = (width as f32 * zoom, height as f32 * zoom);
    let cap = 300.0 * zoom;
    if height > cap {
        width *= cap / height;
        height = cap;
    }
    if width > max_width && width > 0.0 {
        height *= max_width / width;
        width = max_width;
    }
    (width, height)
}

/// The prose format with uniform rows, and the baseline within a row.
fn text_format(graphics: &Graphics, fonts: &LiveFonts) -> Result<(IDWriteTextFormat, f32)> {
    let format = graphics.text_format(
        &fonts.family,
        fonts.size,
        fonts.weight,
        DWRITE_FONT_STYLE_NORMAL,
    )?;
    // Every row gets the same height, the extra space split evenly above and below the text, as
    // the preview spaces its lines.
    let sample = "Ag".encode_utf16().collect::<Vec<_>>();
    let layout = unsafe {
        graphics
            .dwrite
            .CreateTextLayout(&sample, &format, 1000.0, f32::MAX)
    }
    .map_err(hresult_error)?;
    let natural = line_metrics(&layout)?.first().copied().unwrap_or_default();
    let baseline = natural.baseline + (fonts.line_height - natural.height) / 2.0;
    unsafe {
        format.SetLineSpacing(
            DWRITE_LINE_SPACING_METHOD_UNIFORM,
            fonts.line_height,
            baseline,
        )
    }
    .map_err(hresult_error)?;
    Ok((format, baseline))
}

/// The width of "0" in `format`: digits share one width in the fonts FastPad offers.
fn digit_width(graphics: &Graphics, format: &IDWriteTextFormat) -> Result<f32> {
    let zero = "0".encode_utf16().collect::<Vec<_>>();
    let layout = unsafe {
        graphics
            .dwrite
            .CreateTextLayout(&zero, format, 1000.0, f32::MAX)
    }
    .map_err(hresult_error)?;
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut metrics) }.map_err(hresult_error)?;
    Ok(metrics.widthIncludingTrailingWhitespace)
}

fn line_metrics(layout: &IDWriteTextLayout) -> Result<Vec<DWRITE_LINE_METRICS>> {
    let mut count = 0_u32;
    // The first call only reports how many lines there are.
    let _ = unsafe { layout.GetLineMetrics(None, &mut count) };
    let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
    unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count) }.map_err(hresult_error)?;
    lines.truncate(count as usize);
    Ok(lines)
}

/// The caret point (x, y within the line) at UTF-16 position `position`.
fn position_point(layout: &IDWriteTextLayout, position: u32) -> Result<(f32, f32)> {
    let (mut x, mut y) = (0.0, 0.0);
    let mut hit = DWRITE_HIT_TEST_METRICS::default();
    unsafe { layout.HitTestTextPosition(position, false, &mut x, &mut y, &mut hit) }
        .map_err(hresult_error)?;
    Ok((x, y))
}

/// The row holding `y` (within the line), clamped to the first and last rows.
fn row_at_y(rows: &[RowMetrics], y: f32) -> RowMetrics {
    rows.iter()
        .find(|row| y < row.top + row.height)
        .or(rows.last())
        .copied()
        .unwrap_or(RowMetrics {
            start: 0,
            length: 0,
            top: 0.0,
            height: 0.0,
        })
}

/// The byte offset within the line nearest `x` on `row`, never past the row's last caret
/// position (a click past a wrapped row's end stays on that row).
fn hit_row(laid: &LineLayout, row: &RowMetrics, x: f32) -> Result<usize> {
    let mut trailing = BOOL::default();
    let mut inside = BOOL::default();
    let mut hit = DWRITE_HIT_TEST_METRICS::default();
    let y = row.top + row.height / 2.0;
    unsafe {
        laid.layout
            .HitTestPoint(x, y, &mut trailing, &mut inside, &mut hit)
    }
    .map_err(hresult_error)?;
    let trailing = trailing.as_bool();
    let position = hit.textPosition + if trailing { hit.length } else { 0 };
    let last_row = row.start + row.length >= laid.offsets.layout_text().len() as u32;
    let row_end = if last_row || row.length == 0 {
        row.start + row.length
    } else {
        row.start + row.length - 1
    };
    let position = position.clamp(row.start, row_end);
    // A trailing hit stays inside the character it hit, before hidden markup that follows it
    // (`**bold**`: the right half of `d` puts the caret before the closing `**`).
    Ok(if trailing && position > row.start {
        laid.offsets.to_byte_after(position)
    } else {
        laid.offsets.to_byte(position)
    })
}

fn range_rects(
    layout: &IDWriteTextLayout,
    start: u32,
    end: u32,
    x: f32,
    y: f32,
) -> Result<Vec<RectF>> {
    let length = end.saturating_sub(start);
    if length == 0 {
        return Ok(Vec::new());
    }
    let mut count = 0_u32;
    // The first call only reports how many rectangles the range needs.
    let _ = unsafe { layout.HitTestTextRange(start, length, x, y, None, &mut count) };
    let mut hits = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
    unsafe { layout.HitTestTextRange(start, length, x, y, Some(&mut hits), &mut count) }
        .map_err(hresult_error)?;
    Ok(hits
        .iter()
        .take(count as usize)
        .map(|hit| {
            RectF::new(
                hit.left,
                hit.top,
                hit.left + hit.width,
                hit.top + hit.height,
            )
        })
        .collect())
}

mod tables;

#[cfg(test)]
mod tests;
