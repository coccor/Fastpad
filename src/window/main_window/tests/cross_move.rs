//! Moving notes and folders between open notebooks (multi-root move spec §2.2, §3.2, §4.1, §5).

use super::*;
use crate::window::cross_move_host::{self, Destination};
use crate::window::library_host::{self, NotebookId};
use std::path::{Path, PathBuf};

/// Two open notebooks, A (the default, folder "A") and B (folder "B"), holding `a_notes` and
/// `b_notes` (relative path, text), with the sidebar shown and both indexes built.
pub(super) struct Two {
    pub(super) _scratch: LibraryScratch,
    pub(super) window: ProductionWindow,
    pub(super) editor: crate::editor::Editor,
    pub(super) a: PathBuf,
    pub(super) b: PathBuf,
    pub(super) a_id: NotebookId,
    pub(super) b_id: NotebookId,
}

fn write(root: &Path, relative: &str, text: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

pub(super) fn two(label: &str, a_notes: &[(&str, &str)], b_notes: &[(&str, &str)]) -> Two {
    let scratch = LibraryScratch::new(label);
    let (a, b) = (scratch.root.join("A"), scratch.root.join("B"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    for (relative, text) in a_notes {
        write(&a, relative, text);
    }
    for (relative, text) in b_notes {
        write(&b, relative, text);
    }
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    ensure_sidebar(window.hwnd);
    let a_id = open_and_load_folder(&window, &a);
    let b_id = open_and_load_folder(&window, &b);
    pump_until(window.hwnd, || {
        library_host::index_loaded_of(window.hwnd, a_id)
            && library_host::index_loaded_of(window.hwnd, b_id)
    });
    // Autosave would save a dirty tab the moment another opens.
    for id in [a_id, b_id] {
        library_host::with_notebook(window.hwnd, id, |state| state.local.autosave = false);
    }
    app_mut(window.hwnd).notifications.dismiss_all();
    Two {
        _scratch: scratch,
        window,
        editor,
        a,
        b,
        a_id,
        b_id,
    }
}

fn listed(hwnd: HWND, id: NotebookId, relative: &str) -> bool {
    library_host::with_notebook(hwnd, id, |state| state.is_listed(Path::new(relative)))
        .unwrap_or(false)
}

fn note(path: &str) -> RowKind {
    RowKind::Note(path.into())
}

fn folder(path: &str) -> RowKind {
    RowKind::Folder(path.into())
}

fn into(id: NotebookId, folder: &str) -> Destination {
    Destination::Open {
        id,
        folder: folder.into(),
    }
}

#[test]
fn a_note_moves_into_a_folder_of_another_notebook_with_its_tab_and_pin() {
    // Break caught: a move that leaves the source listing the note, the destination not listing
    // it until a rescan, the tab on the old path, or the pin dropped (spec §4.1).
    let _scintilla = load_native_scintilla();
    let t = two(
        "xmove-note",
        &[("a.md", "alpha")],
        &[(r"projects\keep.md", "")],
    );
    let hwnd = t.window.hwnd;
    library_host::toggle_pin(hwnd, &t.a.join("a.md"));
    super::super::open_path(hwnd, &t.a.join("a.md")).unwrap();
    app_mut(hwnd).notifications.dismiss_all();

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, "projects"));

    let moved = t.b.join(r"projects\a.md");
    assert!(!t.a.join("a.md").exists());
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "alpha");
    assert!(!listed(hwnd, t.a_id, "a.md"));
    assert!(listed(hwnd, t.b_id, r"projects\a.md"));
    assert_eq!(active_path(hwnd), Some(moved.clone()));
    assert_eq!(
        library_host::with_notebook(hwnd, t.b_id, |state| state.is_pinned(&moved)),
        Some(true),
        "the pin moved"
    );
    assert_eq!(
        selected_row_of(hwnd),
        Some((t.b_id, note(r"projects\a.md")))
    );
    assert!(notices(hwnd).contains(&"Moved a.md to B › projects.".to_owned()));
}

#[test]
fn a_folder_moves_with_its_notes_and_expanded_subfolder() {
    // Break caught: a folder whose inner notes or subfolders the destination doesn't list, or
    // whose expanded subfolder comes back collapsed.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xmove-folder",
        &[(r"work\x.md", "x"), (r"work\deep\y.md", "y")],
        &[],
    );
    let hwnd = t.window.hwnd;
    library_host::set_expanded_of(hwnd, t.a_id, Path::new("work"), true);
    library_host::set_expanded_of(hwnd, t.a_id, Path::new(r"work\deep"), true);

    cross_move_host::move_items(hwnd, t.a_id, vec![folder("work")], into(t.b_id, ""));

    assert!(!t.a.join("work").exists());
    assert!(t.b.join(r"work\deep\y.md").exists());
    assert!(!listed(hwnd, t.a_id, "work"));
    assert!(!listed(hwnd, t.a_id, r"work\x.md"));
    assert!(listed(hwnd, t.b_id, r"work\x.md"));
    assert!(listed(hwnd, t.b_id, r"work\deep\y.md"));
    assert!(listed(hwnd, t.b_id, r"work\deep"));
    let expanded = library_host::with_notebook(hwnd, t.b_id, |state| {
        state
            .local
            .expanded
            .iter()
            .any(|path| crate::library::model::same_path(path, Path::new(r"work\deep")))
    });
    assert_eq!(expanded, Some(true));
    assert!(notices(hwnd).contains(&"Moved work (2 notes) to B.".to_owned()));
}

#[test]
fn the_dirty_active_tab_inside_a_moved_folder_is_saved_follows_and_stays_active() {
    // Break caught (Review Focus 1): unsaved edits left behind in the old notebook, the tab
    // pointing at a path that no longer exists, or its undo history thrown away.
    let _scintilla = load_native_scintilla();
    let t = two("xmove-dirty", &[(r"work\x.md", "x")], &[]);
    let hwnd = t.window.hwnd;
    super::super::open_path(hwnd, &t.a.join(r"work\x.md")).unwrap();
    t.editor.set_text("x, edited").unwrap();

    cross_move_host::move_items(hwnd, t.a_id, vec![folder("work")], into(t.b_id, ""));

    let moved = t.b.join(r"work\x.md");
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "x, edited");
    assert_eq!(active_path(hwnd), Some(moved));
    assert!(!app_mut(hwnd).tabs.active().unwrap().dirty);
    assert!(t.editor.can_undo().unwrap());
}

#[test]
fn a_preview_tab_follows_and_becomes_a_normal_tab() {
    // Break caught (Review Focus 3): the preview replaced by the next click while it shows a
    // note that just changed notebook.
    let _scintilla = load_native_scintilla();
    let t = two("xmove-preview", &[("a.md", "a")], &[]);
    let hwnd = t.window.hwnd;
    super::super::open_note(
        hwnd,
        &t.a.join("a.md"),
        super::super::OpenMode::Preview,
        false,
    )
    .unwrap();
    let id = app_mut(hwnd)
        .tabs
        .find_stored_path(&t.a.join("a.md"))
        .unwrap();

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    let tabs = &app_mut(hwnd).tabs;
    assert_eq!(tabs.document(id).unwrap().path, Some(t.b.join("a.md")));
    assert_eq!(tabs.preview_id(), None);
}

#[test]
fn a_case_only_clash_refuses_and_nothing_moves() {
    // Break caught (Review Focus 2): plan.md moved over Plan.md because the names differ in
    // case, destroying the destination's note.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xmove-clash",
        &[("a.md", "mine")],
        &[(r"projects\A.MD", "theirs")],
    );
    let hwnd = t.window.hwnd;

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, "projects"));

    assert_eq!(std::fs::read_to_string(t.a.join("a.md")).unwrap(), "mine");
    assert_eq!(
        std::fs::read_to_string(t.b.join(r"projects\A.MD")).unwrap(),
        "theirs"
    );
    assert!(listed(hwnd, t.a_id, "a.md"));
    assert!(
        notices(hwnd)
            .contains(&"a.md already exists in B › projects. Nothing was moved.".to_owned())
    );
}

#[test]
fn a_changed_on_disk_tab_stops_the_move_after_others_saved() {
    // Break caught (Review Focus 4): a move that goes ahead with one tab's edits unsaved, or
    // that writes over a file changed outside FastPad.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xmove-changed",
        &[(r"work\x.md", "x"), (r"work\y.md", "y")],
        &[],
    );
    let hwnd = t.window.hwnd;
    let (x, y) = (t.a.join(r"work\x.md"), t.a.join(r"work\y.md"));
    super::super::open_path(hwnd, &x).unwrap();
    t.editor.set_text("x, edited").unwrap();
    super::super::open_path(hwnd, &y).unwrap();
    t.editor.set_text("y, edited").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    // Not an append, which FastPad would take in: a real conflict.
    std::fs::write(&y, "replaced outside").unwrap();
    app_mut(hwnd).notifications.dismiss_all();

    cross_move_host::move_items(hwnd, t.a_id, vec![folder("work")], into(t.b_id, ""));

    assert!(x.exists() && y.exists(), "nothing moved");
    assert!(!t.b.join("work").exists());
    assert_eq!(std::fs::read_to_string(&x).unwrap(), "x, edited", "x saved");
    assert_eq!(std::fs::read_to_string(&y).unwrap(), "replaced outside");
    assert!(
        notices(hwnd)
            .iter()
            .any(|notice| notice.contains("changed on disk. Nothing was moved")),
        "{:?}",
        notices(hwnd)
    );
}

#[test]
fn breaking_links_ask_and_cancel_changes_nothing() {
    // Break caught: links broken without a word, or a Cancel that moved anyway.
    let _scintilla = load_native_scintilla();
    let t = two(
        "xmove-ask-cancel",
        &[("a.md", "a"), ("stay.md", "[[a]]")],
        &[],
    );
    let hwnd = t.window.hwnd;
    crate::window::modal::answer_next_choice(|_| 1);

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    let (text, buttons) = crate::window::modal::take_last_choice().unwrap();
    assert_eq!(text, "Move a.md to B? 1 link in A will stop working.");
    assert_eq!(buttons, ["Move", "Cancel"]);
    assert!(t.a.join("a.md").exists());
    assert!(!t.b.join("a.md").exists());
}

#[test]
fn breaking_links_ask_and_move_proceeds() {
    let _scintilla = load_native_scintilla();
    let t = two(
        "xmove-ask-move",
        &[("a.md", "a"), ("stay.md", "[[a]]")],
        &[],
    );
    let hwnd = t.window.hwnd;
    crate::window::modal::answer_next_choice(|_| 0);

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    assert!(!t.a.join("a.md").exists());
    assert!(t.b.join("a.md").exists());
}

#[test]
fn a_tab_that_cannot_follow_undoes_the_move() {
    // Break caught: two tabs left on one path, or a note moved under a tab that still shows the
    // old place.
    let _scintilla = load_native_scintilla();
    let t = two("xmove-tab-stuck", &[("a.md", "a")], &[("a.md", "b's")]);
    let hwnd = t.window.hwnd;
    super::super::open_path(hwnd, &t.a.join("a.md")).unwrap();
    super::super::open_path(hwnd, &t.b.join("a.md")).unwrap();
    // B's file goes, but its tab still has the path the move wants.
    std::fs::remove_file(t.b.join("a.md")).unwrap();
    app_mut(hwnd).notifications.dismiss_all();

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    assert!(t.a.join("a.md").exists(), "moved back");
    assert!(!t.b.join("a.md").exists());
    assert!(
        notices(hwnd)
            .contains(&"Couldn't move a.md: another tab already has that file open.".to_owned()),
        "{:?}",
        notices(hwnd)
    );
}

#[test]
fn a_closed_destination_moves_nothing() {
    let _scintilla = load_native_scintilla();
    let t = two("xmove-closed", &[("a.md", "a")], &[]);
    let hwnd = t.window.hwnd;
    assert!(library_host::close_notebook_of(hwnd, t.b_id));

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    assert!(t.a.join("a.md").exists());
    assert!(!t.b.join("a.md").exists());
}

#[test]
fn a_folder_to_another_drive_is_refused_for_now() {
    let _scintilla = load_native_scintilla();
    let t = two("xmove-drive-later", &[(r"work\x.md", "x")], &[]);
    let hwnd = t.window.hwnd;
    crate::platform::files::force_other_volume_for_test(true);

    cross_move_host::move_items(hwnd, t.a_id, vec![folder("work")], into(t.b_id, ""));

    crate::platform::files::force_other_volume_for_test(false);
    assert!(t.a.join(r"work\x.md").exists());
    assert!(notices(hwnd).contains(
        &"Couldn't move work: moving folders to another drive isn't ready yet.".to_owned()
    ));
}

#[test]
fn a_note_to_another_drive_moves_through_move_file() {
    // Break caught: a cross-drive note refused like a folder, or rename_no_replace used where it
    // cannot work.
    let _scintilla = load_native_scintilla();
    let t = two("xmove-drive-note", &[("a.md", "a")], &[]);
    let hwnd = t.window.hwnd;
    crate::platform::files::force_other_volume_for_test(true);

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    crate::platform::files::force_other_volume_for_test(false);
    assert!(!t.a.join("a.md").exists());
    assert!(t.b.join("a.md").exists());
    assert!(listed(hwnd, t.b_id, "a.md"));
}

/// A with `a.md` embedding `x.png` (only it) and `y.png` (shared with `stay.md`), both in A's
/// attachments folder; `a.md` open in a clean tab.
fn with_images(label: &str, b_notes: &[(&str, &str)]) -> Two {
    let t = two(
        label,
        &[
            ("a.md", "![x](attachments/x.png)\n![y](attachments/y.png)\n"),
            ("stay.md", "![y](attachments/y.png)\n"),
            (r"attachments\x.png", "x-image"),
            (r"attachments\y.png", "y-image"),
        ],
        b_notes,
    );
    super::super::open_path(t.window.hwnd, &t.a.join("a.md")).unwrap();
    app_mut(t.window.hwnd).notifications.dismiss_all();
    t
}

#[test]
fn move_with_images_moves_a_sole_image_copies_a_shared_one_and_rewrites_the_link() {
    // Break caught: a moved note that loses its images, a shared image taken from the note that
    // stays, or links left pointing at the old notebook.
    let _scintilla = load_native_scintilla();
    let t = with_images("ximg-with", &[(r"projects\keep.md", "")]);
    let hwnd = t.window.hwnd;
    crate::window::modal::answer_next_choice(|_| 0);

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, "projects"));

    let (text, buttons) = crate::window::modal::take_last_choice().unwrap();
    assert_eq!(
        text,
        "Move a.md to B? It uses 2 images from A's attachments folder."
    );
    assert_eq!(
        buttons,
        ["Move with images", "Move without images", "Cancel"]
    );
    assert!(t.b.join(r"attachments\x.png").exists());
    assert!(!t.a.join(r"attachments\x.png").exists(), "x moved");
    assert!(t.b.join(r"attachments\y.png").exists());
    assert!(t.a.join(r"attachments\y.png").exists(), "y copied");
    let expected = "![x](../attachments/x.png)\n![y](../attachments/y.png)\n";
    assert_eq!(
        std::fs::read_to_string(t.b.join(r"projects\a.md")).unwrap(),
        expected
    );
    assert_eq!(t.editor.text().unwrap(), expected, "the clean tab shows it");
}

#[test]
fn move_without_images_touches_no_image() {
    let _scintilla = load_native_scintilla();
    let t = with_images("ximg-without", &[]);
    let hwnd = t.window.hwnd;
    crate::window::modal::answer_next_choice(|_| 1);

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    assert!(t.b.join("a.md").exists());
    assert!(t.a.join(r"attachments\x.png").exists());
    assert!(!t.b.join("attachments").exists());
    assert_eq!(
        std::fs::read_to_string(t.b.join("a.md")).unwrap(),
        "![x](attachments/x.png)\n![y](attachments/y.png)\n"
    );
}

#[test]
fn a_taken_image_name_gets_a_number_and_the_link_follows() {
    // Break caught: a brought-along image written over the destination's own image.
    let _scintilla = load_native_scintilla();
    let t = with_images("ximg-taken", &[(r"attachments\x.png", "b's own")]);
    let hwnd = t.window.hwnd;
    crate::window::modal::answer_next_choice(|_| 0);

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    assert_eq!(
        std::fs::read_to_string(t.b.join(r"attachments\x.png")).unwrap(),
        "b's own"
    );
    assert_eq!(
        std::fs::read_to_string(t.b.join(r"attachments\x (2).png")).unwrap(),
        "x-image"
    );
    assert_eq!(
        std::fs::read_to_string(t.b.join("a.md")).unwrap(),
        "![x](attachments/x%20(2).png)\n![y](attachments/y.png)\n"
    );
}

#[test]
fn an_image_that_fails_keeps_its_link_and_the_notice_says_so() {
    // Break caught: a link rewritten to an image that never arrived, or a failure nobody hears
    // about.
    let _scintilla = load_native_scintilla();
    let t = with_images("ximg-fail", &[]);
    let hwnd = t.window.hwnd;
    let gone = t.a.join(r"attachments\x.png");
    crate::window::modal::answer_next_choice(move |_| {
        std::fs::remove_file(&gone).unwrap();
        0
    });

    cross_move_host::move_items(hwnd, t.a_id, vec![note("a.md")], into(t.b_id, ""));

    assert_eq!(
        std::fs::read_to_string(t.b.join("a.md")).unwrap(),
        "![x](attachments/x.png)\n![y](attachments/y.png)\n",
        "x's link unchanged, y's needed no change"
    );
    assert!(t.b.join(r"attachments\y.png").exists());
    assert!(
        notices(hwnd)
            .iter()
            .any(|notice| notice.ends_with("1 image couldn't be brought along.")),
        "{:?}",
        notices(hwnd)
    );
}
