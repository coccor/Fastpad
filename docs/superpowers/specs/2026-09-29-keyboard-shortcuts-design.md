# Keyboard shortcuts page: design

- Status: approved in conversation on 2026-09-29.
- Branch: `feat/keyboard-shortcuts`, stacked on `feat/settings-dialog`, whose dialog, `soft_paint` painting and dropdown it builds on.
- Model: VS Code's Keyboard Shortcuts editor, minus chords.

## 1. Goal

- **See every shortcut:** a page listing every command with its keys, searchable by name or by key.
- **Change them:** change, add, remove and reset the keys of any command, with a warning when a key is already taken.
- **One source of truth:** menus, the palette's hints and the page all read the same keymap, so they can't disagree.
- **No startup cost:** the defaults stay a constant table; user overrides are applied when settings load, as today.

## 2. Decisions

| Question | Decision |
|---|---|
| Where | A second page of the Settings dialog, **Keyboard Shortcuts**, beside **General** (today's rows). Not a separate dialog or an editor tab. |
| Storage | Overrides only, in `fastpad.ini`: `key.<command-id>=<keys>`. Defaults stay in code. No separate keybindings file. |
| Dispatch | Keep the Win32 accelerator table, rebuilt from the resolved keymap when settings load and after each change. |
| Chords | Out of scope for now (no `Ctrl+K Ctrl+S`). If added later, the dispatcher is replaced by our own; the keymap and ini format don't change. |
| Several keys per command | Yes. Zoom In already has three defaults. |
| Conflicts | Allowed, with a warning. A user binding beats a default on the same key. Between two defaults the earlier table entry wins; between two user bindings, the command whose ID sorts first wins. |
| Search | By command name, command ID or key text, plus a record-keys mode that filters by an exact key. |
| Commands listed | Every command: the palette's `ENTRIES`, plus the bound commands the palette leaves out (Select tab 1–9, Command Palette, Focus editor group 1–8 and last, Focus next/previous pane, Markdown preview cycle). |
| How to open | The new **Preferences: Open Keyboard Shortcuts** palette command (no default key), and the dialog's left nav. |
| Applying | Each confirmed change saves one ini line and applies at once. No OK/Cancel, like the rest of the dialog. |

## 3. Keymap (`src/keymap.rs`)

A pure module: no HWNDs, fully unit-tested.

### 3.1 Key strokes

- `KeyStroke { ctrl, shift, alt, vk }`, one virtual key plus modifiers.
- Text form in VS Code style, modifiers in the order `Ctrl+Shift+Alt+`, keys by name: `A`–`Z`, `0`–`9`, `F1`–`F24`, `Enter`, `Escape`, `Space`, `Tab`, `Backspace`, `Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`, `Up`, `Down`, `Left`, `Right`, the OEM punctuation by its US character (`=`, `-`, `,`, `.`, `/`, `\`, `;`, `'`, `[`, `]`, `` ` ``), and `Numpad0`–`Numpad9`, `NumpadAdd`, `NumpadSubtract`, `NumpadMultiply`, `NumpadDivide`, `NumpadDecimal`.
- `parse` is case-insensitive and ignores spaces around `+`; `format` gives the canonical form. `format(parse(x)) == canonical(x)` is tested for every default.
- `Shift+=` is kept as its own stroke: the current `Ctrl+Shift+=` default for Zoom In stays.

### 3.2 Commands

- Every rebindable `CommandId` gets a stable text ID, grouped like VS Code: `file.save`, `file.saveAs`, `view.toggleSidebar`, `tabs.select1`, `groups.focus1`, and so on. The table of IDs lives in `keymap.rs`; a test checks they are unique and cover the listed commands.
- Display names stay in the palette's `ENTRIES`. The page reads them from there, adding names for the bound commands the palette leaves out (§2).

### 3.3 Defaults and resolving

- `DEFAULT_BINDINGS: &[(KeyStroke, CommandId)]` replaces `menus::accelerator_specs()`, with the same entries.
- `Overrides` maps a command to its full key list. `Some(vec![])` means unbound; no entry means defaults.
- `resolve(overrides) -> Vec<Binding>`, where a `Binding` is `{ stroke, command, source: Default | User }`.
- `command_for(stroke)` applies the conflict rule. `conflicts(stroke, except)` lists the other commands on a key.
- `first_key(command)` gives the text menus and the palette show.

## 4. Storage

- One line per overridden command: `key.file.save=Ctrl+Alt+S, Ctrl+Shift+Q`. An empty value means unbound. Any change rewrites the command's whole list.
- `persisted::parse` collects `key.*` lines, as text, into `SettingsDelta.key_overrides` (`config` stays free of window types); `apply_delta` hands them to `Settings`. The keymap validates them when settings load: an unknown command ID, an unknown key, or a key the recording box would refuse (§6.5) is skipped with a `fastpad.ini:` warning, and the rest of the line is kept. A line with a value but no usable key is ignored as a whole, so the defaults stay.
- Reset deletes the line (a new `remove_setting`), restoring the defaults. Setting a command's keys to exactly its defaults does the same.
- No migration: the format is new.

## 5. Dispatch and derived text

- `AcceleratorTable::create` takes the resolved bindings. The `App` rebuilds it after `load_settings` and after each change, destroying the old handle.
- Until settings load, the table built from the defaults is used, which is today's behaviour.
- `translate_accelerator`'s special cases (tab drag, menu activation, palette and inline-name fields, typing into no tab) are unchanged.
- Menu labels lose their typed-in `\tCtrl+S` text. The shortcut text is added from `first_key` when menus and context menus are built, and the menu band's cached text is refreshed on change. The palette's `shortcut_text` reads the resolved keymap too.
- The tests in `menus.rs` that compared the labels with the table are replaced by tests that labels get their text from the keymap.

## 6. The dialog

### 6.1 Navigation

- The dialog gets a left column (about 170 px at 96 DPI) listing **General** and **Keyboard Shortcuts**, and widens to fit the table.
- Clicking a nav item, or Ctrl+Page Up / Ctrl+Page Down anywhere in the dialog, switches pages. The nav item is also a Tab stop; Up/Down on it switch pages.
- **General** is today's column of rows, unchanged. `Layout` gains a page and a nav rectangle; the General arrays stay as they are.
- `show` takes the page to open on.

### 6.2 Search row

- A themed native EDIT, subclassed like the palette's field, with the cue text "Type to search in keybindings".
- Plain text matches, case-insensitively, the display name, the command ID or the key text (`ctrl+s` matches `Ctrl+S`).
- The ⌨ toggle beside it (and Alt+K) switches to record-keys mode. In that mode each key stroke replaces the field's text with its canonical form and filters rows to that exact key. Escape leaves the mode.
- Down arrow in the field moves the focus to the first row.

### 6.3 Table

- Columns: **Command**, **Keybinding**, **Source**. The command column shows the display name; on the selected row the command ID is shown dimmed, right-aligned in the same column.
- One row per binding. A command with no keys has one row showing "—". Rows are sorted by display name, then by binding order.
- Keys are drawn as keycaps: each part of the stroke in a rounded box, joined by `+`, painted with `soft_paint`.
- Only visible rows are painted; the list scrolls with the wheel, Page Up/Down, Home/End and the arrows.
- Selection follows the arrows and a click. Hovering a row shows ✎ (change) at its left.

### 6.4 Row actions

| Action | Keys | Mouse |
|---|---|---|
| Change keybinding | Enter | Double-click, ✎ |
| Add keybinding | Ctrl+Enter | Context menu |
| Remove keybinding | Delete | Context menu |
| Reset keybinding (User rows only) | — | Context menu |
| Copy command ID (to the clipboard) | Ctrl+C | Context menu |

- Change on a "—" row adds. Change on a row replaces that one key in the command's list; the whole list is saved as the override.
- Remove on the last key of a command leaves it unbound (an empty override) and shows the "—" row.
- The context menu uses the app's existing themed context menu.

### 6.5 Recording box

- A centred panel over the table: "Press desired key combination and then press ENTER."
- Below it, the captured stroke as keycaps and, if other commands use it, a link "N existing commands have this keybinding". Clicking the link closes the box and puts the stroke in the search field in record-keys mode.
- **Enter** confirms, **Escape** cancels, so plain Enter and Escape can't be bound (as in VS Code). A lone modifier changes nothing.
- A stroke without Ctrl or Alt whose key types text or edits it (letters, digits, OEM punctuation, numpad digits and operators, Space, Backspace, Delete, Tab, Enter, Escape) is refused with an inline line: "Needs Ctrl or Alt: it would stop typing." Alt with a numpad digit is refused (it types Alt codes), and so are F10 and Shift+F10 (the menu and the context menu). Other F-keys, arrows, Home/End, Page Up/Down and Insert are allowed alone or with Shift.
- Confirming the same key the row already has changes nothing.

### 6.6 Model

- The page's state and key handling live in a pure `shortcuts_model.rs` next to `settings_model.rs`: rows from the resolved keymap and the filter, selection and scroll, record-box state and its acceptance rule. It returns `Effect`s (`SetKeys(command, Vec<KeyStroke>)`, `Reset(command)`, `CopyId`, ...) that the dialog runs.
- Running an effect goes through `main_window`: update `App.settings`, rebuild the accelerator table, refresh menus and palette, save the line (or show the usual "could not save fastpad.ini" notice), then refresh the dialog.

## 7. Testing

- `keymap.rs`: parse/format round-trip for every default and every key name; bad input rejected; resolve with user-over-default; conflicts; `first_key`.
- `persisted.rs`: `key.*` lines parsed, bad IDs and keys warned and skipped, empty value is unbound, reset removes the line.
- `shortcuts_model.rs`: rows for multi-key and unbound commands, text and key filtering, record-keys mode, recording acceptance and refusal rules, change/add/remove/reset effects.
- Window test: after rebinding Save to `Ctrl+Alt+S`, that key saves and `Ctrl+S` no longer does; a menu label shows the new key.
- Latency: `fastpad-bench` before and after. Nothing new runs before first input.

## 8. Out of scope

- Chords.
- `when` clauses (context-dependent bindings).
- Editing the defaults, importing VS Code's `keybindings.json`, and a "reset all" button.
- Rebinding the editor's built-in keys (arrow movement, Ctrl+A in the editor, and so on), which aren't commands.
