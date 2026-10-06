# Editing Shortcuts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** VS Code's default editing keys (line editing, comments, multi-cursor) as sixteen rebindable FastPad commands.

**Architecture:** Comment toggling is a pure text module (`src/editor/comment.rs`). The Scintilla-backed operations are `Editor` methods in a new `src/editor/scintilla/line_ops.rs`, set up once per visible editor by `configure_editing` (multi-select on, Scintilla's clashing keys cleared). The commands are wired like Undo: `CommandId` → keymap ID and default key → palette and Edit menu → `execute_command` → `with_editor`. Keys bound to them are translated only while an editor has focus.

**Tech Stack:** Rust (2024 edition), `windows-sys`, Scintilla 5.6.6 through its direct function.

**Spec:** `docs/superpowers/specs/2026-10-05-editing-shortcuts-design.md`

## Global Constraints

- Keys and IDs exactly as spec §3; no existing default binding changes.
- Each command invocation is one undo step.
- No new `windows-sys` features (if one becomes necessary, update `tools/audit-dependencies.ps1`).
- Never edit `src/editor/scintilla_constants.rs` by hand: add names to `tools/generate-scintilla-constants.ps1` and regenerate.
- Window tests need `native/out/x64` DLLs in the worktree and `-- --test-threads=1`.
- Compile gate per part: `cargo clippy --all-targets --all-features -- -D warnings`. Run only the targeted tests per part; full suite and every integration target once, at the end (Part 9).
- Comments in code follow the surrounding style: a doc line saying what and, where useful, the spec section.

## Review Focus

1. **Keys typed outside the editor** (find field, palette, tree): `Ctrl+Enter`, `Alt+Up`, `Ctrl+D` there must not edit the document → Part 7 test `editing_keys_stay_with_controls_outside_the_editor`.
2. **Last line without a line end:** copy line down / delete line / select line on the final line must not lose or merge text → Part 3 tests `copy_lines_down_on_the_last_line_adds_a_line_end` and `delete_lines_on_the_last_line_removes_the_preceding_line_end`.
3. **Mixed and blank lines in a comment toggle:** some lines commented, some not, blank lines between → Part 1 tests `mixed_lines_get_commented_not_uncommented` and `blank_lines_are_left_alone`.
4. **Releasing Alt after Alt+Click** must not open the menu band → Part 8 test `a_mouse_press_cancels_the_pending_alt_tap`.
5. **Ctrl+D whole-word vs substring:** a word started from an empty caret must skip `food` when looking for `foo` → Part 5 test `add_next_occurrence_from_a_caret_matches_whole_words_only`.

---

## Part 0: Worktree setup

- [ ] **Step 1: Copy the native sources and DLLs from the main checkout**

```bash
cp -r /d/Projects/FastPad/native/src /d/Projects/FastPad-editing-shortcuts/native/
cp -r /d/Projects/FastPad/native/out /d/Projects/FastPad-editing-shortcuts/native/
```

Both are gitignored. `native/src` feeds the constants generator; `native/out/x64` is loaded by window tests.

---

## Part 1: Comment logic (`src/editor/comment.rs`)

**Files:**
- Create: `src/editor/comment.rs`
- Modify: `src/editor/mod.rs` (add `pub mod comment;`)

**Interfaces:**
- Produces: `comment::CommentSyntax { line: Option<&'static str>, block: Option<(&'static str, &'static str)> }`, `Language::comment_syntax(self) -> CommentSyntax`, `comment::LineEdit { line, column, remove, insert }`, `comment::toggle_line(&[&str], CommentSyntax) -> Vec<LineEdit>`, `comment::BlockToggle { replacement: String, caret: Option<usize> }`, `comment::toggle_block(&str, CommentSyntax) -> Option<BlockToggle>`.

- [ ] **Step 1: Write the module with its tests first (implementation bodies `todo!()`)**

Create `src/editor/comment.rs` with the types and signatures below, function bodies as `todo!()`, and this test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const RUST: CommentSyntax = CommentSyntax {
        line: Some("//"),
        block: Some(("/*", "*/")),
    };
    const HTML: CommentSyntax = CommentSyntax {
        line: None,
        block: Some(("<!--", "-->")),
    };
    const NONE: CommentSyntax = CommentSyntax {
        line: None,
        block: None,
    };

    /// Applies `edits` (ascending) to `lines`, as the editor does: last first.
    fn apply(lines: &[&str], edits: &[LineEdit]) -> Vec<String> {
        let mut out: Vec<String> = lines.iter().map(|line| (*line).to_owned()).collect();
        for edit in edits.iter().rev() {
            out[edit.line].replace_range(edit.column..edit.column + edit.remove, &edit.insert);
        }
        out
    }

    #[test]
    fn every_language_has_the_spec_table_syntax() {
        use crate::document::Language as L;
        let c_like = RUST;
        let hash = CommentSyntax { line: Some("#"), block: None };
        let markup = HTML;
        for language in [L::C, L::Cpp, L::CSharp, L::JavaScript, L::TypeScript, L::Rust] {
            assert_eq!(language.comment_syntax(), c_like, "{language:?}");
        }
        for language in [L::Python, L::Bash, L::Yaml, L::Toml, L::Properties, L::Env] {
            assert_eq!(language.comment_syntax(), hash, "{language:?}");
        }
        for language in [L::Html, L::Xml, L::Svg, L::Markdown] {
            assert_eq!(language.comment_syntax(), markup, "{language:?}");
        }
        assert_eq!(L::Css.comment_syntax(), CommentSyntax { line: None, block: Some(("/*", "*/")) });
        assert_eq!(L::Sql.comment_syntax(), CommentSyntax { line: Some("--"), block: Some(("/*", "*/")) });
        assert_eq!(L::PowerShell.comment_syntax(), CommentSyntax { line: Some("#"), block: Some(("<#", "#>")) });
        assert_eq!(L::Ini.comment_syntax(), CommentSyntax { line: Some(";"), block: None });
        assert_eq!(L::Batch.comment_syntax(), CommentSyntax { line: Some("REM"), block: None });
        assert_eq!(L::PlainText.comment_syntax(), NONE);
        assert_eq!(L::Json.comment_syntax(), NONE);
    }

    #[test]
    fn uncommented_lines_get_markers_aligned_at_the_smallest_indent() {
        let lines = ["    let a = 1;", "        let b = 2;"];
        assert_eq!(
            apply(&lines, &toggle_line(&lines, RUST)),
            ["    // let a = 1;", "    //     let b = 2;"]
        );
    }

    #[test]
    fn commented_lines_lose_the_marker_and_one_space() {
        let lines = ["    // a", "  //b", "//  c"];
        assert_eq!(apply(&lines, &toggle_line(&lines, RUST)), ["    a", "  b", " c"]);
    }

    #[test]
    fn mixed_lines_get_commented_not_uncommented() {
        let lines = ["// a", "b"];
        assert_eq!(apply(&lines, &toggle_line(&lines, RUST)), ["// // a", "// b"]);
    }

    #[test]
    fn blank_lines_are_left_alone() {
        let lines = ["a", "", "   ", "b"];
        let edits = toggle_line(&lines, RUST);
        assert_eq!(apply(&lines, &edits), ["// a", "", "   ", "// b"]);
        let only_blank = ["", "  "];
        assert!(toggle_line(&only_blank, RUST).is_empty());
    }

    #[test]
    fn batch_uses_rem() {
        let syntax = crate::document::Language::Batch.comment_syntax();
        let lines = ["echo hi"];
        let commented = apply(&lines, &toggle_line(&lines, syntax));
        assert_eq!(commented, ["REM echo hi"]);
        let commented: Vec<&str> = commented.iter().map(String::as_str).collect();
        assert_eq!(apply(&commented, &toggle_line(&commented, syntax)), ["echo hi"]);
    }

    #[test]
    fn languages_without_a_line_marker_wrap_the_lines_in_a_block() {
        let lines = ["  <p>", "  </p>"];
        let wrapped = apply(&lines, &toggle_line(&lines, HTML));
        assert_eq!(wrapped, ["  <!-- <p>", "  </p> -->"]);
        let wrapped: Vec<&str> = wrapped.iter().map(String::as_str).collect();
        assert_eq!(apply(&wrapped, &toggle_line(&wrapped, HTML)), ["  <p>", "  </p>"]);
    }

    #[test]
    fn a_single_wrapped_line_unwraps() {
        let lines = ["<!-- hi -->"];
        assert_eq!(apply(&lines, &toggle_line(&lines, HTML)), ["hi"]);
        let empty = ["<!-- -->"];
        assert_eq!(apply(&empty, &toggle_line(&empty, HTML)), [""]);
    }

    #[test]
    fn languages_without_comments_do_nothing() {
        assert!(toggle_line(&["a"], NONE).is_empty());
        assert_eq!(toggle_block("a", NONE), None);
    }

    #[test]
    fn block_wraps_a_selection() {
        assert_eq!(
            toggle_block("a + b", RUST),
            Some(BlockToggle { replacement: "/* a + b */".to_owned(), caret: None })
        );
    }

    #[test]
    fn block_unwraps_keeping_outer_whitespace() {
        assert_eq!(
            toggle_block(" /* a + b */ ", RUST),
            Some(BlockToggle { replacement: " a + b ".to_owned(), caret: None })
        );
        assert_eq!(
            toggle_block("/**/", RUST),
            Some(BlockToggle { replacement: String::new(), caret: None })
        );
    }

    #[test]
    fn block_on_an_empty_selection_inserts_a_pair_with_the_caret_inside() {
        assert_eq!(
            toggle_block("", RUST),
            Some(BlockToggle { replacement: "/*  */".to_owned(), caret: Some(3) })
        );
    }

    #[test]
    fn too_short_to_be_a_comment_is_wrapped() {
        assert_eq!(
            toggle_block("/*/", RUST),
            Some(BlockToggle { replacement: "/* /*/ */".to_owned(), caret: None })
        );
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --lib comment::tests`
Expected: FAIL (panics at `todo!()`).

- [ ] **Step 3: Implement**

Replace the `todo!()` bodies so the file reads:

```rust
//! Toggling line and block comments (editing shortcuts spec §4). Pure: text in, edits out; the
//! editor reads the lines and applies the edits.

use crate::document::Language;

/// A language's comment markers: a line marker, a block pair, either or neither.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommentSyntax {
    pub line: Option<&'static str>,
    pub block: Option<(&'static str, &'static str)>,
}

/// One change inside line `line` of the run: at byte `column`, remove `remove` bytes, then insert
/// `insert`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineEdit {
    pub line: usize,
    pub column: usize,
    pub remove: usize,
    pub insert: String,
}

/// What a block toggle replaces the selection with. `caret` is where the caret goes, as an offset
/// into the replacement, when the selection was empty; otherwise the replacement is selected.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockToggle {
    pub replacement: String,
    pub caret: Option<usize>,
}

const C_LIKE: CommentSyntax = CommentSyntax {
    line: Some("//"),
    block: Some(("/*", "*/")),
};
const HASH: CommentSyntax = CommentSyntax {
    line: Some("#"),
    block: None,
};
const MARKUP: CommentSyntax = CommentSyntax {
    line: None,
    block: Some(("<!--", "-->")),
};

impl Language {
    /// The markers Toggle line comment and Toggle block comment use (spec §4.1).
    pub const fn comment_syntax(self) -> CommentSyntax {
        match self {
            Self::C | Self::Cpp | Self::CSharp | Self::JavaScript | Self::TypeScript | Self::Rust => {
                C_LIKE
            }
            Self::Css => CommentSyntax {
                line: None,
                block: Some(("/*", "*/")),
            },
            Self::Sql => CommentSyntax {
                line: Some("--"),
                block: Some(("/*", "*/")),
            },
            Self::Python | Self::Bash | Self::Yaml | Self::Toml | Self::Properties | Self::Env => {
                HASH
            }
            Self::PowerShell => CommentSyntax {
                line: Some("#"),
                block: Some(("<#", "#>")),
            },
            Self::Ini => CommentSyntax {
                line: Some(";"),
                block: None,
            },
            Self::Batch => CommentSyntax {
                line: Some("REM"),
                block: None,
            },
            Self::Html | Self::Xml | Self::Svg | Self::Markdown => MARKUP,
            Self::PlainText | Self::Json => CommentSyntax {
                line: None,
                block: None,
            },
        }
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Toggles comments on one contiguous run of `lines` (without line ends), numbered from 0 in the
/// edits, which come in ascending order. Uses the line marker, else wraps the run in the block
/// pair (spec §4.2).
pub fn toggle_line(lines: &[&str], syntax: CommentSyntax) -> Vec<LineEdit> {
    match (syntax.line, syntax.block) {
        (Some(marker), _) => toggle_with_marker(lines, marker),
        (None, Some((open, close))) => toggle_run_in_block(lines, open, close),
        (None, None) => Vec::new(),
    }
}

fn toggle_with_marker(lines: &[&str], marker: &str) -> Vec<LineEdit> {
    let filled: Vec<(usize, &str)> = lines
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, line)| !is_blank(line))
        .collect();
    let commented = !filled.is_empty()
        && filled
            .iter()
            .all(|(_, line)| line[indent_of(line)..].starts_with(marker));
    if commented {
        return filled
            .iter()
            .map(|&(line, text)| {
                let column = indent_of(text);
                let after = &text[column + marker.len()..];
                LineEdit {
                    line,
                    column,
                    remove: marker.len() + usize::from(after.starts_with(' ')),
                    insert: String::new(),
                }
            })
            .collect();
    }
    let column = filled
        .iter()
        .map(|(_, line)| indent_of(line))
        .min()
        .unwrap_or(0);
    filled
        .iter()
        .map(|&(line, _)| LineEdit {
            line,
            column,
            remove: 0,
            insert: format!("{marker} "),
        })
        .collect()
}

fn toggle_run_in_block(lines: &[&str], open: &str, close: &str) -> Vec<LineEdit> {
    let Some(first) = lines.iter().position(|line| !is_blank(line)) else {
        return Vec::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !is_blank(line))
        .unwrap_or(first);
    let (head, tail) = (lines[first], lines[last]);
    let open_at = indent_of(head);
    let tail_end = tail.trim_end().len();
    let long_enough = first != last || tail_end - open_at >= open.len() + close.len();
    let wrapped =
        long_enough && head[open_at..].starts_with(open) && tail[..tail_end].ends_with(close);
    if !wrapped {
        return vec![
            LineEdit {
                line: first,
                column: open_at,
                remove: 0,
                insert: format!("{open} "),
            },
            LineEdit {
                line: last,
                column: tail_end,
                remove: 0,
                insert: format!(" {close}"),
            },
        ];
    }
    let open_remove = open.len() + usize::from(head[open_at + open.len()..].starts_with(' '));
    let close_start = tail_end - close.len();
    // On one line the space before the close marker may be the one the open marker's removal
    // already takes ("<!-- -->").
    let open_end = if first == last { open_at + open_remove } else { 0 };
    let space_before =
        close_start > open_end && tail.as_bytes()[close_start - 1] == b' ';
    let close_at = close_start - usize::from(space_before);
    vec![
        LineEdit {
            line: first,
            column: open_at,
            remove: open_remove,
            insert: String::new(),
        },
        LineEdit {
            line: last,
            column: close_at,
            remove: tail_end - close_at,
            insert: String::new(),
        },
    ]
}

/// Wraps `selected` in the block pair, or unwraps it when its trimmed text already is one
/// (spec §4.3). `None` for a language with no block pair.
pub fn toggle_block(selected: &str, syntax: CommentSyntax) -> Option<BlockToggle> {
    let (open, close) = syntax.block?;
    if selected.is_empty() {
        return Some(BlockToggle {
            replacement: format!("{open}  {close}"),
            caret: Some(open.len() + 1),
        });
    }
    let trimmed = selected.trim();
    if trimmed.len() >= open.len() + close.len()
        && trimmed.starts_with(open)
        && trimmed.ends_with(close)
    {
        let lead = &selected[..selected.len() - selected.trim_start().len()];
        let trail = &selected[selected.trim_end().len()..];
        let inner = &trimmed[open.len()..trimmed.len() - close.len()];
        let inner = inner.strip_prefix(' ').unwrap_or(inner);
        let inner = inner.strip_suffix(' ').unwrap_or(inner);
        return Some(BlockToggle {
            replacement: format!("{lead}{inner}{trail}"),
            caret: None,
        });
    }
    Some(BlockToggle {
        replacement: format!("{open} {selected} {close}"),
        caret: None,
    })
}
```

Note on `a_single_wrapped_line_unwraps`: for `"<!-- hi -->"`, `open_remove` = 5, `open_end` = 5, `close_start` = 8, the byte at 7 is a space and 8 > 5, so `close_at` = 7: edits remove `0..5` and `7..11`, leaving `"hi"`. For `"<!-- -->"`, `close_start` = 5 is not > `open_end` = 5, so only `5..8` goes with `0..5`, leaving `""`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test --lib comment::tests`
Expected: 13 passed.

---

## Part 2: Scintilla constants, editor setup, copy/cut whole line

**Files:**
- Modify: `tools/generate-scintilla-constants.ps1` (`$RequiredNames`)
- Regenerate: `src/editor/scintilla_constants.rs`
- Create: `src/editor/scintilla/line_ops.rs`
- Modify: `src/editor/scintilla.rs` (`mod line_ops;`, `EditorEndpoint` fields, `finish`, imports)
- Modify: `src/editor/scintilla/editing.rs` (`cut`, `copy`)
- Modify: `src/editor/scintilla/tests.rs` (`cut_copy_paste_send_the_matching_scintilla_messages`)
- Create: `src/window/main_window/tests/editing_shortcuts.rs`
- Modify: `src/window/main_window/tests.rs` (`mod editing_shortcuts;`)

**Interfaces:**
- Produces: `Editor::configure_editing(&self) -> Result<()>` (called from `Editor::finish`), `Editor::selections(&self) -> Result<Vec<Range<usize>>>`, `Editor::carets(&self) -> Result<Vec<usize>>`, and private helpers on `Editor` used by Parts 3–5: `send`, `line_start`, `line_end`, `touched_lines`, `touched_runs`, `line_ending`.

- [ ] **Step 1: Add the constant names and regenerate**

In `tools/generate-scintilla-constants.ps1`, change the last line of `$RequiredNames` from

```powershell
    "SCI_FOLDALL", "SC_FOLDACTION_CONTRACT", "SC_FOLDACTION_EXPAND"
)
```

to

```powershell
    "SCI_FOLDALL", "SC_FOLDACTION_CONTRACT", "SC_FOLDACTION_EXPAND",
    "SCI_ASSIGNCMDKEY", "SCI_CLEARCMDKEY", "SCK_UP", "SCK_DOWN", "SCK_LEFT", "SCK_RIGHT",
    "SCMOD_SHIFT", "SCMOD_CTRL", "SCMOD_ALT", "SCI_LINEUPRECTEXTEND", "SCI_LINEDOWNRECTEXTEND",
    "SCI_CHARLEFTRECTEXTEND", "SCI_CHARRIGHTRECTEXTEND", "SCI_SETMULTIPLESELECTION",
    "SCI_SETADDITIONALSELECTIONTYPING", "SCI_SETMULTIPASTE", "SC_MULTIPASTE_EACH",
    "SCI_MOVESELECTEDLINESUP", "SCI_MOVESELECTEDLINESDOWN", "SCI_GETSELECTIONS",
    "SCI_GETMAINSELECTION", "SCI_SETMAINSELECTION", "SCI_GETSELECTIONNSTART",
    "SCI_GETSELECTIONNEND", "SCI_GETSELECTIONNCARET", "SCI_GETSELECTIONNANCHOR",
    "SCI_ADDSELECTION", "SCI_SETSELECTION", "SCI_MULTIPLESELECTADDNEXT",
    "SCI_MULTIPLESELECTADDEACH", "SCI_TARGETWHOLEDOCUMENT", "SCI_COPYALLOWLINE",
    "SCI_CUTALLOWLINE", "SCI_GETLINEINDENTATION", "SCI_SETLINEINDENTATION",
    "SCI_GETLINEINDENTPOSITION", "SCI_GETINDENT", "SCI_GETEOLMODE", "SC_EOL_CR", "SC_EOL_LF",
    "SCI_FINDCOLUMN", "SCI_POSITIONFROMPOINT", "SCI_POINTXFROMPOSITION", "SCI_POINTYFROMPOSITION"
)
```

Run: `pwsh -NoProfile -File tools/generate-scintilla-constants.ps1`
Expected: exits 0; `git diff --stat src/editor/scintilla_constants.rs` shows only added lines.

- [ ] **Step 2: Write the failing window tests**

Create `src/window/main_window/tests/editing_shortcuts.rs`:

```rust
//! Editing shortcuts (editing shortcuts spec): line operations, comments, multiple carets and
//! their keys, on a real Scintilla.

use super::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;

/// A main window with a real editor holding `text`. Fields drop in order: the editor and window
/// before the Scintilla DLL.
struct Fixture {
    editor: crate::editor::Editor,
    window: ProductionWindow,
    _scintilla: crate::platform::OwnedModule,
}

fn fixture(text: &str) -> Fixture {
    let scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    // Four-space indentation, whatever Scintilla's defaults (tabs, width 8).
    unsafe {
        SendMessageW(editor.hwnd(), crate::editor::scintilla_constants::SCI_SETTABWIDTH, 4, 0);
        SendMessageW(editor.hwnd(), crate::editor::scintilla_constants::SCI_SETUSETABS, 0, 0);
    }
    editor.populate_clean(text).unwrap();
    Fixture {
        editor,
        window,
        _scintilla: scintilla,
    }
}

/// Holds `keys` down in this thread's keyboard state while `run` runs.
fn with_keys_down<R>(keys: &[u16], run: impl FnOnce() -> R) -> R {
    let mut state = [0u8; 256];
    unsafe { GetKeyboardState(state.as_mut_ptr()) };
    let original = state;
    for key in keys {
        state[usize::from(*key)] = 0x80;
    }
    unsafe { SetKeyboardState(state.as_ptr()) };
    let result = run();
    unsafe { SetKeyboardState(original.as_ptr()) };
    result
}

/// One Undo restores `original` and leaves nothing more to undo.
fn assert_one_undo_restores(editor: &crate::editor::Editor, original: &str) {
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), original);
    assert!(!editor.can_undo().unwrap(), "more than one undo step");
}

#[test]
fn copy_with_an_empty_selection_copies_the_line_and_paste_puts_it_above() {
    // Break caught: Copy with nothing selected copying nothing, so Paste does nothing (VS Code
    // copies the whole line and pastes it as a line).
    let f = fixture("one\r\ntwo\r\nthree");
    f.editor.set_selection(6..6).unwrap();
    f.editor.copy().unwrap();
    f.editor.set_selection(1..1).unwrap();
    f.editor.paste().unwrap();
    assert_eq!(f.editor.text().unwrap(), "two\r\none\r\ntwo\r\nthree");
}

#[test]
fn cut_with_an_empty_selection_cuts_the_line() {
    let f = fixture("one\r\ntwo\r\nthree");
    f.editor.set_selection(6..6).unwrap();
    f.editor.cut().unwrap();
    assert_eq!(f.editor.text().unwrap(), "one\r\nthree");
}

#[test]
fn every_caret_types() {
    // Break caught: multiple selection off, or additional selections not typing, so a second
    // caret is ignored.
    let f = fixture("ab\r\ncd");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_selection_for_test(4);
    unsafe { SendMessageW(f.editor.hwnd(), windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR, usize::from(b'x'), 0) };
    assert_eq!(f.editor.text().unwrap(), "xab\r\nxcd");
}

#[test]
fn escape_in_the_editor_drops_the_extra_carets() {
    let f = fixture("ab\r\ncd");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_selection_for_test(4);
    assert_eq!(f.editor.carets().unwrap().len(), 2);
    unsafe { SendMessageW(f.editor.hwnd(), WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    assert_eq!(f.editor.carets().unwrap().len(), 1);
}

#[test]
fn scintillas_own_line_keys_are_cleared() {
    // Break caught: Ctrl+D / Ctrl+L / Ctrl+Shift+L reaching Scintilla (a rebound key, or focus
    // in the editor with the accelerator unbound) duplicating, cutting or deleting a line.
    let f = fixture("one\r\ntwo");
    f.editor.set_selection(1..1).unwrap();
    for (keys, letter) in [
        (&[VK_CONTROL][..], b'D'),
        (&[VK_CONTROL][..], b'L'),
        (&[VK_CONTROL, VK_SHIFT][..], b'L'),
        (&[VK_CONTROL][..], b'T'),
    ] {
        with_keys_down(keys, || unsafe {
            SendMessageW(f.editor.hwnd(), WM_KEYDOWN, usize::from(letter), 0)
        });
    }
    assert_eq!(f.editor.text().unwrap(), "one\r\ntwo");
}
```

The tests use `Editor::add_selection_for_test`, defined in Step 3, to add a caret without going through a command.

In `src/window/main_window/tests.rs`, add `mod editing_shortcuts;` in the alphabetical `mod` list (after `mod copy_host_and_panel_drop;`).

`VK_MENU` is imported for later parts; until Part 5 uses it, add `#[allow(unused_imports)]` on that `use` line and remove it in Part 8.

- [ ] **Step 3: Create `line_ops.rs` with the setup and shared helpers**

Create `src/editor/scintilla/line_ops.rs`:

```rust
//! `Editor`'s editing-shortcut operations (editing shortcuts spec): line editing, comments and
//! multiple carets, plus the one-time setup that turns VS Code's editing model on.

use super::*;
use crate::editor::scintilla_constants::{
    SC_EOL_CR, SC_EOL_LF, SC_MULTIPASTE_EACH, SCI_ADDSELECTION, SCI_ASSIGNCMDKEY,
    SCI_CHARLEFTRECTEXTEND, SCI_CHARRIGHTRECTEXTEND, SCI_CLEARCMDKEY, SCI_GETEOLMODE,
    SCI_GETLINEENDPOSITION, SCI_GETSELECTIONNCARET, SCI_GETSELECTIONNEND,
    SCI_GETSELECTIONNSTART, SCI_GETSELECTIONS, SCI_LINEDOWNRECTEXTEND, SCI_LINEUPRECTEXTEND,
    SCI_POSITIONFROMLINE, SCI_SETADDITIONALSELECTIONTYPING, SCI_SETMULTIPASTE,
    SCI_SETMULTIPLESELECTION, SCK_DOWN, SCK_LEFT, SCK_RIGHT, SCK_UP, SCMOD_ALT, SCMOD_CTRL,
    SCMOD_SHIFT,
};
use std::ops::RangeInclusive;

/// A Scintilla key definition: the key in the low word, `SCMOD_*` modifiers in the high word.
const fn key_definition(key: u32, modifiers: u32) -> usize {
    (key | (modifiers << 16)) as usize
}

const SHIFT_ALT: u32 = SCMOD_SHIFT | SCMOD_ALT;
const CTRL_SHIFT_ALT: u32 = SCMOD_CTRL | SCMOD_SHIFT | SCMOD_ALT;

/// Scintilla's defaults on keys FastPad's commands now own (spec §5).
const CLEARED_KEYS: [usize; 11] = [
    key_definition(b'D' as u32, SCMOD_CTRL),
    key_definition(b'L' as u32, SCMOD_CTRL),
    key_definition(b'L' as u32, SCMOD_CTRL | SCMOD_SHIFT),
    key_definition(b'T' as u32, SCMOD_CTRL),
    key_definition(b'T' as u32, SCMOD_CTRL | SCMOD_SHIFT),
    key_definition(b'[' as u32, SCMOD_CTRL),
    key_definition(b']' as u32, SCMOD_CTRL),
    key_definition(SCK_UP, SHIFT_ALT),
    key_definition(SCK_DOWN, SHIFT_ALT),
    key_definition(SCK_LEFT, SHIFT_ALT),
    key_definition(SCK_RIGHT, SHIFT_ALT),
];

/// Column selection by keyboard, on VS Code's keys (spec §5).
const RECTANGLE_KEYS: [(usize, u32); 4] = [
    (key_definition(SCK_UP, CTRL_SHIFT_ALT), SCI_LINEUPRECTEXTEND),
    (key_definition(SCK_DOWN, CTRL_SHIFT_ALT), SCI_LINEDOWNRECTEXTEND),
    (key_definition(SCK_LEFT, CTRL_SHIFT_ALT), SCI_CHARLEFTRECTEXTEND),
    (key_definition(SCK_RIGHT, CTRL_SHIFT_ALT), SCI_CHARRIGHTRECTEXTEND),
];

/// Sorted, merged runs of lines from `spans` (each `first..=last`).
fn merge_runs(mut spans: Vec<RangeInclusive<usize>>) -> Vec<RangeInclusive<usize>> {
    spans.sort_by_key(|span| *span.start());
    let mut runs: Vec<RangeInclusive<usize>> = Vec::with_capacity(spans.len());
    for span in spans {
        match runs.last_mut() {
            Some(last) if *span.start() <= last.end() + 1 => {
                *last = *last.start()..=(*last.end()).max(*span.end());
            }
            _ => runs.push(span),
        }
    }
    runs
}

impl Editor {
    fn send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// Several carets that all type and paste, and Scintilla's own keys moved off the strokes
    /// FastPad's commands own (spec §2, §5).
    pub(crate) fn configure_editing(&self) -> Result<()> {
        self.send(SCI_SETMULTIPLESELECTION, 1, 0)?;
        self.send(SCI_SETADDITIONALSELECTIONTYPING, 1, 0)?;
        self.send(SCI_SETMULTIPASTE, SC_MULTIPASTE_EACH as usize, 0)?;
        for key in CLEARED_KEYS {
            self.send(SCI_CLEARCMDKEY, key, 0)?;
        }
        for (key, command) in RECTANGLE_KEYS {
            self.send(SCI_ASSIGNCMDKEY, key, command as isize)?;
        }
        Ok(())
    }

    /// Every selection, the main one included, in Scintilla's order.
    pub(crate) fn selections(&self) -> Result<Vec<Range<usize>>> {
        let count = self.send(SCI_GETSELECTIONS, 0, 0)?.max(1) as usize;
        (0..count)
            .map(|n| {
                let start = self.send(SCI_GETSELECTIONNSTART, n, 0)?.max(0) as usize;
                let end = self.send(SCI_GETSELECTIONNEND, n, 0)?.max(0) as usize;
                Ok(start..end)
            })
            .collect()
    }

    /// Every selection's caret, in Scintilla's order.
    pub(crate) fn carets(&self) -> Result<Vec<usize>> {
        let count = self.send(SCI_GETSELECTIONS, 0, 0)?.max(1) as usize;
        (0..count)
            .map(|n| Ok(self.send(SCI_GETSELECTIONNCARET, n, 0)?.max(0) as usize))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn add_selection_for_test(&self, caret: usize) {
        self.send(SCI_ADDSELECTION, caret, caret as isize).unwrap();
    }

    /// Where `line` starts; the document's length past the last line.
    fn line_start(&self, line: usize) -> Result<usize> {
        if line >= self.line_count()? {
            return self.length();
        }
        Ok(self.send(SCI_POSITIONFROMLINE, line, 0)?.max(0) as usize)
    }

    /// Where `line`'s text ends, before its line end.
    fn line_end(&self, line: usize) -> Result<usize> {
        Ok(self.send(SCI_GETLINEENDPOSITION, line, 0)?.max(0) as usize)
    }

    /// The lines `range` touches. A non-empty range ending at a line's start leaves that line
    /// out, as VS Code does.
    fn touched_lines(&self, range: Range<usize>) -> Result<RangeInclusive<usize>> {
        let first = self.line_from_position(range.start)?;
        let mut last = self.line_from_position(range.end)?;
        if last > first && range.end == self.line_start(last)? {
            last -= 1;
        }
        Ok(first..=last)
    }

    /// The lines every selection touches, as sorted, merged runs.
    fn touched_runs(&self) -> Result<Vec<RangeInclusive<usize>>> {
        let spans = self
            .selections()?
            .into_iter()
            .map(|range| self.touched_lines(range))
            .collect::<Result<Vec<_>>>()?;
        Ok(merge_runs(spans))
    }

    /// The line end `line` has, else the one before it, else the document's end-of-line mode:
    /// new lines match the document rather than the platform.
    fn line_ending(&self, line: usize) -> Result<String> {
        let count = self.line_count()?;
        for candidate in [line, line.saturating_sub(1)] {
            if candidate + 1 < count {
                let range = self.line_end(candidate)?..self.line_start(candidate + 1)?;
                return Ok(String::from_utf8_lossy(self.range_bytes(range)?).into_owned());
            }
        }
        Ok(match self.send(SCI_GETEOLMODE, 0, 0)? as u32 {
            SC_EOL_LF => "\n",
            SC_EOL_CR => "\r",
            _ => "\r\n",
        }
        .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::merge_runs;

    #[test]
    fn runs_merge_when_they_overlap_or_touch() {
        assert_eq!(merge_runs(vec![5..=6, 0..=1, 2..=2, 9..=9]), vec![0..=2, 5..=6, 9..=9]);
        assert_eq!(merge_runs(vec![3..=8, 4..=5]), vec![3..=8]);
    }
}
```

Each later part extends this file's `scintilla_constants` import with the names it lists.

- [ ] **Step 4: Wire it into `scintilla.rs`**

1. After `mod editing;` add `mod line_ops;`.
2. In `Editor::finish`, after `editor.initialize_view(...)?;`, add:

```rust
        // Cosmetic like the chrome: an editor without the editing keys still edits.
        let _ = editor.configure_editing();
```

- [ ] **Step 5: Copy and Cut take the whole line when nothing is selected**

In `src/editor/scintilla/editing.rs`, change the Windows `cut` to send `SCI_CUTALLOWLINE` and `copy` to send `SCI_COPYALLOWLINE`, and add a doc line on each:

```rust
    /// Cuts the selection, or the caret's whole line when the selection is empty (spec §3).
    #[cfg(windows)]
    pub fn cut(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_CUTALLOWLINE, 0, 0)?;
        Ok(())
    }
```

```rust
    /// Copies the selection, or the caret's whole line when the selection is empty (spec §3).
    #[cfg(windows)]
    pub fn copy(&self) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_COPYALLOWLINE, 0, 0)?;
        Ok(())
    }
```

Add `SCI_COPYALLOWLINE, SCI_CUTALLOWLINE` to the `use crate::editor::scintilla_constants::{...}` block in `scintilla.rs` and remove `SCI_COPY`, `SCI_CUT` from it. In `src/editor/scintilla/tests.rs`, swap `SCI_COPY, SCI_CUT` for `SCI_COPYALLOWLINE, SCI_CUTALLOWLINE` in the import and change the assertion in `cut_copy_paste_send_the_matching_scintilla_messages` to:

```rust
    assert_eq!(
        harness.messages(),
        vec![SCI_CUTALLOWLINE, SCI_COPYALLOWLINE, SCI_PASTE]
    );
```

- [ ] **Step 6: Compile and run the part's tests**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: clean.

Run: `cargo test --lib -- editing_shortcuts line_ops::tests cut_copy_paste --test-threads=1`
Expected: all pass (6 window tests, 1 merge test, 1 fake-harness test).

---

## Part 3: Line operations

**Files:**
- Modify: `src/editor/scintilla/line_ops.rs`
- Modify: `src/window/main_window/tests/editing_shortcuts.rs`

**Interfaces:**
- Consumes: Part 2 helpers.
- Produces: `Editor::move_lines(&self, up: bool)`, `copy_lines(&self, down: bool)`, `delete_lines(&self)`, `insert_line(&self, below: bool)`, `indent_lines(&self, outdent: bool)`, `expand_line_selection(&self)`, all `-> Result<()>`.

- [ ] **Step 1: Write the failing tests** (append to `editing_shortcuts.rs`)

```rust
#[test]
fn move_lines_moves_the_touched_lines_in_one_undo_step() {
    let f = fixture("a\nb\nc");
    f.editor.set_selection(2..3).unwrap(); // "b"
    f.editor.move_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "b\na\nc");
    f.editor.move_lines(false).unwrap();
    f.editor.move_lines(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\nc\nb");
    f.editor.undo().unwrap();
    f.editor.undo().unwrap();
    assert_one_undo_restores(&f.editor, "a\nb\nc");
}

#[test]
fn copy_lines_up_keeps_the_selection_on_the_upper_copy() {
    let f = fixture("a\nbc\nd");
    f.editor.set_selection(3..4).unwrap(); // "c"
    f.editor.copy_lines(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\nbc\nbc\nd");
    assert_eq!(f.editor.selection().unwrap(), 3..4);
    assert_one_undo_restores(&f.editor, "a\nbc\nd");
}

#[test]
fn copy_lines_down_moves_the_selection_to_the_lower_copy() {
    let f = fixture("a\nbc\nd");
    f.editor.set_selection(3..4).unwrap();
    f.editor.copy_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\nbc\nbc\nd");
    assert_eq!(f.editor.selection().unwrap(), 6..7);
}

#[test]
fn copy_lines_down_on_the_last_line_adds_a_line_end() {
    let f = fixture("a\r\nb");
    f.editor.set_selection(4..4).unwrap();
    f.editor.copy_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\r\nb\r\nb");
}

#[test]
fn delete_lines_removes_every_touched_line_and_keeps_the_column() {
    let f = fixture("aa\nbb\ncc\ndd\nee");
    f.editor.set_selection(4..4).unwrap(); // line 1, column 1
    f.editor.add_selection_for_test(10); // line 3
    f.editor.delete_lines().unwrap();
    assert_eq!(f.editor.text().unwrap(), "aa\ncc\nee");
    assert_eq!(f.editor.carets().unwrap(), vec![4]); // line 1 ("cc"), column 1
    assert_one_undo_restores(&f.editor, "aa\nbb\ncc\ndd\nee");
}

#[test]
fn delete_lines_on_the_last_line_removes_the_preceding_line_end() {
    let f = fixture("a\nb");
    f.editor.set_selection(2..2).unwrap();
    f.editor.delete_lines().unwrap();
    assert_eq!(f.editor.text().unwrap(), "a");
    let only = fixture("solo");
    only.editor.delete_lines().unwrap();
    assert_eq!(only.editor.text().unwrap(), "");
}

#[test]
fn insert_line_below_and_above_keep_the_indentation() {
    let f = fixture("  a\nb");
    f.editor.set_selection(1..1).unwrap();
    f.editor.insert_line(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "  a\n  \nb");
    assert_eq!(f.editor.selection().unwrap(), 6..6);
    f.editor.set_selection(2..2).unwrap();
    f.editor.insert_line(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "  \n  a\n  \nb");
    assert_eq!(f.editor.selection().unwrap(), 2..2);
}

#[test]
fn indent_and_outdent_move_lines_by_one_level_whatever_the_selection() {
    let f = fixture("a\n\n   b");
    f.editor.set_selection(0..6).unwrap(); // touches all three lines
    f.editor.indent_lines(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "    a\n\n    b"); // blank line left alone; 3 → 4
    assert_one_undo_restores(&f.editor, "a\n\n   b");
    f.editor.set_selection(0..6).unwrap(); // Undo moved the selection
    f.editor.indent_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\n\nb"); // 3 → 0, 0 stays 0
}

#[test]
fn select_line_selects_the_line_then_extends_it() {
    let f = fixture("ab\ncd\nef");
    f.editor.set_selection(4..4).unwrap();
    f.editor.expand_line_selection().unwrap();
    assert_eq!(f.editor.selection().unwrap(), 3..6);
    f.editor.expand_line_selection().unwrap();
    assert_eq!(f.editor.selection().unwrap(), 3..8);
    f.editor.expand_line_selection().unwrap();
    assert_eq!(f.editor.selection().unwrap(), 3..8);
}
```

The fixture sets four-space indentation, so the expected text is deterministic.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --lib -- editing_shortcuts --test-threads=1`
Expected: compile errors (methods missing).

- [ ] **Step 3: Implement** (add to `impl Editor` in `line_ops.rs`; extend its constant import with `SCI_FINDCOLUMN, SCI_GETCOLUMN, SCI_GETCURRENTPOS, SCI_GETANCHOR, SCI_GETINDENT, SCI_GETLINEINDENTATION, SCI_GETLINEINDENTPOSITION, SCI_GETTABWIDTH, SCI_MOVESELECTEDLINESDOWN, SCI_MOVESELECTEDLINESUP, SCI_SETLINEINDENTATION, SCI_SETSEL`)

```rust
    /// The touched lines of the main selection swap with the line above or below (spec §3).
    pub fn move_lines(&self, up: bool) -> Result<()> {
        let message = if up {
            SCI_MOVESELECTEDLINESUP
        } else {
            SCI_MOVESELECTEDLINESDOWN
        };
        self.begin_undo_action();
        let result = self.send(message, 0, 0);
        self.end_undo_action();
        result.map(drop)
    }

    /// Duplicates the main selection's touched lines; the selection ends on the lower copy when
    /// copying down, the upper one when copying up (spec §3).
    pub fn copy_lines(&self, down: bool) -> Result<()> {
        let anchor = self.send(SCI_GETANCHOR, 0, 0)?.max(0) as usize;
        let caret = self.send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        let lines = self.touched_lines(anchor.min(caret)..anchor.max(caret))?;
        let start = self.line_start(*lines.start())?;
        let end = self.line_end(*lines.end())?;
        let text = String::from_utf8_lossy(self.range_bytes(start..end)?).into_owned();
        let eol = self.line_ending(*lines.end())?;
        let (at, inserted) = if down {
            (end, format!("{eol}{text}"))
        } else {
            (start, format!("{text}{eol}"))
        };
        self.begin_undo_action();
        let result = self.replace_target(at..at, &inserted);
        self.end_undo_action();
        result?;
        let shift = if down { inserted.len() } else { 0 };
        self.send(SCI_SETSEL, anchor + shift, (caret + shift) as isize)
            .map(drop)
    }

    /// Deletes every line any selection touches, line ends included; one caret stays, on the
    /// line that took the main caret's line's place, in the same column (spec §3).
    pub fn delete_lines(&self) -> Result<()> {
        let caret = self.send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        let caret_line = self.line_from_position(caret)?;
        let column = self.send(SCI_GETCOLUMN, caret, 0)?;
        let runs = self.touched_runs()?;
        self.begin_undo_action();
        let result = runs.iter().rev().try_for_each(|run| {
            let range = if run.end() + 1 < self.line_count()? {
                self.line_start(*run.start())?..self.line_start(run.end() + 1)?
            } else if *run.start() > 0 {
                self.line_end(run.start() - 1)?..self.length()?
            } else {
                0..self.length()?
            };
            self.replace_target(range, "").map(drop)
        });
        self.end_undo_action();
        result?;
        let removed_above: usize = runs
            .iter()
            .filter(|run| *run.end() < caret_line)
            .map(|run| run.end() - run.start() + 1)
            .sum();
        let base = runs
            .iter()
            .find(|run| run.contains(&caret_line))
            .map_or(caret_line, |run| *run.start());
        let line = (base - removed_above).min(self.line_count()?.saturating_sub(1));
        let position = self.send(SCI_FINDCOLUMN, line, column)?.max(0) as usize;
        self.send(SCI_SETSEL, position, position as isize).map(drop)
    }

    /// A new line below or above the main caret's line, with that line's indentation; the caret
    /// moves to it (spec §3).
    pub fn insert_line(&self, below: bool) -> Result<()> {
        let caret = self.send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        let line = self.line_from_position(caret)?;
        let indentation = self.send(SCI_GETLINEINDENTATION, line, 0)?;
        let eol = self.line_ending(line)?;
        let (at, new_line) = if below {
            (self.line_end(line)?, line + 1)
        } else {
            (self.line_start(line)?, line)
        };
        self.begin_undo_action();
        let result = self
            .replace_target(at..at, &eol)
            .and_then(|_| self.send(SCI_SETLINEINDENTATION, new_line, indentation));
        self.end_undo_action();
        result?;
        let position = self.send(SCI_GETLINEINDENTPOSITION, new_line, 0)?.max(0) as usize;
        self.send(SCI_SETSEL, position, position as isize).map(drop)
    }

    /// Indents (or outdents) every touched line to the next (or previous) indent stop, whatever
    /// the selection. Indenting skips empty lines (spec §3).
    pub fn indent_lines(&self, outdent: bool) -> Result<()> {
        let width = match self.send(SCI_GETINDENT, 0, 0)? {
            0 => self.send(SCI_GETTABWIDTH, 0, 0)?,
            width => width,
        }
        .max(1);
        let runs = self.touched_runs()?;
        self.begin_undo_action();
        let result = runs.iter().flat_map(Clone::clone).try_for_each(|line| {
            let current = self.send(SCI_GETLINEINDENTATION, line, 0)?;
            let next = if outdent {
                if current == 0 {
                    return Ok(());
                }
                (current - 1) / width * width
            } else {
                if self.line_end(line)? == self.line_start(line)? {
                    return Ok(());
                }
                (current / width + 1) * width
            };
            self.send(SCI_SETLINEINDENTATION, line, next).map(drop)
        });
        self.end_undo_action();
        result
    }

    /// Selects the main selection's lines whole, line end included; again, one more line
    /// (spec §3).
    pub fn expand_line_selection(&self) -> Result<()> {
        let selection = self.selection()?;
        let start = self.line_start(self.line_from_position(selection.start)?)?;
        let end = self.line_start(self.line_from_position(selection.end)? + 1)?;
        self.send(SCI_SETSEL, start, end as isize).map(drop)
    }
```

- [ ] **Step 4: Compile and run**

Run: `cargo clippy --all-targets --all-features -- -D warnings` then `cargo test --lib -- editing_shortcuts --test-threads=1`
Expected: clean; all pass.

---

## Part 4: Comments on the editor

**Files:**
- Modify: `src/editor/scintilla/line_ops.rs`
- Modify: `src/window/main_window/tests/editing_shortcuts.rs`

**Interfaces:**
- Consumes: `comment::toggle_line`, `comment::toggle_block`, `CommentSyntax` (Part 1).
- Produces: `Editor::toggle_line_comment(&self, CommentSyntax) -> Result<()>`, `Editor::toggle_block_comment(&self, CommentSyntax) -> Result<()>`.

- [ ] **Step 1: Write the failing tests**

```rust
const RUST: crate::editor::comment::CommentSyntax = crate::document::Language::Rust.comment_syntax();

#[test]
fn toggle_line_comment_comments_every_selections_lines_in_one_step() {
    let f = fixture("fn a() {}\n    x\ny\nz");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_selection_for_test(18); // "z"
    f.editor.toggle_line_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "// fn a() {}\n    x\ny\n// z");
    assert_one_undo_restores(&f.editor, "fn a() {}\n    x\ny\nz");
}

#[test]
fn toggle_line_comment_round_trips_and_the_caret_follows_its_text() {
    let f = fixture("    foo\nbar");
    f.editor.set_selection(7..7).unwrap(); // end of "    foo"
    f.editor.toggle_line_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "    // foo\nbar");
    assert_eq!(f.editor.selection().unwrap(), 10..10);
    f.editor.toggle_line_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "    foo\nbar");
}

#[test]
fn toggle_block_comment_wraps_and_unwraps_the_main_selection() {
    let f = fixture("let a = b + c;");
    f.editor.set_selection(8..13).unwrap();
    f.editor.toggle_block_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "let a = /* b + c */;");
    assert_eq!(f.editor.selection().unwrap(), 8..19);
    f.editor.toggle_block_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "let a = b + c;");
    let empty = fixture("x");
    empty.editor.set_selection(1..1).unwrap();
    empty.editor.toggle_block_comment(RUST).unwrap();
    assert_eq!(empty.editor.text().unwrap(), "x/*  */");
    assert_eq!(empty.editor.selection().unwrap(), 4..4);
}

#[test]
fn comment_toggles_do_nothing_without_markers() {
    let none = crate::document::Language::PlainText.comment_syntax();
    let f = fixture("a");
    f.editor.toggle_line_comment(none).unwrap();
    f.editor.toggle_block_comment(none).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a");
    assert!(!f.editor.can_undo().unwrap());
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test --lib -- editing_shortcuts --test-threads=1` → compile errors.

- [ ] **Step 3: Implement** (add `use crate::editor::comment::{self, CommentSyntax};` to `line_ops.rs`)

```rust
    /// Toggles line comments on every touched line, one run at a time; the selections follow
    /// their text (spec §4.2).
    pub fn toggle_line_comment(&self, syntax: CommentSyntax) -> Result<()> {
        let mut edits = Vec::new();
        for run in self.touched_runs()? {
            let texts = run
                .clone()
                .map(|line| self.line_text(line))
                .collect::<Result<Vec<_>>>()?;
            let lines: Vec<&str> = texts.iter().map(String::as_str).collect();
            for edit in comment::toggle_line(&lines, syntax) {
                let at = self.line_start(run.start() + edit.line)? + edit.column;
                edits.push((at..at + edit.remove, edit.insert));
            }
        }
        self.replace_ranges_with(&edits).map(drop)
    }

    /// Wraps or unwraps the main selection in the block pair; the result is selected, or the
    /// caret goes between the markers of an empty pair (spec §4.3).
    pub fn toggle_block_comment(&self, syntax: CommentSyntax) -> Result<()> {
        let selection = self.selection()?;
        let selected = String::from_utf8_lossy(self.range_bytes(selection.clone())?).into_owned();
        let Some(toggle) = comment::toggle_block(&selected, syntax) else {
            return Ok(());
        };
        self.begin_undo_action();
        let result = self.replace_target(selection.clone(), &toggle.replacement);
        self.end_undo_action();
        result?;
        let start = selection.start;
        match toggle.caret {
            Some(offset) => self.set_selection(start + offset..start + offset),
            None => self.set_selection(start..start + toggle.replacement.len()),
        }
    }
```

`replace_ranges_with` already groups its edits into one undo action and applies them last first; edits come in ascending order because runs are sorted and each run's edits are ascending.

- [ ] **Step 4: Compile and run** — clippy, then `cargo test --lib -- editing_shortcuts --test-threads=1`. Expected: all pass.

---

## Part 5: Multiple carets

**Files:**
- Modify: `src/editor/scintilla/line_ops.rs`
- Modify: `src/window/main_window/tests/editing_shortcuts.rs`

**Interfaces:**
- Produces: `Editor::add_next_occurrence(&self)`, `select_all_occurrences(&self)`, `add_cursor(&self, above: bool)`, all `-> Result<()>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn add_next_occurrence_selects_the_word_then_adds_matches() {
    let f = fixture("foo bar foo");
    f.editor.set_selection(1..1).unwrap();
    f.editor.add_next_occurrence().unwrap();
    assert_eq!(f.editor.selections().unwrap(), vec![0..3]);
    f.editor.add_next_occurrence().unwrap();
    assert_eq!(f.editor.selections().unwrap(), vec![0..3, 8..11]);
}

#[test]
fn add_next_occurrence_from_a_caret_matches_whole_words_only() {
    let f = fixture("foo food foo");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_next_occurrence().unwrap(); // the word: 0..3
    f.editor.add_next_occurrence().unwrap(); // skips "food"
    assert_eq!(f.editor.selections().unwrap(), vec![0..3, 9..12]);
}

#[test]
fn add_next_occurrence_from_a_selection_matches_substrings_but_not_case() {
    let f = fixture("foo food Foo");
    f.editor.set_selection(0..3).unwrap();
    f.editor.add_next_occurrence().unwrap();
    f.editor.add_next_occurrence().unwrap();
    assert_eq!(f.editor.selections().unwrap(), vec![0..3, 4..7]);
}

#[test]
fn select_all_occurrences_selects_every_match() {
    let f = fixture("ab x ab y ab");
    f.editor.set_selection(0..2).unwrap();
    f.editor.select_all_occurrences().unwrap();
    let mut found = f.editor.selections().unwrap();
    found.sort_by_key(|range| range.start);
    assert_eq!(found, vec![0..2, 5..7, 10..12]);
}

#[test]
fn add_cursor_above_and_below_keep_the_column_and_clamp() {
    let f = fixture("abcd\nab\nabcd");
    f.editor.set_selection(7..7).unwrap(); // line 1, column 2 (end of "ab")
    f.editor.add_cursor(true).unwrap();
    f.editor.add_cursor(false).unwrap();
    let mut carets = f.editor.carets().unwrap();
    carets.sort_unstable();
    assert_eq!(carets, vec![2, 7, 10]);
    f.editor.add_cursor(true).unwrap(); // topmost is line 0: nothing above
    assert_eq!(f.editor.carets().unwrap().len(), 3);
}
```

`"abcd
ab
abcd"`: line 1 starts at 5, so its column 2 is position 7; above is position 2, below is line 2's start 8 + 2 = 10.

- [ ] **Step 2: Run to see them fail** — compile errors.

- [ ] **Step 3: Implement** (extend the constant import with `SCFIND_MATCHCASE, SCFIND_WHOLEWORD, SCI_MULTIPLESELECTADDEACH, SCI_MULTIPLESELECTADDNEXT, SCI_SETSEARCHFLAGS, SCI_TARGETWHOLEDOCUMENT`)

In `scintilla.rs`, add a field to `EditorEndpoint` after `line_numbers`, and `occurrence_whole_word: Cell::new(false),` to `EditorEndpoint::new`:

```rust
    /// Whether Add next occurrence matches whole words: set when it starts from an empty caret
    /// (editing shortcuts spec §3).
    occurrence_whole_word: Cell<bool>,
```

Then in `line_ops.rs`:

```rust
    /// Ctrl+D: the word at an empty caret, then the next match as a new selection (spec §3).
    pub fn add_next_occurrence(&self) -> Result<()> {
        self.add_occurrences(SCI_MULTIPLESELECTADDNEXT)
    }

    /// Ctrl+Shift+L: a selection on every match (spec §3).
    pub fn select_all_occurrences(&self) -> Result<()> {
        self.add_occurrences(SCI_MULTIPLESELECTADDEACH)
    }

    /// Case-sensitive; whole-word only when the run started from an empty caret, as VS Code.
    fn add_occurrences(&self, message: u32) -> Result<()> {
        if self.send(SCI_GETSELECTIONS, 0, 0)? <= 1 {
            let empty = self.selection()?.is_empty();
            self.endpoint.occurrence_whole_word.set(empty);
        }
        let whole_word = if self.endpoint.occurrence_whole_word.get() {
            SCFIND_WHOLEWORD
        } else {
            0
        };
        self.send(SCI_SETSEARCHFLAGS, (SCFIND_MATCHCASE | whole_word) as usize, 0)?;
        self.send(SCI_TARGETWHOLEDOCUMENT, 0, 0)?;
        self.send(message, 0, 0).map(drop)
    }

    /// A caret on the line above the topmost caret (or below the bottommost), in the same
    /// visual column, clamped to that line's end (spec §3).
    pub fn add_cursor(&self, above: bool) -> Result<()> {
        let carets = self
            .carets()?
            .into_iter()
            .map(|caret| Ok((self.line_from_position(caret)?, caret)))
            .collect::<Result<Vec<_>>>()?;
        let edge = if above {
            carets.iter().min()
        } else {
            carets.iter().max()
        };
        let Some(&(line, caret)) = edge else {
            return Ok(());
        };
        let target = if above {
            match line.checked_sub(1) {
                Some(target) => target,
                None => return Ok(()),
            }
        } else if line + 1 < self.line_count()? {
            line + 1
        } else {
            return Ok(());
        };
        let column = self.send(SCI_GETCOLUMN, caret, 0)?;
        let position = self.send(SCI_FINDCOLUMN, target, column)?;
        self.send(SCI_ADDSELECTION, position.max(0) as usize, position)
            .map(drop)
    }
```

The find bar passes its own flags to `search_in_target` each time, so changing the search flags and target here does not leak into Find.

- [ ] **Step 4: Compile and run** — clippy, then `cargo test --lib -- editing_shortcuts --test-threads=1`. Expected: all pass.

---

## Part 6: Commands, keys, palette, menu, dispatch

**Files:**
- Modify: `src/window/commands.rs`
- Modify: `src/window/keymap.rs`
- Modify: `src/window/command_palette.rs`, `src/window/command_palette/tests.rs`
- Modify: `src/window/menus.rs`
- Modify: `src/window/main_window/command_dispatch.rs`
- Modify: `src/window/main_window/tests/editing_shortcuts.rs`

**Interfaces:**
- Consumes: the Part 3–5 `Editor` methods; `active_language(hwnd)` (`language_tools`, already glob-imported in `main_window`).
- Produces: sixteen `CommandId` variants (245–260), `CommandId::is_editing(self) -> bool`.

- [ ] **Step 1: Write the failing tests** (append to `editing_shortcuts.rs`)

```rust
#[test]
fn editing_commands_have_their_spec_keys() {
    use crate::window::keymap::KeyStroke;
    let keymap = crate::window::keymap::Keymap::defaults();
    for (text, command) in [
        ("Alt+Up", CommandId::MoveLinesUp),
        ("Alt+Down", CommandId::MoveLinesDown),
        ("Shift+Alt+Up", CommandId::CopyLinesUp),
        ("Shift+Alt+Down", CommandId::CopyLinesDown),
        ("Ctrl+Shift+K", CommandId::DeleteLines),
        ("Ctrl+Enter", CommandId::InsertLineBelow),
        ("Ctrl+Shift+Enter", CommandId::InsertLineAbove),
        ("Ctrl+]", CommandId::IndentLines),
        ("Ctrl+[", CommandId::OutdentLines),
        ("Ctrl+L", CommandId::ExpandLineSelection),
        ("Ctrl+/", CommandId::ToggleLineComment),
        ("Shift+Alt+A", CommandId::ToggleBlockComment),
        ("Ctrl+D", CommandId::AddNextOccurrence),
        ("Ctrl+Shift+L", CommandId::SelectAllOccurrences),
        ("Ctrl+Alt+Up", CommandId::AddCursorAbove),
        ("Ctrl+Alt+Down", CommandId::AddCursorBelow),
    ] {
        assert_eq!(keymap.command_for(KeyStroke::parse(text).unwrap()), Some(command), "{text}");
        assert!(command.is_editing() && command.needs_text(), "{command:?}");
    }
}

#[test]
fn toggle_line_comment_uses_the_active_tabs_language() {
    let f = fixture("a");
    app_mut(f.window.hwnd).tabs.active_mut().unwrap().language = Language::Python;
    execute_command(f.window.hwnd, CommandId::ToggleLineComment);
    assert_eq!(f.editor.text().unwrap(), "# a");
}

#[test]
fn line_commands_run_on_the_active_editor() {
    let f = fixture("a\nb");
    f.editor.set_selection(0..0).unwrap();
    execute_command(f.window.hwnd, CommandId::MoveLinesDown);
    assert_eq!(f.editor.text().unwrap(), "b\na");
    execute_command(f.window.hwnd, CommandId::DeleteLines);
    assert_eq!(f.editor.text().unwrap(), "b");
}
```

- [ ] **Step 2: Run to see them fail** — compile errors.

- [ ] **Step 3: `commands.rs`**

1. Append to `enum CommandId` after `UnfoldAll = 244,`:

```rust
    MoveLinesUp = 245,
    MoveLinesDown = 246,
    CopyLinesUp = 247,
    CopyLinesDown = 248,
    DeleteLines = 249,
    InsertLineBelow = 250,
    InsertLineAbove = 251,
    IndentLines = 252,
    OutdentLines = 253,
    ExpandLineSelection = 254,
    ToggleLineComment = 255,
    ToggleBlockComment = 256,
    AddNextOccurrence = 257,
    SelectAllOccurrences = 258,
    AddCursorAbove = 259,
    AddCursorBelow = 260,
```

2. Add, above `TEXT_COMMANDS`:

```rust
/// The editing-shortcut commands (editing shortcuts spec §3): text commands whose keys work only
/// while an editor has the focus.
pub const EDITING_COMMANDS: [CommandId; 16] = [
    CommandId::MoveLinesUp,
    CommandId::MoveLinesDown,
    CommandId::CopyLinesUp,
    CommandId::CopyLinesDown,
    CommandId::DeleteLines,
    CommandId::InsertLineBelow,
    CommandId::InsertLineAbove,
    CommandId::IndentLines,
    CommandId::OutdentLines,
    CommandId::ExpandLineSelection,
    CommandId::ToggleLineComment,
    CommandId::ToggleBlockComment,
    CommandId::AddNextOccurrence,
    CommandId::SelectAllOccurrences,
    CommandId::AddCursorAbove,
    CommandId::AddCursorBelow,
];
```

3. `TEXT_COMMANDS: [CommandId; 40]` → `[CommandId; 56]`, appending the same sixteen.
4. In `TryFrom<u16>`, `COMMANDS: [CommandId; 134]` → `[CommandId; 150]`, appending the same sixteen.
5. In `impl CommandId`, after `needs_text`:

```rust
    /// Editing-shortcut commands, whose keys stay with other controls when no editor has the
    /// focus (spec §6).
    pub fn is_editing(self) -> bool {
        EDITING_COMMANDS.contains(&self)
    }
```

- [ ] **Step 4: `keymap.rs`**

1. Append to `COMMAND_IDS` before `(CommandId::About, "help.about"),`:

```rust
    (CommandId::MoveLinesUp, "edit.moveLinesUp"),
    (CommandId::MoveLinesDown, "edit.moveLinesDown"),
    (CommandId::CopyLinesUp, "edit.copyLinesUp"),
    (CommandId::CopyLinesDown, "edit.copyLinesDown"),
    (CommandId::DeleteLines, "edit.deleteLines"),
    (CommandId::InsertLineBelow, "edit.insertLineBelow"),
    (CommandId::InsertLineAbove, "edit.insertLineAbove"),
    (CommandId::IndentLines, "edit.indentLines"),
    (CommandId::OutdentLines, "edit.outdentLines"),
    (CommandId::ExpandLineSelection, "edit.expandLineSelection"),
    (CommandId::ToggleLineComment, "edit.toggleLineComment"),
    (CommandId::ToggleBlockComment, "edit.toggleBlockComment"),
    (CommandId::AddNextOccurrence, "edit.addNextOccurrence"),
    (CommandId::SelectAllOccurrences, "edit.selectAllOccurrences"),
    (CommandId::AddCursorAbove, "edit.addCursorAbove"),
    (CommandId::AddCursorBelow, "edit.addCursorBelow"),
```

2. `DEFAULT_BINDINGS: [(KeyStroke, CommandId); 66]` → `82`, appending after the `SplitDown` row:

```rust
    // VS Code's editing keys (editing shortcuts spec §3).
    (key(A, VK_UP), CommandId::MoveLinesUp),
    (key(A, VK_DOWN), CommandId::MoveLinesDown),
    (key(S | A, VK_UP), CommandId::CopyLinesUp),
    (key(S | A, VK_DOWN), CommandId::CopyLinesDown),
    (key(C | S, ch(b'K')), CommandId::DeleteLines),
    (key(C, VK_RETURN), CommandId::InsertLineBelow),
    (key(C | S, VK_RETURN), CommandId::InsertLineAbove),
    (key(C, VK_OEM_6), CommandId::IndentLines),
    (key(C, VK_OEM_4), CommandId::OutdentLines),
    (key(C, ch(b'L')), CommandId::ExpandLineSelection),
    (key(C, VK_OEM_2), CommandId::ToggleLineComment),
    (key(S | A, ch(b'A')), CommandId::ToggleBlockComment),
    (key(C, ch(b'D')), CommandId::AddNextOccurrence),
    (key(C | S, ch(b'L')), CommandId::SelectAllOccurrences),
    (key(C | A, VK_UP), CommandId::AddCursorAbove),
    (key(C | A, VK_DOWN), CommandId::AddCursorBelow),
```

3. In `src/window/menus.rs` tests, change both `66` assertions (`specs.len()`, `table.entries().len()`) to `82`.

- [ ] **Step 5: Palette**

In `command_palette.rs`, `ENTRIES: [PaletteEntry; 112]` → `128`, inserting after `entry("Edit: Paste", CommandId::Paste),`:

```rust
    entry("Edit: Move line up", CommandId::MoveLinesUp),
    entry("Edit: Move line down", CommandId::MoveLinesDown),
    entry("Edit: Copy line up", CommandId::CopyLinesUp),
    entry("Edit: Copy line down", CommandId::CopyLinesDown),
    entry("Edit: Delete line", CommandId::DeleteLines),
    entry("Edit: Insert line below", CommandId::InsertLineBelow),
    entry("Edit: Insert line above", CommandId::InsertLineAbove),
    entry("Edit: Indent line", CommandId::IndentLines),
    entry("Edit: Outdent line", CommandId::OutdentLines),
    entry("Edit: Select line", CommandId::ExpandLineSelection),
    entry("Edit: Toggle line comment", CommandId::ToggleLineComment),
    entry("Edit: Toggle block comment", CommandId::ToggleBlockComment),
    entry("Edit: Add next occurrence", CommandId::AddNextOccurrence),
    entry("Edit: Select all occurrences", CommandId::SelectAllOccurrences),
    entry("Edit: Add cursor above", CommandId::AddCursorAbove),
    entry("Edit: Add cursor below", CommandId::AddCursorBelow),
```

In `command_palette/tests.rs:93`, `112` → `128`.

- [ ] **Step 6: Edit menu**

In `menus.rs`, the Edit popup becomes:

```rust
            let edit = create_popup(
                &[
                    MenuEntry::command("&Undo", CommandId::Undo),
                    MenuEntry::command("&Redo", CommandId::Redo),
                    MenuEntry::Separator,
                    MenuEntry::command("Cu&t", CommandId::Cut),
                    MenuEntry::command("&Copy", CommandId::Copy),
                    MenuEntry::command("&Paste", CommandId::Paste),
                    MenuEntry::Separator,
                    MenuEntry::Submenu(
                        "&Line",
                        vec![
                            MenuEntry::command("Move line &up", CommandId::MoveLinesUp),
                            MenuEntry::command("Move line &down", CommandId::MoveLinesDown),
                            MenuEntry::command("&Copy line up", CommandId::CopyLinesUp),
                            MenuEntry::command("Copy line do&wn", CommandId::CopyLinesDown),
                            MenuEntry::command("D&elete line", CommandId::DeleteLines),
                            MenuEntry::command("Insert line &below", CommandId::InsertLineBelow),
                            MenuEntry::command("Insert line &above", CommandId::InsertLineAbove),
                            MenuEntry::command("&Indent line", CommandId::IndentLines),
                            MenuEntry::command("&Outdent line", CommandId::OutdentLines),
                            MenuEntry::command("&Select line", CommandId::ExpandLineSelection),
                        ],
                    ),
                    MenuEntry::Submenu(
                        "&Selection",
                        vec![
                            MenuEntry::command("Add &next occurrence", CommandId::AddNextOccurrence),
                            MenuEntry::command(
                                "Select &all occurrences",
                                CommandId::SelectAllOccurrences,
                            ),
                            MenuEntry::command("Add cursor &above", CommandId::AddCursorAbove),
                            MenuEntry::command("Add cursor &below", CommandId::AddCursorBelow),
                        ],
                    ),
                    MenuEntry::command("Toggle line co&mment", CommandId::ToggleLineComment),
                    MenuEntry::command("Toggle bloc&k comment", CommandId::ToggleBlockComment),
                    MenuEntry::Separator,
                    MenuEntry::command("&Format JSON", CommandId::FormatJson),
                ],
                keymap,
            )?;
```

- [ ] **Step 7: Dispatch**

In `command_dispatch.rs`, after the `CommandId::Paste` arm:

```rust
        CommandId::MoveLinesUp => with_editor(hwnd, |editor| {
            let _ = editor.move_lines(true);
        }),
        CommandId::MoveLinesDown => with_editor(hwnd, |editor| {
            let _ = editor.move_lines(false);
        }),
        CommandId::CopyLinesUp => with_editor(hwnd, |editor| {
            let _ = editor.copy_lines(false);
        }),
        CommandId::CopyLinesDown => with_editor(hwnd, |editor| {
            let _ = editor.copy_lines(true);
        }),
        CommandId::DeleteLines => with_editor(hwnd, |editor| {
            let _ = editor.delete_lines();
        }),
        CommandId::InsertLineBelow => with_editor(hwnd, |editor| {
            let _ = editor.insert_line(true);
        }),
        CommandId::InsertLineAbove => with_editor(hwnd, |editor| {
            let _ = editor.insert_line(false);
        }),
        CommandId::IndentLines => with_editor(hwnd, |editor| {
            let _ = editor.indent_lines(false);
        }),
        CommandId::OutdentLines => with_editor(hwnd, |editor| {
            let _ = editor.indent_lines(true);
        }),
        CommandId::ExpandLineSelection => with_editor(hwnd, |editor| {
            let _ = editor.expand_line_selection();
        }),
        CommandId::ToggleLineComment => {
            let syntax = active_language(hwnd).comment_syntax();
            with_editor(hwnd, |editor| {
                let _ = editor.toggle_line_comment(syntax);
            });
        }
        CommandId::ToggleBlockComment => {
            let syntax = active_language(hwnd).comment_syntax();
            with_editor(hwnd, |editor| {
                let _ = editor.toggle_block_comment(syntax);
            });
        }
        CommandId::AddNextOccurrence => with_editor(hwnd, |editor| {
            let _ = editor.add_next_occurrence();
        }),
        CommandId::SelectAllOccurrences => with_editor(hwnd, |editor| {
            let _ = editor.select_all_occurrences();
        }),
        CommandId::AddCursorAbove => with_editor(hwnd, |editor| {
            let _ = editor.add_cursor(true);
        }),
        CommandId::AddCursorBelow => with_editor(hwnd, |editor| {
            let _ = editor.add_cursor(false);
        }),
```

- [ ] **Step 8: Compile and run**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Then: `cargo test --lib -- editing_shortcuts keymap command_palette menus commands shortcuts --test-threads=1`
Expected: all pass. If a Shortcuts-page or palette test asserts a row list that now includes the new commands (`settings_shortcuts_page`, `shortcuts_model`), update its expectation to include them; the new rows are correct behaviour (spec §2 "Real commands").

---

## Part 7: Editing keys only in the editor

**Files:**
- Modify: `src/window/main_window/menu_keys.rs`
- Modify: `src/window/main_window/tests/editing_shortcuts.rs`

- [ ] **Step 1: Write the failing tests**

```rust
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_OEM_2};
use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_SYSKEYDOWN};

fn translate(f: &Fixture, message: MSG) -> bool {
    let identity = unsafe { super::super::window_identity(f.window.hwnd).unwrap() };
    unsafe { super::super::translate_accelerator(f.window.hwnd, &identity, &message) }
}

#[test]
fn alt_down_and_ctrl_slash_in_the_editor_run_their_commands() {
    let f = fixture("one\ntwo");
    f.editor.set_selection(0..0).unwrap();
    app_mut(f.window.hwnd).tabs.active_mut().unwrap().language = Language::Rust;
    let moved = with_keys_down(&[VK_MENU], || {
        translate(&f, MSG {
            hwnd: f.editor.hwnd(),
            message: WM_SYSKEYDOWN,
            wParam: usize::from(VK_DOWN),
            lParam: 1 << 29,
            ..Default::default()
        })
    });
    assert!(moved, "Alt+Down was not translated");
    assert_eq!(f.editor.text().unwrap(), "two\none");
    let commented = with_keys_down(&[VK_CONTROL], || {
        translate(&f, MSG {
            hwnd: f.editor.hwnd(),
            message: WM_KEYDOWN,
            wParam: usize::from(VK_OEM_2),
            ..Default::default()
        })
    });
    assert!(commented, "Ctrl+/ was not translated");
    assert_eq!(f.editor.text().unwrap(), "two\n// one");
}

#[test]
fn editing_keys_stay_with_controls_outside_the_editor() {
    // Break caught: Ctrl+D typed in the find field or the tree adding a selection in the
    // document (VS Code scopes these keys to editorTextFocus).
    let f = fixture("one");
    let elsewhere = with_keys_down(&[VK_CONTROL], || {
        translate(&f, MSG {
            hwnd: f.window.hwnd,
            message: WM_KEYDOWN,
            wParam: usize::from(b'D'),
            ..Default::default()
        })
    });
    assert!(!elsewhere, "Ctrl+D outside the editor was translated");
    assert_eq!(f.editor.selections().unwrap(), vec![0..0]);
}
```

- [ ] **Step 2: Run to see the second fail** — `cargo test --lib -- editing_keys_stay alt_down_and_ctrl_slash --test-threads=1`. Expected: `editing_keys_stay_with_controls_outside_the_editor` FAILS (translated); the first passes already.

- [ ] **Step 3: Implement**

In `translate_accelerator` (`menu_keys.rs`), after the `start_tab_for_typing` check and before the accelerator lookup:

```rust
    if editing_key_off_editor(hwnd, message) {
        return false;
    }
```

and add next to `palette_keeps_key`:

```rust
/// A key bound to an editing-shortcut command while the focus is not in an editor: it stays
/// with the focused control, as VS Code's `editorTextFocus` (editing shortcuts spec §6).
fn editing_key_off_editor(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    if !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return false;
    }
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(stroke) = crate::window::keymap::KeyStroke::from_key(
        message.wParam as u16,
        down(VK_CONTROL),
        down(VK_SHIFT),
        down(VK_MENU),
    ) else {
        return false;
    };
    let editing = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.keymap.command_for(stroke))
        .is_some_and(CommandId::is_editing);
    editing && !is_group_editor(hwnd, message.hwnd)
}

/// Whether `window` is one of the editor groups' editors.
fn is_group_editor(hwnd: HWND, window: HWND) -> bool {
    group_of_child(hwnd, window)
        .and_then(|id| group_editor(hwnd, id))
        .is_some_and(|editor| editor.hwnd() == window)
}
```

If `start_tab_for_typing` has the same editor check inline, make it call `is_group_editor` too.

- [ ] **Step 4: Compile and run** — clippy, then `cargo test --lib -- editing_shortcuts command_palette --test-threads=1`. Expected: all pass (the palette's Ctrl+W/Ctrl+P accelerator tests included).

---

## Part 8: Alt+Click adds a caret

**Files:**
- Modify: `src/editor/scintilla.rs` (subclass proc, `EditorEndpoint` methods, imports)
- Modify: `src/window/main_window/menu_keys.rs` (`menu_activation_message`)
- Modify: `src/window/main_window/tests/editing_shortcuts.rs`

- [ ] **Step 1: Write the failing tests**

```rust
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SYSKEYUP};

/// Client coordinates of `position` in the editor, packed as a mouse `LPARAM`.
fn point_of(editor: &crate::editor::Editor, position: usize) -> isize {
    use crate::editor::scintilla_constants::{SCI_POINTXFROMPOSITION, SCI_POINTYFROMPOSITION};
    let x = unsafe { SendMessageW(editor.hwnd(), SCI_POINTXFROMPOSITION, 0, position as isize) };
    let y = unsafe { SendMessageW(editor.hwnd(), SCI_POINTYFROMPOSITION, 0, position as isize) } + 2;
    (x & 0xFFFF) | ((y & 0xFFFF) << 16)
}

#[test]
fn alt_click_adds_a_caret_and_keeps_the_others() {
    let f = fixture("one\ntwo\nthree");
    f.editor.set_selection(0..0).unwrap();
    let at = point_of(&f.editor, 6); // "tw|o"
    with_keys_down(&[VK_MENU], || unsafe {
        SendMessageW(f.editor.hwnd(), WM_LBUTTONDOWN, 0x0001, at);
        SendMessageW(f.editor.hwnd(), WM_LBUTTONUP, 0, at);
    });
    let mut carets = f.editor.carets().unwrap();
    carets.sort_unstable();
    assert_eq!(carets, vec![0, 6]);
}

#[test]
fn a_mouse_press_cancels_the_pending_alt_tap() {
    let f = fixture("one");
    let alt = |message| MSG {
        hwnd: f.editor.hwnd(),
        message,
        wParam: usize::from(VK_MENU),
        ..Default::default()
    };
    translate(&f, alt(WM_SYSKEYDOWN));
    translate(&f, MSG {
        hwnd: f.editor.hwnd(),
        message: WM_LBUTTONDOWN,
        ..Default::default()
    });
    assert!(!translate(&f, alt(WM_SYSKEYUP)), "releasing Alt after a click opened the menu");
    translate(&f, alt(WM_SYSKEYDOWN));
    assert!(translate(&f, alt(WM_SYSKEYUP)), "a bare Alt tap still opens the menu");
    discard_posted(f.window.hwnd, windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCOMMAND);
}
```

Remove the `#[allow(unused_imports)]` added in Part 2 now that `VK_MENU` is used.

- [ ] **Step 2: Run to see them fail** — `cargo test --lib -- alt_click a_mouse_press --test-threads=1`. Expected: both FAIL (one caret; Alt release translated).

- [ ] **Step 3: Implement the menu fix**

In `menu_activation_message`, before the final `if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)`:

```rust
    // Alt+Click (a caret in the editor) is Alt with other input, not a bare tap.
    if matches!(
        message.message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
    ) {
        app.set_menu_alt_pending(false);
    }
```

- [ ] **Step 4: Implement Alt+Click in the editor subclass**

In `scintilla.rs`, define next to `LineNumberMargin`:

```rust
/// An Alt+Click in progress: where the button went down, and the selections (caret, anchor) and
/// main selection from before Scintilla's own handling.
#[derive(Debug)]
struct AltClick {
    x: i32,
    y: i32,
    selections: Vec<(isize, isize)>,
    main: usize,
}
```

add a field to `EditorEndpoint` after `occurrence_whole_word`, with `alt_click: std::cell::RefCell::new(None),` in `EditorEndpoint::new`:

```rust
    /// The selections and point an Alt+Click started from (spec §5).
    alt_click: std::cell::RefCell<Option<AltClick>>,
```

and add to `impl EditorEndpoint`:

```rust
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
        for (n, (caret, anchor)) in click.selections.iter().enumerate() {
            let message = if n == 0 {
                SCI_SETSELECTION
            } else {
                SCI_ADDSELECTION
            };
            self.send_direct_checked(message, *caret as usize, *anchor)?;
        }
        self.send_direct_checked(SCI_SETMAINSELECTION, click.main, 0)?;
        let position =
            self.send_direct_checked(SCI_POSITIONFROMPOINT, x as usize, y as isize)?;
        self.send_direct_checked(SCI_ADDSELECTION, position.max(0) as usize, position)?;
        Ok(())
    }
```

and a free function next to `editor_endpoint_subclass_proc`:

```rust
/// A mouse message's client point: signed 16-bit x and y.
#[cfg(windows)]
fn mouse_point(lparam: LPARAM) -> (i32, i32) {
    let x = i32::from((lparam & 0xFFFF) as u16 as i16);
    let y = i32::from(((lparam >> 16) & 0xFFFF) as u16 as i16);
    (x, y)
}
```

In `editor_endpoint_subclass_proc`, before the `WM_DPICHANGED_AFTERPARENT` branch:

```rust
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
```

Imports in `scintilla.rs`: add `WM_LBUTTONDOWN, WM_LBUTTONUP` to the `#[cfg(windows)] use windows_sys::Win32::UI::WindowsAndMessaging::{...}` line; `VK_MENU, VK_SHIFT` beside `VK_CONTROL` in the `KeyboardAndMouse` import; and `SCI_ADDSELECTION, SCI_GETMAINSELECTION, SCI_GETSELECTIONNANCHOR, SCI_GETSELECTIONNCARET, SCI_GETSELECTIONS, SCI_POSITIONFROMPOINT, SCI_SETMAINSELECTION, SCI_SETSELECTION` to a `#[cfg(windows)] use crate::editor::scintilla_constants::{...}` block. `GetSystemMetrics`/`SM_CXDRAG` are in `Win32_UI_WindowsAndMessaging`, already a dependency feature.

- [ ] **Step 5: Compile and run** — clippy, then `cargo test --lib -- editing_shortcuts --test-threads=1`. Expected: all pass.

---

## Part 9: Integration test, docs, full verification

**Files:**
- Modify: `tests/windows/editing.rs`
- Modify: `README.md` only if it lists keyboard shortcuts (`grep -n "Ctrl+" README.md`); if it does, add the sixteen keys to that list in its existing format.

- [ ] **Step 1: Add the integration test** (after `cut_and_paste_route_through_the_shared_command_model`)

```rust
#[test]
fn line_commands_route_through_the_shared_command_model() {
    // Break caught: the editing-shortcut commands not reaching the editor from WM_COMMAND (menu
    // and accelerator path) in the real binary's window.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let editor = main.editor;
    type_text(editor, "two");
    unsafe {
        SendMessageW(main.hwnd, WM_COMMAND, CommandId::InsertLineAbove as usize, 0);
    }
    type_text(editor, "one");
    wait_text(editor, "one\r\ntwo");

    unsafe {
        SendMessageW(main.hwnd, WM_COMMAND, CommandId::MoveLinesDown as usize, 0);
    }
    wait_text(editor, "two\r\none");

    unsafe {
        SendMessageW(main.hwnd, WM_COMMAND, CommandId::DeleteLines as usize, 0);
    }
    wait_text(editor, "two");
}
```

Run: `cargo test --test editing -- --test-threads=1`
Expected: all pass.

- [ ] **Step 2: Full verification**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- --test-threads=1
for t in acceptance tabs library highlighting json_commands editing save_file open_file recovery session single_instance startup_smoke titlebar markdown_preview image_preview editor_control; do
  cargo test --test "$t" -- --test-threads=1 || echo "FAILED: $t"
done
pwsh -NoProfile -File tools/audit-dependencies.ps1
```

Expected: everything passes; no `FAILED:` lines; the audit is clean (no new `windows-sys` features).

- [ ] **Step 3: Live check** (back up the user's settings first)

```bash
cp "$APPDATA/FastPad/fastpad.ini" "$TMP/fastpad.ini.bak" 2>/dev/null
cargo build --bin fastpad
```

Ask the user to try in `target/debug/fastpad.exe`: `Alt+↑/↓`, `Shift+Alt+↓`, `Ctrl+Shift+K`, `Ctrl+Enter`, `Ctrl+/` in a `.rs` file, `Ctrl+D` ×2, `Alt+Click` then releasing Alt (no menu), `Ctrl+Enter` in the find field (no new line in the document). Then restore: `cp "$TMP/fastpad.ini.bak" "$APPDATA/FastPad/fastpad.ini"`.

- [ ] **Step 4: Commit**

```bash
git add -A src tests tools docs README.md
git commit -m "feat: VS Code editing shortcuts (line editing, comments, multiple carets)"
```
