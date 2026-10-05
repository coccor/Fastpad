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
            Self::C
            | Self::Cpp
            | Self::CSharp
            | Self::JavaScript
            | Self::TypeScript
            | Self::Rust => C_LIKE,
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
    let open_end = if first == last {
        open_at + open_remove
    } else {
        0
    };
    let space_before = close_start > open_end && tail.as_bytes()[close_start - 1] == b' ';
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
        let hash = CommentSyntax {
            line: Some("#"),
            block: None,
        };
        let markup = HTML;
        for language in [
            L::C,
            L::Cpp,
            L::CSharp,
            L::JavaScript,
            L::TypeScript,
            L::Rust,
        ] {
            assert_eq!(language.comment_syntax(), c_like, "{language:?}");
        }
        for language in [L::Python, L::Bash, L::Yaml, L::Toml, L::Properties, L::Env] {
            assert_eq!(language.comment_syntax(), hash, "{language:?}");
        }
        for language in [L::Html, L::Xml, L::Svg, L::Markdown] {
            assert_eq!(language.comment_syntax(), markup, "{language:?}");
        }
        assert_eq!(
            L::Css.comment_syntax(),
            CommentSyntax {
                line: None,
                block: Some(("/*", "*/"))
            }
        );
        assert_eq!(
            L::Sql.comment_syntax(),
            CommentSyntax {
                line: Some("--"),
                block: Some(("/*", "*/"))
            }
        );
        assert_eq!(
            L::PowerShell.comment_syntax(),
            CommentSyntax {
                line: Some("#"),
                block: Some(("<#", "#>"))
            }
        );
        assert_eq!(
            L::Ini.comment_syntax(),
            CommentSyntax {
                line: Some(";"),
                block: None
            }
        );
        assert_eq!(
            L::Batch.comment_syntax(),
            CommentSyntax {
                line: Some("REM"),
                block: None
            }
        );
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
        assert_eq!(
            apply(&lines, &toggle_line(&lines, RUST)),
            ["    a", "  b", " c"]
        );
    }

    #[test]
    fn mixed_lines_get_commented_not_uncommented() {
        let lines = ["// a", "b"];
        assert_eq!(
            apply(&lines, &toggle_line(&lines, RUST)),
            ["// // a", "// b"]
        );
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
        assert_eq!(
            apply(&commented, &toggle_line(&commented, syntax)),
            ["echo hi"]
        );
    }

    #[test]
    fn languages_without_a_line_marker_wrap_the_lines_in_a_block() {
        let lines = ["  <p>", "  </p>"];
        let wrapped = apply(&lines, &toggle_line(&lines, HTML));
        assert_eq!(wrapped, ["  <!-- <p>", "  </p> -->"]);
        let wrapped: Vec<&str> = wrapped.iter().map(String::as_str).collect();
        assert_eq!(
            apply(&wrapped, &toggle_line(&wrapped, HTML)),
            ["  <p>", "  </p>"]
        );
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
            Some(BlockToggle {
                replacement: "/* a + b */".to_owned(),
                caret: None
            })
        );
    }

    #[test]
    fn block_unwraps_keeping_outer_whitespace() {
        assert_eq!(
            toggle_block(" /* a + b */ ", RUST),
            Some(BlockToggle {
                replacement: " a + b ".to_owned(),
                caret: None
            })
        );
        assert_eq!(
            toggle_block("/**/", RUST),
            Some(BlockToggle {
                replacement: String::new(),
                caret: None
            })
        );
    }

    #[test]
    fn block_on_an_empty_selection_inserts_a_pair_with_the_caret_inside() {
        assert_eq!(
            toggle_block("", RUST),
            Some(BlockToggle {
                replacement: "/*  */".to_owned(),
                caret: Some(3)
            })
        );
    }

    #[test]
    fn too_short_to_be_a_comment_is_wrapped() {
        assert_eq!(
            toggle_block("/*/", RUST),
            Some(BlockToggle {
                replacement: "/* /*/ */".to_owned(),
                caret: None
            })
        );
    }
}
