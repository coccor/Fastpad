use crate::editor::ViewState;
use crate::editor::scintilla_constants::{
    SC_CP_UTF8, SCI_ADDREFDOCUMENT, SCI_BEGINUNDOACTION, SCI_CANREDO, SCI_CANUNDO, SCI_COPY,
    SCI_CREATEDOCUMENT, SCI_CUT, SCI_EMPTYUNDOBUFFER, SCI_ENDUNDOACTION, SCI_GETDIRECTFUNCTION,
    SCI_GETDIRECTPOINTER, SCI_GETDOCPOINTER, SCI_GETLENGTH, SCI_GETSELECTIONEND,
    SCI_GETSELECTIONSTART, SCI_GETSELTEXT, SCI_GETTARGETEND, SCI_GETTEXT, SCI_GETTEXTLENGTH,
    SCI_PASTE, SCI_REDO, SCI_RELEASEDOCUMENT, SCI_REPLACETARGET, SCI_SCROLLCARET,
    SCI_SEARCHINTARGET, SCI_SETCODEPAGE, SCI_SETDOCPOINTER, SCI_SETILEXER, SCI_SETKEYWORDS,
    SCI_SETPROPERTY, SCI_SETSAVEPOINT, SCI_SETSEARCHFLAGS, SCI_SETSEL, SCI_SETTARGETRANGE,
    SCI_SETTEXT, SCI_SETUNDOCOLLECTION, SCI_STYLECLEARALL, SCI_STYLESETBACK, SCI_STYLESETBOLD,
    SCI_STYLESETFONT, SCI_STYLESETFORE, SCI_STYLESETITALIC, SCI_UNDO,
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
use std::cell::Cell;
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
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, HWND_MESSAGE, SendMessageW, WM_CHAR,
    WM_DPICHANGED_AFTERPARENT, WM_NCDESTROY, WS_CHILD, WS_CLIPSIBLINGS, WS_TABSTOP, WS_VISIBLE,
};

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
    #[cfg(test)]
    release_counter: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
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

    #[cfg(windows)]
    pub fn set_text(&self, text: &str) -> Result<()> {
        let text = CString::new(text)
            .map_err(|_| FastPadError::Invariant("Scintilla text may not contain NUL bytes"))?;
        self.endpoint
            .send_direct_checked(SCI_SETTEXT, 0, text.as_ptr() as isize)?;
        // File loads suppress the window's edit notifications, so size the gutter here.
        let _ = self.endpoint.size_line_number_margin(false);
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_text(&self, _text: &str) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn text(&self) -> Result<String> {
        let length = self.endpoint.send_direct_checked(SCI_GETTEXTLENGTH, 0, 0)?;
        if length < 0 {
            return Err(FastPadError::Invariant(
                "Scintilla returned a negative text length",
            ));
        }

        let mut bytes = vec![0_u8; length as usize + 1];
        self.endpoint
            .send_direct_checked(SCI_GETTEXT, bytes.len(), bytes.as_mut_ptr() as isize)?;
        bytes.truncate(length as usize);
        String::from_utf8(bytes)
            .map_err(|_| FastPadError::Invariant("Scintilla returned invalid UTF-8"))
    }

    #[cfg(not(windows))]
    pub fn text(&self) -> Result<String> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn create_document(&self) -> Result<EditorDocument> {
        let raw = self
            .documents
            .send_direct_checked(SCI_CREATEDOCUMENT, 0, 0)?;
        if raw == 0 {
            return Err(FastPadError::Invariant(
                "Scintilla did not create a document",
            ));
        }
        Ok(EditorDocument {
            raw,
            endpoint: Rc::clone(&self.documents),
        })
    }

    #[cfg(not(windows))]
    pub fn create_document(&self) -> Result<EditorDocument> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn current_document(&self) -> Result<EditorDocument> {
        let raw = self.endpoint.send_direct_checked(SCI_GETDOCPOINTER, 0, 0)?;
        if raw == 0 {
            return Err(FastPadError::Invariant(
                "Scintilla did not return the current document",
            ));
        }
        self.documents.retain_document(raw);
        Ok(EditorDocument {
            raw,
            endpoint: Rc::clone(&self.documents),
        })
    }

    #[cfg(not(windows))]
    pub fn current_document(&self) -> Result<EditorDocument> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn use_document(&self, document: &EditorDocument) -> Result<()> {
        if !Rc::ptr_eq(&self.documents, &document.endpoint) {
            return Err(FastPadError::Invariant(
                "Scintilla document belongs to a different editor",
            ));
        }
        self.endpoint
            .send_direct_checked(SCI_SETDOCPOINTER, 0, document.raw)?;
        // Scroll width is per view and tracking only grows it; restart from the new document.
        self.endpoint
            .send_direct_checked(SCI_SETSCROLLWIDTH, 1, 0)?;
        let _ = self.endpoint.size_line_number_margin(false);
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn use_document(&self, _document: &EditorDocument) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn length(&self) -> Result<usize> {
        let length = self.endpoint.send_direct_checked(SCI_GETLENGTH, 0, 0)?;
        if length < 0 {
            return Err(FastPadError::Invariant(
                "Scintilla returned a negative document length",
            ));
        }
        Ok(length as usize)
    }

    #[cfg(not(windows))]
    pub fn length(&self) -> Result<usize> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Borrows `range` straight out of Scintilla's buffer. The slice stays valid only until the
    /// document is next modified, so callers must finish with it inside the current message.
    #[cfg(windows)]
    pub fn range_bytes(&self, range: Range<usize>) -> Result<&[u8]> {
        let length = range.end.saturating_sub(range.start);
        if length == 0 {
            return Ok(&[]);
        }
        let pointer =
            self.endpoint
                .send_direct_checked(SCI_GETRANGEPOINTER, range.start, length as isize)?;
        if pointer == 0 {
            return Err(FastPadError::Invariant(
                "Scintilla returned no range pointer",
            ));
        }
        Ok(unsafe { std::slice::from_raw_parts(pointer as *const u8, length) })
    }

    #[cfg(not(windows))]
    pub fn range_bytes(&self, _range: Range<usize>) -> Result<&[u8]> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    /// Runs `f` on the whole document's text, borrowed straight out of Scintilla's buffer
    /// (`SCI_GETCHARACTERPOINTER`, which closes the gap once and then costs nothing until the
    /// next edit). Byte offsets into it are Scintilla positions. `f` sees the text only for the
    /// call, so no edit can move the buffer under it. A document that isn't in the UTF-8 code
    /// page, or whose bytes aren't valid UTF-8, is an error: every FastPad document is UTF-8
    /// (`initialize_view`), so nothing is converted.
    #[cfg(windows)]
    pub fn with_document_text<R>(&self, f: impl FnOnce(&str) -> R) -> Result<R> {
        let code_page = self.endpoint.send_direct_checked(SCI_GETCODEPAGE, 0, 0)?;
        if code_page != SC_CP_UTF8 as isize {
            return Err(FastPadError::Invariant("The document is not UTF-8"));
        }
        let length = self.length()?;
        if length == 0 {
            return Ok(f(""));
        }
        let pointer = self
            .endpoint
            .send_direct_checked(SCI_GETCHARACTERPOINTER, 0, 0)?;
        if pointer == 0 {
            return Err(FastPadError::Invariant(
                "Scintilla returned no character pointer",
            ));
        }
        let bytes = unsafe { std::slice::from_raw_parts(pointer as *const u8, length) };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| FastPadError::Invariant("Scintilla returned invalid UTF-8"))?;
        Ok(f(text))
    }

    #[cfg(not(windows))]
    pub fn with_document_text<R>(&self, _f: impl FnOnce(&str) -> R) -> Result<R> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    #[cfg(windows)]
    pub fn line_from_position(&self, position: usize) -> Result<usize> {
        Ok(self
            .endpoint
            .send_direct_checked(SCI_LINEFROMPOSITION, position, 0)?
            .max(0) as usize)
    }

    #[cfg(not(windows))]
    pub fn line_from_position(&self, _position: usize) -> Result<usize> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    /// Where `line` starts.
    #[cfg(windows)]
    pub fn line_count(&self) -> Result<usize> {
        Ok(self
            .endpoint
            .send_direct_checked(SCI_GETLINECOUNT, 0, 0)?
            .max(0) as usize)
    }

    #[cfg(not(windows))]
    pub fn line_count(&self) -> Result<usize> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    /// The text of `line` without its CR/LF, or empty past the last line.
    #[cfg(windows)]
    pub fn line_text(&self, line: usize) -> Result<String> {
        if line >= self.line_count()? {
            return Ok(String::new());
        }
        let length = self
            .endpoint
            .send_direct_checked(SCI_LINELENGTH, line, 0)?
            .max(0) as usize;
        let mut buffer = vec![0_u8; length + 1];
        self.endpoint
            .send_direct_checked(SCI_GETLINE, line, buffer.as_mut_ptr() as isize)?;
        buffer.truncate(length);
        let text = String::from_utf8_lossy(&buffer);
        Ok(text.trim_end_matches(['\r', '\n']).to_owned())
    }

    #[cfg(not(windows))]
    pub fn line_text(&self, _line: usize) -> Result<String> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    #[cfg(windows)]
    pub fn first_visible_line(&self) -> Result<usize> {
        Ok(self
            .endpoint
            .send_direct_checked(SCI_GETFIRSTVISIBLELINE, 0, 0)?
            .max(0) as usize)
    }

    #[cfg(not(windows))]
    pub fn first_visible_line(&self) -> Result<usize> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    #[cfg(windows)]
    pub fn set_first_visible_line(&self, display_line: usize) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETFIRSTVISIBLELINE, display_line, 0)
            .map(|_| ())
    }

    #[cfg(not(windows))]
    pub fn set_first_visible_line(&self, _display_line: usize) -> Result<()> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    /// This view's selection, first visible line and horizontal scroll, to restore when the tab
    /// is shown again.
    #[cfg(windows)]
    pub fn view_state(&self) -> Result<ViewState> {
        Ok(ViewState {
            caret: self
                .endpoint
                .send_direct_checked(SCI_GETCURRENTPOS, 0, 0)?
                .max(0) as usize,
            anchor: self
                .endpoint
                .send_direct_checked(SCI_GETANCHOR, 0, 0)?
                .max(0) as usize,
            first_line: self.first_visible_line()?,
            x_offset: self.endpoint.send_direct_checked(SCI_GETXOFFSET, 0, 0)? as i32,
        })
    }

    #[cfg(not(windows))]
    pub fn view_state(&self) -> Result<ViewState> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    /// Restores `state`, clamping the selection to the document, which may have shrunk since.
    #[cfg(windows)]
    pub fn apply_view_state(&self, state: ViewState) -> Result<()> {
        let length = self
            .endpoint
            .send_direct_checked(SCI_GETLENGTH, 0, 0)?
            .max(0) as usize;
        self.set_selection(state.anchor.min(length)..state.caret.min(length))?;
        self.set_first_visible_line(state.first_line)?;
        self.endpoint
            .send_direct_checked(SCI_SETXOFFSET, state.x_offset.max(0) as usize, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn apply_view_state(&self, _state: ViewState) -> Result<()> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    #[cfg(windows)]
    pub fn doc_line_from_visible(&self, display_line: usize) -> Result<usize> {
        Ok(self
            .endpoint
            .send_direct_checked(SCI_DOCLINEFROMVISIBLE, display_line, 0)?
            .max(0) as usize)
    }

    #[cfg(not(windows))]
    pub fn doc_line_from_visible(&self, _display_line: usize) -> Result<usize> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    #[cfg(windows)]
    pub fn visible_from_doc_line(&self, doc_line: usize) -> Result<usize> {
        Ok(self
            .endpoint
            .send_direct_checked(SCI_VISIBLEFROMDOCLINE, doc_line, 0)?
            .max(0) as usize)
    }

    #[cfg(not(windows))]
    pub fn visible_from_doc_line(&self, _doc_line: usize) -> Result<usize> {
        Err(FastPadError::Invariant("Scintilla unavailable"))
    }

    /// Scrolls so the caret (the end of the current selection) is visible, without changing it.
    pub fn scroll_caret_into_view(&self) {
        let _ = self.endpoint.send_direct_if_alive(SCI_SCROLLCARET, 0, 0);
    }

    /// Moves the caret to the start of 0-based `line`, removing any selection, and scrolls it
    /// into view. A line past the end goes to the last line (Scintilla clamps it).
    #[cfg(windows)]
    pub fn go_to_line(&self, line: usize) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_GOTOLINE, line, 0)?;
        self.scroll_caret_into_view();
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn go_to_line(&self, _line: usize) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Searches `range` for `needle` with `search_flags`. A backward search passes a range whose
    /// start is after its end. The match's end is Scintilla's own target end, so a regular
    /// expression's match is as long as the text it matched, not as long as the pattern.
    /// Scintilla reports a pattern it can't compile as a miss (-1, or -2 in some versions), and
    /// so does this: `Ok(None)`, never an error.
    #[cfg(windows)]
    pub fn search_in_target(
        &self,
        needle: &str,
        range: Range<usize>,
        search_flags: u32,
    ) -> Result<Option<Range<usize>>> {
        let needle = CString::new(needle).map_err(|_| {
            FastPadError::Invariant("Scintilla search text may not contain NUL bytes")
        })?;
        self.endpoint
            .send_direct_checked(SCI_SETTARGETRANGE, range.start, range.end as isize)?;
        self.endpoint
            .send_direct_checked(SCI_SETSEARCHFLAGS, search_flags as usize, 0)?;
        let found = self.endpoint.send_direct_checked(
            SCI_SEARCHINTARGET,
            needle.as_bytes().len(),
            needle.as_ptr() as isize,
        )?;
        if found < 0 {
            return Ok(None);
        }
        let end = self
            .endpoint
            .send_direct_checked(SCI_GETTARGETEND, 0, 0)?
            .max(found);
        Ok(Some(found as usize..end as usize))
    }

    #[cfg(not(windows))]
    pub fn search_in_target(
        &self,
        _needle: &str,
        _range: Range<usize>,
        _search_flags: u32,
    ) -> Result<Option<Range<usize>>> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn replace_target(&self, range: Range<usize>, replacement: &str) -> Result<Range<usize>> {
        self.endpoint
            .send_direct_checked(SCI_SETTARGETRANGE, range.start, range.end as isize)?;
        let bytes = replacement.as_bytes();
        self.endpoint.send_direct_checked(
            SCI_REPLACETARGET,
            bytes.len(),
            bytes.as_ptr() as isize,
        )?;
        Ok(range.start..range.start + bytes.len())
    }

    #[cfg(not(windows))]
    pub fn replace_target(&self, _range: Range<usize>, _replacement: &str) -> Result<Range<usize>> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Replaces every occurrence of `query` with `replacement`, as one undo action. Searches
    /// incrementally via `search_in_target`/`replace_target`; never retrieves the full document.
    /// For plain search flags (the find bar's regex mode uses `replace_ranges_with`); an empty
    /// match ends it rather than being replaced forever.
    #[cfg(windows)]
    pub fn replace_all(&self, query: &str, replacement: &str, search_flags: u32) -> Result<usize> {
        if query.is_empty() {
            return Ok(0);
        }
        self.begin_undo_action();
        let result = (|| {
            let mut count = 0usize;
            let mut position = 0usize;
            loop {
                let length = self.length()?;
                if position > length {
                    break;
                }
                let Some(found) = self.search_in_target(query, position..length, search_flags)?
                else {
                    break;
                };
                if found.is_empty() {
                    break;
                }
                let replaced = self.replace_target(found, replacement)?;
                count += 1;
                position = replaced.end;
            }
            Ok(count)
        })();
        self.end_undo_action();
        result
    }

    #[cfg(not(windows))]
    pub fn replace_all(
        &self,
        _query: &str,
        _replacement: &str,
        _search_flags: u32,
    ) -> Result<usize> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Replaces each edit's range with its text, as one undo action, and returns how many. The
    /// ranges come in ascending order, none overlapping, as `Matcher::replacements` gives them;
    /// they are applied from the last backwards so the earlier positions stay valid. An empty
    /// list returns `Ok(0)` without opening an undo action.
    #[cfg(windows)]
    pub fn replace_ranges_with(&self, edits: &[(Range<usize>, String)]) -> Result<usize> {
        if edits.is_empty() {
            return Ok(0);
        }
        self.begin_undo_action();
        let result = edits
            .iter()
            .rev()
            .try_for_each(|(range, text)| self.replace_target(range.clone(), text).map(drop));
        self.end_undo_action();
        result.map(|()| edits.len())
    }

    #[cfg(not(windows))]
    pub fn replace_ranges_with(&self, _edits: &[(Range<usize>, String)]) -> Result<usize> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn undo(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_UNDO, 0, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn undo(&self) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn redo(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_REDO, 0, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn redo(&self) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn can_undo(&self) -> Result<bool> {
        Ok(self.endpoint.send_direct_checked(SCI_CANUNDO, 0, 0)? != 0)
    }

    #[cfg(not(windows))]
    pub fn can_undo(&self) -> Result<bool> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn can_redo(&self) -> Result<bool> {
        Ok(self.endpoint.send_direct_checked(SCI_CANREDO, 0, 0)? != 0)
    }

    #[cfg(not(windows))]
    pub fn can_redo(&self) -> Result<bool> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn cut(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_CUT, 0, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn cut(&self) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn copy(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_COPY, 0, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn copy(&self) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn paste(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_PASTE, 0, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn paste(&self) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn selection(&self) -> Result<Range<usize>> {
        let start = self
            .endpoint
            .send_direct_checked(SCI_GETSELECTIONSTART, 0, 0)?;
        let end = self
            .endpoint
            .send_direct_checked(SCI_GETSELECTIONEND, 0, 0)?;
        if start < 0 || end < start {
            return Err(FastPadError::Invariant(
                "Scintilla returned an invalid selection range",
            ));
        }
        Ok(start as usize..end as usize)
    }

    #[cfg(not(windows))]
    pub fn selection(&self) -> Result<Range<usize>> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    #[cfg(windows)]
    pub fn set_selection(&self, range: Range<usize>) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETSEL, range.start, range.end as isize)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_selection(&self, _range: Range<usize>) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// The text of the current selection, retrieved directly via `SCI_GETSELTEXT` rather than by
    /// slicing a full-document read.
    #[cfg(windows)]
    pub fn selected_text(&self) -> Result<String> {
        let length = self.endpoint.send_direct_checked(SCI_GETSELTEXT, 0, 0)?;
        if length < 0 {
            return Err(FastPadError::Invariant(
                "Scintilla returned a negative selection length",
            ));
        }
        let mut bytes = vec![0_u8; length as usize + 1];
        self.endpoint
            .send_direct_checked(SCI_GETSELTEXT, 0, bytes.as_mut_ptr() as isize)?;
        bytes.truncate(length as usize);
        String::from_utf8(bytes)
            .map_err(|_| FastPadError::Invariant("Scintilla returned invalid UTF-8"))
    }

    #[cfg(not(windows))]
    pub fn selected_text(&self) -> Result<String> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Installs `lexer` (an opaque `ILexer5*` from Lexilla's `CreateLexer`, or `0` for Scintilla's
    /// built-in null lexer) via `SCI_SETILEXER`. Scintilla takes ownership of a non-null pointer
    /// and releases it itself when replaced or the document is destroyed; this method never calls
    /// `Release()` and never interprets the pointer beyond forwarding it.
    #[cfg(windows)]
    pub fn set_lexer(&self, lexer: isize) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_SETILEXER, 0, lexer)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_lexer(&self, _lexer: isize) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Resets every style number to Scintilla's current default look via `SCI_STYLECLEARALL`, so a
    /// previous language's style overrides cannot bleed into the next one before `set_style`
    /// reapplies the styles relevant to the newly installed lexer.
    #[cfg(windows)]
    pub fn clear_all_styles(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_STYLECLEARALL, 0, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn clear_all_styles(&self) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Applies user view settings to every style number (lexer styles run past the predefined
    /// ones) without touching text, selection, or colors, then re-sizes the line-number margin.
    #[cfg(windows)]
    pub fn apply_view_settings(
        &self,
        face: &str,
        size_points: u16,
        tab_width: u8,
        word_wrap: bool,
    ) -> Result<()> {
        let face = CString::new(face).map_err(|_| {
            FastPadError::Invariant("Scintilla font face may not contain NUL bytes")
        })?;
        for style in 0..=STYLE_MAX as usize {
            self.endpoint
                .send_direct_checked(SCI_STYLESETFONT, style, face.as_ptr() as isize)?;
            self.endpoint.send_direct_checked(
                SCI_STYLESETSIZEFRACTIONAL,
                style,
                size_points as isize * 100,
            )?;
        }
        self.endpoint
            .send_direct_checked(SCI_SETTABWIDTH, usize::from(tab_width), 0)?;
        let wrap = if word_wrap {
            SC_WRAP_WORD
        } else {
            SC_WRAP_NONE
        };
        self.endpoint
            .send_direct_checked(SCI_SETWRAPMODE, wrap as usize, 0)?;
        self.endpoint.size_line_number_margin(true)
    }

    #[cfg(not(windows))]
    pub fn apply_view_settings(
        &self,
        _face: &str,
        _size_points: u16,
        _tab_width: u8,
        _word_wrap: bool,
    ) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Applies the whitespace settings: whether Tab inserts spaces, and whether spaces and tabs
    /// are drawn.
    #[cfg(windows)]
    pub fn apply_whitespace_settings(
        &self,
        insert_spaces: bool,
        show_whitespace: bool,
    ) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETUSETABS, usize::from(!insert_spaces), 0)?;
        let view = if show_whitespace {
            SCWS_VISIBLEALWAYS
        } else {
            SCWS_INVISIBLE
        };
        self.endpoint
            .send_direct_checked(SCI_SETVIEWWS, view as usize, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn apply_whitespace_settings(
        &self,
        _insert_spaces: bool,
        _show_whitespace: bool,
    ) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Sets plain-text foreground/background on every style up to `STYLE_DEFAULT`, plus the caret.
    #[cfg(windows)]
    pub fn set_base_colors(&self, foreground: u32, background: u32) -> Result<()> {
        for style in 0..=STYLE_DEFAULT as usize {
            self.endpoint
                .send_direct_checked(SCI_STYLESETFORE, style, foreground as isize)?;
            self.endpoint
                .send_direct_checked(SCI_STYLESETBACK, style, background as isize)?;
        }
        self.endpoint
            .send_direct_checked(SCI_SETCARETFORE, foreground as usize, 0)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_base_colors(&self, _foreground: u32, _background: u32) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Keeps margin 0 as the only (line-number) margin, on the text background rather than
    /// Scintilla's grey band, pads the text area, lets the horizontal scrollbar follow the widest
    /// line instead of the default 2000 px scroll width, and shows line numbers until settings load.
    #[cfg(windows)]
    pub fn apply_chrome_defaults(&self, dpi: u32) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETMARGINTYPEN, 0, SC_MARGIN_NUMBER as isize)?;
        for margin in 1..=2 {
            self.endpoint
                .send_direct_checked(SCI_SETMARGINWIDTHN, margin, 0)?;
        }
        let background =
            self.endpoint
                .send_direct_checked(SCI_STYLEGETBACK, STYLE_DEFAULT as usize, 0)?;
        self.endpoint.send_direct_checked(
            SCI_STYLESETBACK,
            STYLE_LINENUMBER as usize,
            background,
        )?;
        self.set_text_padding(dpi)?;
        self.endpoint
            .send_direct_checked(SCI_SETSCROLLWIDTH, 1, 0)?;
        self.endpoint
            .send_direct_checked(SCI_SETSCROLLWIDTHTRACKING, 1, 0)?;
        self.set_line_numbers(true)
    }

    #[cfg(not(windows))]
    pub fn apply_chrome_defaults(&self, _dpi: u32) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Shows margin 0 sized for the current line count, or collapses it to zero width. Repeating
    /// the current, already-applied state sends nothing.
    pub fn set_line_numbers(&self, visible: bool) -> Result<()> {
        let current = self.endpoint.line_numbers.get();
        if current.visible == visible && (!visible || current.digits != 0) {
            return Ok(());
        }
        self.endpoint
            .line_numbers
            .set(LineNumberMargin { visible, digits: 0 });
        if visible {
            self.endpoint.size_line_number_margin(true)
        } else {
            self.endpoint
                .send_direct_checked(SCI_SETMARGINWIDTHN, 0, 0)?;
            Ok(())
        }
    }

    /// Re-sizes visible line numbers only if the line count gained or lost a digit, which keeps it
    /// cheap enough to run after every line-changing edit.
    pub fn refresh_line_numbers(&self) -> Result<()> {
        self.endpoint.size_line_number_margin(false)
    }

    /// Re-measures visible line numbers unconditionally, for font or DPI changes.
    pub fn remeasure_line_numbers(&self) -> Result<()> {
        self.endpoint.size_line_number_margin(true)
    }

    /// Colors the line-number margin. `SCI_STYLECLEARALL` resets it along with every other style,
    /// so callers reapply this after a lexer change.
    pub fn set_line_number_colors(&self, foreground: u32, background: u32) -> Result<()> {
        self.endpoint.send_direct_checked(
            SCI_STYLESETFORE,
            STYLE_LINENUMBER as usize,
            foreground as isize,
        )?;
        self.endpoint.send_direct_checked(
            SCI_STYLESETBACK,
            STYLE_LINENUMBER as usize,
            background as isize,
        )?;
        Ok(())
    }

    /// Left/right text padding in physical pixels for `dpi` (8 px at 96 DPI).
    #[cfg(windows)]
    pub fn set_text_padding(&self, dpi: u32) -> Result<()> {
        let padding = ((8 * i64::from(dpi.max(96)) + 48) / 96) as isize;
        self.endpoint
            .send_direct_checked(SCI_SETMARGINLEFT, 0, padding)?;
        self.endpoint
            .send_direct_checked(SCI_SETMARGINRIGHT, 0, padding)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_text_padding(&self, _dpi: u32) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Magnifies every style by one point. The view notifies `SCN_ZOOM`, which is where the owner
    /// re-measures the line-number margin, so Ctrl+wheel zooming keeps the gutter sized too.
    pub fn zoom_in(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_ZOOMIN, 0, 0)?;
        Ok(())
    }

    pub fn zoom_out(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_ZOOMOUT, 0, 0)?;
        Ok(())
    }

    /// The points added to every style's size by zooming.
    pub fn zoom(&self) -> Result<i32> {
        Ok(self.endpoint.send_direct_checked(SCI_GETZOOM, 0, 0)? as i32)
    }

    /// Matches another editor's zoom (split editors plan amendment 11: one zoom for every group).
    pub fn set_zoom(&self, zoom: i32) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETZOOM, zoom as usize, 0)?;
        Ok(())
    }

    /// Returns to the configured font size.
    pub fn reset_zoom(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_SETZOOM, 0, 0)?;
        Ok(())
    }

    /// The caret's 1-based line and column (tabs expanded, as the view shows them) and how many
    /// characters the main selection spans, for the status bar.
    pub fn caret_status(&self) -> Result<CaretStatus> {
        let caret = self.endpoint.send_direct_checked(SCI_GETCURRENTPOS, 0, 0)?;
        let line = self
            .endpoint
            .send_direct_checked(SCI_LINEFROMPOSITION, caret as usize, 0)?;
        let column = self
            .endpoint
            .send_direct_checked(SCI_GETCOLUMN, caret as usize, 0)?;
        let selection = self.selection()?;
        let selected = self.endpoint.send_direct_checked(
            SCI_COUNTCHARACTERS,
            selection.start,
            selection.end as isize,
        )?;
        Ok(CaretStatus {
            line: line.max(0) as usize + 1,
            column: column.max(0) as usize + 1,
            selected_characters: selected.max(0) as usize,
        })
    }

    /// Sets the selection (focused and unfocused) and caret-line backgrounds as opaque Scintilla 5
    /// element colours. A `None` caret line resets that element, which turns the highlight off.
    #[cfg(windows)]
    pub fn set_chrome_colors(
        &self,
        selection: u32,
        inactive_selection: u32,
        caret_line: Option<u32>,
    ) -> Result<()> {
        for (element, colour) in [
            (SC_ELEMENT_SELECTION_BACK, Some(selection)),
            (SC_ELEMENT_SELECTION_INACTIVE_BACK, Some(inactive_selection)),
            (SC_ELEMENT_CARET_LINE_BACK, caret_line),
        ] {
            match colour {
                Some(colour) => self.endpoint.send_direct_checked(
                    SCI_SETELEMENTCOLOUR,
                    element as usize,
                    ((colour & 0x00FF_FFFF) | 0xFF00_0000) as isize,
                )?,
                None => self.endpoint.send_direct_checked(
                    SCI_RESETELEMENTCOLOUR,
                    element as usize,
                    0,
                )?,
            };
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_chrome_colors(
        &self,
        _selection: u32,
        _inactive_selection: u32,
        _caret_line: Option<u32>,
    ) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Forces the selected-text foreground (focused and unfocused), or resets both elements so the
    /// lexer's own style colors show through again. Only high contrast sets a color, where selected
    /// text must keep the system highlight pair to stay legible.
    #[cfg(windows)]
    pub fn set_selection_text_colors(&self, foreground: Option<u32>) -> Result<()> {
        for element in [
            SC_ELEMENT_SELECTION_TEXT,
            SC_ELEMENT_SELECTION_INACTIVE_TEXT,
        ] {
            match foreground {
                Some(colour) => self.endpoint.send_direct_checked(
                    SCI_SETELEMENTCOLOUR,
                    element as usize,
                    ((colour & 0x00FF_FFFF) | 0xFF00_0000) as isize,
                )?,
                None => self.endpoint.send_direct_checked(
                    SCI_RESETELEMENTCOLOUR,
                    element as usize,
                    0,
                )?,
            };
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_selection_text_colors(&self, _foreground: Option<u32>) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Sets one lexer style's foreground, background, bold flag, and font face. `bold` is always
    /// sent explicitly (both true and false) so a previous language's bold flag cannot leak
    /// through onto this style number.
    #[cfg(windows)]
    pub fn set_style(
        &self,
        style: u32,
        foreground: u32,
        background: u32,
        bold: bool,
        italic: bool,
        face: &str,
    ) -> Result<()> {
        let face = CString::new(face).map_err(|_| {
            FastPadError::Invariant("Scintilla font face may not contain NUL bytes")
        })?;
        self.endpoint
            .send_direct_checked(SCI_STYLESETFORE, style as usize, foreground as isize)?;
        self.endpoint
            .send_direct_checked(SCI_STYLESETBACK, style as usize, background as isize)?;
        self.endpoint
            .send_direct_checked(SCI_STYLESETBOLD, style as usize, isize::from(bold))?;
        self.endpoint.send_direct_checked(
            SCI_STYLESETITALIC,
            style as usize,
            isize::from(italic),
        )?;
        self.endpoint.send_direct_checked(
            SCI_STYLESETFONT,
            style as usize,
            face.as_ptr() as isize,
        )?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_style(
        &self,
        _style: u32,
        _foreground: u32,
        _background: u32,
        _bold: bool,
        _italic: bool,
        _face: &str,
    ) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Hands the installed lexer keyword set `set` (space-separated words), via `SCI_SETKEYWORDS`.
    #[cfg(windows)]
    pub fn set_keywords(&self, set: usize, words: &str) -> Result<()> {
        let words = CString::new(words)
            .map_err(|_| FastPadError::Invariant("keyword lists may not contain NUL bytes"))?;
        self.endpoint
            .send_direct_checked(SCI_SETKEYWORDS, set, words.as_ptr() as isize)?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_keywords(&self, _set: usize, _words: &str) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    /// Sets one of the installed lexer's named options, via `SCI_SETPROPERTY`.
    #[cfg(windows)]
    pub fn set_lexer_property(&self, key: &str, value: &str) -> Result<()> {
        let key = CString::new(key).map_err(|_| {
            FastPadError::Invariant("lexer property names may not contain NUL bytes")
        })?;
        let value = CString::new(value).map_err(|_| {
            FastPadError::Invariant("lexer property values may not contain NUL bytes")
        })?;
        self.endpoint.send_direct_checked(
            SCI_SETPROPERTY,
            key.as_ptr() as usize,
            value.as_ptr() as isize,
        )?;
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn set_lexer_property(&self, _key: &str, _value: &str) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }

    pub fn set_save_point(&self) {
        let _ = self.endpoint.send_direct_if_alive(SCI_SETSAVEPOINT, 0, 0);
    }

    pub(crate) fn populate_clean(&self, text: &str) -> Result<()> {
        self.endpoint
            .send_direct_checked(SCI_SETUNDOCOLLECTION, 0, 0)?;
        let result = self.set_text(text);
        let clear = self.endpoint.send_direct_checked(SCI_EMPTYUNDOBUFFER, 0, 0);
        let enable = self
            .endpoint
            .send_direct_checked(SCI_SETUNDOCOLLECTION, 1, 0);
        result?;
        clear?;
        enable?;
        self.endpoint.send_direct_checked(SCI_SETSAVEPOINT, 0, 0)?;
        Ok(())
    }

    pub fn begin_undo_action(&self) {
        let _ = self
            .endpoint
            .send_direct_if_alive(SCI_BEGINUNDOACTION, 0, 0);
    }

    pub fn end_undo_action(&self) {
        let _ = self.endpoint.send_direct_if_alive(SCI_ENDUNDOACTION, 0, 0);
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
    if message == WM_CHAR {
        let ctrl_down = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        if crate::editor::input_filter::should_ignore_char(wparam as u16, ctrl_down) {
            return 0;
        }
    }
    if message == WM_DPICHANGED_AFTERPARENT {
        // Scintilla adopts the new DPI inside its own handler; measure digits only after that.
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        let _ = endpoint.size_line_number_margin(true);
        return result;
    }
    if message == WM_NCDESTROY {
        endpoint.destroyed.store(true, Ordering::Release);
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
