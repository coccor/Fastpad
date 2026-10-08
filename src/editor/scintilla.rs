use crate::editor::ViewState;
#[cfg(windows)]
use crate::editor::scintilla_constants::{
    SC_AUTOMATICFOLD_CHANGE, SC_AUTOMATICFOLD_CLICK, SC_AUTOMATICFOLD_SHOW, SC_FOLDACTION_CONTRACT,
    SC_FOLDACTION_EXPAND, SC_FOLDFLAG_LINEAFTER_CONTRACTED, SC_MARGIN_SYMBOL, SC_MARK_BOXMINUS,
    SC_MARK_BOXMINUSCONNECTED, SC_MARK_BOXPLUS, SC_MARK_BOXPLUSCONNECTED, SC_MARK_LCORNER,
    SC_MARK_TCORNER, SC_MARK_VLINE, SC_MARKNUM_FOLDER, SC_MARKNUM_FOLDEREND,
    SC_MARKNUM_FOLDERMIDTAIL, SC_MARKNUM_FOLDEROPEN, SC_MARKNUM_FOLDEROPENMID,
    SC_MARKNUM_FOLDERSUB, SC_MARKNUM_FOLDERTAIL, SC_MASK_FOLDERS, SCI_FOLDALL, SCI_MARKERDEFINE,
    SCI_MARKERSETBACK, SCI_MARKERSETBACKSELECTED, SCI_MARKERSETFORE, SCI_SETAUTOMATICFOLD,
    SCI_SETFOLDFLAGS, SCI_SETFOLDMARGINCOLOUR, SCI_SETFOLDMARGINHICOLOUR, SCI_SETMARGINMASKN,
    SCI_SETMARGINSENSITIVEN,
};
use crate::editor::scintilla_constants::{
    SC_CP_UTF8, SCI_ADDREFDOCUMENT, SCI_BEGINUNDOACTION, SCI_CANREDO, SCI_CANUNDO,
    SCI_COPYALLOWLINE, SCI_CREATEDOCUMENT, SCI_CUTALLOWLINE, SCI_EMPTYUNDOBUFFER,
    SCI_ENDUNDOACTION, SCI_GETDIRECTFUNCTION, SCI_GETDIRECTPOINTER, SCI_GETDOCPOINTER,
    SCI_GETLENGTH, SCI_GETSELECTIONEND, SCI_GETSELECTIONSTART, SCI_GETSELTEXT, SCI_GETTARGETEND,
    SCI_GETTEXT, SCI_GETTEXTLENGTH, SCI_PASTE, SCI_REDO, SCI_RELEASEDOCUMENT, SCI_REPLACETARGET,
    SCI_SCROLLCARET, SCI_SEARCHINTARGET, SCI_SETCODEPAGE, SCI_SETDOCPOINTER, SCI_SETILEXER,
    SCI_SETKEYWORDS, SCI_SETPROPERTY, SCI_SETSAVEPOINT, SCI_SETSEARCHFLAGS, SCI_SETSEL,
    SCI_SETTARGETRANGE, SCI_SETTEXT, SCI_SETUNDOCOLLECTION, SCI_STYLECLEARALL, SCI_STYLESETBACK,
    SCI_STYLESETBOLD, SCI_STYLESETFONT, SCI_STYLESETFORE, SCI_STYLESETITALIC, SCI_UNDO,
};
#[cfg(windows)]
use crate::editor::scintilla_constants::{
    SC_ELEMENT_CARET_LINE_BACK, SC_ELEMENT_SELECTION_BACK, SC_ELEMENT_SELECTION_INACTIVE_BACK,
    SC_ELEMENT_SELECTION_INACTIVE_TEXT, SC_ELEMENT_SELECTION_TEXT, SC_WRAP_NONE, SC_WRAP_WORD,
    SCI_GOTOLINE, SCI_RESETELEMENTCOLOUR, SCI_SETCARETFORE, SCI_SETELEMENTCOLOUR,
    SCI_SETMARGINLEFT, SCI_SETMARGINRIGHT, SCI_SETMARGINWIDTHN, SCI_SETSCROLLWIDTH,
    SCI_SETSCROLLWIDTHTRACKING, SCI_SETTABWIDTH, SCI_SETUSETABS, SCI_SETVIEWWS, SCI_SETWRAPMODE,
    SCI_STYLESETSIZEFRACTIONAL, SCWS_INVISIBLE, SCWS_VISIBLEALWAYS, STYLE_DEFAULT,
};
#[cfg(windows)]
use crate::editor::scintilla_constants::{SC_MARGIN_NUMBER, SCI_SETMARGINTYPEN, SCI_STYLEGETBACK};
#[cfg(windows)]
use crate::editor::scintilla_constants::{
    SCI_ADDSELECTION, SCI_GETMAINSELECTION, SCI_GETSELECTIONNANCHOR, SCI_GETSELECTIONNCARET,
    SCI_GETSELECTIONS, SCI_POSITIONFROMPOINT, SCI_SETMAINSELECTION,
};
use crate::editor::scintilla_constants::{
    SCI_COUNTCHARACTERS, SCI_DOCLINEFROMVISIBLE, SCI_GETCHARACTERPOINTER, SCI_GETCODEPAGE,
    SCI_GETCOLUMN, SCI_GETCURRENTPOS, SCI_GETFIRSTVISIBLELINE, SCI_GETLINE, SCI_GETRANGEPOINTER,
    SCI_LINEFROMPOSITION, SCI_LINELENGTH, SCI_SETFIRSTVISIBLELINE, SCI_VISIBLEFROMDOCLINE,
};
#[cfg(windows)]
use crate::editor::scintilla_constants::{SCI_GETANCHOR, SCI_GETXOFFSET, SCI_SETXOFFSET};
use crate::editor::scintilla_constants::{
    SCI_GETLINECOUNT, SCI_GETZOOM, SCI_SETZOOM, SCI_TEXTWIDTH, SCI_ZOOMIN, SCI_ZOOMOUT,
    STYLE_LINENUMBER, STYLE_MAX,
};
use crate::{FastPadError, Result};
use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::HWND;

#[cfg(windows)]
use crate::platform::{last_error, wide_null};
#[cfg(windows)]
use std::mem::transmute;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{LPARAM, RECT, WPARAM};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_SHIFT, VK_TAB,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, HWND_MESSAGE, SendMessageW, WM_CHAR,
    WM_DPICHANGED_AFTERPARENT, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCDESTROY, WS_CHILD,
    WS_CLIPSIBLINGS, WS_TABSTOP, WS_VISIBLE,
};

mod document_text;
mod editing;
mod line_ops;
mod markdown_ops;
mod styling;

pub type SciFnDirect = unsafe extern "C" fn(isize, u32, usize, isize) -> isize;

const ENDPOINT_DESTROYED: &str = "Scintilla editor endpoint is no longer alive";
#[cfg(windows)]
const EDITOR_ENDPOINT_SUBCLASS_ID: usize = 0x4650_4544;

/// Where the caret sits, as `Editor::caret_status` reports it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaretStatus {
    pub line: usize,
    pub column: usize,
    pub selected_characters: usize,
}

/// Scintilla's `SCNotification` (Scintilla.h), complete through `updated` so both `SCN_MODIFIED`
/// and `SCN_UPDATEUI` can be read from one definition.
#[repr(C)]
pub struct ScintillaNotification {
    pub header: windows_sys::Win32::UI::Controls::NMHDR,
    pub position: isize,
    pub ch: i32,
    pub modifiers: i32,
    pub modification_type: i32,
    pub text: *const u8,
    pub length: isize,
    pub lines_added: isize,
    pub message: i32,
    pub wparam: usize,
    pub lparam: isize,
    pub line: isize,
    pub fold_level_now: i32,
    pub fold_level_prev: i32,
    pub margin: i32,
    pub list_type: i32,
    pub x: i32,
    pub y: i32,
    pub token: i32,
    pub annotation_lines_added: isize,
    pub updated: i32,
    pub list_completion_method: i32,
    pub character_source: i32,
}

#[derive(Debug)]
pub struct Editor {
    endpoint: Rc<EditorEndpoint>,
    /// The Scintilla that creates this editor's documents: the shared document host (split
    /// editors spec §3.1), or the editor itself for one made by `create`.
    documents: Rc<EditorEndpoint>,
}

impl Clone for Editor {
    fn clone(&self) -> Self {
        Self {
            endpoint: Rc::clone(&self.endpoint),
            documents: Rc::clone(&self.documents),
        }
    }
}

#[derive(Debug)]
pub struct EditorDocument {
    raw: isize,
    endpoint: Rc<EditorEndpoint>,
}

#[derive(Debug)]
struct EditorEndpoint {
    hwnd: HWND,
    direct_fn: SciFnDirect,
    direct_ptr: isize,
    destroyed: AtomicBool,
    destroy_window_on_drop: bool,
    line_numbers: Cell<LineNumberMargin>,
    /// Whether Add next occurrence matches whole words: set when it starts from an empty caret
    /// (editing shortcuts spec §3).
    occurrence_whole_word: Cell<bool>,
    /// The word Add next occurrence last selected at an empty caret: while the selection is still
    /// exactly that word, matches stay whole words.
    occurrence_word: Cell<Option<(usize, usize)>>,
    /// The selections and point an Alt+Click started from (editing shortcuts spec §5).
    alt_click: RefCell<Option<AltClick>>,
    /// Where the window layer plugs into key handling (the Markdown helpers' Enter and Tab).
    hooks: RefCell<Option<Rc<dyn crate::editor::EditorHooks>>>,
    /// The `WM_CHAR` a consumed `WM_KEYDOWN` will produce, dropped when it arrives.
    swallow_char: Cell<Option<u16>>,
    #[cfg(test)]
    release_counter: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

/// An Alt+Click in progress: where the button went down, and the selections (caret, anchor) and
/// main selection from before Scintilla's own handling.
#[derive(Debug)]
struct AltClick {
    x: i32,
    y: i32,
    selections: Vec<(isize, isize)>,
    main: usize,
}

/// Whether margin 0 shows line numbers, and how many digits its current width was measured for
/// (0 means not measured yet).
#[derive(Clone, Copy, Debug, Default)]
struct LineNumberMargin {
    visible: bool,
    digits: usize,
}

/// Digits needed for the largest line number, never fewer than two so the margin does not resize
/// at line 10.
fn line_number_digits(line_count: isize) -> usize {
    (line_count.max(1).ilog10() as usize + 1).max(2)
}

impl Editor {
    #[cfg(windows)]
    pub fn create(parent: HWND) -> Result<Self> {
        let endpoint = Self::open_endpoint(create_scintilla_child(parent)?)?;
        Self::finish(Rc::clone(&endpoint), endpoint)
    }

    /// A message-only Scintilla that creates every document and never shows one (split editors
    /// spec §3.1). Its notifications go nowhere, so edits made through it notify nobody.
    #[cfg(windows)]
    pub fn create_document_host() -> Result<Self> {
        let endpoint = Self::open_endpoint(create_scintilla_host()?)?;
        Ok(Self {
            documents: Rc::clone(&endpoint),
            endpoint,
        })
    }

    #[cfg(not(windows))]
    pub fn create_document_host() -> Result<Self> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// A visible editor under `parent` that shows documents `host` created.
    #[cfg(windows)]
    pub fn create_with_host(parent: HWND, host: &Editor) -> Result<Self> {
        let endpoint = Self::open_endpoint(create_scintilla_child(parent)?)?;
        Self::finish(endpoint, Rc::clone(&host.documents))
    }

    #[cfg(not(windows))]
    pub fn create_with_host(_parent: HWND, _host: &Editor) -> Result<Self> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// This editor, now creating and showing documents from `host`'s store instead of its own.
    /// For editors made by `create` before the host existed; documents the editor already
    /// created stay valid for it only through `host`'s reference counting, so call this before
    /// creating any.
    pub fn with_document_host(mut self, host: &Editor) -> Self {
        self.documents = Rc::clone(&host.documents);
        self
    }

    /// Whether `other` shows documents from the same host, so either can show the other's.
    pub fn shares_documents_with(&self, other: &Editor) -> bool {
        Rc::ptr_eq(&self.documents, &other.documents)
    }

    #[cfg(windows)]
    fn open_endpoint(hwnd: HWND) -> Result<Rc<EditorEndpoint>> {
        require_hwnd(hwnd)?;

        let direct_fn_raw = unsafe { SendMessageW(hwnd, SCI_GETDIRECTFUNCTION, 0, 0) };
        if direct_fn_raw == 0 {
            return Err(FastPadError::Invariant(
                "Scintilla did not provide a direct function",
            ));
        }

        let direct_ptr = unsafe { SendMessageW(hwnd, SCI_GETDIRECTPOINTER, 0, 0) };
        if direct_ptr == 0 {
            return Err(FastPadError::Invariant(
                "Scintilla did not provide a direct pointer",
            ));
        }

        let endpoint = Rc::new(EditorEndpoint::new(
            hwnd,
            unsafe { transmute::<isize, SciFnDirect>(direct_fn_raw) },
            direct_ptr,
            true,
        ));
        endpoint.install_lifecycle_guard()?;
        Ok(endpoint)
    }

    #[cfg(windows)]
    fn finish(endpoint: Rc<EditorEndpoint>, documents: Rc<EditorEndpoint>) -> Result<Self> {
        let hwnd = endpoint.hwnd;
        let editor = Self {
            endpoint,
            documents,
        };
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) };
        editor.initialize_view(|editor| editor.apply_chrome_defaults(dpi))?;
        // Cosmetic like the chrome: an editor without the editing keys still edits.
        let _ = editor.configure_editing();
        Ok(editor)
    }

    #[cfg(not(windows))]
    pub fn create(_parent: HWND) -> Result<Self> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Sets the code page, which the UTF-8 contract depends on, and then applies the purely
    /// cosmetic chrome defaults. Spec 228 keeps non-fatal failures non-fatal: a failed margin or
    /// scroll-width call must never stop FastPad from opening an editable window.
    fn initialize_view(&self, apply_chrome: impl FnOnce(&Self) -> Result<()>) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETCODEPAGE, SC_CP_UTF8 as usize, 0)?;
        let _ = apply_chrome(self);
        Ok(())
    }

    pub fn hwnd(&self) -> HWND {
        self.endpoint.hwnd
    }

    pub fn set_hooks(&self, hooks: Option<Rc<dyn crate::editor::EditorHooks>>) {
        *self.endpoint.hooks.borrow_mut() = hooks;
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(direct_fn: SciFnDirect, direct_ptr: isize) -> Self {
        let endpoint = Rc::new(EditorEndpoint::new(
            std::ptr::null_mut(),
            direct_fn,
            direct_ptr,
            false,
        ));
        Self {
            documents: Rc::clone(&endpoint),
            endpoint,
        }
    }
}

impl Clone for EditorDocument {
    fn clone(&self) -> Self {
        self.endpoint.retain_document(self.raw);
        Self {
            raw: self.raw,
            endpoint: Rc::clone(&self.endpoint),
        }
    }
}

impl Drop for EditorDocument {
    fn drop(&mut self) {
        self.endpoint.release_document(self.raw);
    }
}

impl EditorDocument {
    #[cfg(test)]
    pub fn test_fixture() -> Self {
        Self {
            raw: 0,
            endpoint: Rc::new(EditorEndpoint::new(
                std::ptr::null_mut(),
                inert_direct_call,
                0,
                false,
            )),
        }
    }

    #[cfg(test)]
    pub fn test_fixture_with_release_counter(
        releases: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        Self {
            raw: 1,
            endpoint: Rc::new(EditorEndpoint {
                hwnd: std::ptr::null_mut(),
                direct_fn: inert_direct_call,
                direct_ptr: 0,
                destroyed: AtomicBool::new(false),
                destroy_window_on_drop: false,
                line_numbers: Cell::new(LineNumberMargin::default()),
                occurrence_whole_word: Cell::new(false),
                occurrence_word: Cell::new(None),
                alt_click: RefCell::new(None),
                hooks: RefCell::new(None),
                swallow_char: Cell::new(None),
                release_counter: Some(releases),
            }),
        }
    }

    #[cfg(test)]
    fn test_fixture_with_raw(raw: isize, editor: &Editor) -> Self {
        Self {
            raw,
            endpoint: Rc::clone(&editor.endpoint),
        }
    }

    #[cfg(test)]
    fn raw(&self) -> isize {
        self.raw
    }
}

impl EditorEndpoint {
    fn new(
        hwnd: HWND,
        direct_fn: SciFnDirect,
        direct_ptr: isize,
        destroy_window_on_drop: bool,
    ) -> Self {
        Self {
            hwnd,
            direct_fn,
            direct_ptr,
            destroyed: AtomicBool::new(false),
            destroy_window_on_drop,
            line_numbers: Cell::new(LineNumberMargin::default()),
            occurrence_whole_word: Cell::new(false),
            occurrence_word: Cell::new(None),
            alt_click: RefCell::new(None),
            hooks: RefCell::new(None),
            swallow_char: Cell::new(None),
            #[cfg(test)]
            release_counter: None,
        }
    }

    /// Sizes margin 0 for the current line count plus one digit of breathing room. Without `force`
    /// the font measurement is skipped while the digit count is unchanged.
    fn size_line_number_margin(&self, force: bool) -> Result<()> {
        let mut margin = self.line_numbers.get();
        if !margin.visible {
            return Ok(());
        }
        let line_count = self.send_direct_checked(SCI_GETLINECOUNT, 0, 0)?;
        let digits = line_number_digits(line_count);
        if !force && digits == margin.digits {
            return Ok(());
        }
        let sample = CString::new("9".repeat(digits + 1)).expect("digits contain no NUL bytes");
        let width = self.send_direct_checked(
            SCI_TEXTWIDTH,
            STYLE_LINENUMBER as usize,
            sample.as_ptr() as isize,
        )?;
        self.send_direct_checked(SCI_SETMARGINWIDTHN, 0, width)?;
        margin.digits = digits;
        self.line_numbers.set(margin);
        Ok(())
    }

    /// Leaves an empty selection at the caret.
    #[cfg(windows)]
    fn collapse_selection(&self) -> Result<()> {
        let caret = self.send_direct_checked(SCI_GETCURRENTPOS, 0, 0)?;
        self.send_direct_checked(SCI_SETSEL, caret.max(0) as usize, caret)?;
        Ok(())
    }

    /// Every selection as (caret, anchor), and which one is main.
    #[cfg(windows)]
    fn selection_snapshot(&self) -> Result<(Vec<(isize, isize)>, usize)> {
        let count = self.send_direct_checked(SCI_GETSELECTIONS, 0, 0)?.max(1) as usize;
        let selections = (0..count)
            .map(|n| {
                Ok((
                    self.send_direct_checked(SCI_GETSELECTIONNCARET, n, 0)?,
                    self.send_direct_checked(SCI_GETSELECTIONNANCHOR, n, 0)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let main = self.send_direct_checked(SCI_GETMAINSELECTION, 0, 0)?.max(0) as usize;
        Ok((selections, main))
    }

    /// A button press with Alt (and no Shift or Ctrl) may become Alt+Click: remember where it
    /// started from (editing shortcuts spec §5).
    #[cfg(windows)]
    fn begin_alt_click(&self, lparam: LPARAM) {
        let alt_only = unsafe { GetKeyState(VK_MENU as i32) } < 0
            && unsafe { GetKeyState(VK_SHIFT as i32) } >= 0
            && unsafe { GetKeyState(VK_CONTROL as i32) } >= 0;
        let click = alt_only
            .then(|| self.selection_snapshot().ok())
            .flatten()
            .map(|(selections, main)| {
                let (x, y) = mouse_point(lparam);
                AltClick {
                    x,
                    y,
                    selections,
                    main,
                }
            });
        *self.alt_click.borrow_mut() = click;
    }

    /// After Scintilla's own button-up: a release near the press restores the earlier
    /// selections and adds a caret at the click, as the main selection. A drag stays
    /// Scintilla's rectangular selection.
    #[cfg(windows)]
    fn finish_alt_click(&self, click: AltClick, lparam: LPARAM) -> Result<()> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDRAG, SM_CYDRAG};
        let (x, y) = mouse_point(lparam);
        let moved = (x - click.x).abs() > unsafe { GetSystemMetrics(SM_CXDRAG) }
            || (y - click.y).abs() > unsafe { GetSystemMetrics(SM_CYDRAG) };
        if moved {
            return Ok(());
        }
        // The Alt press put Scintilla in rectangular mode; SCI_SETSEL clears it back to stream,
        // or typing would rebuild the carets as a column. Its arguments are anchor, caret.
        for (n, (caret, anchor)) in click.selections.iter().enumerate() {
            if n == 0 {
                self.send_direct_checked(SCI_SETSEL, *anchor as usize, *caret)?;
            } else {
                self.send_direct_checked(SCI_ADDSELECTION, *caret as usize, *anchor)?;
            }
        }
        self.send_direct_checked(SCI_SETMAINSELECTION, click.main, 0)?;
        let position = self.send_direct_checked(SCI_POSITIONFROMPOINT, x as usize, y as isize)?;
        self.send_direct_checked(SCI_ADDSELECTION, position.max(0) as usize, position)?;
        Ok(())
    }

    #[cfg(windows)]
    fn install_lifecycle_guard(self: &Rc<Self>) -> Result<()> {
        let installed = unsafe {
            SetWindowSubclass(
                self.hwnd,
                Some(editor_endpoint_subclass_proc),
                EDITOR_ENDPOINT_SUBCLASS_ID,
                Rc::as_ptr(self) as usize,
            )
        };
        if installed == 0 {
            return Err(last_error());
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn install_lifecycle_guard(self: &Rc<Self>) -> Result<()> {
        let _ = self;
        Ok(())
    }

    fn send_direct_checked(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.send_direct_if_alive(message, wparam, lparam)
            .ok_or(FastPadError::Invariant(ENDPOINT_DESTROYED))
    }

    fn send_direct_if_alive(&self, message: u32, wparam: usize, lparam: isize) -> Option<isize> {
        if self.destroyed.load(Ordering::Acquire) {
            return None;
        }
        Some(unsafe { (self.direct_fn)(self.direct_ptr, message, wparam, lparam) })
    }

    fn retain_document(&self, raw: isize) {
        if raw == 0 {
            return;
        }
        let _ = self.send_direct_if_alive(SCI_ADDREFDOCUMENT, 0, raw);
    }

    fn release_document(&self, raw: isize) {
        if raw == 0 {
            return;
        }
        let _release_result = self.send_direct_if_alive(SCI_RELEASEDOCUMENT, 0, raw);
        #[cfg(test)]
        if _release_result.is_some() {
            if let Some(counter) = &self.release_counter {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            release_observation::record(self.hwnd, raw);
        }
    }
}

#[cfg(test)]
pub(crate) mod release_observation {
    use std::cell::RefCell;
    use windows_sys::Win32::Foundation::HWND;

    #[derive(Debug)]
    pub(crate) struct Release {
        pub(crate) hwnd: HWND,
        pub(crate) document: isize,
        pub(crate) window_was_live: bool,
    }

    thread_local! {
        static RELEASES: RefCell<Option<Vec<Release>>> = const { RefCell::new(None) };
    }

    pub(super) fn record(hwnd: HWND, document: isize) {
        RELEASES.with(|releases| {
            if let Some(releases) = releases.borrow_mut().as_mut() {
                releases.push(Release {
                    hwnd,
                    document,
                    window_was_live: unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd) != 0
                    },
                });
            }
        });
    }

    pub(crate) fn during<R>(run: impl FnOnce() -> R) -> (R, Vec<Release>) {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                RELEASES.with(|releases| {
                    releases.replace(None);
                });
            }
        }
        RELEASES.with(|releases| {
            assert!(releases.replace(Some(Vec::new())).is_none());
        });
        let _reset = Reset;
        let result = run();
        let releases = RELEASES.with(|releases| releases.take().unwrap());
        (result, releases)
    }
}

impl Drop for EditorEndpoint {
    fn drop(&mut self) {
        if !self.destroy_window_on_drop {
            return;
        }
        if self.destroyed.swap(true, Ordering::AcqRel) {
            return;
        }

        #[cfg(windows)]
        unsafe {
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
        }
    }
}

#[cfg(windows)]
fn require_hwnd(raw: HWND) -> Result<HWND> {
    if raw.is_null() {
        Err(last_error())
    } else {
        Ok(raw)
    }
}

#[cfg(windows)]
fn create_scintilla_child(parent: HWND) -> Result<HWND> {
    let rect = parent_client_rect(parent)?;
    let class_name = wide_null("Scintilla");
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);
    let hwnd = unsafe {
        // SAFETY: `parent` is treated as an opaque host HWND supplied by the caller. This helper
        // contains the only raw-HWND FFI for `Editor::create`, constraining the unchecked Win32
        // boundary to one private function.
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            // Clipped against siblings so overlays such as the command palette stay on top.
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS,
            0,
            0,
            width,
            height,
            parent,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    Ok(hwnd)
}

/// The document host's window: message-only, so it has no parent to outlive and is never shown.
#[cfg(windows)]
fn create_scintilla_host() -> Result<HWND> {
    let class_name = wide_null("Scintilla");
    let hwnd = unsafe {
        // SAFETY: HWND_MESSAGE is the documented parent for a message-only window.
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            1,
            1,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    Ok(hwnd)
}

#[cfg(windows)]
fn parent_client_rect(parent: HWND) -> Result<RECT> {
    let mut rect = RECT::default();
    let ok = unsafe {
        // SAFETY: `parent` is forwarded unchanged to Win32 so the FFI dereference stays inside
        // this private boundary instead of the public `Editor::create` API.
        GetClientRect(parent, &mut rect)
    };
    if ok == 0 { Err(last_error()) } else { Ok(rect) }
}

/// A mouse message's client point: signed 16-bit x and y.
#[cfg(windows)]
fn mouse_point(lparam: LPARAM) -> (i32, i32) {
    let x = i32::from((lparam & 0xFFFF) as u16 as i16);
    let y = i32::from(((lparam >> 16) & 0xFFFF) as u16 as i16);
    (x, y)
}

#[cfg(windows)]
unsafe extern "system" fn editor_endpoint_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> isize {
    let endpoint = unsafe { &*(ref_data as *const EditorEndpoint) };
    if message == WM_CHAR
        && let Some(expected) = endpoint.swallow_char.take()
        && expected == wparam as u16
    {
        return 0;
    }
    if message == WM_KEYDOWN {
        // A consumed key whose WM_CHAR never came must not eat a later key's character.
        endpoint.swallow_char.set(None);
    }
    // Cloned out first, so no borrow is held while the hook calls back into the editor.
    let hooks = if message == WM_KEYDOWN {
        endpoint.hooks.borrow().clone()
    } else {
        None
    };
    if let Some(hooks) = hooks {
        let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
        let vk = wparam as u16;
        if hooks.key_down(vk, down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU)) {
            // Enter and Tab produce a CR / TAB WM_CHAR after TranslateMessage.
            let produced = match vk {
                VK_RETURN => Some(0x0D),
                VK_TAB => Some(0x09),
                _ => None,
            };
            endpoint.swallow_char.set(produced);
            return 0;
        }
    }
    if message == WM_CHAR {
        let ctrl_down = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        if crate::editor::input_filter::should_ignore_char(wparam as u16, ctrl_down) {
            return 0;
        }
    }
    if message == WM_KEYDOWN && wparam == usize::from(VK_ESCAPE) {
        // Scintilla's Cancel drops extra carets; with one selection it keeps it, where VS Code
        // clears it (editing shortcuts spec §5).
        let single = endpoint
            .send_direct_checked(SCI_GETSELECTIONS, 0, 0)
            .is_ok_and(|count| count <= 1);
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if single {
            let _ = endpoint.collapse_selection();
        }
        return result;
    }
    if message == WM_LBUTTONDOWN {
        endpoint.begin_alt_click(lparam);
    }
    if message == WM_LBUTTONUP {
        let click = endpoint.alt_click.borrow_mut().take();
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        if let Some(click) = click {
            let _ = endpoint.finish_alt_click(click, lparam);
        }
        return result;
    }
    if message == WM_DPICHANGED_AFTERPARENT {
        // Scintilla adopts the new DPI inside its own handler; measure digits only after that.
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        let _ = endpoint.size_line_number_margin(true);
        return result;
    }
    if message == WM_NCDESTROY {
        endpoint.destroyed.store(true, Ordering::Release);
        // Hooks may own the editor; drop them so an Rc cycle cannot outlive the window.
        *endpoint.hooks.borrow_mut() = None;
        unsafe {
            RemoveWindowSubclass(
                hwnd,
                Some(editor_endpoint_subclass_proc),
                EDITOR_ENDPOINT_SUBCLASS_ID,
            );
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
unsafe extern "C" fn inert_direct_call(
    _direct_ptr: isize,
    _message: u32,
    _wparam: usize,
    _lparam: isize,
) -> isize {
    0
}

#[cfg(test)]
mod tests;
