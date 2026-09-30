//! `Editor`'s look: lexers, styles and keywords, view and whitespace settings, chrome and
//! selection colours, the line-number margin, text padding, zoom, and the caret status.

use super::*;

impl Editor {
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
}
