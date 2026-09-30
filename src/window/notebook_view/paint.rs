//! Painting the Notebook view: tree, recent and Open Editors rows with their icons, the
//! header and state sections, the inline name field, the drag label and drop band, and the
//! tooltip texts that follow what is painted.

use super::*;
use crate::config::FileIconSet;
use crate::library::tree::{RowKind, TreeRow};
use crate::window::design::metrics::{SIDEBAR_ROW, scale};
use crate::window::drag_label::LabelImage;
use crate::window::file_icons::note_kind;
use crate::window::icon_sets::images::IconImages;
use crate::window::icon_sets::{TreeIcon, TreeItem, minimal, tree_icon};
use crate::window::inline_name::FieldLayout;
use crate::window::notebook_layout::{self, PanelLayout};
use crate::window::palette::{FileIcons, Palette};
use crate::window::panel::{fill, inset};
use crate::window::panel_cursor::Cursor;
use crate::window::row_list::{self, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{UiFonts, ViewPaint, draw_text};
use crate::window::tree_drag::{self, DragSource};
use windows_sys::Win32::Foundation::{HWND, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_NOPREFIX, DT_WORDBREAK, DrawTextW, GetDC, GetTextExtentPoint32W, HDC, HFONT,
    ReleaseDC, SelectObject,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};

/// Whether row `index` is the one a started drag carries (tree drag spec §3.2): both sides
/// absent (no drag, and `index` past the rows the list actually has, such as the truncated row)
/// must not read as a match. Only a dragged row matches: a tab or files are not tree rows.
pub(super) fn is_dragged_row(dragged: Option<&DragSource>, rows: &[TreeRow], index: usize) -> bool {
    let Some(DragSource::Row(dragged)) = dragged else {
        return false;
    };
    rows.get(index).is_some_and(|row| &row.kind == dragged)
}

/// Draws `item`'s icon from `set` at `px` square, centred in `rect` and clipped to it (a deep row
/// in a narrow panel gets a box narrower than `px`). `muted` is every icon's colour in high
/// contrast.
#[allow(
    clippy::too_many_arguments,
    reason = "one icon's paint inputs, shared by a tree row and the drag label"
)]
pub(crate) fn draw_item_icon(
    dc: HDC,
    item: TreeItem,
    rect: RECT,
    px: i32,
    muted: u32,
    palette: &Palette,
    icons: &FileIcons,
    images: &mut IconImages,
    set: FileIconSet,
    light_theme: bool,
) {
    // A type icon keeps its colour on a selected or hovered row: the colours are mid-tones that
    // read on the selection. High contrast draws Minimal in the muted system pair in every set
    // (icon sets spec §3.2), and a Material bitmap that cannot be made falls back to Minimal
    // (§6).
    let mask = match tree_icon(set, item, light_theme, palette.high_contrast) {
        TreeIcon::Image(icon) if images.draw(dc, icon, rect, px as u32) => return,
        TreeIcon::Image(_) => minimal(item),
        mask => mask,
    };
    if let TreeIcon::Mask { set, icon, color } = mask {
        let color = if palette.high_contrast {
            muted
        } else {
            icons.color(color)
        };
        images.draw_mask(dc, set, icon, color, rect, px as u32);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one row's paint inputs, called from one closure"
)]
pub(super) fn draw_tree_row(
    dc: HDC,
    row: Option<&TreeRow>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    icons: &FileIcons,
    fonts: UiFonts,
    dpi: u32,
    pin_hot: bool,
    editing: Option<TreeItem>,
    images: &mut IconImages,
    set: FileIconSet,
    light_theme: bool,
    // The row being dragged: its name draws muted (tree drag spec §3.2).
    dimmed: bool,
) {
    let foreground = row_foreground(look, palette);
    let muted = if look.selected && look.focused {
        foreground
    } else {
        palette.muted_foreground
    };
    let Some(row) = row else {
        let text = RECT {
            left: rect.left + scale(LEFT_PAD, dpi),
            ..rect
        };
        unsafe { draw_text(dc, TRUNCATED_ROW, text, fonts.italic, muted, LINE) };
        return;
    };
    let parts = row_parts(rect, row.depth, dpi);
    // `px` is the full icon box, not the box clamped by `row_parts` to fit a narrow panel: a
    // clipped box draws part of the icon instead of resampling it smaller and caching a bitmap
    // per clipped width.
    let px = scale(GLYPH_BOX, dpi);
    let mut draw_icon = |item: TreeItem| {
        draw_item_icon(
            dc,
            item,
            parts.icon,
            px,
            muted,
            palette,
            icons,
            images,
            set,
            light_theme,
        );
    };
    match &row.kind {
        RowKind::Folder(_) => {
            let chevron = if row.expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe { draw_text(dc, chevron, parts.chevron, fonts.glyph, muted, CENTERED) };
            draw_icon(TreeItem::Folder {
                expanded: row.expanded,
            });
        }
        RowKind::Note(path) => draw_icon(TreeItem::Note(note_kind(path))),
        RowKind::Draft => {
            if let Some(item) = editing {
                draw_icon(item);
            }
        }
    }
    if matches!(row.kind, RowKind::Note(_)) {
        // Pinned is a filled glyph, never color alone (spec §10). Segoe's PinFill is only the
        // head's fill, with no needle: its tilted outline (Pinned) is drawn over it to complete
        // the shape.
        if row.pinned {
            for glyph in [GLYPH_PINNED, GLYPH_PIN] {
                unsafe { draw_text(dc, glyph, parts.pin, fonts.glyph, foreground, CENTERED) };
            }
        } else if look.hover || look.selected {
            let color = if pin_hot { foreground } else { muted };
            unsafe { draw_text(dc, GLYPH_PIN, parts.pin, fonts.glyph, color, CENTERED) };
        }
    }
    // The field covers an edited row's name (inline naming spec §3.3).
    if editing.is_some() {
        return;
    }
    let font = fonts.text;
    let color = if dimmed {
        palette.muted_foreground
    } else {
        foreground
    };
    unsafe { draw_text(dc, &row.name, parts.name, font, color, LINE) };
}

pub(super) fn draw_recent_row(
    dc: HDC,
    name: Option<&(String, Option<String>)>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    fonts: UiFonts,
    dpi: u32,
) {
    let Some((name, hint)) = name else {
        return;
    };
    let foreground = row_foreground(look, palette);
    let parts = row_parts(rect, 0, dpi);
    unsafe {
        draw_text(
            dc,
            GLYPH_FOLDER,
            parts.icon,
            fonts.glyph,
            palette.muted_foreground,
            CENTERED,
        )
    };
    let text = RECT {
        right: rect.right - scale(LEFT_PAD, dpi),
        ..parts.name
    };
    // A clash between two notebook names shows the parent folder, dimmed, after the name.
    let label = match hint {
        Some(hint) => format!("{name}  {hint}"),
        None => name.clone(),
    };
    unsafe { draw_text(dc, &label, text, fonts.text, foreground, LINE) };
}

/// The icon a dragged row's label shows (tree drag spec §3.2): a closed folder or the note's
/// type. `None` for rows that can't be dragged.
pub(super) fn drag_item(kind: &RowKind) -> Option<TreeItem> {
    match kind {
        RowKind::Folder(_) => Some(TreeItem::Folder { expanded: false }),
        RowKind::Note(path) => Some(TreeItem::Note(note_kind(path))),
        RowKind::Draft => None,
    }
}

/// The drag label's size for a name `text` pixels wide: padding, the icon, a gap, the name,
/// padding.
/// A tab's drag label for window `window` (a group window): its icon and name, painted like a
/// tree drag's (split editors spec §6). `None` if GDI can't make the image. Needs no notebook
/// view, so it works with the sidebar hidden.
pub(crate) fn tab_label_image(
    hwnd: HWND,
    window: HWND,
    item: TreeItem,
    name: &str,
) -> Option<LabelImage> {
    let paint =
        crate::window::side_panel::view_paint(hwnd, window, std::ptr::null_mut(), RECT::default());
    let wide = name.encode_utf16().collect::<Vec<_>>();
    let mut extent = SIZE::default();
    if !wide.is_empty() {
        unsafe {
            let dc = GetDC(window);
            if dc.is_null() {
                return None;
            }
            let previous = SelectObject(dc, paint.fonts.text);
            GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut extent);
            SelectObject(dc, previous);
            ReleaseDC(window, dc);
        }
    }
    let size = drag_label_size(extent.cx.min(scale(LABEL_MAX_TEXT, paint.dpi)), paint.dpi);
    let image = LabelImage::new(size.cx, size.cy)?;
    let mut images = crate::window::icon_sets::images::IconImages::new();
    paint_drag_label(image.dc, size, item, name, &paint, &mut images);
    Some(image)
}

pub(super) fn drag_label_size(text: i32, dpi: u32) -> SIZE {
    SIZE {
        cx: 2 * scale(LABEL_PAD, dpi) + scale(GLYPH_BOX, dpi) + scale(GAP, dpi) + text,
        cy: scale(LABEL_HEIGHT, dpi),
    }
}

/// The drag label's fill, border and text colours: the hover fill with a muted border, or in
/// high contrast only the system window pair.
pub(super) fn drag_label_colors(palette: &Palette) -> (u32, u32, u32) {
    if palette.high_contrast {
        (
            palette.editor_background,
            palette.editor_foreground,
            palette.editor_foreground,
        )
    } else {
        (
            palette.hover_background,
            palette.muted_foreground,
            palette.editor_foreground,
        )
    }
}

/// Paints the drag label for `item` and `name` over the whole of `dc`'s `size` (tree drag spec
/// §3.2): a 1 px (scaled) border, the icon the row shows, and the name, cut with an ellipsis.
pub(super) fn paint_drag_label(
    dc: HDC,
    size: SIZE,
    item: TreeItem,
    name: &str,
    paint: &ViewPaint,
    images: &mut IconImages,
) {
    let (palette, dpi) = (&paint.palette, paint.dpi);
    let (background, border, text) = drag_label_colors(palette);
    let whole = RECT {
        left: 0,
        top: 0,
        right: size.cx,
        bottom: size.cy,
    };
    unsafe {
        fill(dc, whole, border);
        fill(dc, inset(whole, scale(1, dpi).max(1)), background);
    }
    let (pad, px) = (scale(LABEL_PAD, dpi), scale(GLYPH_BOX, dpi));
    let top = (size.cy - px) / 2;
    let icon = RECT {
        left: pad,
        top,
        right: pad + px,
        bottom: top + px,
    };
    draw_item_icon(
        dc,
        item,
        icon,
        px,
        palette.muted_foreground,
        palette,
        &paint.icons,
        images,
        paint.icon_set,
        paint.light_theme,
    );
    let name_rect = RECT {
        left: icon.right + scale(GAP, dpi),
        top: 0,
        right: size.cx - pad,
        bottom: size.cy,
    };
    unsafe { draw_text(dc, name, name_rect, paint.fonts.text, text, LINE) };
}

/// Where `highlight` shows in the tree's `list` area (tree drag spec §3.2): the whole list for
/// the root, else the part of the folder's rows in view; `None` when none of them is.
pub(crate) fn band_rect(
    list: RECT,
    state: &RowListState,
    highlight: tree_drag::Highlight,
) -> Option<RECT> {
    match highlight {
        tree_drag::Highlight::Root => Some(list),
        tree_drag::Highlight::Rows { start, end } => {
            let visible = state.visible_rows(height(list));
            let first = start.max(state.top);
            let last = end.min(state.top + visible);
            (first < last).then(|| RECT {
                left: list.left,
                top: list.top + (first - state.top) as i32 * state.row_height,
                right: list.right,
                bottom: (list.top + (last - state.top) as i32 * state.row_height).min(list.bottom),
            })
        }
    }
}

/// The drop target's band (tree drag spec §3.2). Called before the rows paint
/// (`before_rows`), it fills the band with the softer selection colour; after, in high
/// contrast only, it outlines the band in the system highlight, since a blend is not allowed
/// there.
pub(super) fn paint_band(dc: HDC, band: RECT, palette: &Palette, dpi: u32, before_rows: bool) {
    if before_rows && !palette.high_contrast {
        unsafe { fill(dc, band, palette.inactive_selection_background) };
    } else if !before_rows && palette.high_contrast {
        paint_outline(dc, band, palette.selection_background, dpi);
    }
}

/// A 1 px (scaled) outline just inside `rect`.
pub(super) fn paint_outline(dc: HDC, rect: RECT, color: u32, dpi: u32) {
    let t = scale(1, dpi).max(1);
    for edge in [
        RECT {
            bottom: rect.top + t,
            ..rect
        },
        RECT {
            top: rect.bottom - t,
            ..rect
        },
        RECT {
            right: rect.left + t,
            ..rect
        },
        RECT {
            left: rect.right - t,
            ..rect
        },
    ] {
        unsafe { fill(dc, edge, color) };
    }
}

impl NotebookView {
    /// The inline field's frame, in the accent colour or the error colour while a problem
    /// shows, and the problem under the field, or above it without room below (inline naming
    /// spec §4.4).
    pub(super) fn paint_inline(
        &self,
        dc: HDC,
        layout: FieldLayout,
        list: RECT,
        palette: &Palette,
        fonts: UiFonts,
        dpi: u32,
    ) {
        let problem = self.inline.problem();
        let outline = if problem.is_some() {
            palette.error_foreground
        } else {
            palette.selection_background
        };
        unsafe {
            fill(dc, layout.frame, outline);
            fill(dc, inset(layout.frame, 1), palette.editor_background);
        }
        let Some(problem) = problem else {
            return;
        };
        let pad = scale(4, dpi);
        let text: Vec<u16> = problem.encode_utf16().collect();
        let mut measured = RECT {
            left: 0,
            top: 0,
            right: (layout.frame.right - layout.frame.left - 2 * pad).max(1),
            bottom: 0,
        };
        unsafe {
            let previous = SelectObject(dc, fonts.text);
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut measured,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, previous);
        }
        let rect =
            crate::window::inline_name::message_rect(layout.frame, list, measured.bottom + 2 * pad);
        let inner = RECT {
            left: rect.left + pad,
            top: rect.top + pad,
            right: rect.right - pad,
            bottom: rect.bottom - pad,
        };
        unsafe {
            fill(dc, rect, palette.error_foreground);
            fill(dc, inset(rect, 1), palette.editor_background);
            draw_text(
                dc,
                problem,
                inner,
                fonts.text,
                palette.error_foreground,
                DT_WORDBREAK | DT_NOPREFIX,
            );
        }
    }
}

impl NotebookView {
    /// The started drag's label painted with `paint`, and the dragged row's or tab's name (tree
    /// drag spec §3.2). `None` without a started drag, for dropped files, or if GDI can't make
    /// the image.
    pub(super) fn drag_label_image(&mut self, paint: &ViewPaint) -> Option<(LabelImage, String)> {
        let source = self
            .drag
            .as_ref()
            .filter(|drag| drag.started)?
            .source
            .clone();
        let (item, name) = match &source {
            DragSource::Row(kind) => {
                let item = drag_item(kind)?;
                let name = self.rows.iter().find(|row| &row.kind == kind)?.name.clone();
                (item, name)
            }
            DragSource::Tab { path, name, .. } => (
                TreeItem::Note(
                    path.as_deref()
                        .map_or(crate::window::file_icons::NoteKind::Text, note_kind),
                ),
                name.clone(),
            ),
            DragSource::Files(_) | DragSource::GroupTab { .. } => return None,
        };
        let text = self
            .text_width(&name, paint.fonts.text)
            .min(scale(LABEL_MAX_TEXT, paint.dpi));
        let size = drag_label_size(text, paint.dpi);
        let image = LabelImage::new(size.cx, size.cy)?;
        paint_drag_label(image.dc, size, item, &name, paint, &mut self.images);
        Some((image, name))
    }

    pub(super) fn text_width(&mut self, text: &str, font: HFONT) -> i32 {
        let wide = text.encode_utf16().collect::<Vec<_>>();
        if wide.is_empty() {
            return 0;
        }
        let mut size = SIZE::default();
        unsafe {
            let dc = GetDC(self.panel);
            if dc.is_null() {
                return 0;
            }
            let previous = SelectObject(dc, font);
            GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size);
            SelectObject(dc, previous);
            ReleaseDC(self.panel, dc);
        }
        size.cx
    }

    pub(super) fn track_leave(&mut self) {
        if self.tracking_leave {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.panel,
            dwHoverTime: 0,
        };
        self.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
    }

    /// Destroys the view's tooltip, if it made one. The popup is owned by the main window, so
    /// destroying the panel does not take it along (`side_panel::destroy_windows` calls this).
    pub(crate) fn destroy_tooltip(&self) {
        if let Some(tooltip) = self.tooltip {
            tooltip.destroy();
        }
    }

    /// The hovered row's tip: a truncated note name in full, or a recent notebook's path.
    pub(super) fn row_tip(&mut self, fonts: UiFonts) -> (RECT, String) {
        let none = (RECT::default(), String::new());
        let Some(index) = self.list.hover else {
            return none;
        };
        let list = self.list_rect(self.client());
        let Some(rect) = self.row_rect(list, index) else {
            return none;
        };
        match self.mode {
            Mode::NoNotebook => self
                .recent
                .get(index)
                .map_or(none, |path| (rect, path.display().to_string())),
            Mode::Tree => {
                let Some(row) = self.rows.get(index).cloned() else {
                    return none;
                };
                let parts = row_parts(rect, row.depth, self.dpi());
                let font = fonts.text;
                if self.text_width(&row.name, font) > parts.name.right - parts.name.left {
                    (parts.name, row.name)
                } else {
                    none
                }
            }
            Mode::Loading | Mode::Empty | Mode::Failed => none,
        }
    }

    /// Every tool's ID, rectangle and text, for `apply_tooltips`. `fonts` measures whether the
    /// hovered name is cut off. A tool with an empty text is removed, so a hidden button shows
    /// no tip.
    pub(super) fn tooltip_tools(&mut self, fonts: UiFonts) -> Vec<(usize, RECT, String)> {
        let area = self.client();
        let dpi = self.dpi();
        let layout = self.layout(area, dpi);
        let parts = notebook_layout::root_parts(layout.root, dpi);
        let title = self
            .root
            .as_ref()
            .map(|root| root.display().to_string())
            .unwrap_or_default();
        let favorite = if self.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        let buttons_shown = self.mode != Mode::NoNotebook;
        // An Open Editors row shows its path; a tree row its cut-off name.
        let editor_tip = self.editors.list.hover.and_then(|index| {
            let rect = self.editors_row_rect(layout.editors_list, index)?;
            let row = self.editors.row(index)?;
            Some((rect, crate::window::open_editors::tooltip(row)))
        });
        let (row_rect, row_text) = match editor_tip {
            Some(tip) => tip,
            None => self.row_tip(fonts),
        };
        let mut tools = vec![(TOOL_TITLE, parts.name, title)];
        for (button, rect) in parts.buttons {
            let (id, text) = match button {
                HeaderButton::Favorite => (TOOL_FAVORITE, favorite),
                HeaderButton::NewNote => (TOOL_NEW, "New note"),
                HeaderButton::NewFolder => (TOOL_NEW_FOLDER, "New folder"),
                HeaderButton::Refresh => (TOOL_REFRESH, "Refresh"),
                HeaderButton::ToggleFolders => (TOOL_TOGGLE_FOLDERS, self.toggle_folders_name()),
                HeaderButton::More => (TOOL_MORE, "More actions"),
            };
            // A button a narrow panel left out has an empty rectangle and no tip.
            let shown = buttons_shown && rect.right > rect.left;
            let text = if shown { text } else { "" };
            tools.push((id, rect, text.to_owned()));
        }
        tools.push((TOOL_ROW, row_rect, row_text));
        tools
    }

    pub(super) fn paint(&mut self, paint: &ViewPaint) {
        let (dc, area, dpi, fonts, focused) = (
            paint.hdc,
            paint.client,
            paint.dpi,
            paint.fonts,
            paint.focused,
        );
        let palette = &paint.palette;
        self.list.row_height = scale(SIDEBAR_ROW, dpi);
        self.editors.list.row_height = scale(SIDEBAR_ROW, dpi);
        let sections = self.layout(area, dpi);
        // A panel sized after the rows came (startup) or resized: the scroll stays in range.
        let editors_height = height(sections.editors_list);
        if editors_height > 0 {
            self.editors.list.scroll_lines(0, editors_height);
        }
        self.paint_sections(paint, sections);
        let layout = state_layout(sections.body, dpi);
        match self.mode {
            // The states under the root show only while it is expanded; without a notebook there
            // is no root to collapse.
            Mode::Loading | Mode::Empty | Mode::Failed | Mode::Tree if !self.root_expanded => {}
            Mode::Loading => {
                unsafe {
                    draw_text(
                        dc,
                        "Loading…",
                        layout.message,
                        fonts.text,
                        palette.muted_foreground,
                        LINE,
                    )
                };
            }
            Mode::NoNotebook => {
                let message = "Open a notebook to see its notes.";
                unsafe {
                    draw_text(
                        dc,
                        message,
                        layout.message,
                        fonts.text,
                        palette.editor_foreground,
                        DT_WORDBREAK | DT_NOPREFIX,
                    )
                };
                self.paint_button(
                    dc,
                    layout.button,
                    "Open notebook…",
                    Hit::StateButton,
                    palette,
                    fonts,
                );
                if !self.recent.is_empty() {
                    unsafe {
                        draw_text(
                            dc,
                            "RECENT",
                            layout.label,
                            fonts.bold,
                            palette.muted_foreground,
                            LINE,
                        )
                    };
                }
                let names = &self.recent_names;
                row_list::paint(
                    dc,
                    layout.list,
                    &self.list,
                    palette,
                    focused,
                    &mut |dc, index, rect, look| {
                        draw_recent_row(dc, names.get(index), rect, look, palette, fonts, dpi);
                    },
                );
            }
            Mode::Empty => {
                let message = format!("No notes in {} yet.", self.name);
                unsafe {
                    draw_text(
                        dc,
                        &message,
                        layout.message,
                        fonts.text,
                        palette.editor_foreground,
                        DT_WORDBREAK | DT_NOPREFIX,
                    )
                };
                self.paint_button(
                    dc,
                    layout.button,
                    "New note",
                    Hit::StateButton,
                    palette,
                    fonts,
                );
            }
            Mode::Failed => {
                unsafe {
                    draw_text(
                        dc,
                        LOAD_FAILED,
                        layout.message,
                        fonts.text,
                        palette.editor_foreground,
                        DT_WORDBREAK | DT_NOPREFIX,
                    )
                };
                self.paint_button(dc, layout.button, "Retry", Hit::StateButton, palette, fonts);
                self.paint_button(
                    dc,
                    layout.second,
                    "Open notebook…",
                    Hit::SecondButton,
                    palette,
                    fonts,
                );
            }
            Mode::Tree => {
                // `row_list::paint` draws the rows in view and the scroll thumb.
                let list = self.list_rect(area);
                self.inline.set_colors(*palette);
                let rows = &self.rows;
                let images = &mut self.images;
                let (icon_set, light_theme) = (paint.icon_set, paint.light_theme);
                let hover_pin = self.hover_pin;
                let icons = &paint.icons;
                let drag = self.drag.as_ref().filter(|drag| drag.started);
                let dragged = drag.map(|drag| drag.source.clone());
                let band = drag
                    .and_then(|drag| drag.target.as_deref())
                    .and_then(|folder| tree_drag::highlight(rows, folder))
                    .and_then(|highlight| band_rect(list, &self.list, highlight));
                if let Some(band) = band {
                    paint_band(dc, band, palette, dpi, true);
                }
                // The edited row leaves its name to the field; a draft row shows the icon for
                // what is typed so far (inline naming spec §3.1, §3.3).
                let edited = self
                    .inline
                    .row()
                    .map(|index| (index, self.inline.draft_icon()));
                row_list::paint(
                    dc,
                    list,
                    &self.list,
                    palette,
                    focused,
                    &mut |dc, index, rect, look| {
                        let editing =
                            edited.and_then(|(edited, icon)| (edited == index).then_some(icon));
                        draw_tree_row(
                            dc,
                            rows.get(index),
                            rect,
                            look,
                            palette,
                            icons,
                            fonts,
                            dpi,
                            hover_pin && look.hover,
                            editing,
                            images,
                            icon_set,
                            light_theme,
                            is_dragged_row(dragged.as_ref(), rows, index),
                        );
                    },
                );
                if let Some(band) = band {
                    paint_band(dc, band, palette, dpi, false);
                }
                if let Some(layout) = self.inline_layout_in(area, dpi) {
                    self.paint_inline(dc, layout, list, palette, fonts, dpi);
                }
            }
        }
    }

    pub(super) fn paint_sections(&mut self, paint: &ViewPaint, layout: PanelLayout) {
        let (dc, palette, fonts, dpi) = (paint.hdc, &paint.palette, paint.fonts, paint.dpi);
        let bold = |text: &str, rect: RECT, color: u32| unsafe {
            draw_text(dc, text, rect, fonts.bold, color, LINE)
        };
        let title = RECT {
            left: layout.title.left + scale(12, dpi),
            ..layout.title
        };
        bold("NOTEBOOK", title, palette.muted_foreground);
        // Open Editors header: chevron, label and count.
        let chevron = notebook_layout::section_chevron(layout.editors_header, dpi);
        let glyph = if self.editors_expanded {
            GLYPH_CHEVRON_DOWN
        } else {
            GLYPH_CHEVRON_RIGHT
        };
        unsafe {
            draw_text(
                dc,
                glyph,
                chevron,
                fonts.glyph,
                palette.muted_foreground,
                CENTERED,
            )
        };
        let label = RECT {
            left: chevron.right,
            ..layout.editors_header
        };
        let count = self.editors.view_count();
        bold(
            &format!("OPEN EDITORS  {count}"),
            label,
            palette.muted_foreground,
        );
        // The rows.
        let editors = &self.editors;
        let images = &mut self.images;
        let hover_close = editors.hover_close;
        row_list::paint(
            dc,
            layout.editors_list,
            &editors.list,
            palette,
            paint.focused,
            &mut |dc, index, rect, look| match editors.rows.get(index) {
                Some(crate::window::open_editors::EditorEntry::Header(number)) => {
                    crate::window::open_editors::draw_header(dc, *number, rect, paint);
                }
                Some(crate::window::open_editors::EditorEntry::View(row)) => {
                    crate::window::open_editors::draw_editor_row(
                        dc,
                        row,
                        rect,
                        look,
                        paint,
                        images,
                        hover_close && look.hover,
                    );
                }
                None => {}
            },
        );
        // The root row: chevron, name, and its buttons (not without a notebook).
        let parts = notebook_layout::root_parts(layout.root, dpi);
        if self.mode == Mode::NoNotebook {
            bold(
                "NO NOTEBOOK",
                RECT {
                    left: parts.chevron.right,
                    ..layout.root
                },
                palette.muted_foreground,
            );
        } else {
            let glyph = if self.root_expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe {
                draw_text(
                    dc,
                    glyph,
                    parts.chevron,
                    fonts.glyph,
                    palette.muted_foreground,
                    CENTERED,
                )
            };
            bold(
                &self.name.to_uppercase(),
                parts.name,
                palette.muted_foreground,
            );
            for (button, rect) in parts.shown() {
                let hot = self.hover == Some(Hit::Header(button));
                if hot {
                    unsafe { fill(dc, rect, palette.hover_background) };
                }
                let glyph = match button {
                    HeaderButton::Favorite if self.favorite => GLYPH_STAR_FILLED,
                    HeaderButton::Favorite => GLYPH_STAR,
                    HeaderButton::NewNote => GLYPH_ADD,
                    HeaderButton::NewFolder => GLYPH_NEW_FOLDER,
                    HeaderButton::Refresh => GLYPH_REFRESH,
                    HeaderButton::ToggleFolders if self.folders_open => GLYPH_COLLAPSE_ALL,
                    HeaderButton::ToggleFolders => GLYPH_EXPAND_ALL,
                    HeaderButton::More => GLYPH_MORE,
                };
                let color = if hot {
                    palette.hover_foreground
                } else {
                    palette.muted_foreground
                };
                unsafe { draw_text(dc, glyph, rect, fonts.glyph, color, CENTERED) };
            }
        }
        // The keyboard selection on a header or tab row; the tree shows its own.
        if paint.focused {
            let outlined = match self.cursor {
                Cursor::EditorsHeader => Some(layout.editors_header),
                Cursor::Editor(index) if self.editors_expanded => {
                    let list = layout.editors_list;
                    self.editors_row_rect(list, index)
                        .filter(|row| row.top < list.bottom)
                        .map(|row| RECT {
                            bottom: row.bottom.min(list.bottom),
                            ..row
                        })
                }
                Cursor::Root if self.mode != Mode::NoNotebook => Some(layout.root),
                Cursor::Root | Cursor::Editor(_) | Cursor::Tree => None,
            };
            if let Some(rect) = outlined {
                paint_outline(dc, rect, palette.selection_background, dpi);
            }
        }
    }

    pub(super) fn paint_button(
        &self,
        dc: HDC,
        rect: RECT,
        text: &str,
        hit: Hit,
        palette: &Palette,
        fonts: UiFonts,
    ) {
        let background = if self.hover == Some(hit) {
            palette.hover_background
        } else {
            palette.pressed_background
        };
        unsafe { fill(dc, rect, background) };
        unsafe {
            draw_text(
                dc,
                text,
                rect,
                fonts.text,
                palette.editor_foreground,
                CENTERED,
            )
        };
    }
}
