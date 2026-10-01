//! Painting the Settings dialog: composes every card, control, nav item and the footer into a
//! soft-paint `Frame` and paints it off-screen.

use super::*;

/// Paints through an off-screen bitmap so a hover repaint never shows the erase.
pub(super) fn paint(hwnd: HWND, dialog: &Dialog) {
    paint_buffered(hwnd, |dc, client| paint_into(dc, client, dialog));
}

pub(super) fn paint_into(dc: HDC, client: RECT, dialog: &Dialog) {
    let measure = |text: &str| text_width(dc, dialog.body_font, text);
    let mut frame = Frame::default();
    compose(&mut frame, client, dialog, &measure);
    frame.paint(dc, client, &dialog.canvas);
}

/// `text`'s width in `font` on `dc`.
pub(super) fn text_width(dc: HDC, font: HFONT, text: &str) -> i32 {
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::GetTextExtentPoint32W;
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let mut size = SIZE::default();
    unsafe {
        let previous = SelectObject(dc, font as _);
        GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size);
        SelectObject(dc, previous);
    }
    size.cx
}

/// Everything the dialog paints. `measure` gives a text's width in the body font.
pub(super) fn compose<'a>(
    frame: &mut Frame<'a>,
    client: RECT,
    dialog: &'a Dialog,
    measure: &dyn Fn(&str) -> i32,
) {
    let colors = &dialog.colors;
    let tones = Tones::new(colors);
    let layout = &dialog.layout;
    let view = &dialog.view;
    let dpi = layout.dpi;
    let radius = layout.radius();

    // No border line: the native shadow is the edge (square on Windows 10, rounded on 11).
    frame.shape(Shape::Fill {
        rect: client,
        color: colors.panel_background(),
    });

    // Title row: a strip-coloured band with the name and the × button.
    frame.shape(Shape::Fill {
        rect: RECT {
            bottom: layout.title.bottom,
            ..client
        },
        color: colors.strip_background,
    });
    frame.text(
        dialog.title_font,
        colors.editor_foreground,
        TITLE,
        layout.title,
        DT_LEFT,
    );
    title_close(
        frame,
        colors,
        dialog.glyph_font,
        layout.title_close,
        dialog.hot == Some(Hit::TitleClose),
    );
    // Subtle rules under the title and over the footer.
    for top in [layout.body.top - 1, layout.body.bottom] {
        frame.shape(Shape::Fill {
            rect: RECT {
                top,
                bottom: top + 1,
                ..client
            },
            color: tones.card,
        });
    }

    // The page list on the left.
    frame.shape(Shape::Fill {
        rect: layout.nav,
        color: colors.strip_background,
    });
    for page in Page::ALL {
        let item = layout.nav_items[page as usize];
        let current = dialog.model.page == page;
        let hot = dialog.hot == Some(Hit::Nav(page));
        if current || hot {
            let fill = if current {
                tones.control
            } else {
                tones.card_hot
            };
            tones.soft(frame, item, radius, fill);
        }
        if current {
            let bar = scale(NAV_BAR_WIDTH_AT_96_DPI, dpi);
            let middle = (item.top + item.bottom) / 2;
            frame.shape(Shape::Round {
                rect: RECT {
                    left: item.left,
                    top: middle - scale(8, dpi),
                    right: item.left + bar,
                    bottom: middle + scale(8, dpi),
                },
                radius: bar / 2,
                color: tones.accent,
            });
        }
        frame.text(
            dialog.body_font,
            colors.editor_foreground,
            page.title(),
            RECT {
                left: item.left + scale(12, dpi),
                ..item
            },
            DT_LEFT,
        );
    }

    // The scrolling body, clipped to its area.
    frame.clip(Some(layout.body));
    if dialog.model.page == Page::General {
        for section in Section::ALL {
            let heading = layout.heading_rect(section, dialog.scroll);
            frame.text(
                dialog.heading_font,
                colors.editor_foreground,
                section.title(),
                RECT {
                    top: heading.top + scale(HEADING_SPACE_ABOVE_AT_96_DPI, dpi),
                    ..heading
                },
                DT_LEFT,
            );
        }
        for row in Row::ALL {
            compose_row(frame, &tones, dialog, row);
        }
    }
    frame.clip(None);
    if dialog.model.page == Page::Shortcuts {
        let style = crate::window::shortcuts_page::PageStyle {
            colors,
            tones: &tones,
            link_color: dialog.link_color,
            body_font: dialog.body_font,
            heading_font: dialog.heading_font,
            link_font: dialog.link_font,
            glyph_font: dialog.glyph_font,
            radius,
            hot: match dialog.hot {
                Some(Hit::Page(hit)) => Some(hit),
                _ => None,
            },
            table_focused: dialog.model.focus == Focus::Table,
        };
        crate::window::shortcuts_page::compose(
            frame,
            measure,
            &dialog.page_layout,
            &dialog.shortcuts,
            &style,
        );
    }

    // Footer: the link and a filled accent Close.
    frame.text(
        dialog.link_font,
        dialog.link_color,
        EDIT_INI_LABEL,
        layout.edit_ini,
        DT_LEFT,
    );
    let button = match (dialog.pressed, dialog.hot) {
        (Some(Hit::Close), _) => tones.accent_down,
        (_, Some(Hit::Close)) => tones.accent_hot,
        _ => tones.accent,
    };
    tones.soft(frame, layout.close, radius, button);
    frame.text(
        dialog.body_font,
        tones.on_accent,
        CLOSE_LABEL,
        layout.close,
        DT_CENTER,
    );

    // The focus ring: a rounded accent stroke just outside the focused control, or on the
    // edge of a toggle's card, all of which is its hit area.
    let width = scale(FOCUS_RING, dpi);
    let outside = width + scale(FOCUS_GAP, dpi);
    let ring = match dialog.model.focus {
        Focus::Nav => Some((
            inset(layout.nav_items[dialog.model.page as usize], -outside),
            radius + outside,
        )),
        Focus::Search => Some((inset(dialog.page_layout.search, -outside), radius + outside)),
        // The selected row's accent bar shows the table's focus.
        Focus::Table => None,
        Focus::EditIni => Some((inset(layout.edit_ini, -outside), radius + outside)),
        Focus::Close => Some((inset(layout.close, -outside), radius + outside)),
        Focus::Row(row) => {
            let card = layout.row_rect(row, dialog.scroll);
            let visible = card.top >= layout.body.top && card.bottom <= layout.body.bottom;
            visible.then(|| {
                if row.control() == Control::Check {
                    (card, radius)
                } else {
                    let control = layout.control_rect(row, card, view.segments(row).len());
                    (inset(control, -outside), radius + outside)
                }
            })
        }
    };
    if let Some((rect, radius)) = ring {
        frame.shape(Shape::Ring {
            rect,
            radius,
            width,
            color: tones.accent,
        });
    }
}

pub(super) fn compose_row<'a>(frame: &mut Frame<'a>, tones: &Tones, dialog: &'a Dialog, row: Row) {
    let colors = &dialog.colors;
    let layout = &dialog.layout;
    let view = &dialog.view;
    let dpi = layout.dpi;
    let radius = layout.radius();
    let card = layout.row_rect(row, dialog.scroll);
    if card.bottom < layout.body.top || card.top > layout.body.bottom {
        return;
    }
    let enabled = view.enabled(row);
    let text = if enabled {
        colors.editor_foreground
    } else {
        colors.muted_foreground
    };
    let segments = view.segments(row);
    let control = layout.control_rect(row, card, segments.len());
    let hot = matches!(dialog.hot, Some(Hit::Row(hot_row, _)) if hot_row == row) && enabled;
    // A toggle's whole card is its hit area, so the card shows the hover.
    let card_fill = if hot && row.control() == Control::Check {
        tones.card_hot
    } else {
        tones.card
    };
    tones.soft(frame, card, radius, card_fill);
    let padding = scale(CARD_PADDING_AT_96_DPI, dpi);
    let label = RECT {
        left: card.left + padding,
        right: card.right - padding,
        ..card
    };
    frame.text(dialog.body_font, text, row.label(), label, DT_LEFT);
    // Pills inside a control's track.
    let pill_inset = scale(2, dpi);
    let pill_radius = (radius - pill_inset).max(scale(2, dpi));
    match row.control() {
        Control::Check => {
            let on = row.toggle().is_some_and(|toggle| view.checked(toggle));
            let state = RECT {
                left: control.left
                    - scale(
                        TOGGLE_STATE_GAP_AT_96_DPI + TOGGLE_STATE_WIDTH_AT_96_DPI,
                        dpi,
                    ),
                right: control.left - scale(TOGGLE_STATE_GAP_AT_96_DPI, dpi),
                ..card
            };
            frame.text(
                dialog.body_font,
                text,
                if on { "On" } else { "Off" },
                state,
                DT_RIGHT,
            );
            if !enabled {
                let hint = RECT {
                    left: label.left,
                    right: state.left - scale(12, dpi),
                    ..card
                };
                frame.text(
                    dialog.body_font,
                    colors.muted_foreground,
                    AUTOSAVE_HINT,
                    hint,
                    DT_RIGHT,
                );
            }
            let track_radius = (control.bottom - control.top) / 2;
            let knob = toggle_knob(control, on, dpi);
            let (track, knob_color) = match (enabled, on) {
                (true, true) => (
                    Some(if hot { tones.accent_hot } else { tones.accent }),
                    tones.on_accent,
                ),
                (true, false) => (None, colors.muted_foreground),
                // Greyed: a pale track and knob.
                (false, true) => (Some(colors.pressed_background), tones.card),
                (false, false) => (None, colors.pressed_background),
            };
            match track {
                Some(color) => frame.shape(Shape::Round {
                    rect: control,
                    radius: track_radius,
                    color,
                }),
                // Off: a pill outlined on the card, not filled.
                None => frame.shape(Shape::Ring {
                    rect: control,
                    radius: track_radius,
                    width: scale(1, dpi),
                    color: if enabled {
                        colors.muted_foreground
                    } else {
                        colors.pressed_background
                    },
                }),
            }
            frame.shape(Shape::Round {
                rect: knob,
                radius: (knob.bottom - knob.top) / 2,
                color: knob_color,
            });
        }
        Control::Segmented => {
            tones.soft(frame, control, radius, tones.control);
            for (index, (segment, rect)) in segments
                .iter()
                .zip(layout.segment_rects(control, segments.len()))
                .enumerate()
            {
                let pill = inset(rect, pill_inset);
                let hot_segment = dialog.hot == Some(Hit::Row(row, Part::Segment(index)));
                let (fill, foreground) = if segment.selected {
                    (Some(tones.accent), tones.on_accent)
                } else if hot_segment {
                    (Some(tones.control_hot), colors.editor_foreground)
                } else {
                    (None, colors.editor_foreground)
                };
                if let Some(color) = fill {
                    frame.shape(Shape::Round {
                        rect: pill,
                        radius: pill_radius,
                        color,
                    });
                }
                frame.text(
                    dialog.body_font,
                    foreground,
                    segment.label.clone(),
                    rect,
                    DT_CENTER,
                );
            }
        }
        Control::Dropdown => {
            let open = matches!(&dialog.list, Some((open_row, _)) if *open_row == row);
            let fill = if hot || open {
                tones.control_hot
            } else {
                tones.control
            };
            tones.soft(frame, control, radius, fill);
            let chevron = RECT {
                left: control.right - scale(26, dpi),
                ..control
            };
            let value = RECT {
                left: control.left + scale(8, dpi),
                right: chevron.left,
                ..control
            };
            frame.text(
                dialog.body_font,
                colors.editor_foreground,
                view.dropdown_text(row),
                value,
                DT_LEFT,
            );
            frame.text(
                dialog.glyph_font,
                colors.muted_foreground,
                GLYPH_CHEVRON_DOWN,
                chevron,
                DT_CENTER,
            );
        }
        Control::Stepper => {
            let [minus, value, plus] = layout.stepper_rects(control);
            tones.soft(frame, control, radius, tones.control);
            for (rect, glyph, part) in [
                (minus, GLYPH_REMOVE, Part::Minus),
                (plus, GLYPH_ADD, Part::Plus),
            ] {
                let fill = match (dialog.pressed, dialog.hot) {
                    (Some(pressed), _) if pressed == Hit::Row(row, part) => {
                        Some(tones.control_down)
                    }
                    (_, Some(hot)) if hot == Hit::Row(row, part) => Some(tones.control_hot),
                    _ => None,
                };
                if let Some(color) = fill {
                    frame.shape(Shape::Round {
                        rect: inset(rect, pill_inset),
                        radius: pill_radius,
                        color,
                    });
                }
                frame.text(
                    dialog.glyph_font,
                    colors.editor_foreground,
                    glyph,
                    rect,
                    DT_CENTER,
                );
            }
            // The typed value's field.
            frame.shape(Shape::Round {
                rect: inset(value, pill_inset),
                radius: pill_radius,
                color: colors.editor_background,
            });
            frame.text(
                dialog.body_font,
                colors.editor_foreground,
                dialog.model.font_size_text(view),
                value,
                DT_CENTER,
            );
        }
    }
}
