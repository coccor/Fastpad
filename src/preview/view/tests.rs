use super::*;
use crate::platform::theme::Theme;
use crate::preview::colors::preview_colors;
use crate::preview::render::TestWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_END, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MSG, MoveWindow, PM_REMOVE, PeekMessageW, SendMessageW, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_PAINT,
};

fn view_with(parent: &TestWindow, source: &str) -> PreviewView {
    let graphics = Rc::new(Graphics::load().unwrap());
    let view = PreviewView::create(
        parent.0,
        graphics,
        preview_colors(Theme::Light, false),
        PreviewFonts::from_settings("Segoe UI", "Consolas", 14, 16),
    )
    .unwrap();
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindow};
        ShowWindow(parent.0, SW_SHOWNOACTIVATE);
        MoveWindow(view.hwnd(), 0, 0, 400, 300, 0);
        ShowWindow(view.hwnd(), SW_SHOWNOACTIVATE);
    }
    view.mark_opened(Instant::now());
    view.replace_document(PreviewDocument::parse(source), None, Instant::now());
    // The view paints without BeginPaint, so a direct WM_PAINT renders synchronously.
    unsafe { SendMessageW(view.hwnd(), WM_PAINT, 0, 0) };
    view
}

fn take_posted(parent: &TestWindow, message: u32) -> Option<MSG> {
    let mut msg = MSG::default();
    (unsafe { PeekMessageW(&mut msg, parent.0, message, message, PM_REMOVE) } != 0).then_some(msg)
}

#[test]
fn content_is_padded_and_capped_when_centered() {
    assert_eq!(content_frame(400.0, false), (16.0, 368.0));
    assert_eq!(content_frame(2000.0, true), (510.0, 980.0));
    assert_eq!(content_frame(500.0, true), (16.0, 468.0));
}

#[test]
fn painting_a_document_records_stats() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, "# A\n\ntext\n");
    let stats = view.stats();
    assert_eq!(stats.block_count, 2);
    assert!(stats.revision > 0);
    assert!(stats.first_frame_micros > 0);
    view.destroy();
}

#[test]
fn scrolling_to_a_line_reports_the_same_top_line() {
    let parent = TestWindow::new(800, 600);
    let source = (0..200)
        .map(|index| format!("para {index}\n\n"))
        .collect::<String>();
    let view = view_with(&parent, &source);
    view.scroll_to_line(100);
    assert_eq!(view.top_line(), 100);
    view.destroy();
}

#[test]
fn end_key_scrolls_to_the_bottom_and_reports_the_scroll() {
    let parent = TestWindow::new(800, 600);
    let source = (0..200)
        .map(|index| format!("para {index}\n\n"))
        .collect::<String>();
    let view = view_with(&parent, &source);
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_END as usize, 0) };
    assert!(view.top_line() > 0);
    assert!(take_posted(&parent, WM_FASTPAD_PREVIEW_SCROLLED).is_some());
    view.destroy();
}

#[test]
fn escape_is_forwarded_to_the_parent() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, "text\n");
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert!(take_posted(&parent, WM_FASTPAD_PREVIEW_ESCAPE).is_some());
    view.destroy();
}

#[test]
fn clicking_a_link_posts_its_destination() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, "[site](https://x.dev)\n");
    let rect = view
        .visible_links()
        .into_iter()
        .next()
        .expect("a laid-out link")
        .rect;
    assert_eq!(view.accessible_links().read().unwrap()[0].text, "site");
    let point = (((rect.top + rect.bottom) / 2) << 16 | ((rect.left + rect.right) / 2)) as isize;
    unsafe {
        SendMessageW(view.hwnd(), WM_LBUTTONDOWN, 0, point);
        SendMessageW(view.hwnd(), WM_LBUTTONUP, 0, point);
    }
    let message = take_posted(&parent, WM_FASTPAD_PREVIEW_LINK).expect("link message");
    let dest = unsafe { Box::from_raw(message.lParam as *mut String) };
    assert_eq!(*dest, "https://x.dev");
    view.destroy();
}

fn target_dpi(view: &PreviewView) -> Option<f32> {
    with_state(view.hwnd(), |state| {
        state.target.as_ref().map(|target| {
            let (mut dpi_x, mut dpi_y) = (0.0, 0.0);
            unsafe { target.GetDpi(&mut dpi_x, &mut dpi_y) };
            assert_eq!(dpi_x, dpi_y);
            dpi_x
        })
    })
    .flatten()
}

#[test]
fn a_target_left_at_another_dpi_adopts_the_window_dpi_before_drawing() {
    // Break caught: the render target took its DPI only at creation, so after a monitor move it
    // drew at the old scale while clicks were hit-tested at the new one.
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, "[site](https://x.dev)\n");
    let window_dpi = unsafe { GetDpiForWindow(view.hwnd()) }.max(96) as f32;
    let stale = if window_dpi == 192.0 { 96.0 } else { 192.0 };
    with_state(view.hwnd(), |state| {
        let target = state.target.as_ref().expect("painted");
        unsafe { target.SetDpi(stale, stale) };
    });
    assert_eq!(target_dpi(&view), Some(stale));
    repaint(&view);
    assert_eq!(target_dpi(&view), Some(window_dpi));
    let (view_width, _) = view_size(view.hwnd());
    let layout_width = with_state(view.hwnd(), |state| state.layout_width).unwrap();
    assert_eq!(layout_width, content_frame(view_width, false).1);
    let rect = view.visible_links().first().expect("a laid-out link").rect;
    let point = (((rect.top + rect.bottom) / 2) << 16 | ((rect.left + rect.right) / 2)) as isize;
    unsafe {
        SendMessageW(view.hwnd(), WM_LBUTTONDOWN, 0, point);
        SendMessageW(view.hwnd(), WM_LBUTTONUP, 0, point);
    }
    let message = take_posted(&parent, WM_FASTPAD_PREVIEW_LINK).expect("link message");
    let dest = unsafe { Box::from_raw(message.lParam as *mut String) };
    assert_eq!(*dest, "https://x.dev");
    view.destroy();
}

#[test]
fn accessible_activation_follows_the_posted_destination() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, "[site](https://x.dev)\n");
    let payload = Box::into_raw(Box::new(String::from("notes.md")));
    unsafe {
        SendMessageW(
            view.hwnd(),
            WM_FASTPAD_PREVIEW_ACTIVATE,
            0,
            payload as isize,
        )
    };
    let message = take_posted(&parent, WM_FASTPAD_PREVIEW_LINK).expect("link message");
    let dest = unsafe { Box::from_raw(message.lParam as *mut String) };
    assert_eq!(*dest, "notes.md");
    view.destroy();
}

#[test]
fn releasing_frees_the_model_layouts_and_render_target() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, "# A\n\n[site](https://x.dev)\n");
    assert!(target_dpi(&view).is_some());
    view.release();
    assert_eq!(view.stats().block_count, 0);
    assert!(view.accessible_links().read().unwrap().is_empty());
    let released = with_state(view.hwnd(), |state| {
        state.target.is_none()
            && state.brushes.is_none()
            && state.layouts.is_empty()
            && state.heights.is_empty()
            && state.document.blocks.is_empty()
    });
    assert_eq!(released, Some(true));
    view.replace_document(PreviewDocument::parse("# B\n"), None, Instant::now());
    repaint(&view);
    assert_eq!(view.stats().block_count, 1);
    assert!(target_dpi(&view).is_some());
    view.destroy();
}

#[test]
fn anchors_scroll_to_their_heading() {
    let parent = TestWindow::new(800, 600);
    let mut source = (0..100)
        .map(|index| format!("para {index}\n\n"))
        .collect::<String>();
    source.push_str("## Deep Heading\n");
    let view = view_with(&parent, &source);
    assert!(view.scroll_to_anchor("deep-heading"));
    assert!(view.top_line() >= 150);
    assert!(!view.scroll_to_anchor("missing"));
    view.destroy();
}

fn repaint(view: &PreviewView) {
    unsafe { SendMessageW(view.hwnd(), WM_PAINT, 0, 0) };
}

#[test]
fn width_changes_keep_the_reading_position() {
    let parent = TestWindow::new(800, 600);
    let source = (0..200)
        .map(|index| format!("para {index}\n\n"))
        .collect::<String>();
    let view = view_with(&parent, &source);
    view.scroll_to_line(100);
    repaint(&view);
    assert_eq!(view.top_line(), 100);
    unsafe { MoveWindow(view.hwnd(), 0, 0, 300, 300, 0) };
    repaint(&view);
    assert_eq!(view.top_line(), 100);
    view.destroy();
}

#[test]
fn links_clipped_out_of_a_wide_table_are_hidden_until_focus_reveals_them() {
    let parent = TestWindow::new(800, 600);
    let wide = "wide ".repeat(40);
    let view = view_with(
        &parent,
        &format!("| {wide} | [far](https://far.dev) |\n|---|---|\n| a | b |\n"),
    );
    assert!(view.visible_links().is_empty());
    assert!(view.accessible_links().read().unwrap().is_empty());
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_TAB as usize, 0) };
    repaint(&view);
    let links = view.visible_links();
    assert_eq!(links.len(), 1, "focus scrolls the table to the link");
    let mut client = RECT::default();
    unsafe { GetClientRect(view.hwnd(), &mut client) };
    assert!(links[0].rect.left >= 0 && links[0].rect.right <= client.right);
    view.destroy();
}

#[test]
fn unchanged_updates_do_not_record_an_update() {
    let parent = TestWindow::new(800, 600);
    let source = "# A\n\ntext\n";
    let view = view_with(&parent, source);
    let before = view.stats().last_update_micros;
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(
        view.apply_edits(source, &[], Instant::now(), true),
        Some(Update::Unchanged)
    );
    repaint(&view);
    assert_eq!(view.stats().last_update_micros, before);
    view.destroy();
}

#[test]
fn anchors_follow_incremental_edits() {
    // Break caught: anchors are rebuilt lazily; a stale list would miss a heading typed into
    // the document or point a moved heading at its old block.
    let parent = TestWindow::new(800, 600);
    let mut source = (0..100)
        .map(|index| format!("para {index}\n\n"))
        .collect::<String>();
    source.push_str("## Deep Heading\n");
    let view = view_with(&parent, &source);
    assert!(view.scroll_to_anchor("deep-heading"));
    let position = source.find("para 50").unwrap();
    let inserted = "## Added Heading\n\n";
    source.insert_str(position, inserted);
    let update = view.apply_edits(
        source.as_str(),
        &[Edit {
            position,
            removed: 0,
            inserted: inserted.len(),
            lines_delta: 2,
        }],
        Instant::now(),
        false,
    );
    assert!(
        matches!(update, Some(Update::Replaced { .. })),
        "{update:?}"
    );
    view.scroll_to_line(0);
    repaint(&view);
    assert!(view.scroll_to_anchor("added-heading"));
    assert_eq!(view.top_line(), 100);
    assert!(view.scroll_to_anchor("deep-heading"));
    assert!(view.top_line() >= 152);
    view.destroy();
}

fn click(view: &PreviewView, rect: RECT) {
    let point = (((rect.top + rect.bottom) / 2) << 16 | ((rect.left + rect.right) / 2)) as isize;
    unsafe {
        SendMessageW(view.hwnd(), WM_LBUTTONDOWN, 0, point);
        SendMessageW(view.hwnd(), WM_LBUTTONUP, 0, point);
    }
}

fn visible_target(view: &PreviewView, disclosure: bool) -> VisibleLink {
    view.visible_links()
        .into_iter()
        .find(|link| link.disclosure.is_some() == disclosure)
        .expect("a visible target")
}

fn paragraphs(count: usize) -> String {
    (0..count)
        .map(|index| format!("para {index}\n\n"))
        .collect()
}

const SECTION: &str = "<details>\n<summary>More</summary>\n\nHidden body\n\n</details>\n";

#[test]
fn clicking_a_disclosure_expands_and_collapses_it() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, SECTION);
    let collapsed = view.stats().content_height;
    click(&view, visible_target(&view, true).rect);
    repaint(&view);
    assert!(view.stats().content_height > collapsed);
    assert_eq!(
        visible_target(&view, true).disclosure.map(|d| d.expanded),
        Some(true)
    );
    click(&view, visible_target(&view, true).rect);
    repaint(&view);
    assert_eq!(view.stats().content_height, collapsed);
    assert!(take_posted(&parent, WM_FASTPAD_PREVIEW_LINK).is_none());
    view.destroy();
}

#[test]
fn tab_enter_and_space_toggle_a_focused_disclosure() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, SECTION);
    // The first paint hides the scrollbar this short document does not need; the widened
    // content lays out again on the next paint, which would drop keyboard focus.
    repaint(&view);
    let collapsed = view.stats().content_height;
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_TAB as usize, 0) };
    repaint(&view);
    assert!(visible_target(&view, true).focused);
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_RETURN as usize, 0) };
    repaint(&view);
    assert!(view.stats().content_height > collapsed);
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_SPACE as usize, 0) };
    repaint(&view);
    assert_eq!(view.stats().content_height, collapsed);
    view.destroy();
}

#[test]
fn a_link_in_a_summary_is_followed_without_toggling() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(
        &parent,
        "<details>\n<summary>See <a href=\"https://x.dev\">site</a></summary>\n\nBody\n\n</details>\n",
    );
    let collapsed = view.stats().content_height;
    click(&view, visible_target(&view, false).rect);
    let message = take_posted(&parent, WM_FASTPAD_PREVIEW_LINK).expect("link message");
    let dest = unsafe { Box::from_raw(message.lParam as *mut String) };
    assert_eq!(*dest, "https://x.dev");
    repaint(&view);
    assert_eq!(view.stats().content_height, collapsed);
    view.destroy();
}

#[test]
fn anchors_inside_collapsed_sections_open_them_first() {
    let parent = TestWindow::new(800, 600);
    // Paragraphs after the section keep the scroll from clamping at the end of the document.
    let source = paragraphs(100)
        + "<details>\n<summary>More</summary>\n\n## Deep Heading\n\nBody\n\n</details>\n\n"
        + &paragraphs(40);
    let view = view_with(&parent, &source);
    assert!(view.scroll_to_anchor("deep-heading"));
    repaint(&view);
    assert_eq!(
        visible_target(&view, true).disclosure.map(|d| d.expanded),
        Some(true)
    );
    assert!(view.top_line() >= 200);
    view.destroy();
}

#[test]
fn a_collapsed_section_maps_its_source_lines_to_its_disclosure_row() {
    let parent = TestWindow::new(800, 600);
    let mut source = paragraphs(60);
    let details_line = source.lines().count();
    source.push_str("<details>\n<summary>More</summary>\n\n");
    source.push_str(
        &(0..40)
            .map(|index| format!("hidden {index}\n\n"))
            .collect::<String>(),
    );
    source.push_str("</details>\n\n");
    source.push_str(&paragraphs(60));
    let view = view_with(&parent, &source);
    view.scroll_to_line(details_line + 30);
    repaint(&view);
    view.scroll_to_line(details_line + 30);
    let (top, height, scroll) = with_state(view.hwnd(), |state| {
        let index = state
            .document
            .blocks
            .iter()
            .position(|block| {
                matches!(block.kind, crate::preview::model::BlockKind::Details { .. })
            })
            .unwrap();
        (
            state.heights.top(index),
            state.heights.height(index),
            state.scroll_y,
        )
    })
    .unwrap();
    assert!(
        scroll >= top && scroll <= top + height,
        "{scroll} is outside the section at {top}..{}",
        top + height
    );
    with_state(view.hwnd(), |state| set_scroll(state, top, true));
    assert_eq!(view.top_line(), details_line);
    view.destroy();
}

#[test]
fn accessible_activation_toggles_a_disclosure_and_reports_the_change_once() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, SECTION);
    let key = visible_target(&view, true).disclosure.unwrap().key;
    let payload = Box::into_raw(Box::new(key));
    unsafe {
        SendMessageW(
            view.hwnd(),
            WM_FASTPAD_PREVIEW_ACTIVATE,
            ACTIVATE_DISCLOSURE,
            payload as isize,
        )
    };
    assert!(with_state(view.hwnd(), |state| state.state_change.is_some()).unwrap());
    repaint(&view);
    assert!(with_state(view.hwnd(), |state| state.state_change.is_none()).unwrap());
    assert_eq!(
        view.accessible_links().read().unwrap()[0]
            .disclosure
            .as_ref()
            .map(|d| d.expanded),
        Some(true)
    );
    view.destroy();
}

#[test]
fn a_section_added_above_renumbers_the_kept_layouts_below_it() {
    // Break caught: kept layouts held the old section keys, so clicking the lower section
    // after typing a same-named section above it toggled the new one instead.
    let parent = TestWindow::new(800, 600);
    let mut source = format!("a\n\nb\n\nc\n\n{SECTION}\n{}", paragraphs(20));
    let view = view_with(&parent, &source);
    repaint(&view);
    let inserted = "<details>\n<summary>More</summary>\n\nNew\n\n</details>\n\n";
    let position = source.find("c\n").unwrap();
    source.insert_str(position, inserted);
    let update = view.apply_edits(
        source.as_str(),
        &[Edit {
            position,
            removed: 0,
            inserted: inserted.len(),
            lines_delta: inserted.matches('\n').count() as isize,
        }],
        Instant::now(),
        false,
    );
    assert!(
        matches!(update, Some(Update::Replaced { .. })),
        "{update:?}"
    );
    repaint(&view);
    let mut occurrences = view
        .visible_links()
        .into_iter()
        .filter_map(|link| link.disclosure.map(|disclosure| disclosure.key.occurrence))
        .collect::<Vec<_>>();
    occurrences.sort_unstable();
    assert_eq!(occurrences, vec![0, 1]);
    view.destroy();
}

#[test]
fn a_worker_parse_of_the_same_document_keeps_expanded_sections() {
    let parent = TestWindow::new(800, 600);
    let view = view_with(&parent, SECTION);
    click(&view, visible_target(&view, true).rect);
    repaint(&view);
    view.install_parse(PreviewDocument::parse(SECTION), None, Instant::now());
    repaint(&view);
    assert_eq!(
        visible_target(&view, true).disclosure.map(|d| d.expanded),
        Some(true)
    );
    view.replace_document(PreviewDocument::parse(SECTION), None, Instant::now());
    repaint(&view);
    assert_eq!(
        visible_target(&view, true).disclosure.map(|d| d.expanded),
        Some(false)
    );
    view.destroy();
}
