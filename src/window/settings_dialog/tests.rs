use super::*;

fn view() -> SettingsView {
    SettingsView {
        settings: crate::config::default_settings(),
        notebook_autosave: None,
    }
}

fn center(rect: RECT) -> (i32, i32) {
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

#[test]
fn cards_stack_under_their_headings_with_gaps_and_everything_fits_at_96_dpi() {
    // Break caught: cards touching or overlapping, a heading painted under its first card,
    // cards flush with the dialog's edges, or a dialog over about 700 px tall at 96 DPI
    // (soft-look addendum).
    let layout = Layout::calculate(96, 4000, 2000, 100);
    assert_eq!(layout.width, 860);
    assert_eq!(layout.max_scroll(), 0);
    let mut expected_top = 0;
    for section in Section::ALL {
        assert_eq!(layout.headings[section as usize].top, expected_top);
        expected_top = layout.headings[section as usize].bottom;
        for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
            let card = layout.rows[row as usize];
            assert_eq!(card.top, expected_top, "{row:?}");
            assert_eq!(card.bottom - card.top, 36, "{row:?}");
            assert_eq!((card.left, card.right), (200, 840), "{row:?}");
            expected_top = card.bottom + 3;
        }
    }
    assert_eq!(layout.content_height, expected_top);
    assert_eq!(layout.height, 44 + layout.content_height + 56);
    assert!(layout.height <= 700, "{}", layout.height);
    assert!(layout.edit_ini.right <= layout.close.left);
}

#[test]
fn the_nav_sits_left_of_the_cards_and_hits_its_pages() {
    // Break caught: cards painted under the nav, or a nav item that doesn't switch pages.
    let layout = Layout::calculate(96, 4000, 2000, 100);
    assert!(layout.nav.right <= layout.rows[0].left);
    assert_eq!(layout.body.left, layout.nav.right);
    for page in Page::ALL {
        let (x, y) = center(layout.nav_items[page as usize]);
        assert_eq!(
            layout.hit(x, y, 0, &view(), Page::General),
            Some(Hit::Nav(page))
        );
    }
    let (x, y) = center(layout.row_rect(Row::Theme, 0));
    assert_eq!(
        layout.hit(x, y, 0, &view(), Page::Shortcuts),
        None,
        "no General rows on the other page"
    );
}

#[test]
fn hits_find_controls_checkbox_labels_segments_and_stepper_parts() {
    let layout = Layout::calculate(96, 4000, 2000, 100);
    let view = view();
    let hit = |rect: RECT| {
        let (x, y) = center(rect);
        layout.hit(x, y, 0, &view, Page::General)
    };
    let row = |row| layout.row_rect(row, 0);
    let control = |r: Row| layout.control_rect(r, row(r), view.segments(r).len());

    assert_eq!(
        hit(control(Row::Theme)),
        Some(Hit::Row(Row::Theme, Part::Whole))
    );
    let label = RECT {
        right: row(Row::Theme).left + 40,
        ..row(Row::Theme)
    };
    assert_eq!(hit(label), None, "a dropdown's label is not the dropdown");
    let label = RECT {
        right: row(Row::WordWrap).left + 40,
        ..row(Row::WordWrap)
    };
    assert_eq!(
        hit(label),
        Some(Hit::Row(Row::WordWrap, Part::Whole)),
        "a checkbox's label toggles it"
    );

    let segments = layout.segment_rects(control(Row::TabWidth), 3);
    assert_eq!(
        hit(segments[2]),
        Some(Hit::Row(Row::TabWidth, Part::Segment(2)))
    );
    let [minus, value, plus] = layout.stepper_rects(control(Row::FontSize));
    assert_eq!(hit(minus), Some(Hit::Row(Row::FontSize, Part::Minus)));
    assert_eq!(hit(value), Some(Hit::Row(Row::FontSize, Part::Value)));
    assert_eq!(hit(plus), Some(Hit::Row(Row::FontSize, Part::Plus)));

    let gap = RECT {
        top: row(Row::WordWrap).bottom,
        bottom: row(Row::LineNumbers).top,
        ..row(Row::WordWrap)
    };
    assert_eq!(hit(gap), None, "the gap between two cards");

    assert_eq!(hit(layout.close), Some(Hit::Close));
    assert_eq!(hit(layout.edit_ini), Some(Hit::EditIni));
    assert_eq!(hit(layout.title_close), Some(Hit::TitleClose));
    assert_eq!(hit(layout.title), None);
}

#[test]
fn only_a_quick_nearby_press_after_a_pick_completes_a_double_click() {
    // Break caught: the second click of a double-click on a dropdown item toggling the
    // checkbox row under it, or every later click after a pick being swallowed (final
    // review 3).
    let picked = (1_000, POINT { x: 100, y: 200 });
    let limits = (500, 4, 4);
    let near = POINT { x: 102, y: 198 };
    assert!(completes_double_click(picked, 1_300, near, limits));
    assert!(
        !completes_double_click(picked, 1_600, near, limits),
        "too late"
    );
    assert!(
        !completes_double_click(picked, 1_300, POINT { x: 103, y: 200 }, limits),
        "too far"
    );
    assert!(
        completes_double_click((u32::MAX - 10, picked.1), 100, near, limits),
        "the tick count wrapping between the clicks"
    );
}

#[test]
fn a_sized_layout_fills_the_size_it_is_given() {
    // Break caught: a resized dialog painting its footer, cards or Close button where the
    // old size put them.
    let opened = Layout::calculate(96, 1920, 1080, 90);
    let sized = Layout::sized(96, 1000, 900, 90);
    assert_eq!((sized.width, sized.height), (1000, 900));
    assert_eq!(sized.body.bottom, 900 - 56);
    assert_eq!(sized.close.right, 1000 - 20);
    assert_eq!(sized.rows[0].right, 1000 - 20);
    assert_eq!(sized.title_close.right, 1000);
    // Taller than everything: nothing to scroll.
    assert_eq!(sized.max_scroll(), 0);
    // The opening size is a sized layout too.
    let again = Layout::sized(96, opened.width, opened.height, 90);
    let edges = |rect: RECT| (rect.left, rect.top, rect.right, rect.bottom);
    assert_eq!(edges(again.body), edges(opened.body));
    assert_eq!(edges(again.close), edges(opened.close));
}

#[test]
fn a_saved_size_reopens_scaled_and_fitted_to_the_screen() {
    // Break caught: Settings reopening at its default size after being sized, at the wrong
    // size on a scaled monitor, or larger than the screen or smaller than usable when
    // fastpad.ini says so.
    assert_eq!(opening_size((900, 700), 96, 1920, 1040), (900, 700));
    assert_eq!(opening_size((900, 700), 144, 1920, 1040), (1350, 1040));
    assert_eq!(opening_size((5000, 5000), 96, 1920, 1040), (1920, 1040));
    assert_eq!(opening_size((10, 10), 96, 1920, 1040), (600, 360));
    // A work area under the minimum wins.
    assert_eq!(opening_size((900, 700), 96, 450, 300), (450, 300));
    let layout = Layout::sized(144, 1350, 1050, 90);
    assert_eq!(saved_size(&layout), (900, 700));
}

#[test]
fn the_edges_size_the_dialog_and_the_corners_size_both_ways() {
    // Break caught: a dialog that can't be sized because its hidden frame takes no drags, or
    // an edge band so wide it eats clicks meant for the nav or the cards.
    let edge = |x, y| sizing_edge(x, y, 800, 600, 8);
    assert_eq!(edge(2, 300), Some(HTLEFT));
    assert_eq!(edge(797, 300), Some(HTRIGHT));
    assert_eq!(edge(400, 3), Some(HTTOP));
    assert_eq!(edge(400, 595), Some(HTBOTTOM));
    assert_eq!(edge(1, 1), Some(HTTOPLEFT));
    assert_eq!(edge(799, 0), Some(HTTOPRIGHT));
    assert_eq!(edge(0, 599), Some(HTBOTTOMLEFT));
    assert_eq!(edge(799, 599), Some(HTBOTTOMRIGHT));
    assert_eq!(edge(8, 300), None);
    assert_eq!(edge(400, 300), None);
}

#[test]
fn the_layout_scales_with_dpi() {
    let normal = Layout::calculate(96, 4000, 4000, 100);
    let double = Layout::calculate(192, 4000, 4000, 200);
    assert_eq!(double.width, normal.width * 2);
    assert_eq!(double.content_height, normal.content_height * 2);
    assert_eq!(double.rows[5].top, normal.rows[5].top * 2);
    assert_eq!(
        double.title_close.right - double.title_close.left,
        (normal.title_close.right - normal.title_close.left) * 2
    );
}

#[test]
fn the_title_close_button_fills_the_title_row_corner_like_the_caption_close() {
    // Break caught: a × inset from the corner or shorter than the title row, unlike the
    // main window's caption close button, or a title that runs under it.
    let layout = Layout::calculate(96, 4000, 2000, 100);
    let close = layout.title_close;
    assert_eq!(
        (close.left, close.top, close.right, close.bottom),
        (860 - 46, 0, 860, 44)
    );
    assert_eq!(layout.title.bottom, close.bottom);
    assert_eq!(layout.title.right, close.left);
    assert_eq!(layout.body.top, close.bottom);
}

#[test]
fn a_toggle_row_shows_a_40_by_20_switch_at_its_right_end() {
    // Break caught: the switch drawn in the 18-px square the checkbox had, squashed into a
    // square, off-centre in its row, or not DPI-scaled.
    for dpi in [96, 144, 192] {
        let layout = Layout::calculate(dpi, 4000, 4000, 100);
        for row in Row::ALL
            .into_iter()
            .filter(|row| row.control() == Control::Check)
        {
            let row_rect = layout.row_rect(row, 0);
            let control = layout.control_rect(row, row_rect, 0);
            assert_eq!(control.right - control.left, scale(40, dpi), "{row:?}");
            assert_eq!(control.bottom - control.top, scale(20, dpi), "{row:?}");
            assert_eq!(
                control.right,
                row_rect.right - scale(16, dpi),
                "right-aligned inside the card's padding"
            );
            assert!(
                (control.top - row_rect.top - (row_rect.bottom - control.bottom)).abs() <= 1,
                "centred in the row"
            );
        }
    }
}

#[test]
fn the_knob_is_a_circle_inside_the_track_on_the_right_when_on() {
    // Break caught: a knob that pokes out of its track, is not round, or sits on the same
    // side whether the switch is on or off.
    let track = RECT {
        left: 100,
        top: 10,
        right: 140,
        bottom: 30,
    };
    let off = toggle_knob(track, false, 96);
    let on = toggle_knob(track, true, 96);
    assert_eq!(
        (off.left, off.top, off.right, off.bottom),
        (104, 14, 116, 26)
    );
    assert_eq!((on.left, on.top, on.right, on.bottom), (124, 14, 136, 26));

    let layout = Layout::calculate(192, 4000, 4000, 100);
    let track = layout.control_rect(Row::WordWrap, layout.row_rect(Row::WordWrap, 0), 0);
    for on in [false, true] {
        let knob = toggle_knob(track, on, 192);
        assert_eq!(knob.right - knob.left, knob.bottom - knob.top, "round");
        assert!(knob.left > track.left && knob.right < track.right);
        assert!(knob.top > track.top && knob.bottom < track.bottom);
        let middle = (track.left + track.right) / 2;
        if on {
            assert!(knob.left > middle, "on: right");
        } else {
            assert!(knob.right < middle, "off: left");
        }
    }
}

fn painted_dialog(canvas: Canvas, theme: crate::platform::theme::Theme) -> Dialog {
    Dialog {
        colors: Palette::for_theme(theme, false),
        link_color: 0,
        layout: Layout::calculate(96, 4000, 2000, 100),
        view: view(),
        model: DialogModel::new(Page::General),
        fonts: Vec::new(),
        scroll: 0,
        title_font: std::ptr::null_mut(),
        heading_font: std::ptr::null_mut(),
        body_font: std::ptr::null_mut(),
        link_font: std::ptr::null_mut(),
        glyph_font: std::ptr::null_mut(),
        hot: None,
        pressed: None,
        tracking_leave: false,
        list: None,
        picked_at: None,
        swallowing: false,
        outcome: Rc::new(Cell::new(Outcome::Closed)),
        canvas,
        page_layout: super::super::shortcuts_page::PageLayout::calculate(
            Layout::calculate(96, 4000, 2000, 100).body,
            96,
        ),
        shortcuts: super::super::shortcuts_model::ShortcutsModel::new(
            crate::window::keymap::Keymap::defaults(),
            1,
        ),
        last_row_click: None,
        wheel_rest: 0,
        search: None,
        search_brush: std::ptr::null_mut(),
    }
}

#[test]
fn toggles_show_their_state_whether_direct2d_or_the_gdi_fallback_paints() {
    // Break caught: a switch whose knob or track is missing, on the wrong side, or the same
    // on and off, in either theme, and above all when Direct2D can't load and GDI paints.
    use crate::platform::theme::Theme;
    use crate::window::soft_paint::TestSurface;
    for theme in [Theme::Light, Theme::Dark] {
        for direct2d in [false, true] {
            let canvas = if direct2d {
                Canvas::load()
            } else {
                Canvas::gdi()
            };
            assert_eq!(canvas.uses_direct2d(), direct2d);
            let dialog = painted_dialog(canvas, theme);
            let layout = dialog.layout;
            let colors = dialog.colors;
            let tones = Tones::new(&colors);
            let surface = TestSurface::new(layout.width, layout.height);
            let client = RECT {
                left: 0,
                top: 0,
                right: layout.width,
                bottom: layout.height,
            };
            paint_into(surface.dc, client, &dialog);
            let at = |rect: RECT| surface.pixel(center(rect).0, center(rect).1);
            let control = |row| layout.control_rect(row, layout.row_rect(row, 0), 0);
            let case = format!("{theme:?}, Direct2D {direct2d}");

            // Line numbers are on by default: an accent track, the knob on the right.
            let track = control(Row::LineNumbers);
            assert_eq!(at(toggle_knob(track, true, 96)), tones.on_accent, "{case}");
            assert_eq!(at(toggle_knob(track, false, 96)), tones.accent, "{case}");
            // Word wrap is off: the card shows through the outline, the knob on the left.
            let track = control(Row::WordWrap);
            assert_eq!(
                at(toggle_knob(track, false, 96)),
                colors.muted_foreground,
                "{case}"
            );
            assert_eq!(at(toggle_knob(track, true, 96)), tones.card, "{case}");
            assert_eq!(
                surface.pixel(center(track).0, track.top),
                colors.muted_foreground,
                "{case}: the off outline"
            );
            // Notebook autosave with no notebook: greyed.
            let track = control(Row::NotebookAutosave);
            assert_eq!(
                at(toggle_knob(track, false, 96)),
                colors.pressed_background,
                "{case}"
            );
            // The card, and Close as a filled accent button.
            let card = layout.row_rect(Row::Theme, 0);
            assert_eq!(surface.pixel(card.left + 8, center(card).1), tones.card);
            assert_eq!(
                surface.pixel(layout.close.left + 4, center(layout.close).1),
                tones.accent,
                "{case}"
            );
        }
    }
}

#[test]
fn dropdowns_take_the_extra_width_and_the_other_controls_keep_their_natural_sizes() {
    // Break caught: long font names cut off in a dropdown that stayed 240 px in a wider
    // dialog, a dropdown grown over its label, or segments, steppers and toggles stretched
    // instead of right-aligned at their own sizes.
    let view = view();
    for dpi in [96, 144, 192] {
        let layout = Layout::calculate(dpi, 8000, 8000, 100);
        for row in Row::ALL {
            let card = layout.row_rect(row, 0);
            let control = layout.control_rect(row, card, view.segments(row).len());
            let width = control.right - control.left;
            assert_eq!(control.right, card.right - scale(16, dpi), "{row:?}");
            let natural = match row.control() {
                Control::Dropdown => 360,
                Control::Segmented if row == Row::TabWidth => 44 * 3,
                Control::Segmented => 80 * view.segments(row).len() as i32,
                Control::Stepper => 28 * 2 + 48,
                Control::Check => 40,
            };
            assert_eq!(width, scale(natural, dpi), "{row:?} at {dpi} DPI");
        }
    }
}

#[test]
fn a_narrow_work_area_caps_the_width_and_shrinks_the_dropdowns_not_their_labels() {
    // Break caught: a dialog wider than the screen, its × or Close off the right edge, or a
    // dropdown squeezed over its label instead of giving up its extra width.
    let layout = Layout::calculate(96, 450, 2000, 100);
    assert_eq!(layout.width, 450);
    assert_eq!(layout.title_close.right, 450);
    assert_eq!(layout.close.right, 450 - 20);
    let view = view();
    for row in [Row::Theme, Row::Font] {
        let card = layout.row_rect(row, 0);
        assert_eq!((card.left, card.right), (170, 430));
        let control = layout.control_rect(row, card, 0);
        assert_eq!(control.right, card.right - 16);
        assert!(
            control.left >= card.left + 16 + 120,
            "{row:?}: the label keeps 120 px"
        );
        assert!(control.right - control.left < 360, "{row:?} gave up width");
    }
    let track = layout.control_rect(
        Row::WordWrap,
        layout.row_rect(Row::WordWrap, 0),
        view.segments(Row::WordWrap).len(),
    );
    assert_eq!(track.right - track.left, 40, "a toggle keeps its size");
    assert_eq!(
        Layout::calculate(144, 4000, 2000, 100).width,
        scale(860, 144),
        "a wide work area leaves the scaled width alone"
    );
}

#[test]
fn a_short_work_area_caps_the_height_and_scrolls_the_focused_row_into_view() {
    // Break caught: a dialog taller than a 1366×768 screen at 150%, with Close off-screen,
    // or Tab moving the focus to a row the body never scrolls to (review focus 4).
    let layout = Layout::calculate(144, 4000, 700, 150);
    assert_eq!(layout.height, 700);
    assert!(layout.max_scroll() > 0);
    let visible = layout.body.bottom - layout.body.top;
    let scroll = layout.scroll_to_show(Focus::Row(Row::NotebookAutosave), 0);
    let bottom = layout.rows[Row::NotebookAutosave as usize].bottom;
    assert_eq!(
        scroll,
        bottom - visible,
        "just enough to show the last row whole"
    );
    assert!(scroll <= layout.max_scroll());
    assert_eq!(
        layout.scroll_to_show(Focus::Row(Row::Theme), scroll),
        0,
        "the first row brings its heading back"
    );
    assert_eq!(layout.scroll_to_show(Focus::Close, 37), 37);
    let tiny = Layout::calculate(96, 4000, 100, 100);
    assert!(tiny.height > 100, "at least a few rows always show");
}
