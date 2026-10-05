# Editing shortcuts: design

- Status: scope approved in conversation on 2026-10-05; this spec awaits review.
- Branch: `feat/editing-shortcuts`, off `main`.
- Model: VS Code's default editing keys on Windows. Navigation, folding and chords are out of scope (§9).

## 1. Goal

- **VS Code muscle memory:** the text-editing keys a VS Code user reaches for daily do the same thing in FastPad.
- **Real commands:** every new action is a `CommandId` with a stable ID, so it shows in the palette, the Edit menu and the Keyboard Shortcuts page, and can be rebound like any other (keyboard shortcuts spec §3).
- **Nothing existing moves:** every current FastPad default keeps its key. Only Scintilla's built-in keys that collide are cleared or moved (§5).
- **No latency cost:** each action is a handful of Scintilla messages on the UI thread, one undo step, no allocation proportional to the document beyond the touched lines.

## 2. Decisions

| Question | Decision |
|---|---|
| Scope | Line editing, comments, multi-cursor. |
| Keys | VS Code's Windows defaults (§3). |
| Where the logic lives | Scintilla-backed operations in a new `src/editor/scintilla/line_ops.rs`; the comment-toggling text logic in a pure `src/editor/comment.rs`. |
| Prefer Scintilla built-ins | Yes, where one matches VS Code's behaviour (move lines, duplicate, add next match, indent, copy/cut line). Custom code only where none does. |
| Undo | Each invocation is one undo step (`begin_undo_action` / `end_undo_action`). |
| Multi-cursor | On for every text tab: `SCI_SETMULTIPLESELECTION(1)`, `SCI_SETADDITIONALSELECTIONTYPING(1)`, `SCI_SETMULTIPASTE(SC_MULTIPASTE_EACH)`. |
| Image tabs | Every new command is a no-op on an image tab, like Undo is today. |
| Rebinding | The new commands are rebindable. Scintilla-internal keys (column selection, §5) are not; they aren't commands. |

## 3. Commands and default keys

Palette names follow the existing `Edit: …` pattern. All IDs are new; none renames an existing one.

| Key | Command ID | Palette name | Behaviour |
|---|---|---|---|
| `Alt+Up` | `edit.moveLinesUp` | Edit: Move line up | `SCI_MOVESELECTEDLINESUP`: the lines the selection touches swap with the line above; the selection follows. |
| `Alt+Down` | `edit.moveLinesDown` | Edit: Move line down | `SCI_MOVESELECTEDLINESDOWN`. |
| `Shift+Alt+Up` | `edit.copyLinesUp` | Edit: Copy line up | Duplicate the touched lines; the selection stays on the upper copy. |
| `Shift+Alt+Down` | `edit.copyLinesDown` | Edit: Copy line down | Duplicate the touched lines; the selection moves to the lower copy. |
| `Ctrl+Shift+K` | `edit.deleteLines` | Edit: Delete line | Delete every line any selection touches, including its line end. |
| `Ctrl+Enter` | `edit.insertLineBelow` | Edit: Insert line below | New line after the caret's line, with the line's indentation; caret moves there. |
| `Ctrl+Shift+Enter` | `edit.insertLineAbove` | Edit: Insert line above | Same, above. |
| `Ctrl+]` | `edit.indentLines` | Edit: Indent line | `SCI_LINEINDENT`: indent the touched lines one level, whatever the selection. |
| `Ctrl+[` | `edit.outdentLines` | Edit: Outdent line | `SCI_LINEDEDENT`. |
| `Ctrl+L` | `edit.expandLineSelection` | Edit: Select line | Select the caret's whole line including its line end; if the selection already ends at a line start, extend it by one more line. |
| `Ctrl+/` | `edit.toggleLineComment` | Edit: Toggle line comment | §4. |
| `Shift+Alt+A` | `edit.toggleBlockComment` | Edit: Toggle block comment | §4. |
| `Ctrl+D` | `edit.addNextOccurrence` | Edit: Add next occurrence | Empty selection: select the word at the caret. Otherwise `SCI_MULTIPLESELECTADDNEXT`. Matching is case-sensitive; it is whole-word only when the first selection came from the empty-caret word expansion, as in VS Code. |
| `Ctrl+Shift+L` | `edit.selectAllOccurrences` | Edit: Select all occurrences | Same word rule, then `SCI_MULTIPLESELECTADDEACH`. |
| `Ctrl+Alt+Up` | `edit.addCursorAbove` | Edit: Add cursor above | Add a caret on the line above the topmost caret, at the same visual column (`SCI_GETCOLUMN` / `SCI_FINDCOLUMN`), clamped to the line's end. No-op on line 0. |
| `Ctrl+Alt+Down` | `edit.addCursorBelow` | Edit: Add cursor below | Same, below the bottommost caret. |

Existing commands that change behaviour, keeping their keys:

- **Copy / Cut** (`Ctrl+C` / `Ctrl+X`): `SCI_COPYALLOWLINE` / `SCI_CUTALLOWLINE`. With an empty selection they copy or cut the whole line, and Scintilla marks the clipboard as a line copy so **Paste** inserts it above the caret's line, as VS Code does. With a selection, unchanged.
- **Paste** with several carets and a multi-line clipboard: each caret gets the whole clipboard (`SC_MULTIPASTE_EACH`). VS Code's split-one-line-per-caret is out of scope.

Line operations with several selections: move and copy lines act on the main selection only (Scintilla's built-ins do). Delete lines, indent, outdent and both comment toggles act on every line any selection touches.

No conflicts with today's defaults: none of the keys above is in `DEFAULT_BINDINGS`. `Ctrl+Alt+Left/Right` (move tab to group) stay; accelerators match exact modifiers, so `Ctrl+Alt+Up/Down` and `Ctrl+Shift+Alt+arrows` don't collide with them.

## 4. Comments (`src/editor/comment.rs`)

A pure module: text in, edits out, fully unit-tested.

### 4.1 Syntax table

`Language::comment_syntax() -> CommentSyntax { line: Option<&str>, block: Option<(&str, &str)> }`:

| Language | Line | Block |
|---|---|---|
| C, C++, C#, JavaScript, TypeScript, Rust | `//` | `/*` `*/` |
| CSS | — | `/*` `*/` |
| SQL | `--` | `/*` `*/` |
| Python, Bash, YAML, TOML, Properties, Env | `#` | — |
| PowerShell | `#` | `<#` `#>` |
| INI | `;` | — |
| Batch | `REM` | — |
| HTML, XML, SVG, Markdown | — | `<!--` `-->` |
| Plain text, JSON | — | — |

### 4.2 Toggle line comment

On the lines the selections touch, skipping blank (whitespace-only) lines:

- If every non-blank line starts (after indentation) with the line marker, **uncomment**: remove the marker and one following space if there is one.
- Otherwise **comment**: insert `marker + " "` on every non-blank line at the smallest indentation column among them, so the markers line up.
- A language with no line marker but a block pair wraps the touched lines in one block comment (or unwraps them) instead, as VS Code does for HTML and Markdown.
- A language with neither does nothing.
- Selections keep covering the same text: their ends shift by the inserted or removed length on their line.

### 4.3 Toggle block comment

- Selection whose trimmed text starts with the open marker and ends with the close marker: **unwrap**, removing one space inside each marker if present.
- Otherwise **wrap**: `open + " " + selection + " " + close`. Empty selection inserts `open + "  " + close` with the caret between the spaces.
- A language with no block pair does nothing.

### 4.4 Interface

`comment::toggle_line(lines: &[&str], syntax) -> Vec<LineEdit>` and `comment::toggle_block(text, range, syntax) -> Vec<Edit>`; `line_ops` reads the touched lines, applies the edits with the existing `replace_ranges_with`, and adjusts the selections. The pure functions never see Scintilla.

## 5. Scintilla's own keys

Set once per editor view, next to today's view setup:

- **Cleared** (`SCI_CLEARCMDKEY`), because FastPad now owns these keys and an unhandled stroke must not fall through to Scintilla's different action: `Ctrl+D` (selection duplicate), `Ctrl+L` (line cut), `Ctrl+Shift+L` (line delete), `Ctrl+T` (line transpose; FastPad's New already wins, but clear it so rebinding New doesn't expose it), `Ctrl+Shift+T` (line copy), `Ctrl+[` / `Ctrl+]` (paragraph up/down). `Ctrl+U` / `Ctrl+Shift+U` (lower / upper case) stay Scintilla's: nothing here uses them.
- **Moved** (`SCI_ASSIGNCMDKEY`): column (rectangular) selection by keyboard goes from `Shift+Alt+arrows` to `Ctrl+Shift+Alt+arrows`, VS Code's keys. `Shift+Alt+Left/Right` are cleared (VS Code's smart select is out of scope).
- **Mouse:** `Alt+drag` stays Scintilla's rectangular selection. `Alt+Click` (press and release without moving) adds a caret, VS Code's gesture: the editor's subclass catches `WM_LBUTTONDOWN` with Alt and no Shift/Ctrl, and on `WM_LBUTTONUP` at the same point calls `SCI_ADDSELECTION` at `SCI_POSITIONFROMPOINT`; a drag passes through to Scintilla untouched. Scintilla's own `Ctrl+Click` add-caret stays as a bonus.
- **Escape** with several selections drops back to the main one (Scintilla's `SCI_CANCEL` does this). FastPad's own Escape handling (closing the find bar, the palette) runs only when there is a single selection. To verify during implementation; if the find bar's Escape intercepts first, it checks `SCI_GETSELECTIONS > 1` and cancels the extra carets instead.

## 6. Wiring

- `CommandId`: the sixteen new variants (§3).
- `keymap.rs`: their IDs in `COMMAND_IDS`; their keys appended to `DEFAULT_BINDINGS` (the array length grows from 66 to 82). Precedence: appended last, so they lose to every existing default on a shared key (there is none today).
- `bindable`: unchanged. Every new key has Ctrl or Alt.
- `command_palette.rs`: sixteen `entry("Edit: …", …)` rows after Edit: Paste.
- `menus.rs`, Edit menu, after Paste:
  - **Line ▸** Move line up, Move line down, Copy line up, Copy line down, Delete line, Insert line below, Insert line above, Indent line, Outdent line, Select line.
  - **Toggle line comment**, **Toggle block comment**.
  - **Selection ▸** Add next occurrence, Select all occurrences, Add cursor above, Add cursor below.
- Dispatch: the command handler forwards each to the active text view's `line_ops` method, as Undo is forwarded today.
- `windows` crate features: none new expected. If one is, update `tools/audit-dependencies.ps1`.

## 7. Testing

- **`comment.rs` unit tests:** every row of the §4.1 table; comment / uncomment / mixed lines; blank lines skipped; marker alignment at the minimum indent; one-space removal; block wrap / unwrap / empty selection; block fallback for HTML and Markdown; Plain text and JSON do nothing.
- **`keymap.rs`:** the existing tests (unique IDs, IDs cover every command, `parse(text(k)) == k` for every default, no duplicate default stroke) cover the new rows. Add one asserting each §3 key resolves to its command.
- **`line_ops` with a real Scintilla** (window test setup, `--test-threads=1`): one test per command on a small document, checking text, selections and that one Undo restores the original. Plus copy/cut with an empty selection, multi-caret typing and paste, and Escape dropping extra carets.
- **Shortcuts page, palette and menu tests:** update counts and expected rows.
- **Integration harness:** run every `tests/windows` target locally. Add a test that drives `Alt+Down` and `Ctrl+/` through the accelerator table on a live window.

## 8. Risks

- **Alt and the menu bar:** a bare Alt press activates FastPad's menu band. `Alt+Up/Down` and `Alt+Click` are Alt *with* another input, which the existing menu-key handling already distinguishes from a bare tap (`Alt+Z`, `Alt+1`…`9` work today). Verify `Alt+Click` does not leave the band armed after release.
- **Alt+Click in the subclass:** must not break `Alt+drag` rectangular selection; the click/drag threshold uses `SM_CXDRAG` / `SM_CYDRAG`.
- **Line-copy paste:** Scintilla's line-copy clipboard marker is FastPad-to-FastPad only; pasting into another app is plain text, as expected.

## 9. Out of scope

Navigation (`Ctrl+G` as a direct key, bracket matching, reopen closed tab), folding keys, chords, smart select (`Shift+Alt+Left/Right`), one-line-per-caret paste, `Ctrl+U` cursor undo (it stays lowercase), column-selection mode toggle, transform-case commands.
