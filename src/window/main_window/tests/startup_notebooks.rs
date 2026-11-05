//! Startup restores every notebook that was open last session (multi-root notebooks spec 6).

use super::*;
use crate::window::library_host::{default_notebook, notebooks, with_notebook};

fn open_folders(window: &ProductionWindow) -> Vec<std::path::PathBuf> {
    notebooks(window.hwnd)
        .into_iter()
        .map(|(_, folder)| folder)
        .collect()
}

fn all_loaded(window: &ProductionWindow, count: usize) -> bool {
    let open = notebooks(window.hwnd);
    open.len() == count
        && open
            .iter()
            .all(|(id, _)| with_notebook(window.hwnd, *id, |_| ()).is_some())
}

/// Writes `folders.ini` with these `open=` lines and this default.
fn write_open(
    data: &std::path::Path,
    open: &[std::path::PathBuf],
    default: Option<&std::path::Path>,
) {
    let mut folders = crate::library::local::RecentFolders::default();
    folders.set_open(open, default);
    crate::library::local::write_folders(&crate::library::local::folders_file(data), &folders)
        .unwrap();
}

fn start(a: &LibraryScratch) -> (ProductionWindow, crate::editor::Editor) {
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(a.data());
    (window, editor)
}

#[test]
fn two_remembered_notebooks_both_load_at_startup() {
    // Break caught: startup opening only the first remembered notebook, loading them out of
    // order, or showing a root without its own load.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup2-a");
    a.note("a.md", "a");
    let b = LibraryScratch::new("startup2-b");
    b.note("b.md", "b");
    write_open(&a.data(), &[a.folder(), b.folder()], Some(&b.folder()));
    let (window, _editor) = start(&a);

    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(
        open_folders(&window),
        [a.folder(), b.folder()],
        "both roots show at once, loading"
    );
    pump_until(window.hwnd, || all_loaded(&window, 2));
    assert_eq!(open_folders(&window), [a.folder(), b.folder()]);
    let b_id = notebooks(window.hwnd)[1].0;
    assert_eq!(default_notebook(window.hwnd), Some(b_id));
}

#[test]
fn opening_a_folder_during_startup_keeps_the_restored_roots() {
    // Break caught: a folder arriving by IPC or drop while the restored notebooks load dropping
    // their results, so the session came back with only the newcomer.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup3-a");
    a.note("a.md", "a");
    let b = LibraryScratch::new("startup3-b");
    b.note("b.md", "b");
    let c = LibraryScratch::new("startup3-c");
    c.note("c.md", "c");
    write_open(&a.data(), &[a.folder(), b.folder()], None);
    let (window, _editor) = start(&a);

    crate::window::library_host::open_library_step(window.hwnd);
    crate::window::library_host::open_folder(window.hwnd, &c.folder());
    pump_until(window.hwnd, || all_loaded(&window, 3));
    assert_eq!(open_folders(&window), [a.folder(), b.folder(), c.folder()]);
}

#[test]
fn a_missing_notebook_is_skipped_with_one_notice_and_stays_remembered() {
    // Break caught: a missing root falling back to Documents\FastPad, one notice per root, or the
    // worker rewriting open= so the notebook is forgotten when its drive is back next time.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup4-a");
    a.note("a.md", "a");
    let gone = a.root.join("gone");
    write_open(&a.data(), &[gone.clone(), a.folder()], Some(&gone));
    let (window, _editor) = start(&a);

    let before = crate::library::folder_checks();
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(crate::library::folder_checks(), before, "no stat here");
    assert_eq!(open_folders(&window), [gone.clone(), a.folder()]);
    pump_until(window.hwnd, || all_loaded(&window, 1));
    assert_eq!(open_folders(&window), [a.folder()]);
    let a_id = notebooks(window.hwnd)[0].0;
    assert_eq!(
        default_notebook(window.hwnd),
        Some(a_id),
        "a missing default hands over to the first root that opened"
    );
    let wanted = format!("{} is not available.", gone.display());
    assert_eq!(
        notices(window.hwnd)
            .iter()
            .filter(|n| n.contains("is not available"))
            .collect::<Vec<_>>(),
        [&wanted]
    );
    let saved =
        crate::library::local::read_folders(&crate::library::local::folders_file(&a.data()));
    assert_eq!(saved.startup_roots().0, [gone, a.folder()]);
}

#[test]
fn hand_edited_duplicate_and_nested_open_lines_are_dropped_silently() {
    // Break caught: a duplicate or nested root in folders.ini opening twice, which breaks the
    // one-owner rule notebook_of relies on.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup5-a");
    a.note("a.md", "a");
    let inner = a.folder().join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let b = LibraryScratch::new("startup5-b");
    b.note("b.md", "b");
    let doubled = std::path::PathBuf::from(a.folder().to_string_lossy().to_uppercase());
    let folders = crate::library::local::RecentFolders {
        open: vec![a.folder(), doubled, inner, b.folder()],
        ..Default::default()
    };
    crate::library::local::write_folders(&crate::library::local::folders_file(&a.data()), &folders)
        .unwrap();
    let (window, _editor) = start(&a);
    let ini = crate::library::local::folders_file(&a.data());
    let before = std::fs::read(&ini).unwrap();

    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(open_folders(&window), [a.folder(), b.folder()]);
    pump_until(window.hwnd, || all_loaded(&window, 2));
    assert_eq!(
        std::fs::read(&ini).unwrap(),
        before,
        "folders.ini untouched"
    );
    assert!(
        notices(window.hwnd).is_empty(),
        "{:?}",
        notices(window.hwnd)
    );
}

fn saved_open(a: &LibraryScratch) -> Vec<std::path::PathBuf> {
    crate::library::local::read_folders(&crate::library::local::folders_file(&a.data()))
        .startup_roots()
        .0
}

#[test]
fn an_offline_notebook_does_not_stall_the_others() {
    // Break caught: every notebook sitting on "Loading…" until an offline drive's existence check
    // times out, because loads only started once all the checks had finished.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup10-a");
    a.note("a.md", "a");
    let b = LibraryScratch::new("startup10-b");
    b.note("b.md", "b");
    write_open(&a.data(), &[a.folder(), b.folder()], None);
    let (window, _editor) = start(&a);

    let gate = crate::library::block_folder_checks(&a.folder());
    crate::window::library_host::open_library_step(window.hwnd);
    let b_id = notebooks(window.hwnd)[1].0;
    pump_until(window.hwnd, || {
        with_notebook(window.hwnd, b_id, |_| ()).is_some()
    });
    let a_id = notebooks(window.hwnd)[0].0;
    assert!(
        with_notebook(window.hwnd, a_id, |_| ()).is_none(),
        "A is still being checked"
    );
    drop(gate);
    pump_until(window.hwnd, || all_loaded(&window, 2));
}

#[test]
fn a_missing_notebook_stays_remembered_when_a_launch_folder_is_added() {
    // Break caught: open=[Missing, A] plus `fastpad.exe B` rewriting open= to [A, B], so the
    // notebook is forgotten although its drive is only offline.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup11-a");
    a.note("a.md", "a");
    let b = LibraryScratch::new("startup11-b");
    b.note("b.md", "b");
    let gone = a.root.join("gone");
    write_open(&a.data(), &[gone.clone(), a.folder()], None);
    let (window, _editor) = start(&a);
    launch_with(&window, &b.folder());

    crate::window::library_host::open_library_step(window.hwnd);
    pump_until(window.hwnd, || all_loaded(&window, 2));
    assert_eq!(open_folders(&window), [a.folder(), b.folder()]);
    assert_eq!(saved_open(&a), [gone, a.folder(), b.folder()]);
}

#[test]
fn a_missing_notebook_stays_remembered_when_another_is_opened_later() {
    // Break caught: the first open or close after startup forgetting a notebook whose drive was
    // offline.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup12-a");
    a.note("a.md", "a");
    let c = LibraryScratch::new("startup12-c");
    c.note("c.md", "c");
    let gone = a.root.join("gone");
    write_open(&a.data(), &[gone.clone(), a.folder()], None);
    let (window, _editor) = start(&a);

    crate::window::library_host::open_library_step(window.hwnd);
    pump_until(window.hwnd, || all_loaded(&window, 1));
    crate::window::library_host::open_folder(window.hwnd, &c.folder());
    assert_eq!(saved_open(&a), [gone, a.folder(), c.folder()]);
}

#[test]
fn a_remembered_notebook_that_comes_back_opens_once_in_its_place() {
    // Break caught: opening a folder that was unavailable at startup listing it twice in open=.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup13-a");
    a.note("a.md", "a");
    let gone = a.root.join("gone");
    write_open(&a.data(), &[gone.clone(), a.folder()], None);
    let (window, _editor) = start(&a);

    crate::window::library_host::open_library_step(window.hwnd);
    pump_until(window.hwnd, || all_loaded(&window, 1));
    std::fs::create_dir_all(&gone).unwrap();
    crate::window::library_host::open_folder(window.hwnd, &gone);
    pump_until(window.hwnd, || all_loaded(&window, 2));

    assert_eq!(open_folders(&window), [a.folder(), gone.clone()]);
    assert_eq!(saved_open(&a), [gone, a.folder()]);
}

fn launch_with(window: &ProductionWindow, folder: &std::path::Path) {
    app_mut(window.hwnd).launch.request =
        crate::launch::LaunchRequest::Open(folder.as_os_str().to_owned());
}

#[test]
fn a_launch_folder_opens_after_the_restored_notebooks_and_is_remembered() {
    // Break caught: `fastpad.exe D:\Notes` replacing the restored notebooks, opening before
    // them, or not being remembered for the next start.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup7-a");
    a.note("a.md", "a");
    let b = LibraryScratch::new("startup7-b");
    b.note("b.md", "b");
    write_open(&a.data(), &[a.folder()], None);
    let (window, _editor) = start(&a);
    launch_with(&window, &b.folder());

    crate::window::library_host::open_library_step(window.hwnd);
    pump_until(window.hwnd, || all_loaded(&window, 2));
    assert_eq!(open_folders(&window), [a.folder(), b.folder()]);
    let a_id = notebooks(window.hwnd)[0].0;
    assert_eq!(default_notebook(window.hwnd), Some(a_id));
    let saved =
        crate::library::local::read_folders(&crate::library::local::folders_file(&a.data()));
    assert_eq!(saved.startup_roots().0, [a.folder(), b.folder()]);
}

#[test]
fn a_launch_folder_opens_after_a_closed_session_and_clears_the_close() {
    // Break caught: a folder on the command line refused after a close, or open=none left so the
    // next plain start opens nothing again.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup8-a");
    a.note("a.md", "a");
    let mut folders = crate::library::local::RecentFolders::default();
    folders.set_closed(true);
    crate::library::local::write_folders(&crate::library::local::folders_file(&a.data()), &folders)
        .unwrap();
    let (window, _editor) = start(&a);
    launch_with(&window, &a.folder());

    crate::window::library_host::open_library_step(window.hwnd);
    pump_until(window.hwnd, || all_loaded(&window, 1));
    assert_eq!(open_folders(&window), [a.folder()]);
    let saved =
        crate::library::local::read_folders(&crate::library::local::folders_file(&a.data()));
    assert!(!saved.closed);
    assert_eq!(saved.startup_roots().0, [a.folder()]);
}

#[test]
fn a_launch_folder_that_is_already_open_is_not_opened_twice() {
    // Break caught: the same folder becoming two roots when it is named on the command line and
    // also remembered.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup9-a");
    a.note("a.md", "a");
    let b = LibraryScratch::new("startup9-b");
    b.note("b.md", "b");
    write_open(&a.data(), &[a.folder(), b.folder()], None);
    let (window, _editor) = start(&a);
    launch_with(&window, &b.folder());

    crate::window::library_host::open_library_step(window.hwnd);
    pump_until(window.hwnd, || all_loaded(&window, 2));
    pump_for_a_moment(&window);
    assert_eq!(open_folders(&window), [a.folder(), b.folder()]);
    assert!(
        notices(window.hwnd).is_empty(),
        "{:?}",
        notices(window.hwnd)
    );
}

fn pump_for_a_moment(window: &ProductionWindow) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
    pump_until(window.hwnd, || std::time::Instant::now() >= deadline);
}

#[test]
fn a_session_with_no_notebook_open_clears_any_slot_and_starts_no_worker() {
    // Break caught: open=none at startup leaving a slot behind, or starting a load for it.
    let _scintilla = load_native_scintilla();
    let a = LibraryScratch::new("startup6-a");
    let mut folders = crate::library::local::RecentFolders::default();
    folders.set_closed(true);
    crate::library::local::write_folders(&crate::library::local::folders_file(&a.data()), &folders)
        .unwrap();
    let (window, _editor) = start(&a);
    app_mut(window.hwnd).library.assume_folder(a.folder());

    crate::window::library_host::open_library_step(window.hwnd);
    assert!(open_folders(&window).is_empty());
    assert!(!app_mut(window.hwnd).library.scanning());
}
