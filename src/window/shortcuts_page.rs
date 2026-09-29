//! The Keyboard Shortcuts page of the Settings dialog: where its parts sit, what the pointer is
//! on, and how it paints (keyboard shortcuts spec §6.2–§6.5). Behaviour lives in
//! `shortcuts_model`; `settings_dialog` routes input here.

use super::keymap::KeyStroke;
use super::palette::Palette;
use super::panel::{inset, scale};
use super::shortcuts_model::ShortcutsModel;
use super::soft_paint::{Frame, Shape, Tones};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{DT_CENTER, DT_LEFT, DT_RIGHT, HFONT};

const PADDING_AT_96_DPI: i32 = 20;
const SEARCH_TOP_AT_96_DPI: i32 = 12;
const FIELD_HEIGHT_AT_96_DPI: i32 = 30;
const GAP_AT_96_DPI: i32 = 8;
const HEADER_HEIGHT_AT_96_DPI: i32 = 28;
const ROW_HEIGHT_AT_96_DPI: i32 = 28;
const PENCIL_WIDTH_AT_96_DPI: i32 = 24;
const CELL_INSET_AT_96_DPI: i32 = 8;
const KEYCAP_HEIGHT_AT_96_DPI: i32 = 20;
const KEYCAP_PADDING_AT_96_DPI: i32 = 6;
const KEYCAP_GAP_AT_96_DPI: i32 = 4;
const RECORD_WIDTH_AT_96_DPI: i32 = 440;
const RECORD_HEIGHT_AT_96_DPI: i32 = 150;
const RECORD_INSET_AT_96_DPI: i32 = 16;
/// The selected row's accent bar while the table has the focus.
const SELECTED_BAR_AT_96_DPI: i32 = 3;
pub(crate) const GLYPH_PENCIL: &str = "\u{E70F}";
pub(crate) const GLYPH_KEYBOARD: &str = "\u{E765}";
const RECORD_PROMPT: &str = "Press desired key combination and then press ENTER.";
const HEADERS: [&str; 3] = ["Command", "Keybinding", "Source"];

/// What the pointer is on. While the recording box is open, everything outside it is
/// `OutsideRecordBox`: a click there closes the box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageHit {
    RecordToggle,
    Row(usize),
    Pencil(usize),
    ConflictLink,
    RecordBox,
    OutsideRecordBox,
}

/// Where the page's parts sit, in client coordinates.
#[derive(Clone, Copy)]
pub(crate) struct PageLayout {
    pub search: RECT,
    pub record_toggle: RECT,
    pub header: RECT,
    pub table: RECT,
    /// Command, Keybinding and Source, left and right edges.
    pub columns: [(i32, i32); 3],
    pub record_box: RECT,
    pub row_height: i32,
    dpi: u32,
}

impl PageLayout {
    /// The page inside the dialog's `body` at `dpi`.
    pub(crate) fn calculate(body: RECT, dpi: u32) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let field = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
        let gap = scale(GAP_AT_96_DPI, dpi);
        let (left, right) = (body.left + padding, body.right - padding);
        let top = body.top + scale(SEARCH_TOP_AT_96_DPI, dpi);
        let record_toggle = RECT {
            left: right - field,
            top,
            right,
            bottom: top + field,
        };
        let search = RECT {
            left,
            top,
            right: record_toggle.left - gap,
            bottom: top + field,
        };
        let header_top = search.bottom + gap;
        let header = RECT {
            left,
            top: header_top,
            right,
            bottom: header_top + scale(HEADER_HEIGHT_AT_96_DPI, dpi),
        };
        let table = RECT {
            left,
            top: header.bottom,
            right,
            bottom: (body.bottom - gap).max(header.bottom),
        };
        let width = right - left;
        let command_end = left + width * 55 / 100;
        let keys_end = left + width * 85 / 100;
        let record_width = scale(RECORD_WIDTH_AT_96_DPI, dpi).min(width);
        let record_height = scale(RECORD_HEIGHT_AT_96_DPI, dpi).min(table.bottom - header.top);
        let box_left = (left + right - record_width) / 2;
        let box_top = (header.top + table.bottom - record_height) / 2;
        Self {
            search,
            record_toggle,
            header,
            table,
            columns: [
                (left, command_end),
                (command_end, keys_end),
                (keys_end, right),
            ],
            record_box: RECT {
                left: box_left,
                top: box_top,
                right: box_left + record_width,
                bottom: box_top + record_height,
            },
            row_height: scale(ROW_HEIGHT_AT_96_DPI, dpi),
            dpi,
        }
    }

    /// How many whole rows the table shows (at least one).
    pub(crate) fn visible_rows(&self) -> usize {
        ((self.table.bottom - self.table.top) / self.row_height).max(1) as usize
    }

    /// The `slot`th visible row.
    pub(crate) fn row_rect(&self, slot: usize) -> RECT {
        let top = self.table.top + slot as i32 * self.row_height;
        RECT {
            left: self.table.left,
            top,
            right: self.table.right,
            bottom: top + self.row_height,
        }
    }

    /// The pencil at the left end of `row`.
    pub(crate) fn pencil_rect(&self, row: RECT) -> RECT {
        RECT {
            right: row.left + scale(PENCIL_WIDTH_AT_96_DPI, self.dpi),
            ..row
        }
    }

    fn cell(&self, row: RECT, column: usize) -> RECT {
        let (left, right) = self.columns[column];
        let inset = scale(CELL_INSET_AT_96_DPI, self.dpi);
        RECT {
            left: left + inset,
            right: right - inset,
            ..row
        }
    }

    /// The recording box's prompt, keys, and refusal-or-link lines.
    pub(crate) fn record_lines(&self) -> [RECT; 3] {
        let inner = inset(self.record_box, scale(RECORD_INSET_AT_96_DPI, self.dpi));
        let height = (inner.bottom - inner.top) / 3;
        std::array::from_fn(|index| RECT {
            top: inner.top + index as i32 * height,
            bottom: inner.top + (index as i32 + 1) * height,
            ..inner
        })
    }

    /// What client point `x`, `y` is on, with `model`'s rows, scroll and recording box.
    pub(crate) fn hit(&self, x: i32, y: i32, model: &ShortcutsModel) -> Option<PageHit> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if model.recording.is_some() {
            if model.conflict_count() > 0 && inside(&self.record_lines()[2]) {
                return Some(PageHit::ConflictLink);
            }
            return Some(if inside(&self.record_box) {
                PageHit::RecordBox
            } else {
                PageHit::OutsideRecordBox
            });
        }
        if inside(&self.record_toggle) {
            return Some(PageHit::RecordToggle);
        }
        if !inside(&self.table) {
            return None;
        }
        let slot = ((y - self.table.top) / self.row_height) as usize;
        let index = model.top + slot;
        if slot >= self.visible_rows() || index >= model.rows.len() {
            return None;
        }
        let row = self.row_rect(slot);
        Some(if inside(&self.pencil_rect(row)) {
            PageHit::Pencil(index)
        } else {
            PageHit::Row(index)
        })
    }
}

/// What the page paints with.
pub(crate) struct PageStyle<'a> {
    pub colors: &'a Palette,
    pub tones: &'a Tones,
    pub link_color: u32,
    pub body_font: HFONT,
    pub heading_font: HFONT,
    pub link_font: HFONT,
    pub glyph_font: HFONT,
    pub radius: i32,
    pub hot: Option<PageHit>,
    pub table_focused: bool,
}

/// Keycaps for `stroke` in `line`, from `left` (or centred when `left` is `None`). `measure`
/// gives a text's width in the body font.
fn keycaps(
    frame: &mut Frame<'_>,
    style: &PageStyle<'_>,
    measure: &dyn Fn(&str) -> i32,
    stroke: KeyStroke,
    line: RECT,
    left: Option<i32>,
    dpi: u32,
) {
    let parts = stroke.parts();
    let padding = scale(KEYCAP_PADDING_AT_96_DPI, dpi);
    let gap = scale(KEYCAP_GAP_AT_96_DPI, dpi);
    let plus = measure("+");
    let widths = parts
        .iter()
        .map(|part| measure(part) + 2 * padding)
        .collect::<Vec<_>>();
    let total = widths.iter().sum::<i32>() + (widths.len() as i32 - 1) * (plus + 2 * gap);
    let mut x = left.unwrap_or((line.left + line.right - total) / 2);
    let height = scale(KEYCAP_HEIGHT_AT_96_DPI, dpi);
    let top = (line.top + line.bottom - height) / 2;
    for (index, (part, width)) in parts.into_iter().zip(widths).enumerate() {
        if index > 0 {
            let sign = RECT {
                left: x + gap,
                top,
                right: x + gap + plus,
                bottom: top + height,
            };
            frame.text(
                style.body_font,
                style.colors.muted_foreground,
                "+",
                sign,
                DT_CENTER,
            );
            x += plus + 2 * gap;
        }
        let cap = RECT {
            left: x,
            top,
            right: x + width,
            bottom: top + height,
        };
        style
            .tones
            .soft(frame, cap, style.radius, style.tones.control);
        frame.shape(Shape::Ring {
            rect: cap,
            radius: style.radius,
            width: 1,
            color: style.tones.outline.unwrap_or(style.tones.control_down),
        });
        frame.text(
            style.body_font,
            style.colors.editor_foreground,
            part,
            cap,
            DT_CENTER,
        );
        x += width;
    }
}

/// Paints the page. `measure` gives a text's width in the body font.
pub(crate) fn compose<'a>(
    frame: &mut Frame<'a>,
    measure: &dyn Fn(&str) -> i32,
    layout: &PageLayout,
    model: &'a ShortcutsModel,
    style: &PageStyle<'_>,
) {
    let dpi = layout.dpi;
    let colors = style.colors;
    let tones = style.tones;

    // The search field's box (the EDIT sits inside it) and the record-keys toggle.
    tones.soft(frame, layout.search, style.radius, tones.control);
    let toggle_color = match (model.record_keys, style.hot == Some(PageHit::RecordToggle)) {
        (true, _) => tones.accent,
        (false, true) => tones.control_hot,
        (false, false) => tones.control,
    };
    tones.soft(frame, layout.record_toggle, style.radius, toggle_color);
    let glyph = if model.record_keys {
        tones.on_accent
    } else {
        colors.editor_foreground
    };
    frame.text(
        style.glyph_font,
        glyph,
        GLYPH_KEYBOARD,
        layout.record_toggle,
        DT_CENTER,
    );

    // Header, with a rule under it.
    let pencil = scale(PENCIL_WIDTH_AT_96_DPI, dpi);
    for (column, title) in HEADERS.into_iter().enumerate() {
        let cell = layout.cell(layout.header, column);
        let cell = if column == 0 {
            RECT {
                left: cell.left + pencil - scale(CELL_INSET_AT_96_DPI, dpi),
                ..cell
            }
        } else {
            cell
        };
        frame.text(
            style.heading_font,
            colors.muted_foreground,
            title,
            cell,
            DT_LEFT,
        );
    }
    frame.shape(Shape::Fill {
        rect: RECT {
            top: layout.header.bottom - 1,
            ..layout.header
        },
        color: tones.card,
    });

    // Rows.
    frame.clip(Some(layout.table));
    for slot in 0..layout.visible_rows() {
        let index = model.top + slot;
        let Some(row) = model.rows.get(index) else {
            break;
        };
        let rect = layout.row_rect(slot);
        let selected = index == model.selected;
        let hot = matches!(style.hot, Some(PageHit::Row(i) | PageHit::Pencil(i)) if i == index);
        if selected || hot {
            let fill = match (selected, style.table_focused) {
                (true, true) => tones.control_hot,
                (true, false) => tones.control,
                (false, _) => tones.card_hot,
            };
            tones.soft(frame, rect, style.radius, fill);
        }
        if selected && style.table_focused {
            let bar = scale(SELECTED_BAR_AT_96_DPI, dpi);
            frame.shape(Shape::Fill {
                rect: RECT {
                    right: rect.left + bar,
                    ..rect
                },
                color: tones.accent,
            });
        }
        if selected || hot {
            frame.text(
                style.glyph_font,
                colors.muted_foreground,
                GLYPH_PENCIL,
                layout.pencil_rect(rect),
                DT_CENTER,
            );
        }
        let command = layout.cell(rect, 0);
        let command = RECT {
            left: command.left + pencil - scale(CELL_INSET_AT_96_DPI, dpi),
            ..command
        };
        frame.text(
            style.body_font,
            colors.editor_foreground,
            row.title,
            command,
            DT_LEFT,
        );
        if selected {
            frame.text(
                style.body_font,
                colors.muted_foreground,
                row.id,
                command,
                DT_RIGHT,
            );
        }
        let keys = layout.cell(rect, 1);
        match row.stroke {
            Some(stroke) => keycaps(frame, style, measure, stroke, keys, Some(keys.left), dpi),
            None => frame.text(
                style.body_font,
                colors.muted_foreground,
                "\u{2014}",
                keys,
                DT_LEFT,
            ),
        }
        let source = match (row.user, row.stroke.is_some()) {
            (true, _) => "User",
            (false, true) => "Default",
            (false, false) => "",
        };
        frame.text(
            style.body_font,
            colors.muted_foreground,
            source,
            layout.cell(rect, 2),
            DT_LEFT,
        );
    }
    frame.clip(None);

    // The recording box, over the table.
    if let Some(recording) = &model.recording {
        let box_rect = layout.record_box;
        tones.soft(frame, box_rect, style.radius, colors.panel_background());
        frame.shape(Shape::Ring {
            rect: box_rect,
            radius: style.radius,
            width: scale(2, dpi),
            color: tones.accent,
        });
        let [prompt, keys, note] = layout.record_lines();
        frame.text(
            style.body_font,
            colors.editor_foreground,
            RECORD_PROMPT,
            prompt,
            DT_CENTER,
        );
        if let Some(stroke) = recording.stroke {
            keycaps(frame, style, measure, stroke, keys, None, dpi);
        }
        if let Some(refusal) = recording.refusal {
            frame.text(
                style.body_font,
                colors.error_foreground,
                refusal,
                note,
                DT_CENTER,
            );
        } else {
            let count = model.conflict_count();
            if count > 0 {
                let text = if count == 1 {
                    "1 existing command has this keybinding".to_owned()
                } else {
                    format!("{count} existing commands have this keybinding")
                };
                frame.text(style.link_font, style.link_color, text, note, DT_CENTER);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::keymap::{KeyStroke, Keymap};
    use crate::window::shortcuts_model::ShortcutsModel;

    fn body() -> RECT {
        RECT {
            left: 180,
            top: 44,
            right: 860,
            bottom: 700,
        }
    }

    fn center(rect: RECT) -> (i32, i32) {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    #[test]
    fn the_table_fills_the_body_under_the_search_row() {
        // Break caught: the header or table overlapping the search field, or rows running past
        // the body into the footer.
        let layout = PageLayout::calculate(body(), 96);
        assert!(layout.search.bottom < layout.header.top);
        assert_eq!(layout.header.bottom, layout.table.top);
        assert!(layout.table.bottom <= body().bottom);
        let last = layout.row_rect(layout.visible_rows() - 1);
        assert!(last.bottom <= layout.table.bottom);
        assert!(layout.record_toggle.left > layout.search.right);
    }

    #[test]
    fn rows_pencils_and_the_record_box_hit() {
        // Break caught: a click landing on the wrong row once the table has scrolled, the pencil
        // opening nothing, or clicks behind the open recording box reaching the rows.
        let layout = PageLayout::calculate(body(), 96);
        let mut model = ShortcutsModel::new(Keymap::defaults(), layout.visible_rows());
        model.scroll(2);
        let row = layout.row_rect(1);
        let (x, y) = center(row);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::Row(3)));
        let (x, y) = center(layout.pencil_rect(row));
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::Pencil(3)));
        let (x, y) = center(layout.record_toggle);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::RecordToggle));

        model.select(3);
        model.start_change();
        let (x, y) = center(layout.record_box);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::RecordBox));
        assert_eq!(
            layout.hit(layout.table.left + 1, layout.table.bottom - 1, &model),
            Some(PageHit::OutsideRecordBox)
        );
        // The link only exists while the recorded key clashes.
        let (x, y) = center(layout.record_lines()[2]);
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::RecordBox));
        model.record_key(KeyStroke::parse("Ctrl+S").unwrap());
        assert_eq!(layout.hit(x, y, &model), Some(PageHit::ConflictLink));
    }

    #[test]
    fn a_row_below_the_last_one_is_not_a_hit() {
        // Break caught: a click under a short filtered list selecting a row that isn't there.
        let layout = PageLayout::calculate(body(), 96);
        let mut model = ShortcutsModel::new(Keymap::defaults(), layout.visible_rows());
        model.set_text("save as");
        let (x, y) = center(layout.row_rect(3));
        assert_eq!(layout.hit(x, y, &model), None);
    }
}
