#![cfg(windows)]
// Requires that no other FastPad runs in this session: the library belongs to the primary window.

mod support;

use fastpad::window::commands::CommandId;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use support::process::{FastPadProcess, wait_and_cancel_dialog, wait_for_process_exit};
use support::win32::{Deadline, find_child_by_class, focused_window, scintilla_text, send_text};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SysFreeString, SysStringLen};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Variant::{VARIANT, VT_I4};
use windows_sys::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_OUTLINEITEM,
    ROLE_SYSTEM_PAGETAB, ROLE_SYSTEM_TEXT,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F3, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    OBJID_CLIENT, PostMessageW, SendMessageW, WM_ACTIVATEAPP, WM_CHAR, WM_CLOSE, WM_COMMAND,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
};
use windows_sys::core::{BSTR, GUID, HRESULT};

static LIBRARY_TEST_LOCK: Mutex<()> = Mutex::new(());
const WAIT: Duration = Duration::from_secs(5);
/// For the steps that wait on a rescan, a rebind and then a one-second autosave in turn: under
/// the full suite's load that chain can outlast `WAIT`.
const RESCAN_CHAIN_WAIT: Duration = Duration::from_secs(15);

/// A scratch `LOCALAPPDATA` (`<root>`, with `FastPad\` inside) and a notes folder beside it.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    /// `folders.ini` names the scratch notes folder, so the harness never seeds its own and no
    /// launch can fall back to the real `Documents\FastPad`.
    fn new(label: &str) -> Self {
        assert_no_fastpad_running();
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
            ..Default::default()
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

/// A FastPad already running in this session would own the single-instance mutex: the spawns
/// below would forward to it, and it would write the scratch folders into the real `folders.ini`.
fn assert_no_fastpad_running() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    let names = fastpad::ipc::InstanceNames::for_current_session().unwrap();
    let mutex = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, names.mutex.as_ptr()) };
    if !mutex.is_null() {
        unsafe {
            CloseHandle(mutex);
        }
        panic!("close every FastPad window in this session before running the library tests");
    }
}

fn command(hwnd: HWND, command: CommandId) {
    unsafe {
        PostMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    wait_within(WAIT, what, done);
}

fn wait_within(wait: Duration, what: &str, done: impl Fn() -> bool) {
    let deadline = Deadline::after(wait);
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

/// The main window's text: "<active tab's title> - FastPad".
fn window_text(hwnd: HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    let mut text = [0_u16; 260];
    let length = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) };
    String::from_utf16_lossy(&text[..length.max(0) as usize])
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

const ACTIVITY_BAR_CLASS: &str = "FastPadActivityBar";
const SIDE_PANEL_CLASS: &str = "FastPadSidePanel";
const IID_IACCESSIBLE: GUID = GUID::from_u128(0x618736e0_3c3d_11cf_810c_00aa00389b71);
const MK_LBUTTON: usize = 0x0001;

/// MSAA locations are physical pixels; this thread must read them the same way.
struct DpiContext(DPI_AWARENESS_CONTEXT);

impl DpiContext {
    fn per_monitor_v2() -> Self {
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Self {
        let result = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        assert!(result >= 0, "CoInitializeEx failed: {result:#x}");
        Self
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[repr(C)]
struct AccessibleVtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_type_info_count: usize,
    get_type_info: usize,
    get_ids_of_names: usize,
    invoke: usize,
    get_acc_parent: usize,
    get_acc_child_count: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    get_acc_child: unsafe extern "system" fn(*mut c_void, VARIANT, *mut *mut c_void) -> HRESULT,
    get_acc_name: unsafe extern "system" fn(*mut c_void, VARIANT, *mut BSTR) -> HRESULT,
    get_acc_value: usize,
    get_acc_description: usize,
    get_acc_role: unsafe extern "system" fn(*mut c_void, VARIANT, *mut VARIANT) -> HRESULT,
    get_acc_state: usize,
    get_acc_help: usize,
    get_acc_help_topic: usize,
    get_acc_keyboard_shortcut: usize,
    get_acc_focus: usize,
    get_acc_selection: usize,
    get_acc_default_action: usize,
    acc_select: usize,
    acc_location: unsafe extern "system" fn(
        *mut c_void,
        *mut i32,
        *mut i32,
        *mut i32,
        *mut i32,
        VARIANT,
    ) -> HRESULT,
}

fn child_variant(id: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    variant.Anonymous.Anonymous.vt = VT_I4;
    variant.Anonymous.Anonymous.Anonymous.lVal = id;
    variant
}

/// A window's MSAA object, read out of process as a screen reader reads it.
struct Accessible(*mut c_void);

impl Accessible {
    fn from_window(hwnd: HWND) -> Option<Self> {
        let mut object = std::ptr::null_mut();
        let result = unsafe {
            AccessibleObjectFromWindow(hwnd, OBJID_CLIENT as u32, &IID_IACCESSIBLE, &mut object)
        };
        (result >= 0 && !object.is_null()).then_some(Self(object))
    }

    fn vtable(&self) -> &AccessibleVtable {
        unsafe { &**(self.0 as *const *const AccessibleVtable) }
    }

    fn child_count(&self) -> i32 {
        let mut count = 0;
        unsafe { (self.vtable().get_acc_child_count)(self.0, &mut count) };
        count
    }

    fn name(&self, child: i32) -> Option<String> {
        let mut value: BSTR = std::ptr::null();
        let result =
            unsafe { (self.vtable().get_acc_name)(self.0, child_variant(child), &mut value) };
        if result < 0 || value.is_null() {
            return None;
        }
        let length = unsafe { SysStringLen(value) } as usize;
        let name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) });
        unsafe { SysFreeString(value) };
        Some(name)
    }

    fn role(&self, child: i32) -> Option<u32> {
        let mut value = VARIANT::default();
        let result =
            unsafe { (self.vtable().get_acc_role)(self.0, child_variant(child), &mut value) };
        (result >= 0).then_some(unsafe { value.Anonymous.Anonymous.Anonymous.lVal } as u32)
    }

    fn location(&self, child: i32) -> Option<RECT> {
        let (mut left, mut top, mut width, mut height) = (0, 0, 0, 0);
        let result = unsafe {
            (self.vtable().acc_location)(
                self.0,
                &mut left,
                &mut top,
                &mut width,
                &mut height,
                child_variant(child),
            )
        };
        (result >= 0).then_some(RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        })
    }

    /// Whether child `child` has a full object of its own (a native control), releasing it.
    fn has_child_object(&self, child: i32) -> bool {
        let mut object = std::ptr::null_mut();
        let result =
            unsafe { (self.vtable().get_acc_child)(self.0, child_variant(child), &mut object) };
        if result < 0 || object.is_null() {
            return false;
        }
        let vtable = unsafe { &**(object as *const *const AccessibleVtable) };
        unsafe { (vtable.release)(object) };
        true
    }

    /// The child IDs and names, in order.
    fn children(&self) -> Vec<(i32, String)> {
        (1..=self.child_count())
            .filter_map(|id| Some((id, self.name(id)?)))
            .collect()
    }
}

impl Drop for Accessible {
    fn drop(&mut self) {
        unsafe {
            (self.vtable().release)(self.0);
        }
    }
}

/// The main window's tab titles, from its title-strip MSAA object.
fn tab_titles(hwnd: HWND) -> Vec<String> {
    let Some(strip) = Accessible::from_window(hwnd) else {
        return Vec::new();
    };
    (1..=strip.child_count())
        .filter(|&id| strip.role(id) == Some(ROLE_SYSTEM_PAGETAB))
        .filter_map(|id| strip.name(id))
        .collect()
}

/// Whether the panel lists a child named `name` now.
fn panel_lists(panel: HWND, name: &str) -> bool {
    Accessible::from_window(panel)
        .is_some_and(|accessible| accessible.children().iter().any(|(_, n)| n == name))
}

/// The note and folder rows the panel lists, in order. Rows for untitled tabs (", unsaved") are
/// left out: every launch starts with one.
fn tree_rows(panel: HWND) -> Vec<String> {
    let Some(accessible) = Accessible::from_window(panel) else {
        return Vec::new();
    };
    (1..=accessible.child_count())
        .filter(|&id| accessible.role(id) == Some(ROLE_SYSTEM_OUTLINEITEM))
        .filter_map(|id| accessible.name(id))
        .filter(|name| !name.ends_with(", unsaved"))
        .collect()
}

/// The panel-client center of the child named `name`, once the panel lists it.
fn child_center(panel: HWND, name: &str) -> isize {
    let deadline = Deadline::after(WAIT);
    loop {
        if let Some(accessible) = Accessible::from_window(panel)
            && let Some((id, _)) = accessible.children().into_iter().find(|(_, n)| n == name)
            && let Some(rect) = accessible.location(id)
        {
            let mut center = POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            };
            unsafe {
                ScreenToClient(panel, &mut center);
            }
            return (center.x as u16 as u32 | ((center.y as u16 as u32) << 16)) as isize;
        }
        assert!(
            !deadline.expired(),
            "timed out waiting for the sidebar to list {name}"
        );
        deadline.sleep_step();
    }
}

fn click_child(panel: HWND, name: &str) {
    let point = child_center(panel, name);
    unsafe {
        PostMessageW(panel, WM_LBUTTONDOWN, MK_LBUTTON, point);
        PostMessageW(panel, WM_LBUTTONUP, 0, point);
    }
}

fn double_click_child(panel: HWND, name: &str) {
    let point = child_center(panel, name);
    unsafe {
        PostMessageW(panel, WM_LBUTTONDOWN, MK_LBUTTON, point);
        PostMessageW(panel, WM_LBUTTONUP, 0, point);
        PostMessageW(panel, WM_LBUTTONDBLCLK, MK_LBUTTON, point);
        PostMessageW(panel, WM_LBUTTONUP, 0, point);
    }
}

/// The first `folder=` line of `folders.ini`: the open notebook.
fn first_folder(data: &Scratch) -> Option<String> {
    read(&data.data().join("folders.ini"))
        .lines()
        .find(|line| line.starts_with("folder="))
        .map(str::to_owned)
}

fn write_folders(data: &Scratch, folders: fastpad::library::local::RecentFolders) {
    std::fs::write(
        fastpad::library::local::folders_file(&data.data()),
        folders.encode(),
    )
    .unwrap();
}

/// The editor's selection, as Scintilla byte positions.
fn selection(editor: HWND) -> (isize, isize) {
    use fastpad::editor::scintilla_constants::{SCI_GETSELECTIONEND, SCI_GETSELECTIONSTART};
    unsafe {
        (
            SendMessageW(editor, SCI_GETSELECTIONSTART, 0, 0),
            SendMessageW(editor, SCI_GETSELECTIONEND, 0, 0),
        )
    }
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
fn a_note_keeps_its_pin_after_being_renamed_in_explorer() {
    // Break caught: a rescan after reactivation treating a renamed file as a new note and
    // dropping its pin, or never rescanning at all. The rename is matched by file ID, so
    // %TEMP% must be on NTFS (or another volume with stable file IDs).
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
    command(hwnd, CommandId::NoteTogglePin);
    // A record made by a command carries no fingerprint until a rescan fills it; the rename below
    // is followed by the file ID the load cached.
    wait_until("library.ini", || read(&data.library_ini()).contains("|p|"));
    assert!(read(&data.library_ini()).ends_with("|a.md\r\n"));

    std::fs::rename(&note, data.folder().join("b.md")).unwrap();
    unsafe {
        PostMessageW(hwnd, WM_ACTIVATEAPP, 0, 0);
    }
    // Only a return after `RESCAN_AFTER` (5 s) away rescans; a full second of margin keeps a slow
    // machine from posting the return too early.
    std::thread::sleep(Duration::from_secs(6));
    unsafe {
        PostMessageW(hwnd, WM_ACTIVATEAPP, 1, 0);
    }
    wait_within(RESCAN_CHAIN_WAIT, "the record to follow the rename", || {
        read(&data.library_ini()).ends_with("|b.md\r\n")
    });
    let library = read(&data.library_ini());
    assert!(
        !library.lines().any(|line| line.ends_with("|a.md")),
        "the old path must not keep a record: {library:?}"
    );
    let renamed = library
        .lines()
        .find(|line| line.ends_with("|b.md"))
        .unwrap();
    assert!(
        renamed.contains("|p|"),
        "the pin must follow the rename: {renamed:?}"
    );
    // The open tab followed too: its next autosave lands in b.md and never re-creates a.md.
    // Waiting for the rebind first makes sure the edit below is autosaved under the new name.
    wait_within(RESCAN_CHAIN_WAIT, "the tab to follow the rename", || {
        window_text(hwnd) == "b.md - FastPad"
    });
    let moved = data.folder().join("b.md");
    type_more(editor, "z");
    wait_within(
        RESCAN_CHAIN_WAIT,
        "the tab to autosave into the renamed file",
        || scintilla_text(editor).is_ok_and(|t| t.len() == 2 && read(&moved) == t),
    );
    assert!(!note.exists(), "the old name must not be re-created");
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
    // user's pins) or blocking the folder's notes from opening.
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
    command(hwnd, CommandId::NoteTogglePin);
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
    unsafe {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    // restore_session is on by default, so closing does not prompt.
    wait_for_process_exit(process.id(), WAIT).unwrap();
    // Checked after exit, so a write at shutdown would be caught too.
    assert!(!data.data().join("libraries").exists());
    assert!(!data.folder().join(".fastpad").exists());
}

#[test]
fn a_click_opens_a_preview_tab_a_second_click_replaces_it_and_a_double_click_keeps_it() {
    // Break caught: every click opening a new tab, the replacement landing at the end of the
    // strip instead of in place, or a double-click leaving the tab to be replaced.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("preview-tab");
    data.note("a.md", "alpha");
    data.note("b.md", "beta");
    data.note("c.md", "gamma");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();

    click_child(panel, "a");
    wait_until("a to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "alpha")
    });
    let first = tab_titles(hwnd);
    let slot = first
        .iter()
        .position(|t| t == "a.md")
        .expect("a preview tab");

    click_child(panel, "b");
    wait_until("b to replace a", || {
        scintilla_text(editor).is_ok_and(|t| t == "beta")
    });
    let replaced = tab_titles(hwnd);
    assert_eq!(replaced.len(), first.len(), "{replaced:?}");
    assert!(!replaced.iter().any(|t| t == "a.md"));
    assert_eq!(replaced.iter().position(|t| t == "b.md"), Some(slot));

    double_click_child(panel, "b");
    click_child(panel, "c");
    wait_until("c to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "gamma")
    });
    let kept = tab_titles(hwnd);
    assert_eq!(kept.len(), first.len() + 1, "{kept:?}");
    assert!(kept.iter().any(|t| t == "b.md") && kept.iter().any(|t| t == "c.md"));
    close(process, hwnd);
}

#[test]
fn pinning_from_the_tree_writes_a_version_2_record_that_survives_a_restart() {
    // Break caught: a pin written in the version 1 format, pinned from the active tab instead of
    // the tree's selected row, or lost (and unsorted) after a restart.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("pin-tree");
    data.note("a.md", "a");
    data.note("b.md", "b");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    click_child(panel, "b");
    // Ctrl+Shift+E moves the focus into the tree, on the active tab's row; the pin then acts on it.
    command(hwnd, CommandId::ShowNotebookView);
    command(hwnd, CommandId::NoteTogglePin);
    wait_until("a version 2 pin record", || {
        let library = read(&data.library_ini());
        library.starts_with("version=2")
            && library
                .lines()
                .any(|l| l.starts_with("note=") && l.contains("|p|") && l.ends_with("|b.md"))
    });
    wait_until("the pinned row to sort first", || {
        tree_rows(panel).first().map(String::as_str) == Some("b, pinned")
    });
    close(process, hwnd);

    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    wait_until("the pin to come back after a restart", || {
        tree_rows(panel) == ["b, pinned", "a"]
    });
    close(process, hwnd);
}

#[test]
fn a_favorite_notebook_opens_from_the_favorites_view() {
    // Break caught: "Toggle favorite notebook" not writing favorite=, or a click in the
    // Favorites view not switching notebooks and showing the Notebook view.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("favorites");
    data.note("a.md", "a");
    let other = data.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("c.md"), "c").unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    command(hwnd, CommandId::ToggleNotebookFavorite);
    let favorite = format!("favorite={}", data.folder().display());
    wait_until("folders.ini to list the favorite", || {
        read(&data.data().join("folders.ini"))
            .lines()
            .any(|l| l == favorite)
    });

    forward(&data, &other);
    let other_line = format!("folder={}", other.display());
    wait_until("the other notebook to open", || {
        first_folder(&data).as_deref() == Some(other_line.as_str())
    });
    wait_until("the tree to list c", || tree_rows(panel) == ["c"]);

    command(hwnd, CommandId::ShowFavoritesView);
    click_child(panel, "notes");
    let notes_line = format!("folder={}", data.folder().display());
    wait_until("the favorite to open", || {
        first_folder(&data).as_deref() == Some(notes_line.as_str())
    });
    wait_until("the Notebook view to list a", || tree_rows(panel) == ["a"]);
    close(process, hwnd);
}

#[test]
fn closing_the_notebook_writes_open_none_and_the_next_start_opens_nothing() {
    // Break caught: Close notebook not remembered, so the next start reopens it, or falls back
    // to Documents\FastPad.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("close-notebook");
    data.note("a.md", "a");
    let folders = data.data().join("folders.ini");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    command(hwnd, CommandId::CloseNotebook);
    wait_until("open=none", || {
        read(&folders).lines().any(|l| l == "open=none")
    });
    close(process, hwnd);

    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    wait_until("the no-notebook state", || {
        panel_lists(panel, "Open notebook\u{2026}")
    });
    // The worker decides the startup notebook; give a wrong decision time to show up.
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(tree_rows(panel).is_empty());
    close(process, hwnd);
    assert!(read(&folders).lines().any(|l| l == "open=none"));
}

#[test]
fn moving_a_note_to_another_notebook_moves_the_file_and_its_tab_follows() {
    // Break caught: Move to notebook copying instead of moving, or the open tab still saving to
    // (and re-creating) the old path.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("move-note");
    let note = data.note("a.md", "alpha");
    let other = data.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    write_folders(
        &data,
        fastpad::library::local::RecentFolders {
            folders: vec![data.folder(), other.clone()],
            ..Default::default()
        },
    );
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    click_child(panel, "a");
    wait_until("a to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "alpha")
    });

    command(hwnd, CommandId::NoteMoveToNotebook);
    wait_until("the picker to take focus", || {
        focused_window(hwnd).is_ok_and(|f| f != editor)
    });
    // The first row is the recent notebook that is not the open one.
    let field = focused_window(hwnd).unwrap();
    unsafe {
        PostMessageW(field, WM_KEYDOWN, VK_RETURN as usize, 0);
    }
    let moved = other.join("a.md");
    wait_until("the file to move", || moved.exists() && !note.exists());

    type_more(editor, "!");
    command(hwnd, CommandId::Save);
    // The caret sits wherever the open left it, so compare with the document, not "alpha!".
    wait_until("the tab to save into the moved file", || {
        scintilla_text(editor).is_ok_and(|t| t.len() == "alpha!".len() && read(&moved) == t)
    });
    assert!(!note.exists(), "the old path must not be re-created");
    close(process, hwnd);
}

#[test]
fn with_notes_mode_off_there_is_no_activity_bar_or_side_panel() {
    // Break caught: the sidebar created, or its space reserved, with notes mode off.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("mode-off-sidebar");
    std::fs::write(data.data().join("fastpad.ini"), "notes_mode=false\n").unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    find_child_by_class(hwnd, "Scintilla").unwrap();
    assert!(find_child_by_class(hwnd, ACTIVITY_BAR_CLASS).is_err());
    assert!(find_child_by_class(hwnd, SIDE_PANEL_CLASS).is_err());
    close(process, hwnd);
}

#[test]
fn searching_the_notebook_opens_a_result_at_its_first_match_and_f3_steps_on() {
    // Break caught: the Search view not searching note text in the real exe, the box or
    // toggles missing from what a screen reader sees, a result opening without the find bar
    // seeded, the first match not selected, or F3 not reaching the next match.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("text-search");
    data.note(
        "a.md",
        "alpha\r\nthe invoice march is paid\r\ninvoice again\r\n",
    );
    data.note("b.md", "nothing to see");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();

    // Ctrl+Shift+F's command; the in-process tests pin the accelerator itself.
    command(hwnd, CommandId::ShowSearchView);
    wait_until("the search box to take focus", || {
        focused_window(hwnd).is_ok_and(|focus| focus != editor && focus != panel)
    });
    let search_box = focused_window(hwnd).unwrap();
    for unit in "invoice".encode_utf16() {
        unsafe {
            PostMessageW(search_box, WM_CHAR, unit as usize, 0);
        }
    }
    let result = "a: the invoice march is paid";
    wait_until("the result", || panel_lists(panel, result));
    assert!(!panel_lists(panel, "b: nothing to see"));

    let accessible = Accessible::from_window(panel).unwrap();
    let children = accessible.children();
    assert!(
        children.iter().any(|(id, name)| {
            name == "Match case" && accessible.role(*id) == Some(ROLE_SYSTEM_CHECKBUTTON)
        }),
        "{children:?}"
    );
    let (box_id, _) = children
        .iter()
        .find(|(id, _)| accessible.role(*id) == Some(ROLE_SYSTEM_TEXT))
        .expect("the search box is one of the panel's children");
    assert!(accessible.has_child_object(*box_id));
    drop(accessible);

    click_child(panel, result);
    // "alpha\r\n" is 7 bytes and "the " 4 more: the first "invoice" is 11..18.
    wait_until("a to open at its first match", || {
        selection(editor) == (11, 18)
    });
    assert!(scintilla_text(editor).is_ok_and(|text| text.starts_with("alpha\r\n")));
    unsafe {
        PostMessageW(editor, WM_KEYDOWN, VK_F3 as usize, 0);
    }
    // The next line starts at 7 + 27 = 34.
    wait_until("F3 to reach the next match", || {
        selection(editor) == (34, 41)
    });
    close(process, hwnd);
}
