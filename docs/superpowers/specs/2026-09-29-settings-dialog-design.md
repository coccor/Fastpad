# Settings dialog: design

- Status: approved in conversation on 2026-09-29.
- Branch: `feat/settings-dialog`, stacked on `feat/about-dialog` (PR #35), whose menu, command and palette changes it builds on.
- It replaces the activity bar's filtered Settings palette.

## 1. Goal

- **One place for settings:** a dialog that shows every user-facing `fastpad.ini` setting with its current value and lets you change it, so you don't need to know the palette commands.
- **Live:** each change applies straight away and saves its one `fastpad.ini` line, exactly as the palette commands do today.
- **Themed:** the dialog follows every theme, including the Catppuccin ones, like the About box does.
- **Three new editor settings:** indent with spaces, show whitespace, highlight current line.

## 2. Decisions

| Question | Decision |
|---|---|
| Form | A modal, owner-drawn popup like About, not an editor tab or a sidebar view. |
| When changes apply | Immediately, and each one is saved immediately. There's one **Close** button: no OK/Cancel and no revert. |
| Controls | Owner-drawn: section headings, checkboxes, segmented choices, dropdowns and a numeric stepper. No standard Win32 controls, because their dark mode is incomplete on Windows 10. |
| Settings shown | Theme, file icons, font face, font size, tab width, indent with spaces, word wrap, line numbers, show whitespace, highlight current line, notes mode, restore session, notebook autosave. |
| Settings not shown | `sidebar_view`, `sidebar_width` and `open_editors_expanded`, which are UI state saved as you use the sidebar; and `recovery_interval_seconds`, which stays a file-only setting. |
| New ini keys | `insert_spaces` (default `false`), `show_whitespace` (default `false`), `highlight_current_line` (default `true`). |
| How to open it | The activity bar's gear, **File → Settings…**, the **Ctrl+,** shortcut, and the **Preferences: Open Settings** palette command. |
| The filtered Settings palette | Removed (`SETTINGS_COMMANDS` and the palette's subset mode). The individual commands stay in the full palette. |
| Hand edits | The dialog's **Edit fastpad.ini** link, and a **Preferences: Edit fastpad.ini** palette command, open the file in a tab. |

## 3. The dialog

### 3.1 Window

- An owned popup, window class `FastPadSettings`, 520 px wide at 96 DPI and scaled with the window's DPI. It has rounded corners (`DWMWCP_ROUND`) and a drop shadow (`CS_DROPSHADOW`), like About.
- A title row reads **Settings** and has a × close button. Dragging the title row moves the dialog (`HTCAPTION`).
- The dialog is centred on the main window, which is disabled while it is open. The dialog runs its own modal loop inside a `ModalScope`, like About.
- The dialog is painted from the current `Palette`. A theme change made in the dialog repaints the dialog in the new colours too.

### 3.2 Rows

The dialog is a single column. Each section heading is followed by its rows, with the label on the left and the control right-aligned.

| Section | Row | Control | Key |
|---|---|---|---|
| Appearance | Theme | Dropdown: System, Light, Dark, Catppuccin, Catppuccin Latte, Catppuccin Frappé, Catppuccin Macchiato, Catppuccin Mocha | `theme` |
| | File icons | Segmented: Material \| Minimal \| Solid | `file_icons` |
| Editor | Font | Dropdown of the installed font families (§3.5) | `font_face` |
| | Font size | Stepper `[–] 11 [+]`, 6 to 72 | `font_size` |
| | Tab width | Segmented: 2 \| 4 \| 8 | `tab_width` |
| | Indent with spaces | Checkbox | `insert_spaces` |
| | Word wrap | Checkbox | `word_wrap` |
| | Line numbers | Checkbox | `line_numbers` |
| | Show whitespace | Checkbox | `show_whitespace` |
| | Highlight current line | Checkbox | `highlight_current_line` |
| Notes and session | Notes mode | Checkbox | `notes_mode` |
| | Restore session | Checkbox | `restore_session` |
| | Notebook autosave | Checkbox | the open notebook's autosave switch |

- **Footer:** the **Edit fastpad.ini** link on the left, the **Close** button on the right.
- **Tab width not in 2, 4 or 8:** when `fastpad.ini` sets another width, none of the three segments is selected, and a fourth segment shows the actual number, selected. Picking 2, 4 or 8 replaces it and removes that segment.
- **Font size:** the + and – buttons step by 1 within 6 to 72. The number can be typed. A typed value is committed on Enter or when the field loses focus; it is clamped to 6–72, and text that isn't a number puts back the current size. A size set outside 6–72 in `fastpad.ini` is shown as is, and the first step brings it back into range.
- **Notebook autosave:** when no notebook is open, the row is greyed out, shows the hint "Open a notebook to change this", and Tab skips it.
- **Height:** all rows fit without scrolling at the usual sizes (about 560 px at 96 DPI). When the monitor's work area is shorter, the dialog is capped at the work-area height and the rows between the title and the footer scroll with the mouse wheel, and follow keyboard focus.

### 3.3 Keyboard

| Key | Action |
|---|---|
| Tab / Shift+Tab | Next / previous control, in row order, then the link, then Close. It wraps around. |
| Space | Toggles the focused checkbox; presses the focused button or link. |
| Enter | Opens the focused dropdown; commits a typed font size; presses the focused button or link. |
| Left / Right | Previous / next choice in the focused segmented control. |
| Up / Down | Font size +1 / –1 on the stepper. |
| Alt+Down | Opens the focused dropdown. |
| Esc | Closes the open dropdown list; otherwise closes the dialog. |

Focus is drawn with `DrawFocusRect`, as in About.

### 3.4 Mouse

- Controls highlight when hovered.
- Clicking a checkbox's label toggles it, as well as clicking the box.
- Clicking a segment picks it. Clicking a dropdown opens its list. Clicking – or + steps the size.

### 3.5 Dropdown list

- A separate owned popup directly below the dropdown, the dropdown's width, up to 10 rows tall, painted in the theme's colours. The current value is selected and scrolled into view when the list opens.
- It scrolls with the mouse wheel and a thin scrollbar drawn in the theme.
- Up/Down, Page Up/Down and Home/End move the selection. Typing jumps to the next item that starts with the typed letters (the typed text resets after a short pause). Enter or a click picks the item. Esc or clicking elsewhere closes the list without changing anything.
- **Fonts:** the list is built each time the dialog is opened, not at startup. It uses `EnumFontFamiliesExW` for `DEFAULT_CHARSET`, removes duplicates and vertical (`@`-prefixed) families, and lists fixed-pitch families first, then the others, each group sorted by name ignoring case. If the current `font_face` isn't installed, it's listed at the top anyway so the dropdown can still show it.

### 3.6 Edit fastpad.ini

This closes the dialog. If `fastpad.ini` doesn't exist yet, it's created empty. Then the file is opened in a tab through the normal file-open path. If creating or opening it fails, the existing notice shows the error.

## 4. Code structure

### 4.1 New modules

- **`src/window/settings_model.rs`** is pure and has no window handles.
  - It lists the rows (§3.2), each with its label, section and control kind: `Check`, `Segmented`, `Dropdown` or `Stepper`.
  - `SettingsView` is a snapshot the dialog paints from: the current `Settings`, whether a notebook is open, and that notebook's autosave switch.
  - It turns a row and an input into a `SettingsAction`: `SetTheme(ThemePreference)`, `SetFileIcons(FileIconSet)`, `SetFontFace(String)`, `SetFontSize(u16)`, `SetTabWidth(u8)`, `Toggle(Toggle)` (one variant per checkbox) and `EditIni`.
  - It holds the stepper's clamping and the parsing of typed digits.
- **`src/window/settings_dialog.rs`** holds the popup: creation, its modal loop, painting, and mouse and keyboard handling.
  - It is shaped like `about.rs`: a pure `Layout::calculate` taking DPI and text metrics, `Layout::target_at(x, y)`, a `Target` enum, and `next_target` for Tab order.
  - The window procedure only turns input into `SettingsAction`s and hands them to `main_window`.
- **`src/window/dropdown_list.rs`** is the list popup of §3.5. It takes a `&[String]` and the selected index, and reports the picked index, or nothing if it was dismissed. Theme and font share it.
- **`src/platform/fonts.rs`** provides `installed_font_families() -> Vec<FontFamily>`, where `FontFamily { name, fixed_pitch }`. Sorting and removing duplicates is a pure function over the raw list, so it can be tested without GDI.

### 4.2 Applying a change

- `main_window::apply_settings_action(hwnd, action)` maps each action onto the existing `change_setting`. The notebook autosave toggle goes through `execute_command(hwnd, CommandId::ToggleFolderAutosave)`.
- So every write takes the same path the palette commands take: update `App::settings`, re-apply the theme or the editor settings, and save one line with `save_setting`. A failed save shows the existing "FastPad could not save fastpad.ini" notice.
- After each action, the dialog rebuilds its `SettingsView` from `App` and repaints.

### 4.3 Commands, menu and shortcut

- New `CommandId::OpenSettings`: in `accelerator_specs` on **Ctrl+,**, on the File menu as **Settings…\tCtrl+,**, and in the palette as **Preferences: Open Settings**.
- New `CommandId::EditSettingsFile`: in the palette as **Preferences: Edit fastpad.ini**.
- New `CommandId::ToggleInsertSpaces`, `ToggleShowWhitespace` and `ToggleHighlightCurrentLine`: in the palette as **Editor: Toggle indent with spaces**, **Editor: Toggle show whitespace** and **Editor: Toggle highlight current line**.
- The activity bar's gear calls the settings dialog instead of `open_settings_palette`. `open_settings_palette`, `SETTINGS_COMMANDS`, `CommandPalette::set_subset` and the subset plumbing are removed.

### 4.4 New settings

- `config/persisted.rs` and `config/defaults.rs`:
  - add the three keys to `Settings`, `SettingsDelta`, `parse`, `apply_delta` and `default_settings`;
  - each parses like the existing booleans, and a missing key keeps its default;
  - there is no migration.
- The editor (`src/editor/scintilla.rs`):
  - `insert_spaces` sets `SCI_SETUSETABS` to `!insert_spaces`;
  - `show_whitespace` sets `SCI_SETVIEWWS` to `SCWS_VISIBLEALWAYS` or `SCWS_INVISIBLE`;
  - `highlight_current_line` off resets the `SC_ELEMENT_CARET_LINE_BACK` element colour (`SCI_RESETELEMENTCOLOUR`) instead of setting it to `palette.caret_line_background`.
- `main_window::apply_settings_to` and `apply_colors_to` apply these to every editor, including editors created later and in other groups.
- Missing Scintilla constants are added to `scintilla_constants.rs`.

### 4.5 Windows features

- `EnumFontFamiliesExW` is in `Win32_Graphics_Gdi`, which is already enabled.
- If the dialog needs any other new `windows-sys` feature, add it to the allowlist in `tools/audit-dependencies.ps1`.

## 5. Testing

- **`settings_model` (unit):**
  - each row maps an input to the right action;
  - stepper clamping at 6 and 72;
  - parsing typed digits, including non-numbers, empty text and out-of-range values;
  - the fourth tab-width segment appears for a custom width.
- **`settings_dialog::Layout` (unit):**
  - rows stack in order;
  - hit-testing finds each control, the link and Close;
  - it scales with DPI;
  - Tab and Shift+Tab order wraps and skips the greyed-out autosave row.
- **`fonts` (unit):** sorting and removing duplicates over an injected list — fixed-pitch first, case-insensitive order, `@` families dropped, a missing current face listed first.
- **`dropdown_list` (unit):** keyboard selection, and type-to-jump including the reset after a pause, over a pure selection model.
- **`persisted` (unit):** the three new keys parse, reject bad values with a warning, default correctly, and save and load back.
- **`main_window` (unit, with `save_settings_to` pointing at a scratch ini):**
  - each `SettingsAction` changes `App::settings` and writes only its own line, leaving hand-written lines alone;
  - `SetTheme` goes through `apply_theme`;
  - the three new toggle commands change the editor, e.g. `SCI_GETUSETABS`, `SCI_GETVIEWWS`.
- **Integration (`tests/windows/`, run with `--test-threads=1`):** posting `CommandId::OpenSettings` shows an owned, visible `FastPadSettings` window with the main window disabled. Esc closes it, and the main window is enabled again.
- **Latency:** nothing is created at startup (the dialog, the dropdown and the font list are all created when the dialog opens). Check with `fastpad-bench` that startup doesn't change.

## 6. Out of scope

- Search or filtering inside the dialog.
- Settings per notebook or per language.
- Editing `recovery_interval_seconds` or the sidebar UI state in the dialog.
- Key bindings.
