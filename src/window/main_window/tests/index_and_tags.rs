//! The note index and the Tags view (second brain phase 1 spec §6, §8).

use super::*;
use crate::window::tags_view::TagRowKind;

fn tags_of(hwnd: HWND, path: &str) -> Vec<String> {
    crate::window::library_host::note_index(hwnd)
        .facts(std::path::Path::new(path))
        .map(|facts| facts.tags.iter().map(|tag| tag.name.clone()).collect())
        .unwrap_or_default()
}

fn open_notebook(label: &str) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
    let _ = load_native_scintilla();
    let scratch = LibraryScratch::new(label);
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    (scratch, window, editor)
}

#[test]
fn the_index_reads_tags_and_follows_saves_and_outside_writes() {
    // Break caught: tags only known for notes opened in FastPad, or stale after a save.
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = open_notebook("index-follow");
    scratch.note("b.md", "---\ntags: [project/atlas]\n---\nBody\n");
    open_note(&window, &scratch, "a.md", "Plan #alpha\n");
    pump_until(window.hwnd, || {
        crate::window::library_host::index_loaded(window.hwnd)
            && tags_of(window.hwnd, "a.md") == ["alpha"]
    });
    assert_eq!(tags_of(window.hwnd, "b.md"), ["project/atlas"]);

    editor.set_text("Plan #beta\n").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    pump_until(window.hwnd, || tags_of(window.hwnd, "a.md") == ["beta"]);

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(scratch.folder().join("b.md"), "Now #gamma\n").unwrap();
    pump_until(window.hwnd, || tags_of(window.hwnd, "b.md") == ["gamma"]);
}

#[test]
fn the_index_is_cached_for_the_next_start() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = open_notebook("index-cache");
    open_note(&window, &scratch, "a.md", "#alpha\n");
    pump_until(window.hwnd, || tags_of(window.hwnd, "a.md") == ["alpha"]);
    let local = app_mut(window.hwnd)
        .library
        .state
        .as_ref()
        .unwrap()
        .local_path
        .clone();
    let cache = std::fs::read_to_string(crate::library::index::cache_file(&local)).unwrap();
    let cached = crate::library::index::NoteIndex::parse(&cache).unwrap();
    assert!(cached.facts(std::path::Path::new("a.md")).is_some());
}

#[test]
fn the_tags_view_lists_tags_and_opens_a_note_from_the_keyboard() {
    // Break caught: no way to browse the notebook by tag, or a tag's notes out of reach.
    let _scintilla = load_native_scintilla();
    let (scratch, window, editor) = open_notebook("tags-view");
    ensure_sidebar(window.hwnd);
    scratch.note("tagged.md", "Tagged #project/atlas\n");
    open_note(&window, &scratch, "a.md", "plain\n");
    pump_until(window.hwnd, || {
        tags_of(window.hwnd, "tagged.md") == ["project/atlas"]
    });

    crate::window::tags_view::reveal(window.hwnd, "project/atlas");
    assert_eq!(
        app_mut(window.hwnd).settings.sidebar_view,
        crate::config::SidebarView::Tags
    );
    let rows = crate::window::tags_view::shown_rows(window.hwnd);
    let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, ["#project", "#atlas"], "the parent opens to show it");

    let (_, panel) = crate::window::side_panel::windows(window.hwnd).unwrap();
    let key = |vk: u16| {
        crate::window::tags_view::handle(
            window.hwnd,
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            usize::from(vk),
            0,
        )
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_RETURN, VK_RIGHT};
    key(VK_RIGHT);
    let rows = crate::window::tags_view::shown_rows(window.hwnd);
    assert_eq!(
        rows.last().map(|row| row.kind.clone()),
        Some(TagRowKind::Note(std::path::PathBuf::from("tagged.md")))
    );
    key(VK_DOWN);
    key(VK_RETURN);
    pump_posted_messages(window.hwnd);
    assert_eq!(editor.text().unwrap(), "Tagged #project/atlas\n");
}

#[test]
fn the_tags_view_says_so_when_there_are_no_tags() {
    let _scintilla = load_native_scintilla();
    let (scratch, window, _editor) = open_notebook("tags-empty");
    ensure_sidebar(window.hwnd);
    open_note(&window, &scratch, "a.md", "no tags here\n");
    pump_until(window.hwnd, || {
        crate::window::library_host::index_loaded(window.hwnd)
    });
    execute_command(window.hwnd, CommandId::ShowTagsView);
    let mode = app_mut(window.hwnd).sidebar.as_ref().unwrap().tags.mode;
    assert_eq!(mode, crate::window::tags_view::Mode::Empty);
}
