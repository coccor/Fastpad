//! One row per language: what FastPad calls it, which files are it, and how Lexilla lexes it.
//! Detection, `LanguageManager::apply`, the status bar, the default save extension, the View →
//! Language submenu and the command palette all read this table.

use crate::document::Language;
use crate::languages::keywords as kw;
use crate::languages::{
    LexerStyle, bash, batch, cpp, css, json, markdown, powershell, props, python, rust, sql, toml,
    xml, yaml,
};
use crate::platform::theme::Theme;
use std::path::Path;

/// Everything FastPad knows about one language. Exactly one row per `Language`.
pub(crate) struct LanguageSpec {
    pub(crate) language: Language,
    /// Status bar, View → Language and palette name.
    pub(crate) name: &'static str,
    /// Lower-case, without the dot; the first is the default save extension.
    pub(crate) extensions: &'static [&'static str],
    /// Lower-case whole file names; a trailing `*` matches any rest (`.env.*`).
    pub(crate) file_names: &'static [&'static str],
    /// Lexilla's lexer name; `None` installs the null lexer without loading Lexilla.
    pub(crate) lexer: Option<&'static str>,
    /// `SCI_SETKEYWORDS` lists, indexed by keyword-set number.
    pub(crate) keywords: &'static [&'static str],
    /// `SCI_SETPROPERTY` pairs applied after the lexer is installed.
    pub(crate) properties: &'static [(&'static str, &'static str)],
    pub(crate) styles: fn(Theme) -> &'static [LexerStyle],
}

fn no_styles(_: Theme) -> &'static [LexerStyle] {
    &[]
}

/// Applied to every lexer before its own properties. Lexers without folding ignore them; the rest
/// fold on a block's last non-blank line rather than swallowing the blank lines after it.
pub(crate) const FOLD_PROPERTIES: &[(&str, &str)] = &[("fold", "1"), ("fold.compact", "0")];

// Preprocessor tracking restyles inactive `#if` branches as styles 64-91, which no table maps, so
// they would show as uncoloured text; colour every branch like the active one instead.
const CPP_PROPERTIES: &[(&str, &str)] = &[
    ("lexer.cpp.escape.sequence", "1"),
    ("lexer.cpp.track.preprocessor", "0"),
    ("fold.comment", "1"),
    ("fold.preprocessor", "1"),
];
const JS_PROPERTIES: &[(&str, &str)] = &[
    ("lexer.cpp.escape.sequence", "1"),
    ("lexer.cpp.backquoted.strings", "2"),
    ("lexer.cpp.track.preprocessor", "0"),
    ("fold.comment", "1"),
    ("fold.preprocessor", "1"),
];
const CSS_PROPERTIES: &[(&str, &str)] = &[("fold.comment", "1")];
const POWERSHELL_PROPERTIES: &[(&str, &str)] = &[("fold.comment", "1")];
const PYTHON_PROPERTIES: &[(&str, &str)] =
    &[("fold.comment.python", "1"), ("fold.quotes.python", "1")];
const SQL_PROPERTIES: &[(&str, &str)] = &[("fold.comment", "1")];
// Tag folding is off by default in the hypertext and XML lexers.
const MARKUP_PROPERTIES: &[(&str, &str)] = &[("fold.html", "1"), ("fold.hypertext.comment", "1")];

/// Plain Text first, then alphabetical by name: this is the menu and palette order.
pub(crate) static LANGUAGES: [LanguageSpec; 23] = [
    LanguageSpec {
        language: Language::PlainText,
        name: "Plain Text",
        extensions: &[],
        file_names: &[],
        lexer: None,
        keywords: &[],
        properties: &[],
        styles: no_styles,
    },
    LanguageSpec {
        language: Language::Bash,
        name: "Bash",
        extensions: &["sh", "bash", "zsh"],
        file_names: &[".bashrc", ".bash_profile", ".zshrc", ".profile"],
        lexer: Some("bash"),
        keywords: &[kw::BASH],
        properties: &[],
        styles: bash::styles,
    },
    LanguageSpec {
        language: Language::Batch,
        name: "Batch",
        extensions: &["bat", "cmd"],
        file_names: &[],
        lexer: Some("batch"),
        keywords: &[kw::BATCH],
        properties: &[],
        styles: batch::styles,
    },
    LanguageSpec {
        language: Language::C,
        name: "C",
        extensions: &["c", "h"],
        file_names: &[],
        lexer: Some("cpp"),
        keywords: &[kw::C, kw::C_TYPES],
        properties: CPP_PROPERTIES,
        styles: cpp::styles,
    },
    LanguageSpec {
        language: Language::CSharp,
        name: "C#",
        extensions: &["cs", "csx"],
        file_names: &[],
        lexer: Some("cpp"),
        keywords: &[kw::CSHARP, kw::CSHARP_TYPES],
        properties: CPP_PROPERTIES,
        styles: cpp::styles,
    },
    LanguageSpec {
        language: Language::Cpp,
        name: "C++",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "inl"],
        file_names: &[],
        lexer: Some("cpp"),
        keywords: &[kw::CPP, kw::CPP_TYPES],
        properties: CPP_PROPERTIES,
        styles: cpp::styles,
    },
    LanguageSpec {
        language: Language::Css,
        name: "CSS",
        extensions: &["css"],
        file_names: &[],
        lexer: Some("css"),
        keywords: &[],
        properties: CSS_PROPERTIES,
        styles: css::styles,
    },
    LanguageSpec {
        language: Language::Env,
        name: "Env",
        extensions: &["env"],
        file_names: &[".env", ".env.*"],
        lexer: Some("props"),
        keywords: &[],
        properties: &[],
        styles: props::styles,
    },
    LanguageSpec {
        language: Language::Html,
        name: "HTML",
        extensions: &["html", "htm", "xhtml"],
        file_names: &[],
        lexer: Some("hypertext"),
        // Set 0 empty: Lexilla then treats every tag and attribute as known.
        keywords: &["", kw::JAVASCRIPT],
        properties: MARKUP_PROPERTIES,
        styles: xml::styles,
    },
    LanguageSpec {
        language: Language::Ini,
        name: "INI",
        extensions: &["ini", "cfg", "conf", "inf", "reg"],
        file_names: &[".editorconfig", ".gitconfig", ".npmrc"],
        lexer: Some("props"),
        keywords: &[],
        properties: &[],
        styles: props::styles,
    },
    LanguageSpec {
        language: Language::JavaScript,
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: &[],
        lexer: Some("cpp"),
        keywords: &[kw::JAVASCRIPT, kw::JS_GLOBALS],
        properties: JS_PROPERTIES,
        styles: cpp::styles,
    },
    LanguageSpec {
        language: Language::Json,
        name: "JSON",
        extensions: &[
            "json",
            "jsonc",
            "json5",
            "jsonl",
            "geojson",
            "webmanifest",
            "code-workspace",
        ],
        file_names: &[],
        lexer: Some("json"),
        keywords: &[kw::JSON, kw::JSON_LD],
        properties: &[
            ("lexer.json.allow.comments", "1"),
            ("lexer.json.escape.sequence", "1"),
        ],
        styles: json::styles,
    },
    LanguageSpec {
        language: Language::Markdown,
        name: "Markdown",
        extensions: &["md", "markdown", "mdown", "mkd"],
        file_names: &[],
        lexer: Some("markdown"),
        keywords: &[],
        properties: &[],
        styles: markdown::styles,
    },
    LanguageSpec {
        language: Language::PowerShell,
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        file_names: &[],
        lexer: Some("powershell"),
        keywords: &[
            kw::POWERSHELL,
            kw::POWERSHELL_CMDLETS,
            kw::POWERSHELL_ALIASES,
        ],
        properties: POWERSHELL_PROPERTIES,
        styles: powershell::styles,
    },
    LanguageSpec {
        language: Language::Properties,
        name: "Properties",
        extensions: &["properties"],
        file_names: &[],
        lexer: Some("props"),
        keywords: &[],
        properties: &[],
        styles: props::styles,
    },
    LanguageSpec {
        language: Language::Python,
        name: "Python",
        extensions: &["py", "pyw", "pyi"],
        file_names: &[],
        lexer: Some("python"),
        keywords: &[kw::PYTHON, kw::PYTHON_BUILTINS],
        properties: PYTHON_PROPERTIES,
        styles: python::styles,
    },
    LanguageSpec {
        language: Language::Rust,
        name: "Rust",
        extensions: &["rs"],
        file_names: &[],
        lexer: Some("rust"),
        keywords: &[kw::RUST, kw::RUST_TYPES],
        properties: &[],
        styles: rust::styles,
    },
    LanguageSpec {
        language: Language::Sql,
        name: "SQL",
        extensions: &["sql"],
        file_names: &[],
        lexer: Some("sql"),
        keywords: &[kw::SQL, kw::SQL_TYPES],
        properties: SQL_PROPERTIES,
        styles: sql::styles,
    },
    LanguageSpec {
        language: Language::Svg,
        name: "SVG",
        extensions: &["svg"],
        file_names: &[],
        lexer: Some("xml"),
        keywords: &[],
        properties: MARKUP_PROPERTIES,
        styles: xml::styles,
    },
    LanguageSpec {
        language: Language::Toml,
        name: "TOML",
        extensions: &["toml"],
        file_names: &["cargo.lock"],
        lexer: Some("toml"),
        keywords: &[kw::TOML],
        properties: &[],
        styles: toml::styles,
    },
    LanguageSpec {
        language: Language::TypeScript,
        name: "TypeScript",
        extensions: &["ts", "mts", "cts", "tsx"],
        file_names: &[],
        lexer: Some("cpp"),
        keywords: &[kw::TYPESCRIPT, kw::JS_GLOBALS],
        properties: JS_PROPERTIES,
        styles: cpp::styles,
    },
    LanguageSpec {
        language: Language::Xml,
        name: "XML",
        extensions: &[
            "xml", "xaml", "xsd", "xsl", "xslt", "csproj", "vbproj", "fsproj", "props", "targets",
            "config", "resx", "nuspec", "manifest", "plist", "rss", "atom",
        ],
        file_names: &[],
        lexer: Some("xml"),
        keywords: &[],
        properties: MARKUP_PROPERTIES,
        styles: xml::styles,
    },
    LanguageSpec {
        language: Language::Yaml,
        name: "YAML",
        extensions: &["yaml", "yml"],
        file_names: &[],
        lexer: Some("yaml"),
        keywords: &[kw::YAML],
        properties: &[],
        styles: yaml::styles,
    },
];

pub(crate) fn spec(language: Language) -> &'static LanguageSpec {
    LANGUAGES
        .iter()
        .find(|row| row.language == language)
        .expect("every Language has a registry row")
}

/// Detects a file's language from its whole file name, then its extension, case-insensitively.
/// Never touches disk or Win32; this is the pure-logic half of language handling.
pub fn detect_language(path: &Path) -> Language {
    let Some(name) = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
    else {
        return Language::PlainText;
    };
    let by_name = LANGUAGES.iter().find(|row| {
        row.file_names
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => name.starts_with(prefix),
                None => name == *pattern,
            })
    });
    if let Some(row) = by_name {
        return row.language;
    }
    let extension = Path::new(&name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    LANGUAGES
        .iter()
        .find(|row| row.extensions.contains(&extension))
        .map_or(Language::PlainText, |row| row.language)
}

/// Whether some language claims `extension` (without the dot), ignoring case.
pub fn is_language_extension(extension: &str) -> bool {
    LANGUAGES.iter().any(|row| {
        row.extensions
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

/// Whether some language claims the whole file name `name` (`.env`, `Cargo.lock`), ignoring
/// case. Allocation-free, since the notebook scan asks it of every unlisted file.
pub fn is_language_file_name(name: &str) -> bool {
    let name = name.as_bytes();
    LANGUAGES.iter().any(|row| {
        row.file_names
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => name
                    .get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix.as_bytes())),
                None => name.eq_ignore_ascii_case(pattern.as_bytes()),
            })
    })
}

pub fn display_name(language: Language) -> &'static str {
    spec(language).name
}

/// The extension a new file in `language` is saved with; plain text notes are Markdown files.
pub fn default_extension(language: Language) -> &'static str {
    spec(language).extensions.first().copied().unwrap_or("md")
}

#[cfg(test)]
mod tests {
    use super::{LANGUAGES, default_extension, detect_language, spec};
    use crate::document::Language;
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn detects_every_language_by_extension_and_file_name_case_insensitively() {
        for (path, language) in [
            ("CONFIG.JSON", Language::Json),
            ("a.jsonc", Language::Json),
            ("readme.md", Language::Markdown),
            ("notes.txt", Language::PlainText),
            (r"C:\x\Logo.SVG", Language::Svg),
            ("App.xaml", Language::Xml),
            ("build.CSPROJ", Language::Xml),
            ("index.htm", Language::Html),
            ("site.css", Language::Css),
            ("app.mjs", Language::JavaScript),
            ("view.tsx", Language::TypeScript),
            ("ci.yml", Language::Yaml),
            ("Cargo.toml", Language::Toml),
            ("Cargo.lock", Language::Toml),
            ("setup.cfg", Language::Ini),
            (".editorconfig", Language::Ini),
            ("app.properties", Language::Properties),
            (".ENV", Language::Env),
            (".env.local", Language::Env),
            ("build.ps1", Language::PowerShell),
            (".bashrc", Language::Bash),
            ("run.sh", Language::Bash),
            ("make.CMD", Language::Batch),
            ("tool.py", Language::Python),
            ("main.c", Language::C),
            ("main.hpp", Language::Cpp),
            ("Program.cs", Language::CSharp),
            ("lib.rs", Language::Rust),
            ("schema.sql", Language::Sql),
            ("no_extension", Language::PlainText),
        ] {
            assert_eq!(detect_language(Path::new(path)), language, "{path}");
        }
    }

    #[test]
    fn no_extension_or_file_name_is_claimed_twice() {
        // Break caught: a later row silently shadowing an earlier one's extension.
        let mut seen = HashSet::new();
        for row in &LANGUAGES {
            for key in row.extensions.iter().chain(row.file_names) {
                assert!(seen.insert(*key), "{key} appears twice");
                assert_eq!(*key, key.to_ascii_lowercase(), "{key} must be lower case");
            }
        }
    }

    #[test]
    fn every_language_has_exactly_one_row_and_plain_text_leads() {
        assert_eq!(LANGUAGES[0].language, Language::PlainText);
        let languages = LANGUAGES
            .iter()
            .map(|row| row.language)
            .collect::<HashSet<_>>();
        assert_eq!(languages.len(), LANGUAGES.len());
        assert_eq!(spec(Language::Rust).name, "Rust");
    }

    #[test]
    fn default_extension_comes_from_the_registry() {
        assert_eq!(default_extension(Language::Json), "json");
        assert_eq!(default_extension(Language::Yaml), "yaml");
        assert_eq!(default_extension(Language::Svg), "svg");
        assert_eq!(default_extension(Language::PlainText), "md");
    }

    #[test]
    fn every_lexed_language_styles_its_default_text_in_every_theme_without_duplicates() {
        // Break caught: a table missing style 0 leaves ordinary text in Scintilla's built-in colour
        // after a theme switch; a duplicated id means one mapping silently overrides another.
        for row in LANGUAGES.iter().filter(|row| row.lexer.is_some()) {
            for theme in crate::platform::theme::Theme::ALL {
                let table = (row.styles)(theme);
                let colors = crate::languages::syntax_colors(theme);
                let default = table
                    .iter()
                    .find(|style| style.style == 0)
                    .unwrap_or_else(|| panic!("{} has no default style", row.name));
                assert_eq!(default.foreground, colors.text, "{}", row.name);
                assert_eq!(default.background, colors.background, "{}", row.name);
                let ids = table
                    .iter()
                    .map(|style| style.style)
                    .collect::<HashSet<_>>();
                assert_eq!(ids.len(), table.len(), "{} maps a style twice", row.name);
            }
        }
    }

    #[test]
    fn json_distinguishes_keys_values_and_keywords() {
        // Break caught: the original three-style table, where keys and values looked the same.
        use crate::editor::scintilla_constants::{
            SCE_JSON_KEYWORD, SCE_JSON_PROPERTYNAME, SCE_JSON_STRING,
        };
        let theme = crate::platform::theme::Theme::Dark;
        let colors = crate::languages::syntax_colors(theme);
        let fore = |id| {
            (spec(Language::Json).styles)(theme)
                .iter()
                .find(|style| style.style == id)
                .map(|style| style.foreground)
        };
        assert_eq!(fore(SCE_JSON_PROPERTYNAME), Some(colors.key));
        assert_eq!(fore(SCE_JSON_STRING), Some(colors.string));
        assert_eq!(fore(SCE_JSON_KEYWORD), Some(colors.keyword));
    }
}
