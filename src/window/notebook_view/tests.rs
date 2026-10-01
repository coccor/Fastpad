use super::*;
use crate::config::FileIconSet;
use crate::window::file_icons::note_kind;
use crate::window::icon_sets::TreeItem;
use crate::window::palette::{FileIcons, Palette};
use crate::window::panel::fill;
use crate::window::row_list::RowLook;
use crate::window::side_panel::{UiFonts, ViewPaint};
use crate::window::tree_drag::DragSource;

/// `RECT` has no `PartialEq` or `Debug` in windows-sys.
fn edges(rect: RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

fn row(kind: RowKind, depth: u16) -> TreeRow {
    TreeRow {
        kind,
        depth,
        name: String::new(),
        pinned: false,
        expanded: false,
    }
}

#[test]
fn a_row_indents_by_depth_and_keeps_the_pin_at_the_right_edge() {
    // Break caught: deep rows pushing the pin off the row, names drawn over the chevron, or
    // an inverted name rectangle in a panel narrower than the indent.
    let rect = RECT {
        left: 0,
        top: 26,
        right: 260,
        bottom: 52,
    };
    let top = row_parts(rect, 0, 96);
    let deep = row_parts(rect, 3, 96);
    assert_eq!(top.chevron.left, 8);
    assert_eq!(deep.chevron.left, 8 + 3 * 12);
    assert_eq!(edges(top.pin), (236, 26, 260, 52));
    assert_eq!(edges(deep.pin), edges(top.pin));
    assert!(deep.name.left >= deep.icon.right);
    assert_eq!(deep.name.right, deep.pin.left);
    let cramped = row_parts(
        RECT {
            left: 0,
            top: 0,
            right: 40,
            bottom: 26,
        },
        9,
        96,
    );
    assert!(cramped.name.left <= cramped.name.right);
    assert_eq!(row_parts(rect, 1, 192).chevron.left, 16 + 24);
}

#[test]
fn the_no_notebook_state_lists_recent_notebooks_below_its_button() {
    // Break caught: the RECENT rows painted over the Open notebook… button, or a list rect
    // that turns inside out in a short panel.
    let body = RECT {
        left: 0,
        top: 38,
        right: 260,
        bottom: 600,
    };
    let layout = state_layout(body, 96);
    assert!(layout.message.bottom <= layout.button.top);
    assert!(layout.button.bottom <= layout.label.top);
    assert_eq!(layout.list.top, layout.label.bottom);
    assert_eq!(layout.list.bottom, 600);
    let short = state_layout(
        RECT {
            left: 0,
            top: 38,
            right: 260,
            bottom: 60,
        },
        96,
    );
    assert!(short.list.top <= short.list.bottom);
}

#[test]
fn a_vanished_selection_moves_to_the_row_that_took_its_place() {
    // Break caught: a stale index past the end after a rescan removed rows, or a selection
    // that jumps to the top instead of staying where it was.
    let rows = vec![
        row(RowKind::Folder("sub".into()), 0),
        row(RowKind::Note(r"sub\a.md".into()), 1),
        row(RowKind::Note("c.md".into()), 0),
    ];
    let a = RowKind::Note(r"sub\a.md".into());
    assert_eq!(
        follow(&rows, Some(&a), Some(7)),
        Some(1),
        "found by path first"
    );
    let gone = RowKind::Note(r"sub\b.md".into());
    assert_eq!(follow(&rows, Some(&gone), Some(2)), Some(2));
    assert_eq!(follow(&rows, Some(&gone), Some(9)), Some(2));
    assert_eq!(follow(&[], Some(&gone), Some(1)), None);
    assert_eq!(
        follow(&rows, None, Some(1)),
        None,
        "nothing selected stays so"
    );
}

#[test]
fn type_ahead_extends_the_prefix_within_a_second_and_starts_over_after() {
    // Break caught: a prefix that never resets, so a second search a minute later matches
    // nothing.
    let start = Instant::now();
    let mut typed = TypeAhead::default();
    assert_eq!(typed.push('n', start), "n");
    assert_eq!(typed.push('o', start + Duration::from_millis(900)), "no");
    assert_eq!(typed.push('x', start + Duration::from_millis(2_000)), "x");
}

#[test]
fn a_thousand_expanded_folders_of_ten_notes_flatten_within_a_frame() {
    // Break caught: a linear scan of the expanded list per folder row (with two lowercased
    // allocations per comparison), which made a tab switch in a big, fully expanded
    // notebook take close to 100 ms.
    let notes: Vec<PathBuf> = (0..1_000)
        .flat_map(|folder| {
            (0..10).map(move |note| PathBuf::from(format!(r"Folder {folder}\Note {note}.md")))
        })
        .collect();
    let tree = NoteTree::build(&notes, &[], &[]);
    // Stored as the per-PC file may spell them: case differences still match.
    let expanded: Vec<PathBuf> = (0..1_000)
        .map(|folder| PathBuf::from(format!("folder {folder}")))
        .collect();
    let started = Instant::now();
    let rows = flatten(&tree, &expanded);
    let elapsed = started.elapsed();
    assert_eq!(rows.len(), 11_000);
    assert!(rows[0].expanded);
    assert_eq!(flatten(&tree, &[]).len(), 1_000);
    if !cfg!(debug_assertions) {
        assert!(elapsed < Duration::from_millis(16), "{elapsed:?}");
    }
}

#[test]
fn a_tree_row_draws_the_chosen_sets_icon_and_minimal_in_high_contrast() {
    // Break caught: the closed folder icon on an expanded folder, Solid drawn as Minimal, or
    // a Material bitmap in high contrast (icon sets spec §3.2, §6). The `assert_ne!`s below
    // catch one set chosen and another drawn; `blue > red + 60` only checks that the
    // Markdown bitmap's own blue (#42a5f5) is what got drawn into the icon box.
    use crate::window::icon_sets::images::TestTarget;
    use crate::window::icon_sets::material::MaterialIcon;
    use crate::window::titlebar::create_ui_font;
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL, FW_SEMIBOLD};
    let normal = FW_NORMAL as i32;
    let fonts = UiFonts {
        text: create_ui_font(13, "Segoe UI", normal, false),
        bold: create_ui_font(13, "Segoe UI", FW_SEMIBOLD as i32, false),
        italic: create_ui_font(13, "Segoe UI", normal, true),
        glyph: create_ui_font(
            crate::window::design::metrics::SIDEBAR_ICON,
            "Segoe MDL2 Assets",
            normal,
            false,
        ),
        ..UiFonts::default()
    };
    let rect = RECT {
        left: 0,
        top: 0,
        right: 200,
        bottom: 22,
    };
    let icon_box = row_parts(rect, 0, 96).icon;
    let px = (icon_box.right - icon_box.left) as u32;
    let look = RowLook {
        selected: false,
        hover: false,
        focused: false,
    };
    let note = row(RowKind::Note("a.md".into()), 0);
    let open_folder = TreeRow {
        expanded: true,
        ..row(RowKind::Folder("f".into()), 0)
    };
    let target = TestTarget::new(200, 22);
    let mut images = IconImages::new();
    // The icon box's pixels after drawing `row` in `set` under `palette`.
    let mut draw = |row: &TreeRow, set: FileIconSet, palette: &Palette| {
        unsafe { fill(target.dc, rect, palette.editor_background) };
        draw_tree_row(
            target.dc,
            Some(row),
            rect,
            look,
            palette,
            &FileIcons::neutral(),
            fonts,
            96,
            false,
            None,
            &mut images,
            set,
            true,
            false,
        );
        target.area(icon_box)
    };
    // The icon box's pixels after blending `icon` straight into a fresh target.
    let direct = |icon: MaterialIcon, palette: &Palette| {
        let target = TestTarget::new(200, 22);
        unsafe { fill(target.dc, rect, palette.editor_background) };
        assert!(IconImages::new().draw(target.dc, icon, icon_box, px));
        target.area(icon_box)
    };
    let palette = Palette::neutral();
    let material = draw(&note, FileIconSet::Material, &palette);
    assert!(
        material.iter().any(|&pixel| {
            let (red, blue) = ((pixel >> 16) & 0xFF, pixel & 0xFF);
            blue > red + 60
        }),
        "the Markdown bitmap (#42a5f5) is drawn"
    );
    let folder = draw(&open_folder, FileIconSet::Material, &palette);
    assert_eq!(folder, direct(MaterialIcon::FolderOpen, &palette));
    assert_ne!(folder, direct(MaterialIcon::Folder, &palette));
    let minimal = draw(&note, FileIconSet::Minimal, &palette);
    assert_ne!(
        minimal, material,
        "Minimal draws its outline, not the bitmap"
    );
    assert!(
        minimal.iter().any(|&pixel| pixel != minimal[0]),
        "Minimal draws"
    );
    let solid = draw(&note, FileIconSet::Solid, &palette);
    assert_ne!(
        solid, minimal,
        "Solid draws its filled shape, not the outline"
    );
    assert_ne!(solid, material);
    let contrast = Palette {
        high_contrast: true,
        ..palette
    };
    for row in [&note, &open_folder] {
        let minimal = draw(row, FileIconSet::Minimal, &contrast);
        for set in [FileIconSet::Material, FileIconSet::Solid] {
            assert_eq!(
                draw(row, set, &contrast),
                minimal,
                "high contrast draws Minimal in every set"
            );
        }
    }
    for font in [fonts.text, fonts.bold, fonts.italic, fonts.glyph] {
        unsafe { DeleteObject(font) };
    }
}

#[test]
fn a_clipped_icon_box_draws_part_of_the_icon_not_a_shrunken_one() {
    // Break caught: a deep row in a narrow sidebar clamps `parts.icon` below the icon box,
    // and an icon resampled down to that clipped width and cached per clipped size, or one
    // painted past the box over the name.
    use crate::window::icon_sets::images::TestTarget;
    use crate::window::titlebar::create_ui_font;
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL, FW_SEMIBOLD};
    let normal = FW_NORMAL as i32;
    let fonts = UiFonts {
        text: create_ui_font(13, "Segoe UI", normal, false),
        bold: create_ui_font(13, "Segoe UI", FW_SEMIBOLD as i32, false),
        italic: create_ui_font(13, "Segoe UI", normal, true),
        glyph: create_ui_font(
            crate::window::design::metrics::SIDEBAR_ICON,
            "Segoe MDL2 Assets",
            normal,
            false,
        ),
        ..UiFonts::default()
    };
    // At depth 0 and 96 DPI: chevron sits at [8, 24), leaving only 6 px for the icon box
    // inside a 30 px wide row, well under the 16 px glyph box.
    let rect = RECT {
        left: 0,
        top: 0,
        right: 30,
        bottom: 22,
    };
    let icon_box = row_parts(rect, 0, 96).icon;
    assert!(
        icon_box.right - icon_box.left < GLYPH_BOX,
        "the row must actually clip the icon box for this test to mean anything"
    );
    let look = RowLook {
        selected: false,
        hover: false,
        focused: false,
    };
    let note = row(RowKind::Note("a.md".into()), 0);
    let target = TestTarget::new(30, 22);
    let mut images = IconImages::new();
    let palette = Palette::neutral();
    let mut draw = |set: FileIconSet| {
        unsafe { fill(target.dc, rect, palette.editor_background) };
        draw_tree_row(
            target.dc,
            Some(&note),
            rect,
            look,
            &palette,
            &FileIcons::neutral(),
            fonts,
            96,
            false,
            None,
            &mut images,
            set,
            true,
            false,
        );
        target.area(icon_box)
    };
    for set in [
        FileIconSet::Material,
        FileIconSet::Minimal,
        FileIconSet::Solid,
    ] {
        let drawn = draw(set);
        assert!(
            drawn.iter().any(|&pixel| pixel != drawn[0]),
            "{set:?} draws part of its icon in the clipped box"
        );
    }
    let sizes = images.cached_pixel_sizes();
    assert!(
        !sizes.is_empty() && sizes.iter().all(|&px| px == GLYPH_BOX as u32),
        "a clipped row must never resample and cache a bitmap at the clipped width: {sizes:?}"
    );
    for font in [fonts.text, fonts.bold, fonts.italic, fonts.glyph] {
        unsafe { DeleteObject(font) };
    }
}

#[test]
fn the_drop_band_covers_the_folders_rows_in_view_or_the_whole_list() {
    use crate::window::tree_drag::Highlight;
    let list_rect = RECT {
        left: 0,
        top: 100,
        right: 200,
        bottom: 230,
    };
    let mut state = RowListState::new(26);
    state.set_count(20);
    state.top = 3;
    let band = |highlight| band_rect(list_rect, &state, highlight);
    assert_eq!(band(Highlight::Root).map(edges), Some((0, 100, 200, 230)));
    assert_eq!(
        band(Highlight::Rows { start: 4, end: 6 }).map(edges),
        Some((0, 126, 200, 178))
    );
    assert_eq!(
        band(Highlight::Rows { start: 0, end: 5 }).map(edges),
        Some((0, 100, 200, 152)),
        "clipped at the top of the view"
    );
    assert_eq!(
        band(Highlight::Rows { start: 6, end: 20 }).map(edges),
        Some((0, 178, 200, 230)),
        "clipped at the bottom of the list"
    );
    assert!(
        band(Highlight::Rows { start: 0, end: 2 }).is_none(),
        "above the view"
    );
}

#[test]
fn the_drop_band_fills_outside_high_contrast_and_outlines_in_it() {
    use crate::window::icon_sets::images::TestTarget;
    let target = TestTarget::new(60, 40);
    let whole = RECT {
        left: 0,
        top: 0,
        right: 60,
        bottom: 40,
    };
    let band = RECT {
        left: 10,
        top: 10,
        right: 50,
        bottom: 30,
    };
    let inside = RECT {
        left: 20,
        top: 15,
        right: 21,
        bottom: 16,
    };
    let edge = RECT {
        left: 10,
        top: 20,
        right: 11,
        bottom: 21,
    };
    let reference = |color: u32| {
        let target = TestTarget::new(1, 1);
        unsafe {
            fill(
                target.dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: 1,
                    bottom: 1,
                },
                color,
            )
        };
        target.area(RECT {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        })[0]
    };
    let palette = Palette::neutral();
    unsafe { fill(target.dc, whole, palette.editor_background) };
    paint_band(target.dc, band, &palette, 96, true);
    paint_band(target.dc, band, &palette, 96, false);
    assert_eq!(
        target.area(inside)[0],
        reference(palette.inactive_selection_background)
    );

    let contrast = Palette {
        high_contrast: true,
        ..palette
    };
    unsafe { fill(target.dc, whole, contrast.editor_background) };
    paint_band(target.dc, band, &contrast, 96, true);
    assert_eq!(
        target.area(inside)[0],
        reference(contrast.editor_background),
        "no blend"
    );
    paint_band(target.dc, band, &contrast, 96, false);
    assert_eq!(
        target.area(edge)[0],
        reference(contrast.selection_background)
    );
    assert_eq!(
        target.area(inside)[0],
        reference(contrast.editor_background)
    );
}

#[test]
fn the_dragged_row_draws_its_name_dimmed() {
    use crate::window::icon_sets::images::TestTarget;
    use crate::window::titlebar::create_ui_font;
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL};
    let fonts = UiFonts {
        text: create_ui_font(13, "Segoe UI", FW_NORMAL as i32, false),
        ..UiFonts::default()
    };
    let rect = RECT {
        left: 0,
        top: 0,
        right: 200,
        bottom: 22,
    };
    let name = row_parts(rect, 0, 96).name;
    let look = RowLook {
        selected: false,
        hover: false,
        focused: false,
    };
    let palette = Palette::neutral();
    let note = TreeRow {
        name: "dragged".into(),
        ..row(RowKind::Note("a.md".into()), 0)
    };
    let mut images = IconImages::new();
    let mut draw = |dimmed: bool| {
        let target = TestTarget::new(200, 22);
        unsafe { fill(target.dc, rect, palette.editor_background) };
        draw_tree_row(
            target.dc,
            Some(&note),
            rect,
            look,
            &palette,
            &FileIcons::neutral(),
            fonts,
            96,
            false,
            None,
            &mut images,
            FileIconSet::Minimal,
            true,
            dimmed,
        );
        target.area(name)
    };
    assert_ne!(draw(true), draw(false));
    unsafe { DeleteObject(fonts.text) };
}

#[test]
fn the_drag_label_has_a_border_a_fill_an_icon_and_its_name() {
    // Break caught: a label painted in colours outside the system pairs in high contrast,
    // without its border, or with no name or icon (tree drag spec §3.2).
    use crate::window::icon_sets::images::TestTarget;
    use crate::window::titlebar::create_ui_font;
    use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL};
    let fonts = UiFonts {
        text: create_ui_font(13, "Segoe UI", FW_NORMAL as i32, false),
        ..UiFonts::default()
    };
    let size = drag_label_size(80, 96);
    assert_eq!((size.cx, size.cy), (8 + 16 + 6 + 80 + 8, 24));
    let reference = |color: u32| {
        let target = TestTarget::new(1, 1);
        let pixel = RECT {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        };
        unsafe { fill(target.dc, pixel, color) };
        target.pixel(0, 0)
    };
    let mut images = IconImages::new();
    let mut check = |palette: Palette| {
        let (background, border, _) = drag_label_colors(&palette);
        let target = TestTarget::new(size.cx, size.cy);
        let paint = ViewPaint {
            hdc: target.dc,
            client: RECT::default(),
            palette,
            icons: FileIcons::neutral(),
            icon_set: FileIconSet::Minimal,
            light_theme: true,
            background: palette.panel_background(),
            fonts,
            dpi: 96,
            focused: false,
        };
        paint_drag_label(
            target.dc,
            size,
            TreeItem::Note(note_kind(std::path::Path::new("a.md"))),
            "notes.md",
            &paint,
            &mut images,
        );
        assert_eq!(target.pixel(0, 12), reference(border), "the left border");
        assert_eq!(target.pixel(size.cx - 1, 0), reference(border), "a corner");
        assert_eq!(
            target.pixel(size.cx - 3, 3),
            reference(background),
            "the fill past the name"
        );
        let drawn = |left: i32, right: i32| {
            target
                .area(RECT {
                    left,
                    top: 2,
                    right,
                    bottom: size.cy - 2,
                })
                .iter()
                .any(|&pixel| pixel != reference(background))
        };
        assert!(drawn(8, 24), "the icon");
        assert!(drawn(30, size.cx - 8), "the name");
    };
    check(Palette::neutral());
    check(Palette {
        high_contrast: true,
        ..Palette::neutral()
    });
    let contrast = Palette {
        high_contrast: true,
        ..Palette::neutral()
    };
    assert_eq!(
        drag_label_colors(&contrast),
        (
            contrast.editor_background,
            contrast.editor_foreground,
            contrast.editor_foreground
        )
    );
    unsafe { DeleteObject(fonts.text) };
}

#[test]
fn the_drag_label_shows_a_closed_folder_or_the_note_type() {
    assert_eq!(
        drag_item(&RowKind::Folder("work".into())),
        Some(TreeItem::Folder { expanded: false })
    );
    assert_eq!(
        drag_item(&RowKind::Note(r"work\a.json".into())),
        Some(TreeItem::Note(note_kind(std::path::Path::new("a.json"))))
    );
    assert_eq!(drag_item(&RowKind::Draft), None);
}

#[test]
fn is_dragged_row_never_matches_with_no_drag_even_past_the_last_row() {
    // Break caught: `None == None` reading as a match, dimming a row (e.g. the truncated
    // row, past the last real one) while nothing is being dragged (tree drag spec §3.2).
    let rows = vec![row(RowKind::Note("a.md".into()), 0)];
    assert!(!is_dragged_row(None, &rows, 0));
    assert!(!is_dragged_row(None, &rows, 5), "past the last row too");
    let dragged = DragSource::Row(RowKind::Note("a.md".into()));
    assert!(is_dragged_row(Some(&dragged), &rows, 0));
    assert!(!is_dragged_row(Some(&dragged), &rows, 5), "no row there");
    let other = DragSource::Row(RowKind::Note("b.md".into()));
    assert!(!is_dragged_row(Some(&other), &rows, 0), "a different row");
}

fn nested_tree() -> NoteTree {
    let notes: Vec<PathBuf> = ["top.md", r"a\one.md", r"a\b\two.md", r"c\three.md"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    NoteTree::build(&notes, &[], &[])
}

#[test]
fn expand_all_opens_every_folder_and_collapse_all_closes_them_without_touching_the_root() {
    // Break caught: Expand all opening only the top level, Collapse all leaving a nested folder
    // open, or either one flipping the root row's own state.
    let tree = nested_tree();
    let folders = tree.folder_paths();
    assert_eq!(
        folders,
        [
            PathBuf::from("a"),
            PathBuf::from(r"a\b"),
            PathBuf::from("c")
        ]
    );
    let mut local = crate::library::local::LocalState::new(PathBuf::from(r"D:\Notes"));
    assert!(local.set_all_expanded(true, &folders));
    let rows = flatten(&tree, &local.expanded);
    assert!(
        rows.iter()
            .any(|row| row.kind == RowKind::Note(r"a\b\two.md".into()))
    );
    assert!(
        rows.iter()
            .filter_map(|row| match row.kind {
                RowKind::Folder(_) => Some(row.expanded),
                _ => None,
            })
            .all(|open| open)
    );
    assert!(!local.root_collapsed, "the root row is not the toggle's");
    assert!(
        !local.set_all_expanded(true, &folders),
        "nothing changed, nothing to write"
    );

    assert!(local.set_all_expanded(false, &folders));
    assert!(local.expanded.is_empty());
    assert!(!local.root_collapsed, "collapse keeps the root open");
    let rows = flatten(&tree, &local.expanded);
    assert_eq!(rows.len(), 3, "the folders a and c, and the root's note");
    assert!(!local.set_all_expanded(false, &folders));
}

#[test]
fn collapse_all_keeps_the_selected_row_when_it_is_still_listed() {
    // Break caught: the selection jumping to the top after a collapse, for a row that is
    // still in the list (a top-level note or folder).
    let tree = nested_tree();
    let open = [
        PathBuf::from("a"),
        PathBuf::from(r"a\b"),
        PathBuf::from("c"),
    ];
    let before = flatten(&tree, &open);
    let selected = RowKind::Note("top.md".into());
    let index = tree::row_index(&before, &selected);
    let after = flatten(&tree, &[]);
    assert_eq!(
        follow(&after, Some(&selected), index),
        tree::row_index(&after, &selected)
    );
    // A row inside a collapsed folder is gone: the selection stays in range.
    let hidden = RowKind::Note(r"a\b\two.md".into());
    let index = tree::row_index(&before, &hidden);
    assert!(follow(&after, Some(&hidden), index).is_some_and(|row| row < after.len()));
}

#[test]
fn the_color_under_a_row_is_the_band_inside_the_highlight_and_the_panel_elsewhere() {
    // Break caught: rounded row corners blending toward the panel inside the drop band.
    use crate::window::tree_drag::Highlight;
    let palette = Palette {
        inactive_selection_background: 0x00aa_bbcc,
        ..Palette::neutral()
    };
    let panel = 0x0011_2233;
    let rows = Some(Highlight::Rows { start: 2, end: 4 });
    let under = |highlight, index| paint::under_row(highlight, index, &palette, panel);
    assert_eq!(under(rows, 1), panel);
    assert_eq!(under(rows, 2), palette.inactive_selection_background);
    assert_eq!(under(rows, 3), palette.inactive_selection_background);
    assert_eq!(under(rows, 4), panel);
    assert_eq!(under(None, 2), panel);
    assert_eq!(
        under(Some(Highlight::Root), 9),
        palette.inactive_selection_background
    );
    let contrast = Palette {
        high_contrast: true,
        ..palette
    };
    assert_eq!(
        paint::under_row(Some(Highlight::Root), 0, &contrast, panel),
        panel
    );
}
