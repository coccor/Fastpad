//! `Editor`'s editing operations: search and replace in targets, undo and redo, the
//! clipboard, the selection, save points and undo action groups.

use super::*;

impl Editor {
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
}
