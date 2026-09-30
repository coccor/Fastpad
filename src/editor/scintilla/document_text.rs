//! `Editor`'s document and text access: setting and reading text, swapping documents, lines,
//! byte ranges, and the view's scroll position and view state.

use super::*;

impl Editor {
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
}
