//! Painting the Search view: the header, summary and status lines, the result rows with their
//! snippets, the panel's `WM_PAINT`, and the edit controls' colors.

use super::*;
use crate::search::Snippet;
use crate::window::design::metrics::CONTROL_RADIUS;
use crate::window::design::metrics::scale;
use crate::window::design::round::{Corners, fill_bordered, fill_rounded, radius_for};
use crate::window::design::text_scale::scale_text;
use crate::window::file_icons::note_kind;
use crate::window::icon_sets::TreeItem;
use crate::window::notebook_view::draw_item_icon;
use crate::window::option_toggles;
use crate::window::palette::Palette;
use crate::window::panel::fill;
use crate::window::row_list::{self, RowLook, row_foreground};
use crate::window::side_panel::{UiFonts, ViewPaint, draw_text};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_EXPANDTABS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DrawTextW, HBRUSH, HDC, HFONT, SelectObject, SetBkColor, SetTextColor,
};

/// A painted button's glyph color: dim while it can't run, the hover color under the pointer.
pub(super) fn button_color(enabled: bool, hover: bool, normal: u32, palette: &Palette) -> u32 {
    if !enabled {
        palette.line_number_foreground
    } else if hover {
        palette.hover_foreground
    } else {
        normal
    }
}

/// A row's replace button's glyph color. On the focused selection the line-number color may have
/// little contrast with the selection's background, so a disabled button there is the row's text
/// color blended halfway into that background: dimmer than an enabled button, still legible. High
/// contrast allows only system color pairs, so there it keeps the selection's text color (the
/// line-number color is the window text, drawn for the window background, not the highlight).
pub(super) fn row_button_color(
    enabled: bool,
    hover: bool,
    look: RowLook,
    palette: &Palette,
) -> u32 {
    let foreground = row_foreground(look, palette);
    if !enabled && look.selected && look.focused {
        if palette.high_contrast {
            foreground
        } else {
            crate::catppuccin::blend(foreground, palette.selection_background, 128)
        }
    } else {
        button_color(enabled, hover, foreground, palette)
    }
}

/// The part of a snippet before its match, cut from the start (with `…`) until it is at most
/// `room` pixels wide, so the match stays in view in a narrow panel. `measure` gives a text's
/// width. Empty when not even `…` and one character fit.
pub(super) fn fit_before(before: &str, room: i32, measure: impl Fn(&str) -> i32) -> String {
    if measure(before) <= room {
        return before.to_owned();
    }
    let starts = before
        .char_indices()
        .map(|(index, _)| index)
        .skip(1)
        .collect::<Vec<_>>();
    let cut = |start: usize| format!("\u{2026}{}", &before[start..]);
    // A later start is never wider: find the first that fits.
    let first = starts.partition_point(|&start| measure(&cut(start)) > room);
    starts
        .get(first)
        .map_or_else(String::new, |&start| cut(start))
}

/// `text`'s width in `font`, measured as `draw_snippet` draws it: on one line, tabs expanded.
pub(super) fn text_width(hdc: HDC, text: &str, font: HFONT) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT::default();
    unsafe {
        let previous = (!font.is_null()).then(|| SelectObject(hdc, font));
        DrawTextW(
            hdc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX | DT_EXPANDTABS,
        );
        if let Some(previous) = previous {
            SelectObject(hdc, previous);
        }
    }
    (rect.right - rect.left).max(0)
}

/// A result's second line: the snippet, its match in bold. The text before the match is cut
/// from its start when the row is too narrow, so the match stays in view. The snippet is the raw
/// line and can hold tabs, so they are expanded (`DT_EXPANDTABS`) rather than drawn as boxes.
pub(super) unsafe fn draw_snippet(
    hdc: HDC,
    snippet: &Snippet,
    rect: RECT,
    fonts: UiFonts,
    color: u32,
) {
    let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT | DT_EXPANDTABS;
    let text = snippet.text.as_str();
    let range = snippet.highlight.clone();
    let (Some(before), Some(matched), Some(after)) = (
        text.get(..range.start),
        text.get(range.clone()),
        text.get(range.end..),
    ) else {
        // Never made by `snippet::cut`, but a bad range draws the text plain, never panics.
        unsafe { draw_text(hdc, text, rect, fonts.text, color, line | DT_END_ELLIPSIS) };
        return;
    };
    let available = (rect.right - rect.left).max(0);
    let room = (available - text_width(hdc, matched, fonts.text_bold)).max(available / 3);
    let before = fit_before(before, room, |part| text_width(hdc, part, fonts.text));
    let mut left = rect.left;
    unsafe {
        left += draw_text(hdc, &before, RECT { left, ..rect }, fonts.text, color, line);
        if left < rect.right {
            left += draw_text(
                hdc,
                matched,
                RECT { left, ..rect },
                fonts.text_bold,
                color,
                line | DT_END_ELLIPSIS,
            );
        }
        if left < rect.right {
            draw_text(
                hdc,
                after,
                RECT { left, ..rect },
                fonts.text,
                color,
                line | DT_END_ELLIPSIS,
            );
        }
    }
}

/// A text field's box: `palette.editor_background` inside a one-pixel `selection_background`
/// border, with rounded corners over `behind`.
pub(super) unsafe fn paint_field(hdc: HDC, field: RECT, palette: &Palette, behind: u32, dpi: u32) {
    unsafe {
        fill_bordered(
            hdc,
            field,
            radius_for(palette, CONTROL_RADIUS, dpi),
            palette.editor_background,
            palette.selection_background,
            behind,
        );
    }
}

/// A hovered button's rounded shading over `behind`.
pub(super) unsafe fn paint_hover(hdc: HDC, rect: RECT, palette: &Palette, behind: u32, dpi: u32) {
    unsafe {
        fill_rounded(
            hdc,
            rect,
            radius_for(palette, CONTROL_RADIUS, dpi),
            Corners::ALL,
            palette.hover_background,
            behind,
        );
    }
}

impl SearchView {
    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT | DT_END_ELLIPSIS;
        let glyph = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
        unsafe {
            fill(paint.hdc, client, paint.background);
            let title = Self::title_rect(client, dpi);
            draw_text(
                paint.hdc,
                TITLE,
                RECT {
                    left: title.left + scale(TITLE_INSET_AT_96_DPI, dpi),
                    ..title
                },
                paint.fonts.bold,
                palette.muted_foreground,
                line,
            );
            if self.edit.is_some() {
                let field = Self::field_rect(client, dpi);
                paint_field(paint.hdc, field, &palette, paint.background, dpi);
                option_toggles::paint(
                    paint.hdc,
                    &option_toggles::toggle_rects(field, dpi),
                    self.options,
                    self.toggle_hover,
                    &palette,
                    paint.fonts.text,
                    dpi,
                );
                let chevron = Self::chevron_rect(client, dpi);
                let hover = self.header_hover == Some(HeaderButton::Chevron);
                if hover {
                    paint_hover(paint.hdc, chevron, &palette, paint.background, dpi);
                }
                draw_text(
                    paint.hdc,
                    if self.replace_open {
                        CHEVRON_OPEN_GLYPH
                    } else {
                        CHEVRON_CLOSED_GLYPH
                    },
                    chevron,
                    paint.fonts.glyph,
                    button_color(true, hover, palette.muted_foreground, &palette),
                    glyph,
                );
                if self.clear_shown() {
                    let clear = Self::clear_rect(client, dpi);
                    let hover = self.header_hover == Some(HeaderButton::Clear);
                    if hover {
                        paint_hover(paint.hdc, clear, &palette, paint.background, dpi);
                    }
                    draw_text(
                        paint.hdc,
                        CLEAR_GLYPH,
                        clear,
                        paint.fonts.glyph,
                        button_color(true, hover, palette.muted_foreground, &palette),
                        glyph,
                    );
                }
                if self.replace_open {
                    let replace = Self::replace_field_rect(client, dpi);
                    paint_field(paint.hdc, replace, &palette, paint.background, dpi);
                    let all = Self::replace_all_rect(client, dpi);
                    let enabled = self.replace_all_enabled();
                    let hover = enabled && self.header_hover == Some(HeaderButton::ReplaceAll);
                    if hover {
                        paint_hover(paint.hdc, all, &palette, paint.background, dpi);
                    }
                    draw_text(
                        paint.hdc,
                        REPLACE_GLYPH,
                        all,
                        paint.fonts.glyph,
                        button_color(enabled, hover, palette.editor_foreground, &palette),
                        glyph,
                    );
                }
            }
            let summary = self.summary_rect(client, dpi);
            if let Some(notice) = self.notice() {
                draw_text(
                    paint.hdc,
                    notice,
                    summary,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line,
                );
                return;
            }
            if let Some((text, error)) = self.summary() {
                let color = if error {
                    palette.error_foreground
                } else {
                    palette.muted_foreground
                };
                draw_text(paint.hdc, &text, summary, paint.fonts.text, color, line);
            }
            if let Some(status) = self.status_line() {
                draw_text(
                    paint.hdc,
                    &status,
                    Self::status_rect(client, dpi),
                    paint.fonts.text,
                    palette.muted_foreground,
                    line,
                );
            }
        }
        let area = self.list_area(client, dpi);
        row_list::paint(
            paint.hdc,
            area,
            &self.list,
            &palette,
            paint.focused,
            paint.focused,
            &|_| paint.background,
            dpi,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }

    /// One result on two lines: the file icon, the name and its folder in dim text, then the
    /// snippet with its match in bold. The hovered and the selected row show their replace
    /// button at the right end while the replace field is open; both lines stop short of it.
    pub(super) fn draw_row(
        &self,
        hdc: HDC,
        index: usize,
        rect: RECT,
        look: RowLook,
        paint: &ViewPaint,
    ) {
        let Some(result) = self.results.get(index) else {
            return;
        };
        let dpi = paint.dpi;
        let palette = paint.palette;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let slot = scale_text(ROW_LINE_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let first = RECT {
            top: rect.top + scale(ROW_INSET_AT_96_DPI, dpi),
            bottom: rect.top + scale(ROW_INSET_AT_96_DPI, dpi) + slot,
            ..rect
        };
        let glyph = RECT {
            left: rect.left + pad,
            right: rect.left + pad + scale(GLYPH_AT_96_DPI, dpi),
            ..first
        };
        let foreground = row_foreground(look, &palette);
        // Over the focused selection, dim text takes the selection's text color to stay legible.
        let muted = if look.selected && look.focused {
            foreground
        } else {
            palette.muted_foreground
        };
        let button = self
            .row_button_shown(index)
            .then(|| Self::row_replace_rect(rect, dpi));
        let right = button.map_or(rect.right - pad, |button| {
            button.left - scale(GAP_AT_96_DPI, dpi)
        });
        let text = RECT {
            left: glyph.right + scale(GAP_AT_96_DPI, dpi),
            right,
            ..first
        };
        let second = RECT {
            top: first.bottom,
            bottom: first.bottom + slot,
            ..text
        };
        unsafe {
            draw_item_icon(
                hdc,
                TreeItem::Note(note_kind(&result.path)),
                glyph,
                scale(ICON_AT_96_DPI, dpi),
                muted,
                &palette,
                &paint.icons,
                &mut self.images.borrow_mut(),
                paint.icon_set,
                paint.light_theme,
            );
            let width = draw_text(
                hdc,
                &result.name,
                text,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if !result.folder.is_empty() {
                draw_text(
                    hdc,
                    &result.folder,
                    RECT {
                        left: text.left + width + scale(GAP_AT_96_DPI, dpi),
                        ..text
                    },
                    paint.fonts.text,
                    muted,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
            draw_snippet(hdc, &result.snippet, second, paint.fonts, foreground);
            if let Some(button) = button {
                let enabled = self.replace_all_enabled();
                let hover = enabled && self.row_hover_button == Some(index);
                if hover {
                    fill(hdc, button, palette.hover_background);
                }
                draw_text(
                    hdc,
                    REPLACE_GLYPH,
                    button,
                    paint.fonts.glyph,
                    row_button_color(enabled, hover, look, &palette),
                    line | DT_CENTER,
                );
            }
        }
    }
}

/// The panel's `WM_PAINT` while the Search view shows. The bold snippet font is made here, the
/// first time a result paints, so nothing new is made before the window's first paint.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    let mut paint = *paint;
    if let Some(mut app) = unsafe { crate::window::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_mut() }.sidebar.as_mut()
    {
        sidebar.search.set_colors(paint.palette);
        if !sidebar.search.results.is_empty() && sidebar.search.notice().is_none() {
            paint.fonts.text_bold = sidebar.text_bold(paint.dpi);
        }
    }
    if let Some(app) = unsafe { crate::window::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.search.paint(&paint);
    }
}

/// `WM_CTLCOLOREDIT` for the box.
pub(crate) fn control_color(hwnd: HWND, dc: HDC) -> HBRUSH {
    with_view(hwnd, |view| {
        unsafe {
            SetTextColor(dc, view.colors.editor_foreground);
            SetBkColor(dc, view.colors.editor_background);
        }
        view.brush
    })
    .unwrap_or(std::ptr::null_mut())
}
