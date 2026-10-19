//! The Live view's text layout: one DirectWrite layout per source line, wrapped to the view's
//! width, laid out only when a line is about to be shown. Lines not laid out yet have an
//! estimated height in a `HeightIndex`, as the preview's blocks do. All y values are in document
//! coordinates (0 is the top of the first line); x values start at the text's left edge.

use crate::Result;
use crate::live::lines::LineCache;
use crate::live::motion::{LineGeometry, Row};
use crate::live::offsets::OffsetMap;
use crate::live::text::LiveText;
use crate::preview::dwrite::{Graphics, hresult_error};
use crate::preview::heights::HeightIndex;
use crate::preview::render::RectF;
use std::ops::Range;
use std::rc::Rc;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_HIT_TEST_METRICS, DWRITE_LINE_METRICS,
    DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    IDWriteTextFormat, IDWriteTextLayout,
};
use windows::core::BOOL;

/// The prose font, at the size the view draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveFonts {
    pub family: String,
    pub weight: DWRITE_FONT_WEIGHT,
    /// Text size in DIPs, zoom included.
    pub size: f32,
    /// Height of one row in DIPs.
    pub line_height: f32,
}

impl LiveFonts {
    /// The preview's prose font settings (`preview_font`, `preview_font_size` in 96-DPI pixels,
    /// `preview_line_height` in tenths), scaled by Scintilla's zoom the way Scintilla scales the
    /// editor font: `zoom` points added to `editor_font_size` points.
    pub fn new(
        graphics: &Graphics,
        preview_font: &str,
        preview_font_size: u16,
        line_height_tenths: u8,
        editor_font_size: u16,
        zoom: i32,
    ) -> Self {
        let resolved = graphics.resolve_family(preview_font);
        let base = f32::from(editor_font_size.max(1));
        let scale = ((base + zoom as f32) / base).max(1.0 / base);
        let size = f32::from(preview_font_size.max(1)) * scale;
        Self {
            family: resolved.name,
            weight: resolved.weight,
            size,
            line_height: size * (f32::from(line_height_tenths.max(10)) / 10.0),
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

/// One laid-out source line.
pub struct LineLayout {
    pub layout: IDWriteTextLayout,
    pub offsets: OffsetMap,
    rows: Vec<RowMetrics>,
    pub height: f32,
}

impl LineLayout {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

pub struct LiveLayout {
    format: IDWriteTextFormat,
    fonts: LiveFonts,
    width: f32,
    lines: LineCache<LineLayout>,
    heights: HeightIndex,
    /// The average character width, for height estimates.
    char_width: f32,
    /// The width of one digit, for the line-number gutter.
    digit_width: f32,
    /// Last, so the layouts and format above are released before it can unload DirectWrite.
    graphics: Rc<Graphics>,
}

impl LiveLayout {
    pub fn new(graphics: Rc<Graphics>, fonts: LiveFonts, width: f32) -> Result<Self> {
        let format = text_format(&graphics, &fonts)?;
        let char_width = fonts.size * 0.5;
        let digit_width = digit_width(&graphics, &format)?;
        Ok(Self {
            graphics,
            format,
            fonts,
            width: width.max(1.0),
            lines: LineCache::default(),
            heights: HeightIndex::default(),
            char_width,
            digit_width,
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
    pub fn reset(&mut self, text: &impl LiveText) {
        let count = text.line_count();
        self.lines.reset(count);
        let estimates = (0..count)
            .map(|line| self.estimate(text, line))
            .collect::<Vec<_>>();
        self.heights.reset(estimates);
    }

    /// A new text width; every line is laid out again. The caller keeps its scroll anchor.
    pub fn set_width(&mut self, text: &impl LiveText, width: f32) {
        let width = width.max(1.0);
        if width != self.width {
            self.width = width;
            self.reset(text);
        }
    }

    /// New fonts (zoom, settings, DPI); every line is laid out again.
    pub fn set_fonts(&mut self, text: &impl LiveText, fonts: LiveFonts) -> Result<()> {
        if fonts != self.fonts {
            self.format = text_format(&self.graphics, &fonts)?;
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
        text: &impl LiveText,
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

    /// Lays out the lines in `lines` that are not laid out yet.
    pub fn ensure(&mut self, text: &impl LiveText, lines: Range<usize>) -> Result<()> {
        let end = lines.end.min(self.lines.len());
        for line in lines.start.min(end)..end {
            if self.lines.get(line).is_none() {
                let laid = self.lay_out(&text.line(line))?;
                self.heights.set_measured(line, laid.height);
                self.lines.set(line, laid);
            }
        }
        Ok(())
    }

    /// The laid-out `line`, laying it out first if needed.
    pub fn line(&mut self, text: &impl LiveText, line: usize) -> Result<&LineLayout> {
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
    pub fn caret_rect(&mut self, text: &impl LiveText, position: usize) -> Result<RectF> {
        let line = text.line_of(position);
        let local = position - text.line_start(line);
        let top = self.line_top(line);
        let laid = self.line(text, line)?;
        let (x, y) = position_point(&laid.layout, laid.offsets.to_utf16(local))?;
        let row = row_at_y(&laid.rows, y);
        Ok(RectF::new(x, top + row.top, x, top + row.top + row.height))
    }

    /// The caret position nearest the point (`x`, `y`), on the row under it.
    pub fn position_at(&mut self, text: &impl LiveText, x: f32, y: f32) -> Result<usize> {
        let line = self.line_at_y(y);
        let top = self.line_top(line);
        let start = text.line_start(line);
        let laid = self.line(text, line)?;
        let row = row_at_y(&laid.rows, y - top);
        Ok(start + hit_row(laid, &row, x)?)
    }

    /// The rectangles that show `range` selected on the lines in `visible`, with a narrow box
    /// at the end of each line whose line ending is selected.
    pub fn selection_rects(
        &mut self,
        text: &impl LiveText,
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
            rects.extend(range_rects(&laid.layout, from, to, 0.0, top)?);
            if range.end > end && line + 1 < text.line_count() {
                let (x, y) = position_point(&laid.layout, laid.offsets.layout_text().len() as u32)?;
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
    pub fn geometry<'a, T: LiveText>(
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

    fn estimate(&self, text: &impl LiveText, line: usize) -> f32 {
        let bytes = text.line_end(line).saturating_sub(text.line_start(line));
        let rows = ((bytes as f32 * self.char_width) / self.width)
            .ceil()
            .max(1.0);
        rows * self.fonts.line_height
    }

    fn lay_out(&self, line: &str) -> Result<LineLayout> {
        let offsets = OffsetMap::plain(line);
        let layout = unsafe {
            self.graphics.dwrite.CreateTextLayout(
                offsets.layout_text(),
                &self.format,
                self.width,
                f32::MAX,
            )
        }
        .map_err(hresult_error)?;
        let mut rows = Vec::new();
        let mut start = 0;
        let mut top = 0.0;
        for metrics in line_metrics(&layout)? {
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
        Ok(LineLayout {
            layout,
            offsets,
            rows,
            height: top,
        })
    }

    fn rows_of(&mut self, text: &impl LiveText, line: usize) -> Result<Vec<Row>> {
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
pub struct Geometry<'a, T: LiveText> {
    layout: &'a mut LiveLayout,
    text: &'a T,
    page_height: f32,
}

impl<T: LiveText> Geometry<'_, T> {
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

impl<T: LiveText> LineGeometry for Geometry<'_, T> {
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

fn text_format(graphics: &Graphics, fonts: &LiveFonts) -> Result<IDWriteTextFormat> {
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
    Ok(format)
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
    let mut position = hit.textPosition + if trailing.as_bool() { hit.length } else { 0 };
    let last_row = row.start + row.length >= laid.offsets.layout_text().len() as u32;
    let row_end = if last_row || row.length == 0 {
        row.start + row.length
    } else {
        row.start + row.length - 1
    };
    position = position.clamp(row.start, row_end);
    Ok(laid.offsets.to_byte(position))
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

#[cfg(test)]
mod tests;
