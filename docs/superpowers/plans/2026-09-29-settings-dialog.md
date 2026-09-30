# Settings Dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A themed, owner-drawn Settings dialog (gear button, **File → Settings…**, **Ctrl+,**, palette) shows every user-facing `fastpad.ini` setting and changes it live. It also adds three editor settings: indent with spaces, show whitespace and highlight current line.

**Architecture:**
- **Pure layer:** `settings_model.rs` holds the rows, focus order, keyboard behaviour and the actions a change produces. `dropdown_list::ListModel` holds the dropdown list's selection and type-ahead. `platform::fonts::dropdown_names` orders the font list.
- **Window layer:** `settings_dialog.rs` (a popup with its own modal loop, shaped like `about.rs`) and `dropdown_list::DropdownList` (a non-activating list popup) only translate Win32 input into model calls, and paint.
- **Applying changes:** every change goes through `main_window::apply_settings_action`, which reuses the palette commands' code paths (`change_setting`, `set_theme`, `execute_command(Toggle…)`), so each change applies at once and saves one `fastpad.ini` line.

**Tech Stack:** Rust 2024, windows-sys 0.61 (GDI, WindowsAndMessaging), Scintilla 5.6.6.

**Spec:** `docs/superpowers/specs/2026-09-29-settings-dialog-design.md`

## Global Constraints

- **Branch:** `feat/settings-dialog`, stacked on `feat/about-dialog` (PR #35).
- **Latency:** nothing new before first paint. The dialog, the dropdown and the font enumeration are all created only when the dialog opens.
- **No new crates.** If a new `windows-sys` feature is needed, add it to `Cargo.toml` and to the allowlist in `tools/audit-dependencies.ps1`. None is expected: everything used is in `Win32_Graphics_Gdi`, `Win32_UI_WindowsAndMessaging`, `Win32_UI_Input_KeyboardAndMouse`, `Win32_UI_Controls`, `Win32_UI_HiDpi` and `Win32_Graphics_Dwm`, which are already enabled.
- **Tests never touch the real profile** (`%LOCALAPPDATA%\FastPad`). Use `RecoveryScratch` and `save_settings_to(Some(..))`.
- **Test runs:**
  - Window and GDI tests run with `--test-threads=1`.
  - While working, run `cargo clippy --all-targets -- -D warnings` plus targeted tests only. The full suite runs only in Task 9.
- No backward compatibility or migrations: a missing new key just keeps its default.
- Commit messages have no attribution lines.
- **New ini keys** (booleans, parsed like `word_wrap`):
  - `insert_spaces`, default `false`;
  - `show_whitespace`, default `false`;
  - `highlight_current_line`, default `true`.
- **Command numbers,** verbatim: `ToggleInsertSpaces = 232`, `ToggleShowWhitespace = 233`, `ToggleHighlightCurrentLine = 234`, `OpenSettings = 235`, `EditSettingsFile = 236`.
- **Palette labels,** verbatim:
  - `Editor: Toggle indent with spaces`
  - `Editor: Toggle show whitespace`
  - `Editor: Toggle highlight current line`
  - `Preferences: Open Settings`
  - `Preferences: Edit fastpad.ini`
- **Menu entry,** verbatim: `Se&ttings...\tCtrl+,` on the File menu.
- **Window classes:** `FastPadSettings` for the dialog, `FastPadSettingsList` for the dropdown.
- **Font size range:** 6 to 72.
- **App-borrow rule:** while a `&mut` from `app_ptr`, `with_state` or `host` is held, never call SetFocus, SendMessageW to another window, or modal functions. The same holds for the dialog's own `state(hwnd)` borrow: end it before calling into `main_window`.

## Review Focus

1. **Notes mode switched off from the dialog:** the sidebar is torn down behind the modal dialog. The keyboard must stay in the dialog, and the Notebook autosave row must grey out. (Task 7: `the_dialog_keeps_the_focus_after_a_change_that_moves_it`.)
2. **A hand-edited `tab_width=3` or `font_size=100`:** the dialog shows the real value, doesn't crash, and the first step or pick brings it back to a supported value. (Task 4 model tests: custom segment, stepper from 100.)
3. **Typing a font size, then leaving with Tab or a click:** the typed size is committed, not lost, and text that isn't a number puts back the current size. (Task 4 `tab_commits_a_typed_size…` and Task 7 keyboard test.)
4. **A short screen (1366×768 at 150%):** the dialog fits the work area, the body scrolls, and Tab scrolls the focused row into view. (Task 7 layout `a_short_work_area…`.)
5. **Theme change from the dialog:** the dialog repaints in the new colours, and a theme change doesn't turn "highlight current line" back on. (Task 2 `…a_theme_change_keeps_it_off` and Task 7's refresh.)

---

### Task 1: The three new settings keys

**Files:**
- Modify: `src/config/persisted.rs`: `Settings`, `Settings::apply_delta`, `SettingsDelta`, the `parse` doc comment, `apply_line`, and tests.
- Modify: `src/config/defaults.rs`: constants, `default_settings`, its doc comment, and its test.

**Interfaces:**
- Produces: `Settings { insert_spaces: bool, show_whitespace: bool, highlight_current_line: bool, .. }` and the matching `SettingsDelta` fields of type `Option<bool>`.
- Produces: `crate::config::defaults::{DEFAULT_INSERT_SPACES, DEFAULT_SHOW_WHITESPACE, DEFAULT_HIGHLIGHT_CURRENT_LINE}`.

- [ ] **Step 1: Write the failing test.** Add it at the end of `mod tests` in `src/config/persisted.rs`:

```rust
    #[test]
    fn the_editor_display_keys_parse_as_bools_with_their_defaults() {
        // Break caught: a new key reported as unknown, a typo silently flipping it, or a default
        // that changes how a brand-new profile's editor looks (settings dialog spec §4.4).
        let defaults = default_settings();
        assert!(!defaults.insert_spaces);
        assert!(!defaults.show_whitespace);
        assert!(defaults.highlight_current_line);

        let delta = parse("insert_spaces=yes\nshow_whitespace=ON\nhighlight_current_line=0\n");
        assert!(delta.warnings.is_empty(), "{:?}", delta.warnings);
        assert_eq!(
            (
                delta.insert_spaces,
                delta.show_whitespace,
                delta.highlight_current_line
            ),
            (Some(true), Some(true), Some(false))
        );
        let mut settings = default_settings();
        settings.apply_delta(&delta);
        assert!(settings.insert_spaces);
        assert!(settings.show_whitespace);
        assert!(!settings.highlight_current_line);

        for key in ["insert_spaces", "show_whitespace", "highlight_current_line"] {
            let delta = parse(&format!("{key}=sometimes\nfont_size=12"));
            assert_eq!(delta.warnings.len(), 1, "{key}");
            assert_eq!(delta.font_size, Some(12), "{key} keeps the other lines");
        }
        assert_eq!(parse("insert_spaces=maybe").insert_spaces, None);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib config::persisted::tests::the_editor_display_keys_parse_as_bools_with_their_defaults`
Expected: compile error, `no field insert_spaces on type Settings`.

- [ ] **Step 3: Implement the keys**

In `src/config/defaults.rs`, after `DEFAULT_OPEN_EDITORS_EXPANDED`:

```rust
pub const DEFAULT_INSERT_SPACES: bool = false;
pub const DEFAULT_SHOW_WHITESPACE: bool = false;
pub const DEFAULT_HIGHLIGHT_CURRENT_LINE: bool = true;
```

In `default_settings()`, add these after `open_editors_expanded: DEFAULT_OPEN_EDITORS_EXPANDED,`:

```rust
        insert_spaces: DEFAULT_INSERT_SPACES,
        show_whitespace: DEFAULT_SHOW_WHITESPACE,
        highlight_current_line: DEFAULT_HIGHLIGHT_CURRENT_LINE,
```

Replace the doc comment of `default_settings` with:

```rust
/// FastPad's compiled defaults: Consolas 11pt, 4-wide tabs inserted as tab characters, word wrap
/// off, line numbers on, whitespace hidden, the current line highlighted, system theme, a
/// 30-second crash-recovery interval, session restore on, notes mode on, the side panel showing
/// the Notebook view at 260 pixels, and Material Icon Theme file icons. Every value a settings
/// file does not (validly) specify keeps whatever `default_settings()` produced.
```

In the `default_settings_match_the_compiled_defaults` test, add these after `assert_eq!(settings.sidebar_width, 260);`:

```rust
        assert!(!settings.insert_spaces);
        assert!(!settings.show_whitespace);
        assert!(settings.highlight_current_line);
```

In `src/config/persisted.rs`, add to `pub struct Settings`, after `open_editors_expanded`:

```rust
    /// Whether Tab inserts spaces instead of a tab character.
    pub insert_spaces: bool,
    /// Whether spaces and tabs are drawn as dots and arrows.
    pub show_whitespace: bool,
    /// Whether the caret's line gets the theme's caret-line background.
    pub highlight_current_line: bool,
```

At the end of `Settings::apply_delta`:

```rust
        if let Some(insert_spaces) = delta.insert_spaces {
            self.insert_spaces = insert_spaces;
        }
        if let Some(show_whitespace) = delta.show_whitespace {
            self.show_whitespace = show_whitespace;
        }
        if let Some(highlight_current_line) = delta.highlight_current_line {
            self.highlight_current_line = highlight_current_line;
        }
```

In `pub struct SettingsDelta`, after `open_editors_expanded`:

```rust
    pub insert_spaces: Option<bool>,
    pub show_whitespace: Option<bool>,
    pub highlight_current_line: Option<bool>,
```

In `apply_line`, before the `_ =>` arm:

```rust
        "insert_spaces" => match parse_bool(value) {
            Some(insert_spaces) => delta.insert_spaces = Some(insert_spaces),
            None => warn(delta, line_number, key, value),
        },
        "show_whitespace" => match parse_bool(value) {
            Some(show_whitespace) => delta.show_whitespace = Some(show_whitespace),
            None => warn(delta, line_number, key, value),
        },
        "highlight_current_line" => match parse_bool(value) {
            Some(highlight) => delta.highlight_current_line = Some(highlight),
            None => warn(delta, line_number, key, value),
        },
```

In the doc comment of `parse`, replace ``…`sidebar_view`, `sidebar_width`, `file_icons` and `open_editors_expanded` are recognized.`` with ``…`sidebar_view`, `sidebar_width`, `file_icons`, `open_editors_expanded`, `insert_spaces`, `show_whitespace` and `highlight_current_line` are recognized.``

- [ ] **Step 4: Run the config tests**

Run: `cargo test --lib config::`
Expected: all pass, including the new test and `default_settings_match_the_compiled_defaults`.

- [ ] **Step 5: Commit**

```bash
git add src/config/persisted.rs src/config/defaults.rs
git commit -m "feat(config): insert_spaces, show_whitespace and highlight_current_line keys"
```

---

### Task 2: The editor applies the new keys, plus their palette toggles

**Files:**
- Modify: `tools/generate-scintilla-constants.ps1` (the `$RequiredNames` list), then regenerate `src/editor/scintilla_constants.rs`. Don't hand-edit it.
- Modify: `src/editor/scintilla.rs`: new `apply_whitespace_settings`; `set_chrome_colors` takes `caret_line: Option<u32>`; update its test at the `.set_chrome_colors(0x0078_4F26, …)` call.
- Modify: `src/window/main_window.rs`: `apply_settings_to`, `apply_colors_to`, the `apply_colors_to` call in `apply_theme`, three `execute_command` arms, and a test.
- Modify: `src/window/commands.rs`: three variants, `needs_document`, `TryFrom`, and a test.
- Modify: `src/window/command_palette.rs`: three `ENTRIES` rows and a test.

**Interfaces:**
- Consumes: `Settings::{insert_spaces, show_whitespace, highlight_current_line}` (Task 1).
- Produces:
  - `CommandId::{ToggleInsertSpaces = 232, ToggleShowWhitespace = 233, ToggleHighlightCurrentLine = 234}`;
  - `Editor::apply_whitespace_settings(&self, insert_spaces: bool, show_whitespace: bool) -> Result<()>`;
  - `Editor::set_chrome_colors(&self, selection: u32, inactive_selection: u32, caret_line: Option<u32>) -> Result<()>`, where `None` resets the caret-line element, which hides the highlight.

- [ ] **Step 1: Generate the Scintilla constants.** In `tools/generate-scintilla-constants.ps1`, change the last line of `$RequiredNames` from `"SCI_GETXOFFSET", "SCI_SETXOFFSET", "SCI_GOTOLINE", "SCN_FOCUSIN"` to:

```powershell
    "SCI_GETXOFFSET", "SCI_SETXOFFSET", "SCI_GOTOLINE", "SCN_FOCUSIN",
    "SCI_SETUSETABS", "SCI_GETUSETABS", "SCI_SETVIEWWS", "SCI_GETVIEWWS", "SCWS_INVISIBLE",
    "SCWS_VISIBLEALWAYS", "SCI_GETELEMENTISSET"
```

Run: `pwsh -NoProfile -File tools/generate-scintilla-constants.ps1`, then `git diff --stat src/editor/scintilla_constants.rs`.
Expected: only additions, matching the vendored `Scintilla.h`: `SCI_GETVIEWWS = 2020`, `SCI_SETVIEWWS = 2021`, `SCI_SETUSETABS = 2124`, `SCI_GETUSETABS = 2125`, `SCI_GETELEMENTISSET = 2756`, `SCWS_INVISIBLE = 0` and `SCWS_VISIBLEALWAYS = 1`.

- [ ] **Step 2: Write the failing tests.** Add this to `mod tests` in `src/window/commands.rs`:

```rust
    #[test]
    fn the_editor_display_toggles_are_232_to_234_and_need_no_document() {
        // Break caught: a toggle renumbered onto another command, or greyed out while no tab is
        // open (settings dialog spec §4.3).
        for (value, command) in [
            (232, CommandId::ToggleInsertSpaces),
            (233, CommandId::ToggleShowWhitespace),
            (234, CommandId::ToggleHighlightCurrentLine),
        ] {
            assert_eq!(command as u16, value);
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(!command.needs_document(), "{command:?}");
            assert!(!command.needs_text(), "{command:?}");
            assert!(!command.is_sidebar(), "{command:?}");
        }
    }
```

Add this to `mod tests` in `src/window/command_palette.rs`:

```rust
    #[test]
    fn the_editor_display_toggles_are_listed_once_under_editor() {
        // Break caught: a new setting reachable only from the Settings dialog.
        for (label, command) in [
            (
                "Editor: Toggle indent with spaces",
                CommandId::ToggleInsertSpaces,
            ),
            (
                "Editor: Toggle show whitespace",
                CommandId::ToggleShowWhitespace,
            ),
            (
                "Editor: Toggle highlight current line",
                CommandId::ToggleHighlightCurrentLine,
            ),
        ] {
            let labels = ENTRIES
                .iter()
                .filter(|entry| entry.command == command)
                .map(|entry| entry.label)
                .collect::<Vec<_>>();
            assert_eq!(labels, [label]);
        }
    }
```

Add this to the test module of `src/window/main_window.rs`, next to `setting_commands_apply_to_the_editor_and_save_only_their_own_ini_lines`:

```rust
    #[test]
    fn the_editor_display_toggles_apply_to_the_editor_and_a_theme_change_keeps_them() {
        // Break caught: a toggle that saves but leaves the editor unchanged, a caret line that a
        // theme change turns back on, or a toggle that rewrites the rest of fastpad.ini
        // (settings dialog spec §4.4).
        use crate::editor::scintilla_constants::{
            SC_ELEMENT_CARET_LINE_BACK, SCI_GETELEMENTISSET, SCI_GETUSETABS, SCI_GETVIEWWS,
            SCWS_INVISIBLE, SCWS_VISIBLEALWAYS,
        };
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("display-toggles");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        super::build_chrome(window.hwnd);
        let send = |message, wparam| unsafe { SendMessageW(editor.hwnd(), message, wparam, 0) };
        let caret_line_set = || send(SCI_GETELEMENTISSET, SC_ELEMENT_CARET_LINE_BACK as usize);
        assert_eq!(send(SCI_GETUSETABS, 0), 1, "tab characters by default");
        assert_eq!(send(SCI_GETVIEWWS, 0), SCWS_INVISIBLE as isize);
        assert_eq!(caret_line_set(), 1, "the current line is highlighted by default");

        execute_command(window.hwnd, CommandId::ToggleInsertSpaces);
        execute_command(window.hwnd, CommandId::ToggleShowWhitespace);
        execute_command(window.hwnd, CommandId::ToggleHighlightCurrentLine);
        assert_eq!(send(SCI_GETUSETABS, 0), 0);
        assert_eq!(send(SCI_GETVIEWWS, 0), SCWS_VISIBLEALWAYS as isize);
        assert_eq!(caret_line_set(), 0);

        execute_command(window.hwnd, CommandId::ThemeDark);
        assert_eq!(caret_line_set(), 0, "a theme change keeps it off");
        execute_command(window.hwnd, CommandId::ToggleHighlightCurrentLine);
        assert_eq!(caret_line_set(), 1);
        super::save_settings_to(None);

        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "# kept\r\ninsert_spaces=true\r\nshow_whitespace=true\r\n\
             highlight_current_line=true\r\ntheme=dark\r\n"
        );
    }
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: errors, `no variant ToggleInsertSpaces`.

- [ ] **Step 4: Implement**

In `src/window/commands.rs`, after `About = 231,`:

```rust
    ToggleInsertSpaces = 232,
    ToggleShowWhitespace = 233,
    ToggleHighlightCurrentLine = 234,
```

In `needs_document`, change `| Self::About` to:

```rust
                | Self::About
                | Self::ToggleInsertSpaces
                | Self::ToggleShowWhitespace
                | Self::ToggleHighlightCurrentLine
```

In `TryFrom`, change `const COMMANDS: [CommandId; 121]` to `[CommandId; 124]`, and add these after `CommandId::About,`:

```rust
            CommandId::ToggleInsertSpaces,
            CommandId::ToggleShowWhitespace,
            CommandId::ToggleHighlightCurrentLine,
```

In `src/window/command_palette.rs`, change `ENTRIES: [PaletteEntry; 99]` to `[PaletteEntry; 102]`, and add these after `entry("Editor: Tab width 8", CommandId::TabWidth8),`:

```rust
    entry(
        "Editor: Toggle indent with spaces",
        CommandId::ToggleInsertSpaces,
    ),
    entry(
        "Editor: Toggle show whitespace",
        CommandId::ToggleShowWhitespace,
    ),
    entry(
        "Editor: Toggle highlight current line",
        CommandId::ToggleHighlightCurrentLine,
    ),
```

In `src/editor/scintilla.rs`, add `SCI_SETUSETABS`, `SCI_SETVIEWWS`, `SCWS_INVISIBLE` and `SCWS_VISIBLEALWAYS` to the windows-only `use crate::editor::scintilla_constants::{…}` that already imports `SCI_SETTABWIDTH` and `SCI_RESETELEMENTCOLOUR`. Then add these after `apply_view_settings`, both the windows and non-windows versions:

```rust
    /// Applies the whitespace settings: whether Tab inserts spaces, and whether spaces and tabs
    /// are drawn.
    #[cfg(windows)]
    pub fn apply_whitespace_settings(&self, insert_spaces: bool, show_whitespace: bool) -> Result<()> {
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
```

Replace both versions of `set_chrome_colors` with:

```rust
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
```

In the existing scintilla test that calls `.set_chrome_colors(0x0078_4F26, 0x0041_3D3A, 0x0028_2828)`, change the last argument to `Some(0x0028_2828)`.

In `src/window/main_window.rs`, replace `apply_settings_to` and `apply_colors_to` with:

```rust
fn apply_settings_to(editor: &Editor, settings: &crate::config::Settings, palette: Palette) {
    let _ = editor.set_line_numbers(settings.line_numbers);
    let _ = editor.apply_view_settings(
        &settings.font_face,
        settings.font_size,
        settings.tab_width,
        settings.word_wrap,
    );
    let _ = editor.apply_whitespace_settings(settings.insert_spaces, settings.show_whitespace);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    apply_chrome_colors_to(editor, palette, settings.highlight_current_line);
}

fn apply_colors_to(editor: &Editor, palette: Palette, highlight_current_line: bool) {
    let _ = editor.set_base_colors(palette.editor_foreground, palette.editor_background);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    apply_chrome_colors_to(editor, palette, highlight_current_line);
    let _ = editor.set_selection_text_colors(palette.selection_foreground);
}

/// The selection backgrounds, and the caret line's when `highlight_current_line` is on.
fn apply_chrome_colors_to(editor: &Editor, palette: Palette, highlight_current_line: bool) {
    let _ = editor.set_chrome_colors(
        palette.selection_background,
        palette.inactive_selection_background,
        highlight_current_line.then_some(palette.caret_line_background),
    );
}
```

In the function holding `apply_settings_to(editor, &settings, palette); apply_colors_to(editor, palette);` (around line 1419), change the second call to `apply_colors_to(editor, palette, settings.highlight_current_line);`.

In `apply_theme`, replace

```rust
    for editor in all_editors(hwnd) {
        apply_colors_to(&editor, palette);
    }
```

with

```rust
    let highlight_current_line = unsafe { app_ptr(hwnd) }
        .is_none_or(|app| unsafe { app.as_ref() }.settings.highlight_current_line);
    for editor in all_editors(hwnd) {
        apply_colors_to(&editor, palette, highlight_current_line);
    }
```

In `execute_command`, add these after the `CommandId::ToggleLineNumbers` arm:

```rust
        CommandId::ToggleInsertSpaces => change_setting(hwnd, |settings| {
            settings.insert_spaces = !settings.insert_spaces;
            Some(("insert_spaces", settings.insert_spaces.to_string()))
        }),
        CommandId::ToggleShowWhitespace => change_setting(hwnd, |settings| {
            settings.show_whitespace = !settings.show_whitespace;
            Some(("show_whitespace", settings.show_whitespace.to_string()))
        }),
        CommandId::ToggleHighlightCurrentLine => change_setting(hwnd, |settings| {
            settings.highlight_current_line = !settings.highlight_current_line;
            Some((
                "highlight_current_line",
                settings.highlight_current_line.to_string(),
            ))
        }),
```

`change_setting` calls `apply_editor_settings` for any key it doesn't list as sidebar-only, and that now runs `apply_settings_to`, so the three toggles reach every editor in every group.

- [ ] **Step 5: Run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

Run: `cargo test --lib -- --test-threads=1 the_editor_display_toggles set_chrome_colors native_command_values setting_commands_apply_to_the_editor`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add tools/generate-scintilla-constants.ps1 src/editor/scintilla_constants.rs src/editor/scintilla.rs src/window/main_window.rs src/window/commands.rs src/window/command_palette.rs
git commit -m "feat: indent with spaces, show whitespace and highlight current line toggles"
```

---

### Task 3: The installed font families

**Files:**
- Create: `src/platform/fonts.rs`
- Modify: `src/platform/mod.rs`: add `pub mod fonts;` between `pub mod files;` and `pub mod handles;`.

**Interfaces:**
- Produces:
  - `crate::platform::fonts::FontFamily { pub name: String, pub fixed_pitch: bool }`;
  - `crate::platform::fonts::installed_font_families() -> Vec<FontFamily>`;
  - `crate::platform::fonts::dropdown_names(families: Vec<FontFamily>, current: &str) -> Vec<String>`.

- [ ] **Step 1: Write the module with its tests.** Create `src/platform/fonts.rs`:

```rust
//! The installed font families, for the Settings dialog's Font dropdown (settings dialog spec
//! §3.5). Enumerated each time the dialog opens, never at startup.

use windows_sys::Win32::Foundation::LPARAM;
use windows_sys::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, FIXED_PITCH, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};

/// One installed family, as GDI enumerates it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFamily {
    pub name: String,
    pub fixed_pitch: bool,
}

/// Every family GDI lists for `DEFAULT_CHARSET`, once per charset it supports: unsorted and with
/// duplicates, which `dropdown_names` removes.
pub fn installed_font_families() -> Vec<FontFamily> {
    let mut families = Vec::new();
    let query = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return families;
        }
        EnumFontFamiliesExW(
            dc,
            &query,
            Some(collect_family),
            (&raw mut families) as LPARAM,
            0,
        );
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    families
}

unsafe extern "system" fn collect_family(
    logfont: *const LOGFONTW,
    _metrics: *const TEXTMETRICW,
    _font_type: u32,
    lparam: LPARAM,
) -> i32 {
    // SAFETY: `lparam` is the `Vec` `installed_font_families` passed, alive for the whole call.
    let families = unsafe { &mut *(lparam as *mut Vec<FontFamily>) };
    let Some(logfont) = (unsafe { logfont.as_ref() }) else {
        return 1;
    };
    let face = &logfont.lfFaceName;
    let length = face.iter().position(|&c| c == 0).unwrap_or(face.len());
    families.push(FontFamily {
        name: String::from_utf16_lossy(&face[..length]),
        fixed_pitch: logfont.lfPitchAndFamily & 0x03 == FIXED_PITCH,
    });
    1
}

/// The dropdown's font names: fixed-pitch families first, then the rest, each group sorted by
/// name ignoring case. Empty names, duplicates and vertical (`@`) families are dropped.
/// `current` is listed first when no family has its name, so the dropdown can still show it.
pub fn dropdown_names(families: Vec<FontFamily>, current: &str) -> Vec<String> {
    let mut families = families
        .into_iter()
        .filter(|family| !family.name.is_empty() && !family.name.starts_with('@'))
        .collect::<Vec<_>>();
    families.sort_by(|a, b| {
        b.fixed_pitch
            .cmp(&a.fixed_pitch)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut seen = std::collections::HashSet::new();
    let mut names = families
        .into_iter()
        .filter(|family| seen.insert(family.name.to_lowercase()))
        .map(|family| family.name)
        .collect::<Vec<_>>();
    if !current.is_empty() && !names.iter().any(|name| name.eq_ignore_ascii_case(current)) {
        names.insert(0, current.to_owned());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(name: &str, fixed_pitch: bool) -> FontFamily {
        FontFamily {
            name: name.to_owned(),
            fixed_pitch,
        }
    }

    #[test]
    fn fixed_pitch_families_come_first_sorted_without_duplicates_or_vertical_fonts() {
        // Break caught: a code font buried among hundreds of proportional ones, the same face
        // listed once per charset, or "@MS Gothic" (a vertical font) offered for an editor.
        let names = dropdown_names(
            vec![
                family("Segoe UI", false),
                family("consolas", true),
                family("@MS Gothic", true),
                family("Arial", false),
                family("Cascadia Mono", true),
                family("Arial", false),
                family("", false),
            ],
            "Consolas",
        );
        assert_eq!(names, ["Cascadia Mono", "consolas", "Arial", "Segoe UI"]);
    }

    #[test]
    fn a_face_that_is_not_installed_is_listed_first() {
        // Break caught: a hand-edited font_face the dropdown cannot show, so it reads as blank.
        let names = dropdown_names(vec![family("Consolas", true)], "Iosevka");
        assert_eq!(names, ["Iosevka", "Consolas"]);
        assert_eq!(dropdown_names(Vec::new(), ""), Vec::<String>::new());
    }

    #[test]
    fn this_machine_lists_consolas_as_fixed_pitch() {
        // Break caught: an enumeration that returns nothing, or reads the pitch bits wrong.
        let families = installed_font_families();
        assert!(
            families
                .iter()
                .any(|family| family.name == "Consolas" && family.fixed_pitch),
            "{} families, no fixed-pitch Consolas",
            families.len()
        );
    }
}
```

- [ ] **Step 2: Add the module and run the tests**

Add `pub mod fonts;` to `src/platform/mod.rs`, between `pub mod files;` and `pub mod handles;`.

Run: `cargo test --lib platform::fonts`
Expected: 3 pass.

If clippy flags `DEFAULT_CHARSET` or `FIXED_PITCH` for a type mismatch (both are `u8` in windows-sys 0.61), adjust with `u8::from`/`as u8` only as needed.

- [ ] **Step 3: Clippy and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

```bash
git add src/platform/fonts.rs src/platform/mod.rs
git commit -m "feat(platform): list installed font families for the Settings dialog"
```

---

### Task 4: The pure Settings model

**Files:**
- Create: `src/window/settings_model.rs`
- Modify: `src/window/mod.rs`: add `pub(crate) mod settings_model;` in alphabetical order, after `pub(crate) mod search_view;`.

**Interfaces:**
- Consumes: `Settings`, `ThemePreference`, `FileIconSet` (`crate::config`); `CommandId` (Task 2's toggles, and the existing ones).
- Produces (all `pub(crate)` in `crate::window::settings_model`):
  - `enum Section { Appearance, Editor, NotesAndSession }`, with `ALL` and `title()`;
  - `enum Row { Theme, FileIcons, Font, FontSize, TabWidth, InsertSpaces, WordWrap, LineNumbers, ShowWhitespace, HighlightCurrentLine, NotesMode, RestoreSession, NotebookAutosave }`, with `ALL: [Row; 13]`, `label()`, `section()`, `control()` and `toggle()`. `row as usize` is its index in `ALL`;
  - `enum Control { Check, Segmented, Dropdown, Stepper }`;
  - `enum Toggle { … }`, with `command() -> CommandId`;
  - `struct SettingsView { pub settings: Settings, pub notebook_autosave: Option<bool> }`, with `enabled(row)`, `checked(toggle)`, `segments(row) -> Vec<Segment>`, `selected_segment(row)`, `segment_action(row, index)`, `dropdown(row, fonts) -> (Vec<String>, Option<usize>)` and `dropdown_text(row) -> String`;
  - `fn dropdown_action(row, index, fonts: &[String]) -> Option<SettingsAction>`;
  - `struct Segment { pub label: String, pub selected: bool }`;
  - `enum SettingsAction { SetTheme(ThemePreference), SetFileIcons(FileIconSet), SetFontFace(String), SetFontSize(u16), SetTabWidth(u8), Toggle(Toggle) }`;
  - `const MIN_FONT_SIZE: u16 = 6`, `const MAX_FONT_SIZE: u16 = 72`, `fn step_font_size(size: u16, up: bool) -> u16` and `fn typed_font_size(text: &str, current: u16) -> u16`;
  - `enum Focus { Row(Row), EditIni, Close }` and `fn next_focus(current: Focus, forward: bool, view: &SettingsView) -> Focus`;
  - `enum Key { Tab { back: bool }, Space, Enter, Left, Right, Up, Down, AltDown, Escape, Backspace, Char(char) }`;
  - `enum Effect { None, Repaint, Apply(SettingsAction), OpenDropdown(Row), EditIni, Close }`;
  - `struct DialogModel { pub focus: Focus, pub typed: Option<String> }`, with `new()`, `set_focus(focus, view) -> Effect`, `key(key, view) -> Effect` and `font_size_text(view) -> String`.

- [ ] **Step 1: Write the module, tests first in the same file.** Create `src/window/settings_model.rs`:

```rust
//! The Settings dialog's rows and what each input does to them (settings dialog spec §3, §4.1).
//! Pure: no window handles, so the dialog's behaviour is tested without a window.

use crate::config::{FileIconSet, Settings, ThemePreference};
use crate::window::commands::CommandId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Section {
    Appearance,
    Editor,
    NotesAndSession,
}

impl Section {
    pub(crate) const ALL: [Self; 3] = [Self::Appearance, Self::Editor, Self::NotesAndSession];

    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::NotesAndSession => "Notes and session",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Control {
    Check,
    Segmented,
    Dropdown,
    Stepper,
}

/// One row of the dialog, declared top to bottom: `row as usize` is its index in `Row::ALL`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Row {
    Theme,
    FileIcons,
    Font,
    FontSize,
    TabWidth,
    InsertSpaces,
    WordWrap,
    LineNumbers,
    ShowWhitespace,
    HighlightCurrentLine,
    NotesMode,
    RestoreSession,
    NotebookAutosave,
}

impl Row {
    pub(crate) const ALL: [Self; 13] = [
        Self::Theme,
        Self::FileIcons,
        Self::Font,
        Self::FontSize,
        Self::TabWidth,
        Self::InsertSpaces,
        Self::WordWrap,
        Self::LineNumbers,
        Self::ShowWhitespace,
        Self::HighlightCurrentLine,
        Self::NotesMode,
        Self::RestoreSession,
        Self::NotebookAutosave,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::FileIcons => "File icons",
            Self::Font => "Font",
            Self::FontSize => "Font size",
            Self::TabWidth => "Tab width",
            Self::InsertSpaces => "Indent with spaces",
            Self::WordWrap => "Word wrap",
            Self::LineNumbers => "Line numbers",
            Self::ShowWhitespace => "Show whitespace",
            Self::HighlightCurrentLine => "Highlight current line",
            Self::NotesMode => "Notes mode",
            Self::RestoreSession => "Restore session",
            Self::NotebookAutosave => "Notebook autosave",
        }
    }

    pub(crate) const fn section(self) -> Section {
        match self {
            Self::Theme | Self::FileIcons => Section::Appearance,
            Self::NotesMode | Self::RestoreSession | Self::NotebookAutosave => {
                Section::NotesAndSession
            }
            _ => Section::Editor,
        }
    }

    pub(crate) const fn control(self) -> Control {
        match self {
            Self::Theme | Self::Font => Control::Dropdown,
            Self::FileIcons | Self::TabWidth => Control::Segmented,
            Self::FontSize => Control::Stepper,
            _ => Control::Check,
        }
    }

    /// A checkbox row's setting.
    pub(crate) const fn toggle(self) -> Option<Toggle> {
        Some(match self {
            Self::InsertSpaces => Toggle::InsertSpaces,
            Self::WordWrap => Toggle::WordWrap,
            Self::LineNumbers => Toggle::LineNumbers,
            Self::ShowWhitespace => Toggle::ShowWhitespace,
            Self::HighlightCurrentLine => Toggle::HighlightCurrentLine,
            Self::NotesMode => Toggle::NotesMode,
            Self::RestoreSession => Toggle::RestoreSession,
            Self::NotebookAutosave => Toggle::NotebookAutosave,
            _ => return None,
        })
    }
}

/// A checkbox's setting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Toggle {
    InsertSpaces,
    WordWrap,
    LineNumbers,
    ShowWhitespace,
    HighlightCurrentLine,
    NotesMode,
    RestoreSession,
    NotebookAutosave,
}

impl Toggle {
    /// The palette command that flips it: the dialog runs exactly what the palette runs, notices
    /// and side effects included.
    pub(crate) const fn command(self) -> CommandId {
        match self {
            Self::InsertSpaces => CommandId::ToggleInsertSpaces,
            Self::WordWrap => CommandId::ToggleWordWrap,
            Self::LineNumbers => CommandId::ToggleLineNumbers,
            Self::ShowWhitespace => CommandId::ToggleShowWhitespace,
            Self::HighlightCurrentLine => CommandId::ToggleHighlightCurrentLine,
            Self::NotesMode => CommandId::ToggleNotesMode,
            Self::RestoreSession => CommandId::ToggleRestoreSession,
            Self::NotebookAutosave => CommandId::ToggleFolderAutosave,
        }
    }
}

/// The Theme dropdown's items, in order.
pub(crate) const THEME_CHOICES: [(ThemePreference, &str); 8] = [
    (ThemePreference::System, "System"),
    (ThemePreference::Light, "Light"),
    (ThemePreference::Dark, "Dark"),
    (ThemePreference::Catppuccin, "Catppuccin"),
    (ThemePreference::CatppuccinLatte, "Catppuccin Latte"),
    (ThemePreference::CatppuccinFrappe, "Catppuccin Frapp\u{e9}"),
    (ThemePreference::CatppuccinMacchiato, "Catppuccin Macchiato"),
    (ThemePreference::CatppuccinMocha, "Catppuccin Mocha"),
];

pub(crate) const FILE_ICON_CHOICES: [(FileIconSet, &str); 3] = [
    (FileIconSet::Material, "Material"),
    (FileIconSet::Minimal, "Minimal"),
    (FileIconSet::Solid, "Solid"),
];

pub(crate) const TAB_WIDTH_CHOICES: [u8; 3] = [2, 4, 8];

/// The font size the stepper and the palette's font-size commands keep within.
pub(crate) const MIN_FONT_SIZE: u16 = 6;
pub(crate) const MAX_FONT_SIZE: u16 = 72;

/// Digits the font size field accepts before ignoring more.
const MAX_TYPED_DIGITS: usize = 3;

/// One change the dialog asks `main_window::apply_settings_action` to make.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SettingsAction {
    SetTheme(ThemePreference),
    SetFileIcons(FileIconSet),
    SetFontFace(String),
    SetFontSize(u16),
    SetTabWidth(u8),
    Toggle(Toggle),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Segment {
    pub label: String,
    pub selected: bool,
}

/// What the dialog shows: the current settings, and the open notebook's autosave switch, which
/// is `None` while no notebook is open or its state is still loading.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsView {
    pub settings: Settings,
    pub notebook_autosave: Option<bool>,
}

impl SettingsView {
    /// Whether `row` can be changed now. Only Notebook autosave is ever greyed out.
    pub(crate) fn enabled(&self, row: Row) -> bool {
        row != Row::NotebookAutosave || self.notebook_autosave.is_some()
    }

    pub(crate) fn checked(&self, toggle: Toggle) -> bool {
        let settings = &self.settings;
        match toggle {
            Toggle::InsertSpaces => settings.insert_spaces,
            Toggle::WordWrap => settings.word_wrap,
            Toggle::LineNumbers => settings.line_numbers,
            Toggle::ShowWhitespace => settings.show_whitespace,
            Toggle::HighlightCurrentLine => settings.highlight_current_line,
            Toggle::NotesMode => settings.notes_mode,
            Toggle::RestoreSession => settings.restore_session,
            Toggle::NotebookAutosave => self.notebook_autosave.unwrap_or(false),
        }
    }

    /// A segmented row's segments, left to right. A tab width other than 2, 4 or 8 gets a fourth,
    /// selected segment showing it (spec §3.2).
    pub(crate) fn segments(&self, row: Row) -> Vec<Segment> {
        match row {
            Row::FileIcons => FILE_ICON_CHOICES
                .iter()
                .map(|&(set, label)| Segment {
                    label: label.to_owned(),
                    selected: self.settings.file_icons == set,
                })
                .collect(),
            Row::TabWidth => {
                let width = self.settings.tab_width;
                let mut segments = TAB_WIDTH_CHOICES
                    .iter()
                    .map(|&choice| Segment {
                        label: choice.to_string(),
                        selected: width == choice,
                    })
                    .collect::<Vec<_>>();
                if !TAB_WIDTH_CHOICES.contains(&width) {
                    segments.push(Segment {
                        label: width.to_string(),
                        selected: true,
                    });
                }
                segments
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn selected_segment(&self, row: Row) -> Option<usize> {
        self.segments(row).iter().position(|segment| segment.selected)
    }

    /// What picking segment `index` of `row` does. The custom tab width segment is already the
    /// setting, so picking it does nothing.
    pub(crate) fn segment_action(&self, row: Row, index: usize) -> Option<SettingsAction> {
        match row {
            Row::FileIcons => FILE_ICON_CHOICES
                .get(index)
                .map(|&(set, _)| SettingsAction::SetFileIcons(set)),
            Row::TabWidth => TAB_WIDTH_CHOICES
                .get(index)
                .map(|&width| SettingsAction::SetTabWidth(width)),
            _ => None,
        }
    }

    /// A dropdown row's items and the selected one. `fonts` is the Font row's list.
    pub(crate) fn dropdown(&self, row: Row, fonts: &[String]) -> (Vec<String>, Option<usize>) {
        match row {
            Row::Theme => (
                THEME_CHOICES
                    .iter()
                    .map(|(_, label)| (*label).to_owned())
                    .collect(),
                THEME_CHOICES
                    .iter()
                    .position(|(theme, _)| *theme == self.settings.theme),
            ),
            Row::Font => (
                fonts.to_vec(),
                fonts
                    .iter()
                    .position(|font| font.eq_ignore_ascii_case(&self.settings.font_face)),
            ),
            _ => (Vec::new(), None),
        }
    }

    /// The text a closed dropdown shows.
    pub(crate) fn dropdown_text(&self, row: Row) -> String {
        match row {
            Row::Theme => THEME_CHOICES
                .iter()
                .find(|(theme, _)| *theme == self.settings.theme)
                .map_or_else(String::new, |(_, label)| (*label).to_owned()),
            Row::Font => self.settings.font_face.clone(),
            _ => String::new(),
        }
    }
}

/// What picking item `index` of `row`'s dropdown does.
pub(crate) fn dropdown_action(row: Row, index: usize, fonts: &[String]) -> Option<SettingsAction> {
    match row {
        Row::Theme => THEME_CHOICES
            .get(index)
            .map(|&(theme, _)| SettingsAction::SetTheme(theme)),
        Row::Font => fonts
            .get(index)
            .map(|font| SettingsAction::SetFontFace(font.clone())),
        _ => None,
    }
}

/// One step of the stepper from `size`. The result is within 6–72, so a size set outside that
/// range in `fastpad.ini` comes back into it on the first step (spec §3.2).
pub(crate) fn step_font_size(size: u16, up: bool) -> u16 {
    let next = if up {
        size.saturating_add(1)
    } else {
        size.saturating_sub(1)
    };
    next.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)
}

/// The size a typed entry commits: its number pulled into 6–72, or `current` when the text is
/// empty or not a number.
pub(crate) fn typed_font_size(text: &str, current: u16) -> u16 {
    match text.trim().parse::<u32>() {
        Ok(value) => value.clamp(u32::from(MIN_FONT_SIZE), u32::from(MAX_FONT_SIZE)) as u16,
        Err(_) => current,
    }
}

/// What has the keyboard focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Focus {
    Row(Row),
    EditIni,
    Close,
}

/// Tab order: the rows top to bottom, skipping greyed-out ones, then the Edit fastpad.ini link,
/// then Close. It wraps around.
pub(crate) fn next_focus(current: Focus, forward: bool, view: &SettingsView) -> Focus {
    let order = Row::ALL
        .into_iter()
        .filter(|row| view.enabled(*row))
        .map(Focus::Row)
        .chain([Focus::EditIni, Focus::Close])
        .collect::<Vec<_>>();
    let count = order.len();
    let next = match (order.iter().position(|focus| *focus == current), forward) {
        (Some(index), true) => (index + 1) % count,
        (Some(index), false) => (index + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    };
    order[next]
}

/// A key the dialog passes on, already decoded from its window message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    Tab { back: bool },
    Space,
    Enter,
    Left,
    Right,
    Up,
    Down,
    AltDown,
    Escape,
    Backspace,
    Char(char),
}

/// What the dialog does after an input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    None,
    Repaint,
    Apply(SettingsAction),
    OpenDropdown(Row),
    EditIni,
    Close,
}

/// The dialog's keyboard state: what has the focus, and a font size being typed but not yet
/// committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DialogModel {
    pub focus: Focus,
    pub typed: Option<String>,
}

impl DialogModel {
    pub(crate) fn new() -> Self {
        Self {
            focus: Focus::Row(Row::ALL[0]),
            typed: None,
        }
    }

    /// Moves the focus, committing a typed font size on the way out.
    pub(crate) fn set_focus(&mut self, focus: Focus, view: &SettingsView) -> Effect {
        let commit = self.commit_typed(view);
        self.focus = focus;
        match commit {
            Effect::None => Effect::Repaint,
            other => other,
        }
    }

    /// The font size field's text: what is being typed, or the current size.
    pub(crate) fn font_size_text(&self, view: &SettingsView) -> String {
        self.typed
            .clone()
            .unwrap_or_else(|| view.settings.font_size.to_string())
    }

    pub(crate) fn key(&mut self, key: Key, view: &SettingsView) -> Effect {
        match key {
            Key::Tab { back } => {
                let next = next_focus(self.focus, !back, view);
                self.set_focus(next, view)
            }
            Key::Escape => Effect::Close,
            _ => match self.focus {
                Focus::EditIni if matches!(key, Key::Space | Key::Enter) => Effect::EditIni,
                Focus::Close if matches!(key, Key::Space | Key::Enter) => Effect::Close,
                Focus::Row(row) => self.row_key(row, key, view),
                _ => Effect::None,
            },
        }
    }

    fn row_key(&mut self, row: Row, key: Key, view: &SettingsView) -> Effect {
        if !view.enabled(row) {
            return Effect::None;
        }
        match row.control() {
            Control::Check => match (key, row.toggle()) {
                (Key::Space, Some(toggle)) => Effect::Apply(SettingsAction::Toggle(toggle)),
                _ => Effect::None,
            },
            Control::Segmented => {
                let Some(selected) = view.selected_segment(row) else {
                    return Effect::None;
                };
                let count = view.segments(row).len();
                let target = match key {
                    Key::Left => selected.checked_sub(1),
                    Key::Right => (selected + 1 < count).then_some(selected + 1),
                    _ => None,
                };
                target
                    .and_then(|index| view.segment_action(row, index))
                    .map_or(Effect::None, Effect::Apply)
            }
            Control::Dropdown => match key {
                Key::Enter | Key::AltDown => Effect::OpenDropdown(row),
                _ => Effect::None,
            },
            Control::Stepper => self.stepper_key(key, view),
        }
    }

    fn stepper_key(&mut self, key: Key, view: &SettingsView) -> Effect {
        let current = view.settings.font_size;
        match key {
            Key::Up | Key::Down => {
                // A step replaces whatever was being typed.
                self.typed = None;
                let size = step_font_size(current, key == Key::Up);
                if size == current {
                    Effect::Repaint
                } else {
                    Effect::Apply(SettingsAction::SetFontSize(size))
                }
            }
            Key::Char(digit) if digit.is_ascii_digit() => {
                let typed = self.typed.get_or_insert_with(String::new);
                if typed.len() < MAX_TYPED_DIGITS {
                    typed.push(digit);
                }
                Effect::Repaint
            }
            Key::Backspace => {
                self.typed
                    .get_or_insert_with(|| current.to_string())
                    .pop();
                Effect::Repaint
            }
            Key::Enter => self.commit_typed(view),
            _ => Effect::None,
        }
    }

    /// The typed size's change, if it differs from the current size, clearing the typed text.
    fn commit_typed(&mut self, view: &SettingsView) -> Effect {
        let Some(text) = self.typed.take() else {
            return Effect::None;
        };
        let current = view.settings.font_size;
        let size = typed_font_size(&text, current);
        if size == current {
            Effect::Repaint
        } else {
            Effect::Apply(SettingsAction::SetFontSize(size))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SettingsView {
        SettingsView {
            settings: crate::config::default_settings(),
            notebook_autosave: None,
        }
    }

    #[test]
    fn rows_follow_the_spec_order_and_sections() {
        // Break caught: a row painted under the wrong heading, or `row as usize` drifting from
        // its position in `Row::ALL`, which the layout indexes by.
        for (index, row) in Row::ALL.into_iter().enumerate() {
            assert_eq!(row as usize, index, "{row:?}");
        }
        let sections = Row::ALL.map(Row::section);
        assert_eq!(&sections[..2], [Section::Appearance; 2]);
        assert_eq!(&sections[2..10], [Section::Editor; 8]);
        assert_eq!(&sections[10..], [Section::NotesAndSession; 3]);
        for (index, section) in Section::ALL.into_iter().enumerate() {
            assert_eq!(section as usize, index);
        }
    }

    #[test]
    fn every_checkbox_runs_the_palette_command_for_its_setting() {
        // Break caught: a checkbox that flips a different setting than its label says.
        let checks = Row::ALL
            .into_iter()
            .filter(|row| row.control() == Control::Check)
            .map(|row| row.toggle().unwrap().command())
            .collect::<Vec<_>>();
        assert_eq!(
            checks,
            [
                CommandId::ToggleInsertSpaces,
                CommandId::ToggleWordWrap,
                CommandId::ToggleLineNumbers,
                CommandId::ToggleShowWhitespace,
                CommandId::ToggleHighlightCurrentLine,
                CommandId::ToggleNotesMode,
                CommandId::ToggleRestoreSession,
                CommandId::ToggleFolderAutosave,
            ]
        );
        assert_eq!(Row::Theme.toggle(), None);
    }

    #[test]
    fn tab_order_wraps_and_skips_notebook_autosave_without_a_notebook() {
        // Break caught: Tab stopping on a greyed-out row that ignores every key.
        let closed = view();
        assert_eq!(
            next_focus(Focus::Row(Row::RestoreSession), true, &closed),
            Focus::EditIni
        );
        assert_eq!(next_focus(Focus::Close, true, &closed), Focus::Row(Row::Theme));
        assert_eq!(next_focus(Focus::Row(Row::Theme), false, &closed), Focus::Close);
        let open = SettingsView {
            notebook_autosave: Some(true),
            ..view()
        };
        assert_eq!(
            next_focus(Focus::Row(Row::RestoreSession), true, &open),
            Focus::Row(Row::NotebookAutosave)
        );
        assert!(open.checked(Toggle::NotebookAutosave));
        assert!(!closed.checked(Toggle::NotebookAutosave));
    }

    #[test]
    fn a_custom_tab_width_shows_as_a_fourth_selected_segment() {
        // Break caught: tab_width=3 from fastpad.ini shown as none selected, or picking the
        // custom segment writing a value.
        let mut custom = view();
        custom.settings.tab_width = 3;
        let segments = custom.segments(Row::TabWidth);
        assert_eq!(
            segments.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["2", "4", "8", "3"]
        );
        assert_eq!(custom.selected_segment(Row::TabWidth), Some(3));
        assert_eq!(custom.segment_action(Row::TabWidth, 3), None);
        assert_eq!(
            custom.segment_action(Row::TabWidth, 0),
            Some(SettingsAction::SetTabWidth(2))
        );
        assert_eq!(view().segments(Row::TabWidth).len(), 3);
        assert_eq!(view().selected_segment(Row::TabWidth), Some(1));
        assert_eq!(view().selected_segment(Row::FileIcons), Some(0));
    }

    #[test]
    fn the_stepper_stays_within_6_to_72_and_brings_outside_sizes_back() {
        assert_eq!(step_font_size(11, true), 12);
        assert_eq!(step_font_size(11, false), 10);
        assert_eq!(step_font_size(72, true), 72);
        assert_eq!(step_font_size(6, false), 6);
        assert_eq!(step_font_size(100, true), 72, "a hand-edited 100 comes back");
        assert_eq!(step_font_size(3, false), 6);
    }

    #[test]
    fn a_typed_size_is_clamped_and_text_that_is_not_a_number_keeps_the_current_size() {
        assert_eq!(typed_font_size("16", 11), 16);
        assert_eq!(typed_font_size("999", 11), 72);
        assert_eq!(typed_font_size("0", 11), 6);
        assert_eq!(typed_font_size("", 11), 11);
        assert_eq!(typed_font_size("abc", 11), 11);
    }

    #[test]
    fn tab_commits_a_typed_size_and_moves_on() {
        // Break caught: a typed size lost when the keyboard leaves the field (review focus 3).
        let view = view();
        let mut model = DialogModel {
            focus: Focus::Row(Row::FontSize),
            typed: None,
        };
        assert_eq!(model.key(Key::Char('1'), &view), Effect::Repaint);
        assert_eq!(model.key(Key::Char('6'), &view), Effect::Repaint);
        assert_eq!(model.font_size_text(&view), "16");
        assert_eq!(
            model.key(Key::Tab { back: false }, &view),
            Effect::Apply(SettingsAction::SetFontSize(16))
        );
        assert_eq!(model.focus, Focus::Row(Row::TabWidth));
        assert_eq!(model.typed, None);
    }

    #[test]
    fn the_font_size_field_takes_three_digits_enter_commits_and_a_step_discards_typing() {
        let view = view();
        let mut model = DialogModel {
            focus: Focus::Row(Row::FontSize),
            typed: None,
        };
        for digit in ['1', '2', '3', '4'] {
            model.key(Key::Char(digit), &view);
        }
        assert_eq!(model.font_size_text(&view), "123");
        assert_eq!(
            model.key(Key::Enter, &view),
            Effect::Apply(SettingsAction::SetFontSize(72))
        );
        model.key(Key::Backspace, &view);
        assert_eq!(model.font_size_text(&view), "1", "backspace edits the current 11");
        assert_eq!(
            model.key(Key::Up, &view),
            Effect::Apply(SettingsAction::SetFontSize(12))
        );
        assert_eq!(model.typed, None);
        model.key(Key::Char('x'), &view);
        assert_eq!(model.typed, None, "letters are ignored");
        model.key(Key::Char('1'), &view);
        assert_eq!(model.key(Key::Char('1'), &view), Effect::Repaint);
        assert_eq!(
            model.key(Key::Enter, &view),
            Effect::Repaint,
            "typing the current size changes nothing"
        );
    }

    #[test]
    fn keys_act_on_the_focused_control() {
        let view = view();
        let mut model = DialogModel::new();
        assert_eq!(model.focus, Focus::Row(Row::Theme));
        assert_eq!(model.key(Key::Enter, &view), Effect::OpenDropdown(Row::Theme));
        assert_eq!(model.key(Key::AltDown, &view), Effect::OpenDropdown(Row::Theme));
        assert_eq!(model.key(Key::Space, &view), Effect::None);

        model.focus = Focus::Row(Row::FileIcons);
        assert_eq!(model.key(Key::Left, &view), Effect::None, "already the first");
        assert_eq!(
            model.key(Key::Right, &view),
            Effect::Apply(SettingsAction::SetFileIcons(FileIconSet::Minimal))
        );

        model.focus = Focus::Row(Row::WordWrap);
        assert_eq!(
            model.key(Key::Space, &view),
            Effect::Apply(SettingsAction::Toggle(Toggle::WordWrap))
        );
        model.focus = Focus::Row(Row::NotebookAutosave);
        assert_eq!(model.key(Key::Space, &view), Effect::None, "greyed out");

        model.focus = Focus::EditIni;
        assert_eq!(model.key(Key::Enter, &view), Effect::EditIni);
        model.focus = Focus::Close;
        assert_eq!(model.key(Key::Space, &view), Effect::Close);
        assert_eq!(model.key(Key::Escape, &view), Effect::Close);
    }

    #[test]
    fn dropdowns_select_the_current_value_and_pick_by_index() {
        let mut view = view();
        view.settings.theme = ThemePreference::CatppuccinMocha;
        view.settings.font_face = "consolas".to_owned();
        let fonts = vec!["Cascadia Mono".to_owned(), "Consolas".to_owned()];
        let (themes, selected) = view.dropdown(Row::Theme, &fonts);
        assert_eq!(themes.len(), 8);
        assert_eq!(selected, Some(7));
        assert_eq!(view.dropdown_text(Row::Theme), "Catppuccin Mocha");
        assert_eq!(view.dropdown(Row::Font, &fonts), (fonts.clone(), Some(1)));
        assert_eq!(view.dropdown_text(Row::Font), "consolas");
        assert_eq!(
            dropdown_action(Row::Theme, 2, &fonts),
            Some(SettingsAction::SetTheme(ThemePreference::Dark))
        );
        assert_eq!(
            dropdown_action(Row::Font, 0, &fonts),
            Some(SettingsAction::SetFontFace("Cascadia Mono".to_owned()))
        );
        assert_eq!(dropdown_action(Row::Font, 9, &fonts), None);
    }
}
```

- [ ] **Step 2: Add the module and run the tests**

Add `pub(crate) mod settings_model;` to `src/window/mod.rs`, after `pub(crate) mod search_view;`.

Run: `cargo test --lib window::settings_model`
Expected: 10 pass.

- [ ] **Step 3: Clippy and commit.** The model is used from Task 5 on. Until then, clippy may warn that it is unused: add `#![allow(dead_code, reason = "used by the settings dialog, added in the next tasks")]` at the top of the file for now, and remove it in Task 7.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

```bash
git add src/window/settings_model.rs src/window/mod.rs
git commit -m "feat: pure Settings dialog model (rows, focus, keys, actions)"
```

---

### Task 5: Applying dialog actions in the main window

**Files:**
- Modify: `src/window/main_window.rs`:
  - add `apply_settings_action`, `settings_view` and `link_color`;
  - `show_about` uses `link_color`;
  - drop the local `MIN_FONT_SIZE`/`MAX_FONT_SIZE` in favour of `settings_model`'s;
  - add a test.
- Modify: `src/window/library_host.rs`: add `notebook_autosave`.

**Interfaces:**
- Consumes: `settings_model::{SettingsAction, SettingsView, Toggle, MIN_FONT_SIZE, MAX_FONT_SIZE}` (Task 4).
- Produces:
  - `main_window::apply_settings_action(hwnd: HWND, action: SettingsAction)`;
  - `main_window::settings_view(hwnd: HWND) -> SettingsView`;
  - `main_window::link_color(hwnd: HWND) -> u32`;
  - `library_host::notebook_autosave(hwnd: HWND) -> Option<bool>`.

- [ ] **Step 1: Write the failing test.** Add it to the `main_window` test module:

```rust
    #[test]
    fn settings_actions_apply_and_save_only_their_own_lines() {
        // Break caught: a dialog change that updates the window but is lost on restart, one that
        // rewrites the user's fastpad.ini, or a re-pick of the current value that writes anyway
        // (settings dialog spec §4.2).
        use crate::config::{FileIconSet, ThemePreference};
        use crate::window::settings_model::{SettingsAction, Toggle};
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("settings-actions");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        super::build_chrome(window.hwnd);

        for action in [
            SettingsAction::SetTheme(ThemePreference::CatppuccinMocha),
            SettingsAction::SetFileIcons(FileIconSet::Solid),
            SettingsAction::SetFontFace("Cascadia Mono".to_owned()),
            SettingsAction::SetFontSize(14),
            SettingsAction::SetTabWidth(2),
            SettingsAction::Toggle(Toggle::WordWrap),
            // Picking what is already set writes nothing.
            SettingsAction::SetFontSize(14),
            SettingsAction::SetFontFace("Cascadia Mono".to_owned()),
        ] {
            super::apply_settings_action(window.hwnd, action);
        }

        let settings = app_mut(window.hwnd).settings.clone();
        assert_eq!(settings.theme, ThemePreference::CatppuccinMocha);
        assert_eq!(settings.file_icons, FileIconSet::Solid);
        assert_eq!(settings.font_face, "Cascadia Mono");
        assert_eq!(settings.font_size, 14);
        assert_eq!(settings.tab_width, 2);
        assert!(settings.word_wrap);
        let view = super::settings_view(window.hwnd);
        assert_eq!(view.settings, settings);
        assert_eq!(view.notebook_autosave, None, "no notebook is open");
        super::save_settings_to(None);

        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "# kept\r\ntheme=catppuccin-mocha\r\nfile_icons=solid\r\nfont_face=Cascadia Mono\r\n\
             font_size=14\r\ntab_width=2\r\nword_wrap=true\r\n"
        );
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: error, `cannot find function apply_settings_action`.

- [ ] **Step 3: Implement**

In `src/window/library_host.rs`, add this after `fn folder_autosave`:

```rust
/// The open notebook's autosave switch, or `None` while no notebook is open or its state is still
/// loading. The Settings dialog greys its row out then (settings dialog spec §3.2).
pub(crate) fn notebook_autosave(hwnd: HWND) -> Option<bool> {
    host(hwnd, |host| host.state.as_ref().map(|state| state.local.autosave)).flatten()
}
```

In `src/window/main_window.rs`, delete these lines:

```rust
/// Font-size commands step within this range; a size set outside it in `fastpad.ini` is kept
/// until a step moves it back toward the range.
const MIN_FONT_SIZE: u16 = 6;
const MAX_FONT_SIZE: u16 = 72;
```

Then add this import near the other `crate::window` imports at the top of `main_window.rs`:

```rust
use crate::window::settings_model::{MAX_FONT_SIZE, MIN_FONT_SIZE};
```

The palette's `FontSizeIncrease`/`FontSizeDecrease` keep their behaviour: they still keep a size set outside the range until a step moves it back.

Replace `show_about` with:

```rust
/// The theme's link colour, as the Markdown preview draws links.
pub(crate) fn link_color(hwnd: HWND) -> u32 {
    let theme = effective_theme(hwnd);
    let high_contrast = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .theme
            .is_some_and(|system| system.high_contrast)
    });
    crate::preview::colors::preview_colors(theme, high_contrast).link
}

/// Help → About FastPad, in the current theme's colors and the Markdown preview's link color.
fn show_about(hwnd: HWND) {
    crate::window::about::show(hwnd, current_palette(hwnd), link_color(hwnd));
}
```

After `set_file_icons`, add:

```rust
/// Makes one Settings dialog change through the same code the palette commands use, so it
/// applies at once and saves its one `fastpad.ini` line (settings dialog spec §4.2).
pub(crate) fn apply_settings_action(
    hwnd: HWND,
    action: crate::window::settings_model::SettingsAction,
) {
    use crate::window::settings_model::SettingsAction;
    match action {
        SettingsAction::SetTheme(theme) => set_theme(hwnd, theme),
        SettingsAction::SetFileIcons(set) => set_file_icons(hwnd, set),
        SettingsAction::SetFontFace(face) => change_setting(hwnd, |settings| {
            (settings.font_face != face).then(|| {
                settings.font_face.clone_from(&face);
                ("font_face", face)
            })
        }),
        SettingsAction::SetFontSize(size) => set_font_size(hwnd, |_| size),
        SettingsAction::SetTabWidth(width) => set_tab_width(hwnd, width),
        SettingsAction::Toggle(toggle) => execute_command(hwnd, toggle.command()),
    }
}

/// What the Settings dialog shows. Call it with nothing of the App borrowed.
pub(crate) fn settings_view(hwnd: HWND) -> crate::window::settings_model::SettingsView {
    let settings = unsafe { app_ptr(hwnd) }.map_or_else(crate::config::default_settings, |app| {
        unsafe { app.as_ref() }.settings.clone()
    });
    crate::window::settings_model::SettingsView {
        settings,
        notebook_autosave: crate::window::library_host::notebook_autosave(hwnd),
    }
}
```

If `execute_command` is not visible at that point, it is in the same module, so no import is needed. If clippy reports `needless_pass_by_value` or similar on `face`, follow its suggestion.

- [ ] **Step 4: Run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

Run: `cargo test --lib -- --test-threads=1 settings_actions_apply_and_save_only_their_own_lines about_ setting_commands_apply_to_the_editor`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window/main_window.rs src/window/library_host.rs
git commit -m "feat: apply Settings dialog actions through the palette's code paths"
```

---

### Task 6: The dropdown list

**Files:**
- Create: `src/window/dropdown_list.rs`
- Modify: `src/window/mod.rs`: add `pub(crate) mod dropdown_list;` after `pub(crate) mod drag_label;`.

**Interfaces:**
- Consumes: `Palette`; `panel::{fill, inset}`; `platform::wide_null`.
- Produces:
  - `pub(crate) const VISIBLE_ROWS: usize = 10`;
  - `pub(crate) const WM_LIST_PICKED: u32`, posted to the owner with `wparam` = the picked index;
  - `pub(crate) enum ListKey { Up, Down, PageUp, PageDown, Home, End, Enter, Escape, Char(char) }`;
  - `pub(crate) enum ListOutcome { Ignored, Moved, Picked(usize), Dismissed }`;
  - `pub(crate) struct ListModel { pub items: Vec<String>, pub selected: usize, pub top: usize, .. }`, with `new(items, selected: Option<usize>)`, `key(key, now_ms: u32) -> ListOutcome`, `scroll(rows: isize) -> bool` and `item_at_row(row) -> Option<usize>`;
  - `pub(crate) struct DropdownList`, with `show(owner: HWND, anchor: RECT /* screen */, row_height: i32, font: HFONT, colors: Palette, model: ListModel) -> Option<Self>`, `key(&self, key, now_ms) -> ListOutcome` and `wheel(&self, delta: i16)`. Dropping it destroys the popup.

- [ ] **Step 1: Write the module, with the model's tests.** Create `src/window/dropdown_list.rs`:

```rust
//! The Settings dialog's dropdown list: a themed popup under a dropdown that never takes the
//! activation, so the keyboard stays with the dialog, which forwards keys here (settings dialog
//! spec §3.5). `ListModel` is the pure part: selection, scrolling and type-ahead.

use super::palette::Palette;
use super::panel::{fill, inset};
use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW,
    EndPaint, GetMonitorInfoW, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, PostMessageW, RegisterClassW,
    SW_SHOWNA, SetWindowLongPtrW, ShowWindow, WM_APP, WM_ERASEBKGND, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub(crate) const VISIBLE_ROWS: usize = 10;
/// Letters typed within this long of each other build up one type-ahead prefix.
const TYPE_AHEAD_PAUSE_MS: u32 = 1000;

/// Posted to the owner (the Settings dialog) when a row is clicked; `wparam` is the item index.
/// `WM_APP + 1` is the dialog's own message space, not the main window's.
pub(crate) const WM_LIST_PICKED: u32 = WM_APP + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Escape,
    Char(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListOutcome {
    Ignored,
    Moved,
    Picked(usize),
    Dismissed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ListModel {
    pub items: Vec<String>,
    pub selected: usize,
    /// The first visible item.
    pub top: usize,
    typed: String,
    last_typed_at: u32,
}

impl ListModel {
    /// A list with `selected` (or the first item) selected and scrolled into view.
    pub(crate) fn new(items: Vec<String>, selected: Option<usize>) -> Self {
        let selected = selected.unwrap_or(0).min(items.len().saturating_sub(1));
        let mut model = Self {
            items,
            selected,
            top: 0,
            typed: String::new(),
            last_typed_at: 0,
        };
        model.scroll_into_view();
        model
    }

    pub(crate) fn visible_rows(&self) -> usize {
        self.items.len().min(VISIBLE_ROWS)
    }

    /// The item shown in visible row `row`.
    pub(crate) fn item_at_row(&self, row: usize) -> Option<usize> {
        let index = self.top + row;
        (row < VISIBLE_ROWS && index < self.items.len()).then_some(index)
    }

    /// Scrolls by `rows` without moving the selection; true when the view moved.
    pub(crate) fn scroll(&mut self, rows: isize) -> bool {
        let max_top = self.items.len().saturating_sub(VISIBLE_ROWS);
        let top = self.top.saturating_add_signed(rows).min(max_top);
        let moved = top != self.top;
        self.top = top;
        moved
    }

    pub(crate) fn key(&mut self, key: ListKey, now_ms: u32) -> ListOutcome {
        if self.items.is_empty() {
            return match key {
                ListKey::Enter | ListKey::Escape => ListOutcome::Dismissed,
                _ => ListOutcome::Ignored,
            };
        }
        let last = self.items.len() - 1;
        let page = VISIBLE_ROWS - 1;
        match key {
            ListKey::Up => self.select(self.selected.saturating_sub(1)),
            ListKey::Down => self.select((self.selected + 1).min(last)),
            ListKey::PageUp => self.select(self.selected.saturating_sub(page)),
            ListKey::PageDown => self.select((self.selected + page).min(last)),
            ListKey::Home => self.select(0),
            ListKey::End => self.select(last),
            ListKey::Enter => ListOutcome::Picked(self.selected),
            ListKey::Escape => ListOutcome::Dismissed,
            ListKey::Char(c) => self.type_ahead(c, now_ms),
        }
    }

    fn select(&mut self, index: usize) -> ListOutcome {
        if index == self.selected {
            return ListOutcome::Ignored;
        }
        self.selected = index;
        self.scroll_into_view();
        ListOutcome::Moved
    }

    fn scroll_into_view(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + VISIBLE_ROWS {
            self.top = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    /// Jumps to the next item that starts with the typed text. Typing one letter again cycles
    /// through the items starting with it; a longer prefix stays put while it still matches.
    fn type_ahead(&mut self, c: char, now_ms: u32) -> ListOutcome {
        if c.is_control() {
            return ListOutcome::Ignored;
        }
        if now_ms.wrapping_sub(self.last_typed_at) > TYPE_AHEAD_PAUSE_MS {
            self.typed.clear();
        }
        self.last_typed_at = now_ms;
        self.typed.extend(c.to_lowercase());
        let start = if self.typed.chars().count() == 1 {
            self.selected + 1
        } else {
            self.selected
        };
        let count = self.items.len();
        let found = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&index| self.items[index].to_lowercase().starts_with(&self.typed));
        // A prefix that still matches the selected item leaves it selected: `Ignored`.
        found.map_or(ListOutcome::Ignored, |index| self.select(index))
    }
}

/// The open list popup. Dropping it destroys the window.
pub(crate) struct DropdownList {
    hwnd: HWND,
}

struct ListState {
    owner: HWND,
    model: ListModel,
    colors: Palette,
    /// Borrowed from the dialog, which outlives the list.
    font: HFONT,
    row_height: i32,
    hot: Option<usize>,
}

impl DropdownList {
    /// Shows `model` directly under `anchor` (screen coordinates), as wide as it, or above it when
    /// the monitor's work area has no room below. It never takes the activation from `owner`.
    pub(crate) fn show(
        owner: HWND,
        anchor: RECT,
        row_height: i32,
        font: HFONT,
        colors: Palette,
        model: ListModel,
    ) -> Option<Self> {
        let class = register_class()?;
        let height = model.visible_rows().max(1) as i32 * row_height + 2;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let top = unsafe {
            if GetMonitorInfoW(MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST), &mut monitor)
                != 0
                && anchor.bottom + height > monitor.rcWork.bottom
            {
                anchor.top - height
            } else {
                anchor.bottom
            }
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                anchor.left,
                top,
                anchor.right - anchor.left,
                height,
                owner,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let state = Box::new(ListState {
            owner,
            model,
            colors,
            font,
            row_height,
            hot: None,
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            ShowWindow(hwnd, SW_SHOWNA);
        }
        Some(Self { hwnd })
    }

    /// A key the dialog forwarded; repaints when the selection moved.
    pub(crate) fn key(&self, key: ListKey, now_ms: u32) -> ListOutcome {
        let Some(state) = state(self.hwnd) else {
            return ListOutcome::Dismissed;
        };
        let outcome = state.model.key(key, now_ms);
        if outcome == ListOutcome::Moved {
            invalidate(self.hwnd);
        }
        outcome
    }

    /// A mouse wheel turn the dialog forwarded: three rows per notch.
    pub(crate) fn wheel(&self, delta: i16) {
        if let Some(state) = state(self.hwnd)
            && state.model.scroll(-(isize::from(delta) / 120) * 3)
        {
            invalidate(self.hwnd);
        }
    }
}

impl Drop for DropdownList {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadSettingsList"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(list_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut ListState> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListState;
    // SAFETY: set once in `show` from `Box::into_raw` and cleared in WM_NCDESTROY; this thread
    // only, and never two at once.
    unsafe { pointer.as_mut() }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

/// The item under client `y`, one pixel of border above the first row.
fn item_at(state: &ListState, y: i32) -> Option<usize> {
    let row = (y - 1).max(0) / state.row_height.max(1);
    state.model.item_at_row(row as usize)
}

unsafe extern "system" fn list_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let Some(list) = state(hwnd) else {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_PAINT => {
            paint(hwnd, list);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE => {
            let hot = item_at(list, ((lparam >> 16) & 0xffff) as i16 as i32);
            if hot != list.hot {
                list.hot = hot;
                invalidate(hwnd);
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(index) = item_at(list, ((lparam >> 16) & 0xffff) as i16 as i32) {
                // Posted: the dialog destroys this window when it handles the pick.
                unsafe { PostMessageW(list.owner, WM_LIST_PICKED, index, 0) };
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut ListState;
            // SAFETY: from `Box::into_raw` in `show`, released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, list: &ListState) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if dc.is_null() {
        return;
    }
    let colors = list.colors;
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
        fill(dc, client, colors.muted_foreground);
        fill(dc, inset(client, 1), colors.panel_background());
        SetBkMode(dc, TRANSPARENT as i32);
        let previous = SelectObject(dc, list.font as _);
        let text_inset = list.row_height / 3;
        for row in 0..list.model.visible_rows() {
            let Some(index) = list.model.item_at_row(row) else {
                break;
            };
            let top = 1 + row as i32 * list.row_height;
            let rect = RECT {
                left: 1,
                top,
                right: client.right - 1,
                bottom: top + list.row_height,
            };
            let foreground = if index == list.model.selected {
                fill(dc, rect, colors.selection_background);
                colors.selection_foreground.unwrap_or(colors.editor_foreground)
            } else {
                if list.hot == Some(index) {
                    fill(dc, rect, colors.hover_background);
                }
                colors.editor_foreground
            };
            SetTextColor(dc, foreground);
            let mut text = wide_null(&list.model.items[index]);
            let mut text_rect = RECT {
                left: rect.left + text_inset,
                right: rect.right - text_inset,
                ..rect
            };
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                -1,
                &mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
        // A thin thumb shows where the view is in a list longer than it.
        let count = list.model.items.len();
        if count > VISIBLE_ROWS {
            let track = client.bottom - 2;
            let thumb = (track * VISIBLE_ROWS as i32 / count as i32).max(8);
            let top = 1 + (track - thumb) * list.model.top as i32
                / (count - VISIBLE_ROWS).max(1) as i32;
            fill(
                dc,
                RECT {
                    left: client.right - 5,
                    top,
                    right: client.right - 2,
                    bottom: top + thumb,
                },
                colors.muted_foreground,
            );
        }
        SelectObject(dc, previous);
        EndPaint(hwnd, &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_current_value_opens_selected_and_scrolled_into_view() {
        let model = ListModel::new((0..30).map(|i| format!("Font {i}")).collect(), Some(25));
        assert_eq!(model.selected, 25);
        assert_eq!(model.top, 16, "the selected row is the last visible one");
        assert_eq!(ListModel::new(Vec::new(), Some(3)).selected, 0);
        assert_eq!(ListModel::new(items(&["a"]), None).selected, 0);
    }

    #[test]
    fn arrows_pages_and_ends_move_the_selection_and_the_view() {
        let mut model = ListModel::new((0..30).map(|i| format!("Font {i}")).collect(), Some(0));
        assert_eq!(model.key(ListKey::Up, 0), ListOutcome::Ignored);
        assert_eq!(model.key(ListKey::Down, 0), ListOutcome::Moved);
        assert_eq!(model.selected, 1);
        model.key(ListKey::PageDown, 0);
        assert_eq!(model.selected, 10);
        assert_eq!(model.top, 1);
        model.key(ListKey::End, 0);
        assert_eq!((model.selected, model.top), (29, 20));
        model.key(ListKey::Home, 0);
        assert_eq!((model.selected, model.top), (0, 0));
        assert_eq!(model.key(ListKey::Enter, 0), ListOutcome::Picked(0));
        assert_eq!(model.key(ListKey::Escape, 0), ListOutcome::Dismissed);
        assert!(model.scroll(3));
        assert_eq!(model.top, 3);
        assert!(model.scroll(-10));
        assert_eq!(model.top, 0);
        assert!(!model.scroll(-1), "already at the top");
        assert_eq!(model.item_at_row(2), Some(2));
        assert_eq!(model.item_at_row(VISIBLE_ROWS), None);
    }

    #[test]
    fn type_ahead_builds_a_prefix_cycles_one_letter_and_resets_after_a_pause() {
        // Break caught: typing "cas" in a list of hundreds of fonts landing on "Calibri", or a
        // prefix that never resets so the list stops responding to letters.
        let mut model = ListModel::new(
            items(&["Arial", "Calibri", "Cascadia Code", "Cascadia Mono", "Consolas"]),
            Some(0),
        );
        model.key(ListKey::Char('c'), 5_000);
        assert_eq!(model.selected, 1, "Calibri");
        model.key(ListKey::Char('a'), 5_100);
        model.key(ListKey::Char('s'), 5_200);
        assert_eq!(model.selected, 2, "Cascadia Code");
        model.key(ListKey::Char('c'), 9_000);
        assert_eq!(model.selected, 3, "after a pause a single c moves on to the next C item");
        assert_eq!(model.key(ListKey::Char('c'), 9_100), ListOutcome::Ignored);
        assert_eq!(model.selected, 3, "\"cc\" matches nothing and stays put");
        model.key(ListKey::Char('c'), 11_000);
        assert_eq!(model.selected, 4, "after another pause: Consolas");
        assert_eq!(model.key(ListKey::Char('z'), 20_000), ListOutcome::Ignored);
        assert_eq!(model.key(ListKey::Char('\u{8}'), 21_000), ListOutcome::Ignored);
    }
}
```

- [ ] **Step 2: Add the module and run the tests**

Add `pub(crate) mod dropdown_list;` to `src/window/mod.rs`, after `pub(crate) mod drag_label;`. As in Task 4, add `#![allow(dead_code, reason = "used by the settings dialog, added in the next task")]` at the top of the file until Task 7 uses it.

Run: `cargo test --lib window::dropdown_list`
Expected: 3 pass.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. If `MonitorFromRect` or `MONITORINFO` needs a feature that isn't enabled, they're in `Win32_Graphics_Gdi`, which is.

- [ ] **Step 3: Commit**

```bash
git add src/window/dropdown_list.rs src/window/mod.rs
git commit -m "feat: themed dropdown list popup for the Settings dialog"
```

---

### Task 7: The Settings dialog

**Files:**
- Create: `src/window/settings_dialog.rs`
- Modify: `src/window/mod.rs`: add `pub(crate) mod settings_dialog;` after `pub(crate) mod search_view;`.
- Modify: `src/window/main_window.rs`: add `show_settings`, `edit_settings_file` and `settings_file_for_editing`, and tests.
- Modify: `src/window/settings_model.rs` and `src/window/dropdown_list.rs`: remove the temporary `#![allow(dead_code…)]`.

**Interfaces:**
- Consumes:
  - all of `settings_model` (Task 4) and `dropdown_list` (Task 6);
  - `main_window::{apply_settings_action, settings_view, current_palette, link_color}` (Task 5);
  - `platform::fonts::{installed_font_families, dropdown_names}` (Task 3);
  - `modal::ModalScope`, `panel::{fill, inset, scale}` and `titlebar::create_ui_font`.
- Produces:
  - `settings_dialog::show(owner: HWND, colors: Palette, link_color: u32) -> Outcome`, where `pub(crate) enum Outcome { Closed, EditIni }`;
  - `settings_dialog::Layout` (pure), with `calculate(dpi: u32, max_height: i32, link_width: i32)`, `row_rect`, `control_rect`, `segment_rects`, `stepper_rects`, `hit`, `max_scroll`, `scroll_to_show` and `list_row_height`;
  - `settings_dialog::{Hit, Part}`;
  - test hooks `settings_dialog::answer_next(impl FnOnce(HWND) + 'static)` and `settings_dialog::take_focus_checks() -> Vec<bool>`;
  - `main_window::show_settings(hwnd: HWND)` (`pub(crate)`) and `main_window::edit_settings_file(hwnd: HWND)` (`pub(crate)`).

- [ ] **Step 1: Write the dialog module.** Create `src/window/settings_dialog.rs`:

```rust
//! Settings: a themed, owner-drawn modal popup listing every user-facing `fastpad.ini` setting
//! (settings dialog spec §3). Each change applies and saves at once through
//! `main_window::apply_settings_action`. Like About, it runs its own modal loop with the main
//! window disabled. All behaviour lives in `settings_model`; this module decodes input and
//! paints.

use super::dropdown_list::{DropdownList, ListKey, ListModel, ListOutcome, WM_LIST_PICKED};
use super::modal::ModalScope;
use super::palette::Palette;
use super::panel::{fill, inset, scale};
use super::settings_model::{
    Control, DialogModel, Effect, Focus, Key, Row, Section, SettingsView, dropdown_action,
    step_font_size,
};
use super::titlebar::create_ui_font;
use crate::platform::wide_null;
use std::cell::Cell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, EndPaint, FW_NORMAL,
    FW_SEMIBOLD, GetDC, GetMonitorInfoW, HDC, HFONT, IntersectClipRect, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints, MonitorFromWindow, PAINTSTRUCT,
    ReleaseDC, RestoreDC, SaveDC, ScreenToClient, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE,
    TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GW_OWNER,
    GWLP_USERDATA, GetClientRect, GetCursorPos, GetMessageW, GetWindow, GetWindowLongPtrW,
    GetWindowRect, HCURSOR, HTCAPTION, HTCLIENT, IDC_ARROW, IDC_HAND, IsWindow, LoadCursorW, MSG,
    PostQuitMessage, RegisterClassW, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WA_INACTIVE, WM_ACTIVATE,
    WM_CHAR, WM_CLOSE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_SYSKEYDOWN, WNDCLASSW,
    WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_POPUP,
};

const TITLE: &str = "Settings";
const EDIT_INI_LABEL: &str = "Edit fastpad.ini";
const CLOSE_LABEL: &str = "Close";
const AUTOSAVE_HINT: &str = "Open a notebook to change this";
const GLYPH_FONT: &str = "Segoe MDL2 Assets";
const GLYPH_CHECK: &str = "\u{E73E}";
const GLYPH_CHEVRON_DOWN: &str = "\u{E70D}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_REMOVE: &str = "\u{E738}";
const GLYPH_CLOSE: &str = "\u{E8BB}";

const WIDTH_AT_96_DPI: i32 = 520;
const PADDING_AT_96_DPI: i32 = 20;
const TITLE_HEIGHT_AT_96_DPI: i32 = 44;
const HEADING_HEIGHT_AT_96_DPI: i32 = 30;
const ROW_HEIGHT_AT_96_DPI: i32 = 32;
const CONTROL_HEIGHT_AT_96_DPI: i32 = 26;
const DROPDOWN_WIDTH_AT_96_DPI: i32 = 240;
const SEGMENT_WIDTH_AT_96_DPI: i32 = 80;
const TAB_SEGMENT_WIDTH_AT_96_DPI: i32 = 44;
const STEP_BUTTON_AT_96_DPI: i32 = 28;
const STEP_VALUE_AT_96_DPI: i32 = 48;
const CHECK_AT_96_DPI: i32 = 18;
const CONTENT_BOTTOM_GAP_AT_96_DPI: i32 = 8;
const FOOTER_HEIGHT_AT_96_DPI: i32 = 56;
const BUTTON_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;
/// However short the screen, at least this many rows stay visible.
const MIN_VISIBLE_ROWS: i32 = 3;

/// How the dialog was closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Closed,
    /// The Edit fastpad.ini link: the caller opens the file now that the dialog is gone.
    EditIni,
}

/// Which part of a row the pointer is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Part {
    Whole,
    Segment(usize),
    Minus,
    Value,
    Plus,
}

/// What the pointer is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Hit {
    Row(Row, Part),
    EditIni,
    Close,
    TitleClose,
}

/// Where everything sits. Headings and rows are in content coordinates, placed in the
/// scrolling `body` by `row_rect`/`heading_rect`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title: RECT,
    pub title_close: RECT,
    pub body: RECT,
    pub headings: [RECT; 3],
    pub rows: [RECT; 13],
    pub content_height: i32,
    pub edit_ini: RECT,
    pub close: RECT,
    dpi: u32,
}

impl Layout {
    /// The layout at `dpi`, at most `max_height` tall (the work area), with the Edit fastpad.ini
    /// link `link_width` wide.
    pub(crate) fn calculate(dpi: u32, max_height: i32, link_width: i32) -> Self {
        let width = scale(WIDTH_AT_96_DPI, dpi);
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let title_height = scale(TITLE_HEIGHT_AT_96_DPI, dpi);
        let heading_height = scale(HEADING_HEIGHT_AT_96_DPI, dpi);
        let row_height = scale(ROW_HEIGHT_AT_96_DPI, dpi);
        let footer_height = scale(FOOTER_HEIGHT_AT_96_DPI, dpi);

        let title = RECT {
            left: padding,
            top: 0,
            right: width - title_height,
            bottom: title_height,
        };
        let title_close = RECT {
            left: width - title_height,
            top: 0,
            right: width,
            bottom: title_height,
        };

        let line = |top: i32, height: i32| RECT {
            left: padding,
            top,
            right: width - padding,
            bottom: top + height,
        };
        let mut headings = [RECT::default(); 3];
        let mut rows = [RECT::default(); 13];
        let mut top = 0;
        for section in Section::ALL {
            headings[section as usize] = line(top, heading_height);
            top += heading_height;
            for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
                rows[row as usize] = line(top, row_height);
                top += row_height;
            }
        }
        let content_height = top + scale(CONTENT_BOTTOM_GAP_AT_96_DPI, dpi);

        let smallest = title_height + row_height * MIN_VISIBLE_ROWS + footer_height;
        let height = (title_height + content_height + footer_height).min(max_height.max(smallest));
        let body = RECT {
            left: 0,
            top: title_height,
            right: width,
            bottom: height - footer_height,
        };
        let button_height = scale(BUTTON_HEIGHT_AT_96_DPI, dpi);
        let button_top = body.bottom + (footer_height - button_height) / 2;
        let close = RECT {
            left: width - padding - scale(BUTTON_WIDTH_AT_96_DPI, dpi),
            top: button_top,
            right: width - padding,
            bottom: button_top + button_height,
        };
        let edit_ini = RECT {
            left: padding,
            top: button_top,
            right: (padding + link_width).min(close.left),
            bottom: button_top + button_height,
        };
        Self {
            width,
            height,
            title,
            title_close,
            body,
            headings,
            rows,
            content_height,
            edit_ini,
            close,
            dpi,
        }
    }

    pub(crate) fn max_scroll(&self) -> i32 {
        (self.content_height - (self.body.bottom - self.body.top)).max(0)
    }

    pub(crate) fn list_row_height(&self) -> i32 {
        scale(CONTROL_HEIGHT_AT_96_DPI, self.dpi)
    }

    fn place(&self, rect: RECT, scroll: i32) -> RECT {
        let offset = self.body.top - scroll;
        RECT {
            top: rect.top + offset,
            bottom: rect.bottom + offset,
            ..rect
        }
    }

    /// Row `row`'s full rect in client coordinates at `scroll`.
    pub(crate) fn row_rect(&self, row: Row, scroll: i32) -> RECT {
        self.place(self.rows[row as usize], scroll)
    }

    pub(crate) fn heading_rect(&self, section: Section, scroll: i32) -> RECT {
        self.place(self.headings[section as usize], scroll)
    }

    /// The control of `row`, right-aligned in `row_rect`. `segments` is how many a segmented
    /// row shows.
    pub(crate) fn control_rect(&self, row: Row, row_rect: RECT, segments: usize) -> RECT {
        let dpi = self.dpi;
        let (width, height) = match row.control() {
            Control::Dropdown => (
                scale(DROPDOWN_WIDTH_AT_96_DPI, dpi),
                scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
            ),
            Control::Segmented => {
                let each = if row == Row::TabWidth {
                    TAB_SEGMENT_WIDTH_AT_96_DPI
                } else {
                    SEGMENT_WIDTH_AT_96_DPI
                };
                (
                    segments as i32 * scale(each, dpi),
                    scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
                )
            }
            Control::Stepper => (
                scale(STEP_BUTTON_AT_96_DPI, dpi) * 2 + scale(STEP_VALUE_AT_96_DPI, dpi),
                scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
            ),
            Control::Check => (scale(CHECK_AT_96_DPI, dpi), scale(CHECK_AT_96_DPI, dpi)),
        };
        let top = row_rect.top + (row_rect.bottom - row_rect.top - height) / 2;
        RECT {
            left: row_rect.right - width,
            top,
            right: row_rect.right,
            bottom: top + height,
        }
    }

    /// A segmented control's segments, left to right, sharing its width.
    pub(crate) fn segment_rects(&self, control: RECT, segments: usize) -> Vec<RECT> {
        let count = segments.max(1) as i32;
        let each = (control.right - control.left) / count;
        (0..count)
            .map(|index| RECT {
                left: control.left + index * each,
                right: if index == count - 1 {
                    control.right
                } else {
                    control.left + (index + 1) * each
                },
                ..control
            })
            .collect()
    }

    /// The stepper's –, value and + parts.
    pub(crate) fn stepper_rects(&self, control: RECT) -> [RECT; 3] {
        let button = scale(STEP_BUTTON_AT_96_DPI, self.dpi);
        [
            RECT {
                right: control.left + button,
                ..control
            },
            RECT {
                left: control.left + button,
                right: control.right - button,
                ..control
            },
            RECT {
                left: control.right - button,
                ..control
            },
        ]
    }

    /// What client point `x`, `y` is on at `scroll`. A checkbox row is hit anywhere, label
    /// included. Other rows are hit only on their control.
    pub(crate) fn hit(&self, x: i32, y: i32, scroll: i32, view: &SettingsView) -> Option<Hit> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.title_close) {
            return Some(Hit::TitleClose);
        }
        if inside(&self.close) {
            return Some(Hit::Close);
        }
        if inside(&self.edit_ini) {
            return Some(Hit::EditIni);
        }
        if !inside(&self.body) {
            return None;
        }
        let row = Row::ALL
            .into_iter()
            .find(|row| inside(&self.row_rect(*row, scroll)))?;
        let segments = view.segments(row).len();
        let control = self.control_rect(row, self.row_rect(row, scroll), segments);
        let part = match row.control() {
            Control::Check => Some(Part::Whole),
            Control::Dropdown => inside(&control).then_some(Part::Whole),
            Control::Segmented => self
                .segment_rects(control, segments)
                .iter()
                .position(|rect| inside(rect))
                .map(Part::Segment),
            Control::Stepper => {
                let [minus, value, plus] = self.stepper_rects(control);
                [(minus, Part::Minus), (value, Part::Value), (plus, Part::Plus)]
                    .into_iter()
                    .find(|(rect, _)| inside(rect))
                    .map(|(_, part)| part)
            }
        };
        part.map(|part| Hit::Row(row, part))
    }

    /// The scroll from `scroll` that shows `focus`'s row whole. A section's first row brings its
    /// heading into view too.
    pub(crate) fn scroll_to_show(&self, focus: Focus, scroll: i32) -> i32 {
        let Focus::Row(row) = focus else {
            return scroll;
        };
        let rect = self.rows[row as usize];
        let first_in_section = Row::ALL
            .into_iter()
            .find(|candidate| candidate.section() == row.section())
            == Some(row);
        let top = if first_in_section {
            self.headings[row.section() as usize].top
        } else {
            rect.top
        };
        let visible = self.body.bottom - self.body.top;
        let scroll = if top < scroll {
            top
        } else if rect.bottom > scroll + visible {
            rect.bottom - visible
        } else {
            scroll
        };
        scroll.clamp(0, self.max_scroll())
    }
}

/// The open dialog's state, owned by its window through `GWLP_USERDATA`.
struct Dialog {
    colors: Palette,
    link_color: u32,
    layout: Layout,
    view: SettingsView,
    model: DialogModel,
    fonts: Vec<String>,
    scroll: i32,
    title_font: HFONT,
    heading_font: HFONT,
    body_font: HFONT,
    link_font: HFONT,
    glyph_font: HFONT,
    hot: Option<Hit>,
    pressed: Option<Hit>,
    tracking_leave: bool,
    /// The open dropdown and its row. Dropped before the fonts it borrows.
    list: Option<(Row, DropdownList)>,
    outcome: Rc<Cell<Outcome>>,
}

impl Drop for Dialog {
    fn drop(&mut self) {
        self.list = None;
        unsafe {
            for font in [
                self.title_font,
                self.heading_font,
                self.body_font,
                self.link_font,
                self.glyph_font,
            ] {
                DeleteObject(font as _);
            }
        }
    }
}

/// Shows Settings over `owner` and returns once it is closed.
pub(crate) fn show(owner: HWND, colors: Palette, link_color: u32) -> Outcome {
    let _modal = ModalScope::enter(owner);
    let outcome = Rc::new(Cell::new(Outcome::Closed));
    let Some(dialog) = create(owner, colors, link_color, outcome.clone()) else {
        return Outcome::Closed;
    };
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(dialog, SW_SHOW);
        SetFocus(dialog);
    }
    #[cfg(test)]
    {
        let answer = ANSWERS
            .with(|answers| answers.borrow_mut().pop_front())
            .expect("a Settings dialog opened in a test without answer_next");
        answer(dialog);
    }
    let mut message = MSG::default();
    while unsafe { IsWindow(dialog) } != 0 {
        match unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } {
            0 => {
                // WM_QUIT belongs to the outer loop: put it back for that loop to see.
                unsafe { PostQuitMessage(message.wParam as i32) };
                break;
            }
            -1 => break,
            _ => unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
    close(dialog);
    unsafe { EnableWindow(owner, 1) };
    outcome.get()
}

fn create(
    owner: HWND,
    colors: Palette,
    link_color: u32,
    outcome: Rc<Cell<Outcome>>,
) -> Option<HWND> {
    let class = register_class()?;
    let title = wide_null(TITLE);
    let dialog = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
            0,
            0,
            0,
            0,
            owner,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if dialog.is_null() {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    let title_font = create_ui_font(scale(18, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
    let heading_font = create_ui_font(scale(12, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
    let body_font = create_ui_font(scale(13, dpi), "Segoe UI", FW_NORMAL as i32, false);
    let link_font = create_underlined_font(scale(13, dpi));
    let glyph_font = create_ui_font(scale(11, dpi), GLYPH_FONT, FW_NORMAL as i32, false);

    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let mut frame = RECT::default();
    unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        GetWindowRect(owner, &mut frame);
    }
    let work = monitor.rcWork;
    let max_height = if work.bottom > work.top {
        work.bottom - work.top
    } else {
        i32::MAX
    };
    let layout = Layout::calculate(dpi, max_height, measure(dialog, link_font, EDIT_INI_LABEL));
    let view = super::main_window::settings_view(owner);
    let fonts = crate::platform::fonts::dropdown_names(
        crate::platform::fonts::installed_font_families(),
        &view.settings.font_face,
    );
    let state = Box::new(Dialog {
        colors,
        link_color,
        layout,
        view,
        model: DialogModel::new(),
        fonts,
        scroll: 0,
        title_font,
        heading_font,
        body_font,
        link_font,
        glyph_font,
        hot: None,
        pressed: None,
        tracking_leave: false,
        list: None,
        outcome,
    });
    unsafe { SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize) };

    // Centered over the owner, kept inside the work area, with rounded corners where Windows
    // 11 draws them.
    let mut left = frame.left + (frame.right - frame.left - layout.width) / 2;
    let mut top = frame.top + (frame.bottom - frame.top - layout.height) / 2;
    if work.bottom > work.top {
        left = left.clamp(work.left, (work.right - layout.width).max(work.left));
        top = top.clamp(work.top, (work.bottom - layout.height).max(work.top));
    }
    unsafe {
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            left,
            top,
            layout.width,
            layout.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let corners = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            dialog,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const corners).cast(),
            std::mem::size_of_val(&corners) as u32,
        );
    }
    Some(dialog)
}

/// Re-enables the owner before the dialog goes, so Windows hands activation back to it rather
/// than to some other application.
fn close(dialog: HWND) {
    if unsafe { IsWindow(dialog) } == 0 {
        return;
    }
    unsafe {
        EnableWindow(owner(dialog), 1);
        DestroyWindow(dialog);
    }
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadSettings"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(dialog_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

fn create_underlined_font(pixel_height: i32) -> HFONT {
    use windows_sys::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        OUT_DEFAULT_PRECIS,
    };
    let face = wide_null("Segoe UI");
    unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            1,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            u32::from(DEFAULT_PITCH),
            face.as_ptr(),
        )
    }
}

fn measure(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    unsafe {
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return 0;
        }
        let previous = SelectObject(dc, font as _);
        let mut text = wide_null(text);
        let mut rect = RECT::default();
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        ReleaseDC(hwnd, dc);
        rect.right - rect.left
    }
}

/// The main window; read from the window rather than `Dialog`, which the caller may be
/// borrowing.
fn owner(dialog: HWND) -> HWND {
    unsafe { GetWindow(dialog, GW_OWNER) }
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut Dialog> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Dialog;
    // SAFETY: set once in `create` from `Box::into_raw` and cleared in WM_NCDESTROY. This thread
    // only; callers end one borrow before anything that can re-enter the window procedure
    // (see `run`).
    unsafe { pointer.as_mut() }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam & 0xffff) as i16 as i32,
        ((lparam >> 16) & 0xffff) as i16 as i32,
    )
}

/// Carries out `effect`. No `Dialog` borrow may be alive here: applying a setting runs main
/// window code that can move the focus and re-enter this window's procedure.
fn run(hwnd: HWND, effect: Effect) {
    match effect {
        Effect::None => {}
        Effect::Repaint => {
            if let Some(dialog) = state(hwnd) {
                dialog.scroll = dialog
                    .layout
                    .scroll_to_show(dialog.model.focus, dialog.scroll);
            }
            invalidate(hwnd);
        }
        Effect::Apply(action) => {
            super::main_window::apply_settings_action(owner(hwnd), action);
            // Some changes move the focus (notes mode rebuilds the sidebar); the keyboard stays
            // here.
            if unsafe { GetFocus() } != hwnd {
                unsafe { SetFocus(hwnd) };
            }
            #[cfg(test)]
            FOCUS_AFTER_APPLY
                .with(|checks| checks.borrow_mut().push(unsafe { GetFocus() } == hwnd));
            refresh(hwnd);
        }
        Effect::OpenDropdown(row) => open_list(hwnd, row),
        Effect::EditIni => {
            if let Some(dialog) = state(hwnd) {
                dialog.outcome.set(Outcome::EditIni);
            }
            close(hwnd);
        }
        Effect::Close => close(hwnd),
    }
}

/// Re-reads what the dialog shows after a change: the settings, the notebook's switch, and the
/// theme's colours, which a theme change replaced.
fn refresh(hwnd: HWND) {
    let owner = owner(hwnd);
    let view = super::main_window::settings_view(owner);
    let colors = super::main_window::current_palette(owner);
    let link_color = super::main_window::link_color(owner);
    if let Some(dialog) = state(hwnd) {
        dialog.view = view;
        dialog.colors = colors;
        dialog.link_color = link_color;
        dialog.scroll = dialog
            .layout
            .scroll_to_show(dialog.model.focus, dialog.scroll);
    }
    invalidate(hwnd);
}

fn open_list(hwnd: HWND, row: Row) {
    let Some(dialog) = state(hwnd) else {
        return;
    };
    dialog.list = None;
    let (items, selected) = dialog.view.dropdown(row, &dialog.fonts);
    let control = dialog
        .layout
        .control_rect(row, dialog.layout.row_rect(row, dialog.scroll), 0);
    let mut anchor = control;
    unsafe {
        MapWindowPoints(
            hwnd,
            std::ptr::null_mut(),
            (&raw mut anchor).cast::<POINT>(),
            2,
        );
    }
    dialog.list = DropdownList::show(
        hwnd,
        anchor,
        dialog.layout.list_row_height(),
        dialog.body_font,
        dialog.colors,
        ListModel::new(items, selected),
    )
    .map(|list| (row, list));
    invalidate(hwnd);
}

/// Closes the open dropdown, if any; true when one was open.
fn close_list(hwnd: HWND) -> bool {
    let closed = state(hwnd).and_then(|dialog| dialog.list.take()).is_some();
    if closed {
        invalidate(hwnd);
    }
    closed
}

/// Picks item `index` of the open dropdown.
fn pick(hwnd: HWND, index: usize) {
    let action = state(hwnd).and_then(|dialog| {
        let (row, _) = dialog.list.take()?;
        dropdown_action(row, index, &dialog.fonts)
    });
    invalidate(hwnd);
    if let Some(action) = action {
        run(hwnd, Effect::Apply(action));
    }
}

/// What releasing the mouse on `hit` does.
fn click_effect(dialog: &mut Dialog, hit: Hit) -> Effect {
    let view = &dialog.view;
    match hit {
        Hit::Close | Hit::TitleClose => Effect::Close,
        Hit::EditIni => Effect::EditIni,
        Hit::Row(row, _) if !view.enabled(row) => Effect::None,
        Hit::Row(row, Part::Whole) => match (row.control(), row.toggle()) {
            (Control::Check, Some(toggle)) => {
                Effect::Apply(super::settings_model::SettingsAction::Toggle(toggle))
            }
            (Control::Dropdown, _) => Effect::OpenDropdown(row),
            _ => Effect::None,
        },
        Hit::Row(row, Part::Segment(index)) => view
            .segment_action(row, index)
            .map_or(Effect::None, Effect::Apply),
        Hit::Row(_, part @ (Part::Minus | Part::Plus)) => {
            dialog.model.typed = None;
            let current = view.settings.font_size;
            let size = step_font_size(current, part == Part::Plus);
            if size == current {
                Effect::Repaint
            } else {
                Effect::Apply(super::settings_model::SettingsAction::SetFontSize(size))
            }
        }
        Hit::Row(_, Part::Value) => Effect::Repaint,
    }
}

/// The focus a click on `hit` moves to.
fn focus_of(hit: Hit) -> Focus {
    match hit {
        Hit::Row(row, _) => Focus::Row(row),
        Hit::EditIni => Focus::EditIni,
        Hit::Close | Hit::TitleClose => Focus::Close,
    }
}

fn list_key(virtual_key: u16) -> Option<ListKey> {
    Some(match virtual_key {
        VK_UP => ListKey::Up,
        VK_DOWN => ListKey::Down,
        VK_PRIOR => ListKey::PageUp,
        VK_NEXT => ListKey::PageDown,
        VK_HOME => ListKey::Home,
        VK_END => ListKey::End,
        VK_RETURN => ListKey::Enter,
        VK_ESCAPE => ListKey::Escape,
        _ => return None,
    })
}

fn model_key(virtual_key: u16) -> Option<Key> {
    Some(match virtual_key {
        VK_TAB => Key::Tab {
            back: unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0,
        },
        VK_SPACE => Key::Space,
        VK_RETURN => Key::Enter,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_ESCAPE => Key::Escape,
        _ => return None,
    })
}

fn key_down(hwnd: HWND, virtual_key: u16) {
    // With a dropdown open, its keys go to the list; Tab closes it and moves on.
    let list_outcome = state(hwnd).and_then(|dialog| {
        let (_, list) = dialog.list.as_ref()?;
        let key = list_key(virtual_key)?;
        Some(list.key(key, unsafe { GetTickCount() }))
    });
    match list_outcome {
        Some(ListOutcome::Picked(index)) => return pick(hwnd, index),
        Some(ListOutcome::Dismissed) => {
            close_list(hwnd);
            return;
        }
        Some(_) => return,
        None => {
            if virtual_key == VK_TAB {
                close_list(hwnd);
            }
        }
    }
    let Some(key) = model_key(virtual_key) else {
        return;
    };
    let effect = state(hwnd).map(|dialog| dialog.model.key(key, &dialog.view));
    if let Some(effect) = effect {
        run(hwnd, effect);
    }
}

fn char_typed(hwnd: HWND, c: char) {
    let now = unsafe { GetTickCount() };
    let in_list = state(hwnd).and_then(|dialog| {
        let (_, list) = dialog.list.as_ref()?;
        Some(list.key(ListKey::Char(c), now))
    });
    if in_list.is_some() {
        return;
    }
    let key = match c {
        '\u{8}' => Key::Backspace,
        c if c.is_control() => return,
        c => Key::Char(c),
    };
    let effect = state(hwnd).map(|dialog| dialog.model.key(key, &dialog.view));
    if let Some(effect) = effect {
        run(hwnd, effect);
    }
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if state(hwnd).is_none() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_PAINT => {
            if let Some(dialog) = state(hwnd) {
                paint(hwnd, dialog);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_KEYDOWN => {
            key_down(hwnd, wparam as u16);
            0
        }
        // Alt+Down opens a dropdown, as in a combo box.
        WM_SYSKEYDOWN if wparam as u16 == VK_DOWN => {
            let effect = state(hwnd).and_then(|dialog| {
                dialog.list.is_none().then(|| {
                    dialog.model.key(Key::AltDown, &dialog.view)
                })
            });
            if let Some(effect) = effect {
                run(hwnd, effect);
            }
            0
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam as u32) {
                char_typed(hwnd, c);
            }
            0
        }
        WM_LIST_PICKED => {
            pick(hwnd, wparam);
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xffff) as u32 == WA_INACTIVE {
                close_list(hwnd);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as i16;
            if let Some(dialog) = state(hwnd) {
                if let Some((_, list)) = &dialog.list {
                    list.wheel(delta);
                } else {
                    let row_height = scale(ROW_HEIGHT_AT_96_DPI, dialog.layout.dpi);
                    let scroll = (dialog.scroll - i32::from(delta) * row_height / 40)
                        .clamp(0, dialog.layout.max_scroll());
                    if scroll != dialog.scroll {
                        dialog.scroll = scroll;
                        invalidate(hwnd);
                    }
                }
            }
            0
        }
        // Only the title row drags the dialog.
        WM_NCHITTEST => {
            let (x, y) = lparam_point(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let caption = state(hwnd).is_some_and(|dialog| {
                point.y < dialog.layout.title.bottom
                    && point.x < dialog.layout.title_close.left
            });
            if caption {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_SETCURSOR => {
            let mut point = POINT::default();
            unsafe {
                GetCursorPos(&mut point);
                ScreenToClient(hwnd, &mut point);
            }
            let on_link = state(hwnd).is_some_and(|dialog| {
                dialog.layout.hit(point.x, point.y, dialog.scroll, &dialog.view)
                    == Some(Hit::EditIni)
            });
            let cursor = if on_link { IDC_HAND } else { IDC_ARROW };
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor) as HCURSOR) };
            1
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(dialog) = state(hwnd) {
                let hot = dialog.layout.hit(x, y, dialog.scroll, &dialog.view);
                if !dialog.tracking_leave {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    dialog.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
                }
                if hot != dialog.hot {
                    dialog.hot = hot;
                    invalidate(hwnd);
                }
            }
            0
        }
        windows_sys::Win32::UI::Controls::WM_MOUSELEAVE => {
            if let Some(dialog) = state(hwnd) {
                dialog.tracking_leave = false;
                if dialog.hot.take().is_some() {
                    invalidate(hwnd);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            let open_row = state(hwnd).and_then(|dialog| dialog.list.as_ref().map(|(row, _)| *row));
            close_list(hwnd);
            let effect = state(hwnd).and_then(|dialog| {
                dialog.pressed = None;
                let hit = dialog.layout.hit(x, y, dialog.scroll, &dialog.view)?;
                // A click on the dropdown whose list was open only closes that list.
                dialog.pressed = (open_row.map(|row| Hit::Row(row, Part::Whole)) != Some(hit))
                    .then_some(hit);
                Some(dialog.model.set_focus(focus_of(hit), &dialog.view))
            });
            if let Some(effect) = effect {
                unsafe { SetCapture(hwnd) };
                run(hwnd, effect);
            }
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            unsafe { ReleaseCapture() };
            let effect = state(hwnd).and_then(|dialog| {
                let pressed = dialog.pressed.take()?;
                (dialog.layout.hit(x, y, dialog.scroll, &dialog.view) == Some(pressed))
                    .then(|| click_effect(dialog, pressed))
            });
            invalidate(hwnd);
            if let Some(effect) = effect {
                run(hwnd, effect);
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut Dialog;
            // SAFETY: from `Box::into_raw` in `create`, released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, dialog: &Dialog) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if dc.is_null() {
        return;
    }
    let colors = dialog.colors;
    let layout = &dialog.layout;
    let view = &dialog.view;
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
        fill(dc, client, colors.muted_foreground);
        fill(dc, inset(client, 1), colors.panel_background());
        SetBkMode(dc, TRANSPARENT as i32);

        // Title row: the name and the × button.
        draw_text(dc, dialog.title_font, colors.editor_foreground, TITLE, layout.title, DT_LEFT);
        let close_hot = dialog.hot == Some(Hit::TitleClose);
        if close_hot {
            fill(dc, inset(layout.title_close, 1), colors.close_hover_background);
        }
        draw_text(
            dc,
            dialog.glyph_font,
            if close_hot {
                colors.close_hover_foreground
            } else {
                colors.muted_foreground
            },
            GLYPH_CLOSE,
            layout.title_close,
            DT_CENTER,
        );
        let rule = |top: i32| RECT {
            left: 1,
            top,
            right: client.right - 1,
            bottom: top + 1,
        };
        fill(dc, rule(layout.body.top - 1), colors.strip_background);
        fill(dc, rule(layout.body.bottom), colors.strip_background);

        // The scrolling body, clipped to its area.
        let saved = SaveDC(dc);
        IntersectClipRect(
            dc,
            layout.body.left,
            layout.body.top,
            layout.body.right,
            layout.body.bottom,
        );
        for section in Section::ALL {
            draw_text(
                dc,
                dialog.heading_font,
                colors.muted_foreground,
                &section.title().to_uppercase(),
                layout.heading_rect(section, dialog.scroll),
                DT_LEFT,
            );
        }
        for row in Row::ALL {
            paint_row(dc, dialog, row);
        }
        RestoreDC(dc, saved);

        // Footer: the link and Close.
        draw_text(
            dc,
            dialog.link_font,
            dialog.link_color,
            EDIT_INI_LABEL,
            layout.edit_ini,
            DT_LEFT,
        );
        let button = match (dialog.pressed, dialog.hot) {
            (Some(Hit::Close), _) => colors.pressed_background,
            (_, Some(Hit::Close)) => colors.hover_background,
            _ => colors.strip_background,
        };
        fill(dc, layout.close, colors.muted_foreground);
        fill(dc, inset(layout.close, 1), button);
        draw_text(
            dc,
            dialog.body_font,
            colors.editor_foreground,
            CLOSE_LABEL,
            layout.close,
            DT_CENTER,
        );

        // The focus ring.
        let ring = match dialog.model.focus {
            Focus::EditIni => Some(inset(layout.edit_ini, -2)),
            Focus::Close => Some(inset(layout.close, scale(3, layout.dpi))),
            Focus::Row(row) => {
                let row_rect = layout.row_rect(row, dialog.scroll);
                let visible = row_rect.top >= layout.body.top && row_rect.bottom <= layout.body.bottom;
                visible.then(|| {
                    if row.control() == Control::Check {
                        inset(row_rect, 1)
                    } else {
                        inset(
                            layout.control_rect(row, row_rect, view.segments(row).len()),
                            -2,
                        )
                    }
                })
            }
        };
        if let Some(ring) = ring {
            SetTextColor(dc, colors.editor_foreground);
            DrawFocusRect(dc, &ring);
        }
        EndPaint(hwnd, &paint);
    }
}

unsafe fn paint_row(dc: HDC, dialog: &Dialog, row: Row) {
    let colors = dialog.colors;
    let layout = &dialog.layout;
    let view = &dialog.view;
    let row_rect = layout.row_rect(row, dialog.scroll);
    if row_rect.bottom < layout.body.top || row_rect.top > layout.body.bottom {
        return;
    }
    let enabled = view.enabled(row);
    let text = if enabled {
        colors.editor_foreground
    } else {
        colors.muted_foreground
    };
    let segments = view.segments(row);
    let control = layout.control_rect(row, row_rect, segments.len());
    unsafe {
        draw_text(dc, dialog.body_font, text, row.label(), row_rect, DT_LEFT);
        let hot = matches!(dialog.hot, Some(Hit::Row(hot_row, _)) if hot_row == row) && enabled;
        match row.control() {
            Control::Check => {
                let checked = row.toggle().is_some_and(|toggle| view.checked(toggle));
                if !enabled {
                    let hint = RECT {
                        right: control.left - scale(12, layout.dpi),
                        ..row_rect
                    };
                    draw_text(dc, dialog.body_font, colors.muted_foreground, AUTOSAVE_HINT, hint, DT_RIGHT);
                }
                fill(
                    dc,
                    control,
                    if enabled {
                        colors.muted_foreground
                    } else {
                        colors.strip_background
                    },
                );
                let inner = inset(control, 1);
                if checked {
                    fill(dc, inner, colors.selection_background);
                    draw_text(
                        dc,
                        dialog.glyph_font,
                        colors.selection_foreground.unwrap_or(colors.editor_foreground),
                        GLYPH_CHECK,
                        control,
                        DT_CENTER,
                    );
                } else {
                    fill(
                        dc,
                        inner,
                        if hot {
                            colors.hover_background
                        } else {
                            colors.panel_background()
                        },
                    );
                }
            }
            Control::Segmented => {
                fill(dc, control, colors.muted_foreground);
                for (index, (segment, rect)) in segments
                    .iter()
                    .zip(layout.segment_rects(control, segments.len()))
                    .enumerate()
                {
                    let inner = RECT {
                        left: rect.left + i32::from(index == 0),
                        ..inset(rect, 1)
                    };
                    let hot_segment = dialog.hot == Some(Hit::Row(row, Part::Segment(index)));
                    let (background, foreground) = if segment.selected {
                        (
                            colors.selection_background,
                            colors.selection_foreground.unwrap_or(colors.editor_foreground),
                        )
                    } else if hot_segment {
                        (colors.hover_background, colors.editor_foreground)
                    } else {
                        (colors.strip_background, colors.editor_foreground)
                    };
                    fill(dc, inner, background);
                    draw_text(dc, dialog.body_font, foreground, &segment.label, rect, DT_CENTER);
                }
            }
            Control::Dropdown => {
                let open = matches!(&dialog.list, Some((open_row, _)) if *open_row == row);
                fill(dc, control, colors.muted_foreground);
                fill(
                    dc,
                    inset(control, 1),
                    if hot || open {
                        colors.hover_background
                    } else {
                        colors.strip_background
                    },
                );
                let pad = scale(8, layout.dpi);
                let chevron = RECT {
                    left: control.right - scale(26, layout.dpi),
                    ..control
                };
                let value = RECT {
                    left: control.left + pad,
                    right: chevron.left,
                    ..control
                };
                draw_text(dc, dialog.body_font, colors.editor_foreground, &view.dropdown_text(row), value, DT_LEFT);
                draw_text(dc, dialog.glyph_font, colors.muted_foreground, GLYPH_CHEVRON_DOWN, chevron, DT_CENTER);
            }
            Control::Stepper => {
                let [minus, value, plus] = layout.stepper_rects(control);
                fill(dc, control, colors.muted_foreground);
                for (rect, glyph, part) in [(minus, GLYPH_REMOVE, Part::Minus), (plus, GLYPH_ADD, Part::Plus)] {
                    let background = match (dialog.pressed, dialog.hot) {
                        (Some(pressed), _) if pressed == Hit::Row(row, part) => colors.pressed_background,
                        (_, Some(hot)) if hot == Hit::Row(row, part) => colors.hover_background,
                        _ => colors.strip_background,
                    };
                    fill(dc, inset(rect, 1), background);
                    draw_text(dc, dialog.glyph_font, colors.editor_foreground, glyph, rect, DT_CENTER);
                }
                fill(dc, RECT { top: value.top + 1, bottom: value.bottom - 1, ..value }, colors.editor_background);
                draw_text(
                    dc,
                    dialog.body_font,
                    colors.editor_foreground,
                    &dialog.model.font_size_text(view),
                    value,
                    DT_CENTER,
                );
            }
        }
    }
}

unsafe fn draw_text(dc: HDC, font: HFONT, color: u32, text: &str, rect: RECT, align: u32) {
    let mut text = wide_null(text);
    let mut rect = rect;
    unsafe {
        let previous = SelectObject(dc, font as _);
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(dc, previous);
    }
}

#[cfg(test)]
type Answer = Box<dyn FnOnce(HWND)>;

#[cfg(test)]
thread_local! {
    static ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// After each applied change: whether the dialog still had the keyboard focus.
    static FOCUS_AFTER_APPLY: std::cell::RefCell<Vec<bool>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `answer` with the next dialog this thread opens, right after it shows, before its loop.
/// Tests post their input from here.
#[cfg(test)]
pub(crate) fn answer_next(answer: impl FnOnce(HWND) + 'static) {
    ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Whether the dialog kept the focus after each change applied since the last call.
#[cfg(test)]
pub(crate) fn take_focus_checks() -> Vec<bool> {
    FOCUS_AFTER_APPLY.with(|checks| std::mem::take(&mut *checks.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SettingsView {
        SettingsView {
            settings: crate::config::default_settings(),
            notebook_autosave: None,
        }
    }

    fn center(rect: RECT) -> (i32, i32) {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    #[test]
    fn rows_stack_under_their_headings_and_everything_fits_at_96_dpi() {
        // Break caught: rows overlapping, a heading painted under its first row, or a dialog
        // that scrolls on an ordinary screen.
        let layout = Layout::calculate(96, 2000, 100);
        assert_eq!(layout.width, 520);
        assert_eq!(layout.max_scroll(), 0);
        let mut expected_top = 0;
        for section in Section::ALL {
            assert_eq!(layout.headings[section as usize].top, expected_top);
            expected_top = layout.headings[section as usize].bottom;
            for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
                assert_eq!(layout.rows[row as usize].top, expected_top, "{row:?}");
                expected_top = layout.rows[row as usize].bottom;
            }
        }
        assert_eq!(layout.height, 44 + layout.content_height + 56);
        assert!(layout.edit_ini.right <= layout.close.left);
    }

    #[test]
    fn hits_find_controls_checkbox_labels_segments_and_stepper_parts() {
        let layout = Layout::calculate(96, 2000, 100);
        let view = view();
        let hit = |rect: RECT| {
            let (x, y) = center(rect);
            layout.hit(x, y, 0, &view)
        };
        let row = |row| layout.row_rect(row, 0);
        let control = |r: Row| layout.control_rect(r, row(r), view.segments(r).len());

        assert_eq!(hit(control(Row::Theme)), Some(Hit::Row(Row::Theme, Part::Whole)));
        let label = RECT { right: row(Row::Theme).left + 40, ..row(Row::Theme) };
        assert_eq!(hit(label), None, "a dropdown's label is not the dropdown");
        let label = RECT { right: row(Row::WordWrap).left + 40, ..row(Row::WordWrap) };
        assert_eq!(hit(label), Some(Hit::Row(Row::WordWrap, Part::Whole)), "a checkbox's label toggles it");

        let segments = layout.segment_rects(control(Row::TabWidth), 3);
        assert_eq!(hit(segments[2]), Some(Hit::Row(Row::TabWidth, Part::Segment(2))));
        let [minus, value, plus] = layout.stepper_rects(control(Row::FontSize));
        assert_eq!(hit(minus), Some(Hit::Row(Row::FontSize, Part::Minus)));
        assert_eq!(hit(value), Some(Hit::Row(Row::FontSize, Part::Value)));
        assert_eq!(hit(plus), Some(Hit::Row(Row::FontSize, Part::Plus)));

        assert_eq!(hit(layout.close), Some(Hit::Close));
        assert_eq!(hit(layout.edit_ini), Some(Hit::EditIni));
        assert_eq!(hit(layout.title_close), Some(Hit::TitleClose));
        assert_eq!(hit(layout.title), None);
    }

    #[test]
    fn the_layout_scales_with_dpi() {
        let normal = Layout::calculate(96, 4000, 100);
        let double = Layout::calculate(192, 4000, 200);
        assert_eq!(double.width, normal.width * 2);
        assert_eq!(double.content_height, normal.content_height * 2);
        assert_eq!(double.rows[5].top, normal.rows[5].top * 2);
    }

    #[test]
    fn a_short_work_area_caps_the_height_and_scrolls_the_focused_row_into_view() {
        // Break caught: a dialog taller than a 1366×768 screen at 150%, with Close off-screen,
        // or Tab moving the focus to a row the body never scrolls to (review focus 4).
        let layout = Layout::calculate(144, 700, 150);
        assert_eq!(layout.height, 700);
        assert!(layout.max_scroll() > 0);
        let visible = layout.body.bottom - layout.body.top;
        let scroll = layout.scroll_to_show(Focus::Row(Row::NotebookAutosave), 0);
        let bottom = layout.rows[Row::NotebookAutosave as usize].bottom;
        assert_eq!(scroll, bottom - visible, "just enough to show the last row whole");
        assert!(scroll <= layout.max_scroll());
        assert_eq!(
            layout.scroll_to_show(Focus::Row(Row::Theme), scroll),
            0,
            "the first row brings its heading back"
        );
        assert_eq!(layout.scroll_to_show(Focus::Close, 37), 37);
        let tiny = Layout::calculate(96, 100, 100);
        assert!(tiny.height > 100, "at least a few rows always show");
    }
}
```

- [ ] **Step 2: Add `show_settings` and `edit_settings_file` to `main_window.rs`.** Add them after `show_about`:

```rust
/// The Settings dialog: File → Settings…, Ctrl+, and the activity bar's gear (settings dialog
/// spec §4.3).
pub(crate) fn show_settings(hwnd: HWND) {
    let outcome =
        crate::window::settings_dialog::show(hwnd, current_palette(hwnd), link_color(hwnd));
    if outcome == crate::window::settings_dialog::Outcome::EditIni {
        edit_settings_file(hwnd);
    }
}

/// Preferences: Edit fastpad.ini. Creates the file (empty) when it doesn't exist yet, then opens
/// it in a tab through the normal open path (settings dialog spec §3.6).
pub(crate) fn edit_settings_file(hwnd: HWND) {
    let result = settings_file_for_editing().and_then(|path| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        open_path(hwnd, &path)
    });
    if let Err(error) = result {
        push_notice(hwnd, format!("FastPad could not open fastpad.ini: {error}"));
    }
}

#[cfg(not(test))]
fn settings_file_for_editing() -> Result<std::path::PathBuf> {
    crate::config::persisted::settings_file_path()
}

/// Tests open only the file they chose with `save_settings_to`.
#[cfg(test)]
fn settings_file_for_editing() -> Result<std::path::PathBuf> {
    TEST_SETTINGS_PATH
        .with(|path| path.borrow().clone())
        .ok_or(crate::FastPadError::Invariant(
            "a test opened fastpad.ini without save_settings_to",
        ))
}
```

If `?` on `std::io::Error` doesn't convert inside the closure, write `.map_err(crate::FastPadError::from)?`. `save_setting_to` already uses `?` on io errors, so `From` exists.

- [ ] **Step 3: Write the window tests.** Add these to the `main_window` test module, next to the About tests:

```rust
    #[test]
    fn settings_opens_an_owned_modal_dialog_that_escape_closes() {
        // Break caught: a dialog that can hide behind the main window, leaves it disabled after
        // closing, or never ends its modal scope.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, VK_ESCAPE};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GW_OWNER, GetWindow, PostMessageW, WM_KEYDOWN,
        };
        let window = ProductionWindow::new(make_app());
        let owner = window.hwnd;
        let shown = std::rc::Rc::new(std::cell::Cell::new(None));
        let seen = shown.clone();
        crate::window::settings_dialog::answer_next(move |dialog| {
            let owned = unsafe { GetWindow(dialog, GW_OWNER) } == owner;
            let owner_disabled = unsafe { IsWindowEnabled(owner) } == 0;
            let modal = crate::window::modal::modal_active(owner);
            seen.set(Some((dialog, owned, owner_disabled, modal)));
            unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
        });

        super::show_settings(owner);

        let (dialog, owned, owner_disabled, modal) = shown.get().expect("Settings was shown");
        assert!(owned && owner_disabled && modal);
        assert_eq!(unsafe { IsWindow(dialog) }, 0, "Escape closed it");
        assert_ne!(unsafe { IsWindowEnabled(owner) }, 0);
        assert!(!crate::window::modal::modal_active(owner));
    }

    #[test]
    fn the_settings_dialog_changes_settings_from_the_keyboard() {
        // Break caught: arrows or Space that change nothing, a typed font size lost when Tab
        // leaves the field, or changes that aren't saved (settings dialog spec §3.3).
        use crate::config::FileIconSet;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_ESCAPE, VK_RIGHT, VK_SPACE, VK_TAB,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("settings-dialog-keys");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
            let char = |c: char| PostMessageW(dialog, WM_CHAR, c as usize, 0);
            key(VK_TAB); // File icons
            key(VK_RIGHT); // Minimal
            key(VK_TAB); // Font
            key(VK_TAB); // Font size
            char('1');
            char('6');
            key(VK_TAB); // commits 16; Tab width
            key(VK_RIGHT); // 4 → 8
            key(VK_TAB); // Indent with spaces
            key(VK_SPACE);
            key(VK_ESCAPE);
        });

        super::show_settings(window.hwnd);

        let settings = app_mut(window.hwnd).settings.clone();
        assert_eq!(settings.file_icons, FileIconSet::Minimal);
        assert_eq!(settings.font_size, 16);
        assert_eq!(settings.tab_width, 8);
        assert!(settings.insert_spaces);
        super::save_settings_to(None);
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "# kept\r\nfile_icons=minimal\r\nfont_size=16\r\ntab_width=8\r\ninsert_spaces=true\r\n"
        );
    }

    #[test]
    fn the_theme_dropdown_opens_with_enter_and_picks_with_the_keyboard() {
        // Break caught: a dropdown that opens but ignores the arrows, or picks without applying.
        use crate::config::ThemePreference;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
        let scratch = RecoveryScratch::new("settings-dialog-theme");
        super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
        let window = ProductionWindow::new(make_app());
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
            key(VK_RETURN); // opens the Theme list on System
            key(VK_DOWN); // Light
            key(VK_RETURN); // picks it and closes the list
            key(VK_ESCAPE); // closes the dialog
        });

        super::show_settings(window.hwnd);

        assert_eq!(app_mut(window.hwnd).settings.theme, ThemePreference::Light);
        super::save_settings_to(None);
    }

    #[test]
    fn the_dialog_keeps_the_focus_after_a_change_that_moves_it() {
        // Break caught: switching notes mode off from the dialog tears down the sidebar, the
        // focus lands in the main window, and the dialog stops answering the keyboard (review
        // focus 1). Posted test keys reach the dialog whatever the focus, so the dialog records
        // the focus after each change and the test checks that record.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_SPACE, VK_TAB};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("settings-dialog-focus");
        super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::settings_dialog::take_focus_checks();
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
            // Theme → … → Notes mode is the 11th row: ten Tabs.
            for _ in 0..10 {
                key(VK_TAB);
            }
            key(VK_SPACE); // notes mode off
            key(VK_SPACE); // and on again
            key(VK_ESCAPE);
        });

        super::show_settings(window.hwnd);

        assert!(app_mut(window.hwnd).settings.notes_mode, "both toggles ran");
        assert_eq!(
            crate::window::settings_dialog::take_focus_checks(),
            [true, true],
            "the dialog had the keyboard after each change"
        );
        super::save_settings_to(None);
    }
```

Add the Edit fastpad.ini test:

```rust
    #[test]
    fn edit_fastpad_ini_closes_the_dialog_and_opens_the_file_in_a_tab() {
        // Break caught: the link doing nothing when fastpad.ini doesn't exist yet, or opening it
        // under the still-modal dialog (settings dialog spec §3.6).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("settings-dialog-edit-ini");
        let ini = scratch.path().join("FastPad").join("fastpad.ini");
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::settings_dialog::answer_next(|dialog| unsafe {
            // 12 enabled rows (no notebook, so autosave is skipped): 12 Tabs reach the link.
            for _ in 0..12 {
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_TAB), 0);
            }
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
        });

        super::show_settings(window.hwnd);

        assert!(ini.exists(), "created when missing");
        assert!(app_mut(window.hwnd).tabs.find_path(&ini).is_some(), "opened in a tab");
        super::save_settings_to(None);
    }
```

Remove the `#![allow(dead_code…)]` lines added in Tasks 4 and 6.

- [ ] **Step 4: Add the module and run the targeted tests**

Add `pub(crate) mod settings_dialog;` to `src/window/mod.rs`, after `pub(crate) mod search_view;`.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. Fix any unused imports clippy reports: the import lists above are generous, so drop the unused ones rather than silencing them.

Run: `cargo test --lib -- --test-threads=1 settings_dialog settings_opens the_settings_dialog the_theme_dropdown the_dialog_keeps edit_fastpad_ini`
Expected: all pass.

If `RecoveryScratch::new` requires the directory to exist before `create_dir_all`, the test's nested `FastPad` folder exercises the creation path. That is intended.

- [ ] **Step 5: Commit**

```bash
git add src/window/settings_dialog.rs src/window/settings_model.rs src/window/dropdown_list.rs src/window/mod.rs src/window/main_window.rs
git commit -m "feat: themed Settings dialog with live, saved changes"
```

---

### Task 8: Entry points, and retiring the filtered Settings palette

**Files:**
- Modify: `src/window/commands.rs`: `OpenSettings = 235`, `EditSettingsFile = 236`, `needs_document`, `TryFrom` (124 → 126), and a test.
- Modify: `src/window/menus.rs`: the Ctrl+, accelerator (65 → 66, and update the `assert_eq!(specs.len(), 65)` test to 66), the File menu entry, and a test.
- Modify: `src/window/command_palette.rs`:
  - two `ENTRIES` rows (102 → 104);
  - `shortcut_text` spells `VK_OEM_COMMA` as `,`;
  - remove `SETTINGS_COMMANDS`, the `subset` field, `set_subset`, `subset()`, the `self.subset = None;` reset, and the test `every_settings_command_has_exactly_one_palette_entry`;
  - add tests.
- Modify: `src/window/main_window.rs`:
  - add `execute_command` arms;
  - remove `open_settings_palette` and fold `show_command_palette` into `open_command_palette`;
  - remove the `set_subset(None)` call in the quick-open picker and the subset filter in the refilter;
  - update the tests that referenced `SETTINGS_COMMANDS`.
- Modify: `src/window/activity_bar.rs`: the gear opens the dialog.
- Modify: `tests/windows/titlebar.rs`: add an integration test.
- Modify: `README.md`: the command palette paragraph and the shortcut table.

**Interfaces:**
- Consumes: `main_window::{show_settings, edit_settings_file}` (Task 7).
- Produces: `CommandId::{OpenSettings = 235, EditSettingsFile = 236}`.

- [ ] **Step 1: Write the failing tests.** In `src/window/commands.rs`:

```rust
    #[test]
    fn settings_commands_are_235_and_236_and_need_no_document() {
        // Break caught: Ctrl+, renumbered onto another command, or Settings greyed out while no
        // tab is open.
        for (value, command) in [
            (235, CommandId::OpenSettings),
            (236, CommandId::EditSettingsFile),
        ] {
            assert_eq!(command as u16, value);
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(!command.needs_document(), "{command:?}");
            assert!(!command.needs_text(), "{command:?}");
            assert!(!command.is_sidebar(), "{command:?}");
        }
    }
```

In `src/window/command_palette.rs`:

```rust
    #[test]
    fn settings_is_listed_under_preferences_with_ctrl_comma() {
        // Break caught: the dialog reachable only by mouse, or its row showing no shortcut.
        assert_eq!(labels("open settings")[0], "Preferences: Open Settings");
        assert_eq!(shortcut_text(CommandId::OpenSettings).as_deref(), Some("Ctrl+,"));
        assert_eq!(labels("fastpad.ini")[0], "Preferences: Edit fastpad.ini");
        assert_eq!(shortcut_text(CommandId::EditSettingsFile), None);
    }
```

In `src/window/menus.rs`:

```rust
    #[test]
    fn the_file_menu_opens_settings() {
        // Break caught: Settings missing from the menus, so the only mouse route is the gear,
        // which is hidden with notes mode off.
        use super::MenuBar;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND};
        let bar = MenuBar::create().unwrap();
        let state =
            unsafe { GetMenuState(bar.dropdown(0), CommandId::OpenSettings as u32, MF_BYCOMMAND) };
        assert_ne!(state, u32::MAX, "File: Settings");
    }
```

In `src/window/main_window.rs`, replace the test `the_settings_button_lists_only_settings_and_the_next_palette_lists_everything` with:

```rust
    #[test]
    fn ctrl_comma_and_edit_settings_file_run_from_the_command_table() {
        // Break caught: OpenSettings or EditSettingsFile falling through to `App::execute`,
        // which ignores them.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("settings-commands");
        let ini = scratch.path().join("fastpad.ini");
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let shown = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = shown.clone();
        crate::window::settings_dialog::answer_next(move |dialog| {
            seen.set(true);
            unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
        });
        execute_command(window.hwnd, CommandId::OpenSettings);
        assert!(shown.get());

        execute_command(window.hwnd, CommandId::EditSettingsFile);
        assert!(app_mut(window.hwnd).tabs.find_path(&ini).is_some());
        super::save_settings_to(None);
    }
```

In the activity-bar test `clicking_the_active_view_icon_closes_the_sidebar_panel_and_saves_none`, replace the block from `// Settings opens the command palette listing only the settings commands.` through `assert_eq!(view(), SidebarView::Search);` (the one right after the palette asserts) with:

```rust
        // Settings opens the Settings dialog and leaves the panel alone.
        let shown = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = shown.clone();
        crate::window::settings_dialog::answer_next(move |dialog| {
            seen.set(true);
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    dialog,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                    usize::from(windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE),
                    0,
                )
            };
        });
        let (x, y) = button_center(window.hwnd, ActivityButton::Settings);
        click(bar, x, y);
        assert!(shown.get(), "the gear opened Settings");
        assert_eq!(view(), SidebarView::Search);
```

In the file-icons test that asserts `SETTINGS_COMMANDS.contains(&CommandId::FileIcons…)`, delete that `assert!( … )` block. Keep the `needs_document` asserts after it.

In `tests/windows/titlebar.rs`, after `help_about_shows_an_owned_box_that_escape_closes`:

```rust
#[test]
fn settings_shows_an_owned_dialog_that_escape_closes() -> TestResult<()> {
    // Break caught: Settings doing nothing in the real binary, a dialog that isn't owned by
    // (and so can hide behind) the main window, or one Escape can't close.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow, IsWindowVisible};
    let mut process = FastPadProcess::spawn(["--new-window"])?;
    let hwnd = process.wait_for_main_window(Duration::from_secs(3))?;
    let class = wide_null("FastPadSettings");
    let find_settings = || unsafe {
        FindWindowExW(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            class.as_ptr(),
            std::ptr::null(),
        )
    };
    assert!(find_settings().is_null());

    assert_ne!(
        unsafe { PostMessageW(hwnd, WM_COMMAND, CommandId::OpenSettings as usize, 0) },
        0
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let settings = loop {
        let settings = find_settings();
        if !settings.is_null() && unsafe { IsWindowVisible(settings) } != 0 {
            break settings;
        }
        if std::time::Instant::now() >= deadline {
            return Err("the Settings dialog never showed".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(unsafe { GetWindow(settings, GW_OWNER) }, hwnd);
    assert_eq!(unsafe { IsWindowEnabled(hwnd) }, 0);

    assert_ne!(
        unsafe { PostMessageW(settings, WM_KEYDOWN, VK_ESCAPE as usize, 0) },
        0
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while unsafe { IsWindow(settings) } != 0 {
        if std::time::Instant::now() >= deadline {
            return Err("Escape did not close Settings".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(unsafe { IsWindowEnabled(hwnd) }, 0);
    process.close()
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: errors, `no variant OpenSettings`.

- [ ] **Step 3: Implement**

In `src/window/commands.rs`:
- Add `OpenSettings = 235,` and `EditSettingsFile = 236,` after `ToggleHighlightCurrentLine = 234,`.
- Extend `needs_document`'s list with `| Self::OpenSettings | Self::EditSettingsFile`.
- In `TryFrom`, change `[CommandId; 124]` to `[CommandId; 126]` and add `CommandId::OpenSettings, CommandId::EditSettingsFile,`.

In `src/window/menus.rs`:
- Add `VK_OEM_COMMA` to the `KeyboardAndMouse` import.
- Change `accelerator_specs() -> [AcceleratorSpec; 65]` to `66`, and add `virtual_key(FCONTROL, VK_OEM_COMMA, CommandId::OpenSettings),` after the `CommandPalette` accelerator.
- In the test that asserts `assert_eq!(specs.len(), 65);`, change it to `66`.
- In the File menu, replace

```rust
                MenuEntry::Separator,
                MenuEntry::command(
                    "&Restore session on startup",
                    CommandId::ToggleRestoreSession,
                ),
```

with

```rust
                MenuEntry::Separator,
                MenuEntry::command("Se&ttings...\tCtrl+,", CommandId::OpenSettings),
                MenuEntry::command(
                    "&Restore session on startup",
                    CommandId::ToggleRestoreSession,
                ),
```

In `src/window/command_palette.rs`:
- Change `ENTRIES: [PaletteEntry; 102]` to `104`, and add these before `entry("Help: About FastPad", CommandId::About),`:

```rust
    entry("Preferences: Open Settings", CommandId::OpenSettings),
    entry("Preferences: Edit fastpad.ini", CommandId::EditSettingsFile),
```

- In `shortcut_text`, add `VK_OEM_COMMA` to its `use` and the arm `VK_OEM_COMMA => text.push(','),` after the `VK_OEM_MINUS` arm.
- Delete the `SETTINGS_COMMANDS` const with its doc comment.
- Delete the `subset: Option<&'static [CommandId]>` field, its initializer `subset: None,`, the `self.subset = None;` line, and the methods `set_subset` and `subset`.
- Delete the test `every_settings_command_has_exactly_one_palette_entry`, and remove `SETTINGS_COMMANDS` from the tests' `use super::{…}`.

In `src/window/main_window.rs`:
- In `execute_command`, next to `CommandId::About => show_about(hwnd),`, add:

```rust
        CommandId::OpenSettings => show_settings(hwnd),
        CommandId::EditSettingsFile => edit_settings_file(hwnd),
```

- Replace `open_command_palette`, `open_settings_palette` and `show_command_palette` with a single function:

```rust
pub(crate) fn open_command_palette(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    capture_palette_focus(hwnd);
    let colors = title_chrome(hwnd).0;
    let newly_shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.command_palette.is_none() {
            app.command_palette = CommandPalette::create(hwnd).ok();
        }
        let palette = app.command_palette.as_mut()?;
        let newly_shown = palette.mark_shown(colors);
        // Reopening the palette normally always shows commands, even right after a picker.
        palette.set_picker(None);
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    if newly_shown {
        // Clearing the field sends EN_CHANGE, which lists every available command.
        with_command_palette(hwnd, CommandPalette::clear_query);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}
```

- Delete `palette.set_subset(None);` in the quick-open picker function.
- In the refilter, delete `let subset = with_command_palette(hwnd, CommandPalette::subset).flatten();` and the filter line `subset.is_none_or(|subset| subset.contains(&command))` along with its `&&`, so the closure starts with `(has_tabs || !command.needs_document())`.

In `src/window/activity_bar.rs`, replace `open_settings` and the doc comment of `activate`:

```rust
/// The Settings button's click handler: the Settings dialog.
fn open_settings(main: HWND) {
    super::main_window::show_settings(main);
}

/// A click on `button`: an inactive view opens, the active one closes the panel, and Settings
/// opens the Settings dialog.
```

Check `rg -n "set_subset|SETTINGS_COMMANDS|open_settings_palette|show_command_palette" src tests`.
Expected: no matches.

In `README.md`, replace

```
Open it and start typing. Switch theme, toggle word wrap or line numbers, change font size
or tab width, all without leaving the keyboard. Every change is saved instantly. The
**Settings** button at the bottom of the sidebar opens the palette with just the settings.
```

with

```
Open it and start typing. Switch theme, toggle word wrap or line numbers, change font size
or tab width, all without leaving the keyboard. Every change is saved instantly. Prefer to
see everything at once? **Settings** (`Ctrl+,`, **File > Settings...** or the gear at the
bottom of the sidebar) lists every setting, and changes apply as you make them.
```

Then add this row at the end of the shortcut table:

```
| Settings | `Ctrl+,` | | | |
```

- [ ] **Step 4: Run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.

Run: `cargo test --lib -- --test-threads=1 settings_commands settings_is_listed the_file_menu ctrl_comma clicking_the_active_view_icon accelerator shortcut file_icons`
Expected: all pass.

Run: `cargo build --release; cargo test --test titlebar -- --test-threads=1 settings_shows_an_owned_dialog help_about`
Expected: both pass. Per the worktree notes, copy `native/out` DLLs if running in a worktree.

- [ ] **Step 5: Commit**

```bash
git add src/window/commands.rs src/window/menus.rs src/window/command_palette.rs src/window/main_window.rs src/window/activity_bar.rs tests/windows/titlebar.rs README.md
git commit -m "feat: open Settings from the gear, File menu, Ctrl+, and the palette"
```

---

### Task 9: Final verification

**Files:** none new. This task fixes whatever it finds.

- [ ] **Step 1: Full lint and test suite**

Run: `cargo fmt --check; cargo clippy --all-targets -- -D warnings`
Expected: clean. If `fmt` reports differences, run `cargo fmt` and commit it as `style: cargo fmt`.

Run: `cargo test -- --test-threads=1`
Expected: every test passes. Paste the summary lines into the PR description.

- [ ] **Step 2: Dependency audit**

Run: `pwsh -NoProfile -File tools/audit-dependencies.ps1`
Expected: passes, with no new `windows-sys` features. If it fails on a feature, add it to both `Cargo.toml` and the script's allowlist, and commit.

- [ ] **Step 3: Startup latency.** Nothing the dialog uses runs at startup, so the numbers must not move.

Run: `./tools/benchmark.ps1 -Runs 100 -Warmup 10 -Output benchmarks/settings-dialog.jsonl`. Compare its p50/p95 with a run on `feat/about-dialog` made the same way (`git stash`/checkout, same command, `-Output benchmarks/about-dialog.jsonl`).
Expected: every milestone within noise (±5%). Don't commit the `.jsonl` files.

- [ ] **Step 4: Live check.** First back up the real settings: `Copy-Item $env:LOCALAPPDATA\FastPad\fastpad.ini $env:TEMP\fastpad.ini.bak` (skip if the file doesn't exist). Run `target\release\fastpad.exe`, then:
  1. Press Ctrl+,. The dialog is centred, themed, and has Theme focused.
  2. Change the theme to Catppuccin Mocha. The app and the dialog both recolour.
  3. Open the Font dropdown and type `cas`. It jumps to Cascadia. Press Enter, and the editor font changes.
  4. Type `14` in the font size, then press Tab. The size applies.
  5. Toggle Show whitespace and Highlight current line, and watch the editor.
  6. Switch notes mode off, then on again. The dialog keeps the keyboard, and Notebook autosave greys out while no notebook is open.
  7. Click **Edit fastpad.ini**. The dialog closes, and the file opens in a tab with the lines just written.
  8. Open Settings from the gear and from **File → Settings...**.

  Afterwards, restore the file: `Copy-Item $env:TEMP\fastpad.ini.bak $env:LOCALAPPDATA\FastPad\fastpad.ini -Force`. If there was no file before, delete the one the check created.

- [ ] **Step 5: Whole-branch review.** Use superpowers:requesting-code-review on the branch diff against `feat/about-dialog`. Fix confirmed findings, re-run the affected targeted tests, and commit each fix.

- [ ] **Step 6: Hand off.** Use superpowers:finishing-a-development-branch. The PR targets `feat/about-dialog` until PR #35 merges, then gets retargeted to `main` (main is protected: PR plus 4 CI checks).
