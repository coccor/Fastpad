# FastPad reference

- [Keyboard shortcuts](#keyboard-shortcuts)
- [Settings (fastpad.ini)](#settings-fastpadini)
- [Command line](#command-line)

## Keyboard shortcuts

| Action | Shortcut | | Action | Shortcut |
|---|---|---|---|---|
| New tab | `Ctrl+N` or `Ctrl+T` | | Command palette | `Ctrl+Shift+P` |
| Open | `Ctrl+O` | | Markdown view (cycle) | `Ctrl+Shift+V` |
| Save | `Ctrl+S` | | Format JSON | `Shift+Alt+F` |
| Save as | `Ctrl+Shift+S` | | Word wrap | `Alt+Z` |
| Find | `Ctrl+F` | | Zoom in / out / reset | `Ctrl++` / `Ctrl+-` / `Ctrl+0` |
| Replace | `Ctrl+H` | | Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Undo / redo | `Ctrl+Z` / `Ctrl+Y` | | Go to tab 1–9 | `Alt+1` … `Alt+9` |
| Open notebook | `Ctrl+Shift+O` | | Toggle sidebar | `Ctrl+B` (Bold in Markdown files) |
| Show notebook | `Ctrl+Shift+E` | | Search notes | `Ctrl+Shift+F` |
| Move note to notebook | `Ctrl+Shift+M` | | Sidebar / editor groups focus | `F6` / `Shift+F6` |
| Find next / previous | `F3` / `Shift+F3` | | Match case / whole word / regex | `Alt+C` / `Alt+W` / `Alt+R` |
| Replace in notes | `Ctrl+Shift+H` | | Replace all (in Search) | `Ctrl+Alt+Enter` |
| Replace in the selected result | `Ctrl+Shift+1` | | Go to note | `Ctrl+P` |
| Close tab | `Ctrl+W` or middle-click | | Split editor right / down | `Ctrl+\` / `Ctrl+Shift+\` |
| Focus editor group 1–8 / last | `Ctrl+1` … `Ctrl+8` / `Ctrl+9` | | Move tab to next / previous group | `Ctrl+Alt+Right` / `Ctrl+Alt+Left` |
| Settings | `Ctrl+,` | | Select line | `Ctrl+L` |
| Move line up / down | `Alt+Up` / `Alt+Down` | | Copy line up / down | `Shift+Alt+Up` / `Shift+Alt+Down` |
| Delete line | `Ctrl+Shift+K` | | Insert line below / above | `Ctrl+Enter` / `Ctrl+Shift+Enter` |
| Indent / outdent line | `Ctrl+]` / `Ctrl+[` | | Toggle line / block comment | `Ctrl+/` / `Shift+Alt+A` |
| Add next occurrence | `Ctrl+D` | | Select all occurrences | `Ctrl+Shift+L` |
| Add cursor above / below | `Ctrl+Alt+Up` / `Ctrl+Alt+Down` | | Add cursor | `Alt+Click` |
| Bold / italic (Markdown files) | `Ctrl+B` / `Ctrl+I` | | Code / link (Markdown files) | ``Ctrl+` `` / `Ctrl+K` |
| Strikethrough (Markdown files) | `Ctrl+Shift+X` | | Heading 1–6 / paragraph (Markdown files) | `Ctrl+Shift+1`–`6` / `Ctrl+Shift+0` |
| Bulleted / numbered list (Markdown files) | `Ctrl+Shift+8` / `Ctrl+Shift+7` | | Task / tick task (Markdown files) | `Ctrl+Shift+9` |
| Follow link at caret | `F12` or `Ctrl+Click` | | Today's note | `Ctrl+Alt+D` |
| Live Markdown view | `Ctrl+Alt+V` | | | |

## Settings (fastpad.ini)

Everything in the command palette or Settings is saved to `%LocalAppData%\FastPad\fastpad.ini`. You can also
edit it by hand: one `key=value` per line. A typo never blocks startup; FastPad points out the bad
line in a notification and applies the rest.

| Key | Values | Default |
|---|---|---|
| `font_face` | Any installed font name | `Consolas` |
| `preview_font` | Any installed font name, for Markdown preview text (code in the preview uses `font_face`) | `Segoe UI` |
| `preview_font_size` | Markdown preview text size in pixels (positive integer) | `14` |
| `preview_line_height` | Markdown preview line height, a multiple of the text size from `1.0` to `3.0` | `1.6` |
| `font_size` | Points (positive integer) | `11` |
| `tab_width` | 1–255 | `4` |
| `word_wrap` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `false` |
| `line_numbers` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `true` |
| `insert_spaces` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `false` |
| `show_whitespace` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `false` |
| `highlight_current_line` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `true` |
| `code_folding` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `true` |
| `always_on_top` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `false` |
| `capture_hotkey` | A key like `Ctrl+Alt+N`; empty turns it off | `Ctrl+Alt+N` |
| `capture_source` | `true`/`false`: note the window you were in when you pressed the capture hotkey | `false` |
| `markdown_view` | `source`, `live`, `split`, `preview`: the view Markdown tabs open in (also **Settings > Markdown view**) | `source` |
| `theme` | `system`, `light`, `dark`, `catppuccin`, `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`, `catppuccin-mocha`, `paper-lamp`, `paper`, `lamp` | `system` |
| `recovery_interval_seconds` | Seconds between recovery snapshots | `30` |
| `restore_session` | `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off` | `true` |
| `reload_changed_files` | `auto` (a tab without unsaved edits takes a file's outside changes by itself) or `ask` (every change waits for you) | `auto` |
| `sidebar_view` | `notebook`, `search`, `favorites`, `tags`, `none` | `notebook` |
| `sidebar_width` | 180–480 (pixels at 100% scaling) | `260` |
| `settings_size` | `<width>x<height>` (pixels at 100% scaling), saved when you resize Settings | none: its natural size |
| `window_placement` | `<x>,<y>,<width>x<height>`, then `,maximized` if it was; saved when the main window closes | none: opens at the default spot |
| `file_icons` | `material`, `minimal` or `solid` | `material` |

Hand edits keep your comments and other lines; the palette and Settings rewrite only the lines they change.

## Command line

```text
FastPad.exe [--new-window] [--read-only] [--portable] [path]
FastPad.exe --capture ["text" | -] [--attach <file>]...
```

- `FastPad.exe notes.md` opens the file in your running FastPad window, or starts FastPad if none is
  open.
- `--new-window` always starts a separate, independent window.
- `--read-only` opens the file for viewing only: typing, pasting and Save do nothing to it, while
  changes made to it outside FastPad still show. **File → Read-only** (or *File: Toggle read-only*
  in the command palette) switches any tab either way; the status bar says *Read-only* while it
  is on. A tab with unsaved changes can't be made read-only until they are saved or undone.
- `--portable` runs this launch in [portable mode](install.md#portable-mode), as a `portable` file next to
  `FastPad.exe` does for every launch.
- `--capture` opens the quick capture box. With text (or `-` to read it from standard input) and
  any `--attach` files, it files them into the inbox with no window and exits: `0` when filed, `1`
  when the write failed, `2` when the request was refused (a missing file, or one over 20 MB). It
  goes to your running FastPad when there is one.
- `--diagnostic` is for the startup benchmark harness; you don't need it.
