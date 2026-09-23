#![cfg(windows)]
// Requires that no other FastPad runs in this session: the library belongs to the primary window.

mod support;

use fastpad::window::commands::CommandId;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use support::process::{FastPadProcess, wait_and_cancel_dialog, wait_for_process_exit};
use support::win32::{Deadline, find_child_by_class, focused_window, scintilla_text, send_text};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_ACTIVATEAPP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_KEYDOWN,
};

static LIBRARY_TEST_LOCK: Mutex<()> = Mutex::new(());
const WAIT: Duration = Duration::from_secs(5);

/// A scratch `LOCALAPPDATA` (`<root>`, with `FastPad\` inside) and a notes folder beside it.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// `folders.ini` names the scratch notes folder, so the harness never seeds its own and no
    /// launch can fall back to the real `Documents\FastPad`.
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "fastpad-library-e2e-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("FastPad")).unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let scratch = Self { root };
        let recent = fastpad::library::local::RecentFolders {
            folders: vec![scratch.folder()],
        };
        std::fs::write(
            fastpad::library::local::folders_file(&scratch.data()),
            recent.encode(),
        )
        .unwrap();
        scratch
    }
    fn folder(&self) -> PathBuf {
        self.root.join("notes")
    }
    fn note(&self, name: &str, text: &str) -> PathBuf {
        let path = self.folder().join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    fn library_ini(&self) -> PathBuf {
        self.folder().join(".fastpad").join("library.ini")
    }
    fn data(&self) -> PathBuf {
        self.root.join("FastPad")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn command(hwnd: HWND, command: CommandId) {
    unsafe {
        PostMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Deadline::after(WAIT);
    while !done() {
        assert!(!deadline.expired(), "timed out waiting for {what}");
        deadline.sleep_step();
    }
}

/// The worker has installed the folder once it writes the per-PC local file.
fn wait_for_library(data: &Scratch) {
    wait_until("the folder to load", || {
        data.data().join("libraries").is_dir()
    });
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Types `text` into a document that already holds text (`send_text` waits for the whole
/// document to equal what it typed).
fn type_more(editor: HWND, text: &str) {
    let before = scintilla_text(editor).unwrap().len();
    for unit in text.encode_utf16() {
        unsafe {
            PostMessageW(editor, WM_CHAR, unit as usize, 0);
        }
    }
    wait_until("the typed text", || {
        scintilla_text(editor).is_ok_and(|t| t.len() == before + text.len())
    });
}

/// Opens `path` in the running primary through a second launch, which forwards it and exits.
fn forward(data: &Scratch, path: &Path) {
    let forwarded = FastPadProcess::spawn_with_local_app_data([path], &data.root).unwrap();
    wait_for_process_exit(forwarded.id(), WAIT).unwrap();
}

fn close(process: FastPadProcess, hwnd: HWND) {
    unsafe {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    wait_for_process_exit(process.id(), WAIT).unwrap();
}

#[test]
fn a_new_note_is_named_inline_and_saved_into_the_opened_folder() {
    // Break caught: Ctrl+S on an untitled tab in notes mode still opening Save As, or the name box
    // saving somewhere other than the open folder, or under a name other than the first line.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("first-save");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    send_text(editor, "Grocery list").unwrap();
    command(hwnd, CommandId::Save);
    wait_until("the name box to take focus", || {
        focused_window(hwnd).is_ok_and(|f| f != editor)
    });
    let field = focused_window(hwnd).unwrap();
    unsafe {
        PostMessageW(field, WM_KEYDOWN, VK_RETURN as usize, 0);
    }
    let saved = data.folder().join("Grocery list.md");
    wait_until("the note file", || read(&saved) == "Grocery list");
    close(process, hwnd);
}

#[test]
fn a_note_in_the_folder_autosaves_but_never_overwrites_an_outside_edit() {
    // Break caught: autosave never firing for a note in the folder, or writing over a file that
    // changed on disk since FastPad loaded or saved it.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("autosave");
    let note = data.note("a.md", "one");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    // Autosave stays off until the folder's state has loaded.
    wait_for_library(&data);
    forward(&data, &note);
    wait_until("the note to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "one")
    });
    type_more(editor, "x");
    wait_until("autosave", || {
        scintilla_text(editor).is_ok_and(|t| read(&note) == t)
    });

    std::fs::write(&note, "synced").unwrap();
    type_more(editor, "y");
    // Autosave fires a second after the last edit; give it well past that.
    std::thread::sleep(Duration::from_millis(2_500));
    assert_eq!(
        read(&note),
        "synced",
        "autosave must not overwrite an outside edit"
    );
    close(process, hwnd);
}

#[test]
fn a_note_keeps_its_favorite_after_being_renamed_in_explorer() {
    // Break caught: a rescan after reactivation treating a renamed file as a new note and
    // dropping its favorite, or never rescanning at all.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("explorer-rename");
    let note = data.note("a.md", "a");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    forward(&data, &note);
    wait_until("the note to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "a")
    });
    command(hwnd, CommandId::NoteToggleFavorite);
    // A record made by a command carries no fingerprint until a rescan fills it; the rename below
    // is followed by the file ID the load cached.
    wait_until("library.ini", || {
        read(&data.library_ini()).contains("|f|-|")
    });
    assert!(read(&data.library_ini()).ends_with("|a.md\r\n"));

    std::fs::rename(&note, data.folder().join("b.md")).unwrap();
    unsafe {
        PostMessageW(hwnd, WM_ACTIVATEAPP, 0, 0);
    }
    // Only a return after `RESCAN_AFTER` (5 s) away rescans.
    std::thread::sleep(Duration::from_millis(5_200));
    unsafe {
        PostMessageW(hwnd, WM_ACTIVATEAPP, 1, 0);
    }
    wait_until("the record to follow the rename", || {
        read(&data.library_ini()).ends_with("|b.md\r\n")
    });
    assert!(read(&data.library_ini()).contains("|f|"));
    close(process, hwnd);
}

#[test]
fn a_second_launch_with_a_folder_switches_the_running_window() {
    // Break caught: a forwarded directory opened as a file (or ignored) instead of switching the
    // primary's library and becoming the most recent folder.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("ipc-folder");
    let other = data.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    forward(&data, &other);
    let folders = data.data().join("folders.ini");
    let expected = format!("folder={}", other.display());
    wait_until("folders.ini to list the new folder first", || {
        read(&folders).lines().find(|l| l.starts_with("folder=")) == Some(expected.as_str())
    });
    close(process, hwnd);
}

#[test]
fn a_damaged_library_file_is_left_byte_for_byte_and_notes_still_open() {
    // Break caught: an unreadable or newer-version library.ini being rewritten (losing the
    // user's notebooks and tags) or blocking the folder's notes from opening.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("damaged");
    let note = data.note("a.md", "still readable");
    std::fs::create_dir_all(data.library_ini().parent().unwrap()).unwrap();
    let damaged = b"version=99\r\n\xff\xfe garbage\r\n";
    std::fs::write(data.library_ini(), damaged).unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    forward(&data, &note);
    wait_until("the note to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "still readable")
    });
    command(hwnd, CommandId::NoteToggleFavorite);
    // A write would follow the 500 ms debounce; give it well past that.
    std::thread::sleep(Duration::from_millis(1_500));
    close(process, hwnd);
    assert_eq!(std::fs::read(data.library_ini()).unwrap(), damaged);
}

#[test]
fn with_notes_mode_off_nothing_is_written_and_save_uses_the_dialog() {
    // Break caught: notes mode off still opening the recent folder (writing its local file or
    // .fastpad), or Ctrl+S still showing the name box instead of Save As.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("mode-off");
    std::fs::write(data.data().join("fastpad.ini"), "notes_mode=false\n").unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    send_text(editor, "plain").unwrap();
    command(hwnd, CommandId::Save);
    wait_and_cancel_dialog(process.id(), WAIT).unwrap();
    assert!(!data.data().join("libraries").exists());
    assert!(!data.folder().join(".fastpad").exists());
    unsafe {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    // restore_session is on by default, so closing does not prompt.
    wait_for_process_exit(process.id(), WAIT).unwrap();
}
