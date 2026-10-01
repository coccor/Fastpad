use super::*;

const AREA: RECT = RECT {
    left: 0,
    top: 40,
    right: 1004,
    bottom: 700,
};

#[test]
fn button_hints_name_the_keymaps_cycle_key() {
    // Break caught: the hint still saying Ctrl+Shift+V after the user rebound the cycle
    // command, or naming a key when it has none.
    assert_eq!(
        button_hint(PreviewButton::Side, Some("F9")),
        "Open preview to the side (F9 cycles preview modes)"
    );
    assert_eq!(
        button_hint(PreviewButton::Full, Some("Ctrl+Shift+V")),
        "Open preview (Ctrl+Shift+V cycles preview modes)"
    );
    assert_eq!(button_hint(PreviewButton::Full, None), "Open preview");
    let keymap = crate::window::keymap::Keymap::defaults();
    assert_eq!(
        keymap
            .first_text(CommandId::MarkdownPreviewCycle)
            .as_deref(),
        Some("Ctrl+Shift+V")
    );
}

#[test]
fn off_or_hidden_previews_give_the_editor_the_whole_area() {
    let whole = ContentRects {
        editor: Some(AREA),
        divider: None,
        preview: None,
    };
    assert_eq!(content_rects(AREA, PreviewMode::Off, true, 0.5, 96), whole);
    assert_eq!(
        content_rects(AREA, PreviewMode::Split, false, 0.5, 96),
        whole
    );
}

#[test]
fn split_places_a_divider_between_editor_and_preview() {
    let rects = content_rects(AREA, PreviewMode::Split, true, 0.5, 96);
    let expected = ContentRects {
        editor: Some(RECT { right: 500, ..AREA }),
        divider: Some(RECT {
            left: 500,
            right: 504,
            ..AREA
        }),
        preview: Some(RECT { left: 504, ..AREA }),
    };
    assert_eq!(rects, expected);
}

#[test]
fn split_ratio_is_clamped() {
    let wide = content_rects(AREA, PreviewMode::Split, true, 0.95, 96);
    assert_eq!(wide.editor.unwrap().right, 800);
    let narrow = content_rects(AREA, PreviewMode::Split, true, 0.01, 96);
    assert_eq!(narrow.editor.unwrap().right, 200);
}

#[test]
fn full_mode_hides_the_editor() {
    let rects = content_rects(AREA, PreviewMode::Full, true, 0.5, 96);
    assert_eq!(
        rects,
        ContentRects {
            editor: None,
            divider: None,
            preview: Some(AREA)
        }
    );
}

fn edit() -> Edit {
    Edit {
        position: 0,
        removed: 0,
        inserted: 1,
        lines_delta: 0,
    }
}

#[test]
fn edits_wait_for_an_outstanding_worker_parse() {
    let small = 1_000;
    let large = WORKER_PARSE_THRESHOLD + 1;
    assert_eq!(
        plan_flush(Pending::Edits(vec![edit()]), large, false, true),
        FlushPlan::Defer(Pending::Edits(vec![edit()]))
    );
    assert_eq!(
        plan_flush(Pending::Full, large, false, true),
        FlushPlan::Defer(Pending::Full)
    );
    assert_eq!(
        plan_flush(Pending::Edits(vec![edit()]), small, true, true),
        FlushPlan::Defer(Pending::Edits(vec![edit()]))
    );
    assert_eq!(
        plan_flush(Pending::Nothing, large, false, true),
        FlushPlan::Nothing
    );
    assert_eq!(
        plan_flush(Pending::Edits(vec![edit()]), large, false, false),
        FlushPlan::Incremental {
            edits: vec![edit()],
            allow_full_parse: false
        }
    );
    assert_eq!(
        plan_flush(Pending::Edits(vec![edit()]), small, false, false),
        FlushPlan::Incremental {
            edits: vec![edit()],
            allow_full_parse: true
        }
    );
}

#[test]
fn huge_documents_pause_and_a_paused_preview_parses_everything_again() {
    assert_eq!(
        plan_flush(
            Pending::Edits(vec![edit()]),
            LIVE_UPDATE_LIMIT + 1,
            false,
            false
        ),
        FlushPlan::Pause
    );
    assert_eq!(
        plan_flush(Pending::Nothing, 10, true, false),
        FlushPlan::Reparse
    );
    assert_eq!(
        plan_flush(
            Pending::Edits(vec![edit()]),
            WORKER_PARSE_THRESHOLD + 1,
            true,
            false
        ),
        FlushPlan::WorkerParse
    );
}

#[test]
fn dragging_maps_the_pointer_to_a_clamped_ratio() {
    assert_eq!(ratio_for_x(AREA, 502, 96), 0.5);
    assert_eq!(ratio_for_x(AREA, 0, 96), 0.2);
    assert_eq!(ratio_for_x(AREA, 5000, 96), 0.8);
}
