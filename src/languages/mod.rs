mod json;
mod json_commands;
mod lexilla;
mod markdown;
mod registry;
mod bash;
mod batch;
mod cpp;
mod css;
mod keywords;
mod powershell;
mod props;
mod python;
mod rust;
mod sql;
mod toml;
mod xml;
mod yaml;

use crate::Result;
use crate::catppuccin::{self, Flavor};
use crate::document::Language;
use crate::editor::Editor;
use crate::platform::theme::Theme;
use lexilla::LexillaLibrary;
use std::path::PathBuf;

pub use json_commands::{JsonIssue, format_json, json_invocation_count, validate_json};
#[cfg(test)]
pub(crate) use registry::LANGUAGES;
pub use registry::{default_extension, detect_language, display_name};

/// One Scintilla lexer style's look: `style` is the lexer-specific `SCE_*` style number.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LexerStyle {
    pub(crate) style: u32,
    pub(crate) foreground: u32,
    pub(crate) background: u32,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
}

impl LexerStyle {
    pub(crate) const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub(crate) const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
}

/// What a token is, independent of language; each theme gives every role a colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Role {
    Text,
    Comment,
    Keyword,
    String,
    Number,
    Operator,
    Key,
    Tag,
    Attribute,
    Type,
    Function,
    Preprocessor,
    Variable,
    Escape,
    Heading,
    Emphasis,
    Link,
    Error,
}

/// The syntax roles every language's style table draws from, one set per theme. Each language
/// builds its per-theme tables from these at compile time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxColors {
    pub(crate) background: u32,
    pub(crate) text: u32,
    pub(crate) comment: u32,
    pub(crate) keyword: u32,
    pub(crate) string: u32,
    pub(crate) number: u32,
    pub(crate) operator: u32,
    pub(crate) key: u32,
    pub(crate) tag: u32,
    pub(crate) attribute: u32,
    pub(crate) type_name: u32,
    pub(crate) function: u32,
    pub(crate) preprocessor: u32,
    pub(crate) variable: u32,
    pub(crate) escape: u32,
    pub(crate) heading: u32,
    pub(crate) emphasis: u32,
    pub(crate) link: u32,
    pub(crate) code: u32,
    pub(crate) code_background: u32,
    pub(crate) error: u32,
}

impl SyntaxColors {
    pub(crate) const fn role(&self, role: Role) -> u32 {
        match role {
            Role::Text => self.text,
            Role::Comment => self.comment,
            Role::Keyword => self.keyword,
            Role::String => self.string,
            Role::Number => self.number,
            Role::Operator => self.operator,
            Role::Key => self.key,
            Role::Tag => self.tag,
            Role::Attribute => self.attribute,
            Role::Type => self.type_name,
            Role::Function => self.function,
            Role::Preprocessor => self.preprocessor,
            Role::Variable => self.variable,
            Role::Escape => self.escape,
            Role::Heading => self.heading,
            Role::Emphasis => self.emphasis,
            Role::Link => self.link,
            Role::Error => self.error,
        }
    }
}

/// Style `id` in `role`'s colour on the theme background; comments are italic.
pub(crate) const fn style(colors: &SyntaxColors, id: u32, role: Role) -> LexerStyle {
    LexerStyle {
        style: id,
        foreground: colors.role(role),
        background: colors.background,
        bold: false,
        italic: matches!(role, Role::Comment),
    }
}

/// Style `id` as code: the code colour on the code background.
pub(crate) const fn code(colors: &SyntaxColors, id: u32) -> LexerStyle {
    LexerStyle {
        style: id,
        foreground: colors.code,
        background: colors.code_background,
        bold: false,
        italic: false,
    }
}

// VS Code Light+ roles; text, background, string, number, heading and code are FastPad's originals.
const LIGHT_SYNTAX: SyntaxColors = SyntaxColors {
    background: rgb(255, 255, 255),
    text: rgb(32, 32, 32),
    comment: rgb(0, 128, 0),
    keyword: rgb(0, 0, 255),
    string: rgb(163, 21, 21),
    number: rgb(9, 134, 88),
    operator: rgb(0, 0, 0),
    key: rgb(4, 81, 165),
    tag: rgb(128, 0, 0),
    attribute: rgb(229, 0, 0),
    type_name: rgb(38, 127, 153),
    function: rgb(121, 94, 38),
    preprocessor: rgb(175, 0, 219),
    variable: rgb(0, 16, 128),
    escape: rgb(238, 0, 0),
    heading: rgb(0, 92, 197),
    emphasis: rgb(32, 32, 32),
    link: rgb(0, 112, 193),
    code: rgb(110, 65, 15),
    code_background: rgb(246, 248, 250),
    error: rgb(205, 49, 49),
};

// VS Code Dark+ roles; text, background, string, number, heading and code are FastPad's originals.
const DARK_SYNTAX: SyntaxColors = SyntaxColors {
    background: rgb(30, 30, 30),
    text: rgb(220, 220, 220),
    comment: rgb(106, 153, 85),
    keyword: rgb(86, 156, 214),
    string: rgb(206, 145, 120),
    number: rgb(181, 206, 168),
    operator: rgb(212, 212, 212),
    key: rgb(156, 220, 254),
    tag: rgb(86, 156, 214),
    attribute: rgb(156, 220, 254),
    type_name: rgb(78, 201, 176),
    function: rgb(220, 220, 170),
    preprocessor: rgb(197, 134, 192),
    variable: rgb(156, 220, 254),
    escape: rgb(215, 186, 125),
    heading: rgb(86, 156, 214),
    emphasis: rgb(220, 220, 220),
    link: rgb(79, 193, 255),
    code: rgb(215, 186, 125),
    code_background: rgb(45, 45, 45),
    error: rgb(244, 71, 71),
};

/// Catppuccin style guide roles.
const fn catppuccin_syntax(flavor: &Flavor) -> SyntaxColors {
    SyntaxColors {
        background: flavor.base,
        text: flavor.text,
        comment: flavor.overlay2,
        keyword: flavor.mauve,
        string: flavor.green,
        number: flavor.peach,
        operator: flavor.sky,
        key: flavor.blue,
        tag: flavor.blue,
        attribute: flavor.yellow,
        type_name: flavor.yellow,
        function: flavor.blue,
        preprocessor: flavor.pink,
        variable: flavor.flamingo,
        escape: flavor.pink,
        heading: flavor.red,
        emphasis: flavor.maroon,
        link: flavor.rosewater,
        code: flavor.blue,
        code_background: flavor.mantle,
        error: flavor.red,
    }
}

/// Indexed by `Theme as usize`; order must match `Theme::ALL`.
const SYNTAX_COLORS: [SyntaxColors; Theme::COUNT] = [
    LIGHT_SYNTAX,
    DARK_SYNTAX,
    catppuccin_syntax(&catppuccin::LATTE),
    catppuccin_syntax(&catppuccin::FRAPPE),
    catppuccin_syntax(&catppuccin::MACCHIATO),
    catppuccin_syntax(&catppuccin::MOCHA),
];

#[cfg(test)]
pub(crate) const fn syntax_colors(theme: Theme) -> SyntaxColors {
    SYNTAX_COLORS[theme as usize]
}

/// Evaluates to one style table per theme, indexed by `Theme as usize`, from a language's
/// `const fn(&SyntaxColors) -> [LexerStyle; N]` builder. Meant for a `static` initializer, so every
/// table is built at compile time.
macro_rules! per_theme_styles {
    ($builder:path) => {{
        use $crate::platform::theme::Theme;
        let colors = &$crate::languages::SYNTAX_COLORS;
        let mut tables = [$builder(&colors[0]); Theme::COUNT];
        let mut index = 1;
        while index < Theme::COUNT {
            tables[index] = $builder(&colors[index]);
            index += 1;
        }
        tables
    }};
}
pub(crate) use per_theme_styles;

/// Packs 8-bit components into the `0x00BBGGRR` layout Scintilla's `SCI_STYLESETFORE`/`_BACK`
/// expect (Windows `COLORREF` byte order), so style tables can be written as ordinary `(r, g, b)`.
pub(crate) const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

/// FastPad's compiled font-face fallback; there is no font configuration mechanism yet, so every
/// applied style currently uses this face unconditionally.
const DEFAULT_FONT_FACE: &str = "Consolas";

/// Lexilla's lexer catalog is not safe to touch concurrently from multiple OS threads on the same
/// loaded module (observed as a real `STATUS_ACCESS_VIOLATION` under `cargo test`'s default
/// parallelism). Every test that loads or calls into the real `Lexilla.dll` — here and in
/// `lexilla::tests` — holds this for its duration so the language test suite is reliable
/// regardless of `--test-threads`, matching the integration target's own `--test-threads=1` note.
#[cfg(test)]
pub(crate) static NATIVE_LEXILLA_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Owns the deferred `Lexilla.dll` and applies a document's language (lexer + style table) to the
/// live editor. `Lexilla.dll` is only ever loaded the first time `apply` is called with a lexed
/// language; a plain-text-only session never touches it.
#[derive(Debug, Default)]
pub struct LanguageManager {
    lexilla: Option<LexillaLibrary>,
    #[cfg(test)]
    dll_path_override: Option<PathBuf>,
}

impl LanguageManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies `language`'s lexer, lexer options, keyword sets and `theme`'s style table to
    /// `editor`. A language without a lexer installs Scintilla's null lexer (`SCI_SETILEXER` with
    /// a null pointer) and never touches Lexilla; `Lexilla.dll` loads on the first lexed language.
    /// On a Lexilla load or lexer-creation failure the editor is left exactly as it was (this
    /// method never calls `set_lexer` before a real lexer pointer is in hand) and the error is
    /// returned for the caller's notification.
    pub fn apply(&mut self, editor: &Editor, language: Language, theme: Theme) -> Result<()> {
        let spec = registry::spec(language);
        let Some(name) = spec.lexer else {
            editor.set_lexer(0)?;
            return Ok(());
        };
        let lexer = self.ensure_lexilla()?.create_lexer(name)?;
        editor.set_lexer(lexer)?;
        for (key, value) in spec.properties {
            editor.set_lexer_property(key, value)?;
        }
        for (set, words) in spec.keywords.iter().enumerate() {
            editor.set_keywords(set, words)?;
        }
        editor.clear_all_styles()?;
        for style in (spec.styles)(theme) {
            editor.set_style(
                style.style,
                style.foreground,
                style.background,
                style.bold,
                style.italic,
                DEFAULT_FONT_FACE,
            )?;
        }
        Ok(())
    }

    fn ensure_lexilla(&mut self) -> Result<&LexillaLibrary> {
        if self.lexilla.is_none() {
            let path = self.dll_path()?;
            self.lexilla = Some(LexillaLibrary::load(&path)?);
        }
        Ok(self
            .lexilla
            .as_ref()
            .expect("just populated above if it was absent"))
    }

    fn dll_path(&self) -> Result<PathBuf> {
        #[cfg(test)]
        if let Some(path) = &self.dll_path_override {
            return Ok(path.clone());
        }
        lexilla::default_dll_path()
    }

    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        self.lexilla.is_some()
    }

    #[cfg(test)]
    pub(crate) fn with_dll_path_for_test(path: PathBuf) -> Self {
        Self {
            lexilla: None,
            dll_path_override: Some(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LanguageManager, rgb};
    use crate::document::Language;
    use crate::editor::Editor;
    use crate::editor::scintilla_constants::{
        SCE_JSON_DEFAULT, SCI_SETILEXER, SCI_SETKEYWORDS, SCI_SETPROPERTY, SCI_STYLESETFONT,
        SCI_STYLESETFORE, SCI_STYLESETITALIC, STYLE_MAX,
    };
    use crate::platform::theme::Theme;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn rgb_packs_components_in_windows_colorref_byte_order() {
        // Break caught: swapping the byte order would silently flip red and blue on every style.
        assert_eq!(rgb(0x11, 0x22, 0x33), 0x00332211);
    }

    #[test]
    fn apply_plain_text_sends_the_null_lexer_and_never_loads_lexilla() {
        // Break caught: a plain-text-only session must never touch Lexilla.dll.
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let mut manager = LanguageManager::new();

        manager
            .apply(&editor, Language::PlainText, Theme::Light)
            .unwrap();

        assert_eq!(harness.setilexer_calls(), vec![0]);
        assert!(harness.strings(SCI_SETKEYWORDS).is_empty());
        assert!(!manager.is_loaded());
    }

    #[test]
    fn apply_json_with_a_missing_lexilla_dll_returns_an_error_and_leaves_the_editor_untouched() {
        // Break caught: a failed Lexilla load must not install a bad/partial lexer, and must
        // surface an error the caller can show in a notification instead of silently keeping
        // stale state.
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let missing = std::env::temp_dir().join(format!(
            "fastpad-languages-missing-lexilla-{}-{}.dll",
            std::process::id(),
            next_unique()
        ));
        let mut manager = LanguageManager::with_dll_path_for_test(missing);

        assert!(
            manager
                .apply(&editor, Language::Json, Theme::Light)
                .is_err()
        );

        assert!(harness.setilexer_calls().is_empty());
        assert!(!manager.is_loaded());
    }

    #[test]
    fn json_activation_loads_lexilla_and_installs_a_non_null_lexer() {
        let _guard = super::NATIVE_LEXILLA_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let mut manager = LanguageManager::with_dll_path_for_test(native_lexilla_path());

        manager
            .apply(&editor, Language::Json, Theme::Light)
            .unwrap();

        let calls = harness.setilexer_calls();
        assert_eq!(calls.len(), 1);
        assert_ne!(calls[0], 0);
        assert!(manager.is_loaded());
    }

    #[test]
    fn lexilla_is_loaded_once_and_reused_across_subsequent_language_switches() {
        // Break caught: reloading Lexilla.dll on every language switch instead of caching it in
        // the lazy `Option<LexillaLibrary>`. Proven by pointing the override at a nonexistent path
        // right after the first load succeeds: a second load attempt would now fail, so the second
        // `apply` only succeeds if it reused the already-loaded library instead of reloading.
        let _guard = super::NATIVE_LEXILLA_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let mut manager = LanguageManager::with_dll_path_for_test(native_lexilla_path());

        manager
            .apply(&editor, Language::Json, Theme::Light)
            .unwrap();
        manager.dll_path_override = Some(std::env::temp_dir().join(format!(
            "fastpad-languages-lexilla-missing-after-first-load-{}-{}.dll",
            std::process::id(),
            next_unique()
        )));

        manager
            .apply(&editor, Language::Markdown, Theme::Light)
            .unwrap();

        assert_eq!(harness.setilexer_calls().len(), 2);
        assert!(manager.is_loaded());
    }

    #[test]
    fn apply_selects_the_theme_style_table_for_the_default_style() {
        let _guard = super::NATIVE_LEXILLA_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let default_foreground = |theme| {
            let harness = FakeHarness::new();
            let editor = fake_editor(&harness);
            let mut manager = LanguageManager::with_dll_path_for_test(native_lexilla_path());
            manager.apply(&editor, Language::Json, theme).unwrap();
            harness.style_fore_for(SCE_JSON_DEFAULT as usize)
        };

        assert_eq!(
            default_foreground(Theme::Light),
            Some(rgb(32, 32, 32) as isize)
        );
        assert_eq!(
            default_foreground(Theme::Dark),
            Some(rgb(220, 220, 220) as isize)
        );
        assert_eq!(
            default_foreground(Theme::CatppuccinMocha),
            Some(crate::catppuccin::MOCHA.text as isize)
        );
    }

    #[test]
    fn csharp_sends_its_keyword_sets_and_escape_property() {
        let _guard = super::NATIVE_LEXILLA_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let mut manager = LanguageManager::with_dll_path_for_test(native_lexilla_path());

        manager
            .apply(&editor, Language::CSharp, Theme::Light)
            .unwrap();

        let keywords = harness.strings(SCI_SETKEYWORDS);
        assert_eq!(keywords.len(), 2);
        assert_eq!(keywords[0].0, "0");
        assert!(keywords[0].1.split(' ').any(|word| word == "namespace"));
        assert_eq!(
            harness.strings(SCI_SETPROPERTY),
            vec![("lexer.cpp.escape.sequence".to_owned(), "1".to_owned())]
        );
    }

    #[test]
    fn every_registry_lexer_name_is_accepted_by_lexilla() {
        // Break caught: a typo such as "html" instead of "hypertext" failing only when a user
        // opens that file type.
        let _guard = super::NATIVE_LEXILLA_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let mut manager = LanguageManager::with_dll_path_for_test(native_lexilla_path());
        for row in super::LANGUAGES.iter() {
            manager
                .apply(&editor, row.language, Theme::Dark)
                .unwrap_or_else(|error| panic!("{}: {error:?}", row.name));
        }
    }

    #[test]
    fn reapplying_with_another_theme_restyles_xml_tags() {
        use crate::editor::scintilla_constants::SCE_H_TAG;
        let _guard = super::NATIVE_LEXILLA_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);
        let mut manager = LanguageManager::with_dll_path_for_test(native_lexilla_path());

        manager.apply(&editor, Language::Xml, Theme::Light).unwrap();
        manager
            .apply(&editor, Language::Xml, Theme::CatppuccinMocha)
            .unwrap();

        assert_eq!(
            harness.style_fore_for(SCE_H_TAG as usize),
            Some(crate::catppuccin::MOCHA.blue as isize)
        );
    }

    #[test]
    fn existing_theme_colors_are_unchanged() {
        // Break caught: growing the palette must not repaint what users already see.
        let light = super::syntax_colors(Theme::Light);
        assert_eq!(light.text, rgb(32, 32, 32));
        assert_eq!(light.string, rgb(163, 21, 21));
        assert_eq!(light.number, rgb(9, 134, 88));
        let mocha = super::syntax_colors(Theme::CatppuccinMocha);
        assert_eq!(mocha.string, crate::catppuccin::MOCHA.green);
        assert_eq!(mocha.keyword, crate::catppuccin::MOCHA.mauve);
    }

    #[test]
    fn comments_are_italic_and_code_sits_on_the_code_background() {
        let colors = super::syntax_colors(Theme::Dark);
        let comment = super::style(&colors, 7, super::Role::Comment);
        assert!(comment.italic);
        assert_eq!(comment.foreground, colors.comment);
        let keyword = super::style(&colors, 8, super::Role::Keyword).bold();
        assert!(keyword.bold && !keyword.italic);
        let code = super::code(&colors, 9);
        assert_eq!(code.background, colors.code_background);
    }

    #[test]
    fn view_settings_reach_every_style_number() {
        // Break caught: HTML's embedded-JS styles (40-53) and any lexer style above
        // STYLE_LINENUMBER kept Consolas at the default size instead of the user's font.
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);

        let _ = editor.apply_view_settings("Cascadia Mono", 11, 4, false);

        let styled = harness
            .sent(SCI_STYLESETFONT)
            .into_iter()
            .map(|(style, _)| style)
            .collect::<Vec<_>>();
        assert!(styled.contains(&53));
        assert!(styled.contains(&(STYLE_MAX as usize)));
    }

    #[test]
    fn editor_sends_italic_keywords_and_properties() {
        let harness = FakeHarness::new();
        let editor = fake_editor(&harness);

        editor
            .set_style(1, rgb(1, 2, 3), rgb(4, 5, 6), false, true, "Consolas")
            .unwrap();
        editor.set_keywords(1, "true false").unwrap();
        editor
            .set_lexer_property("lexer.json.allow.comments", "1")
            .unwrap();

        assert_eq!(harness.sent(SCI_STYLESETITALIC), vec![(1, 1)]);
        assert_eq!(
            harness.strings(SCI_SETKEYWORDS),
            vec![("1".to_owned(), "true false".to_owned())]
        );
        assert_eq!(
            harness.strings(SCI_SETPROPERTY),
            vec![("lexer.json.allow.comments".to_owned(), "1".to_owned())]
        );
    }

    fn next_unique() -> u64 {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed)
    }

    fn native_lexilla_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("native")
            .join("out")
            .join("x64")
            .join("Lexilla.dll")
    }

    #[derive(Default)]
    struct FakeState {
        messages: Vec<(u32, usize, isize)>,
        /// NUL-terminated strings the fake read out of pointer arguments, in send order.
        strings: Vec<(u32, String, String)>,
    }

    struct FakeHarness {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeHarness {
        fn new() -> Self {
            Self {
                state: Arc::new(Mutex::new(FakeState::default())),
            }
        }

        fn direct_ptr(&self) -> isize {
            Arc::as_ptr(&self.state) as isize
        }

        fn setilexer_calls(&self) -> Vec<isize> {
            self.sent(SCI_SETILEXER)
                .into_iter()
                .map(|(_, lparam)| lparam)
                .collect()
        }

        fn style_fore_for(&self, style: usize) -> Option<isize> {
            self.sent(SCI_STYLESETFORE)
                .into_iter()
                .rev()
                .find(|(wparam, _)| *wparam == style)
                .map(|(_, lparam)| lparam)
        }

        fn sent(&self, message: u32) -> Vec<(usize, isize)> {
            self.state
                .lock()
                .unwrap()
                .messages
                .iter()
                .filter(|(sent, _, _)| *sent == message)
                .map(|(_, wparam, lparam)| (*wparam, *lparam))
                .collect()
        }

        fn strings(&self, message: u32) -> Vec<(String, String)> {
            self.state
                .lock()
                .unwrap()
                .strings
                .iter()
                .filter(|(sent, _, _)| *sent == message)
                .map(|(_, first, second)| (first.clone(), second.clone()))
                .collect()
        }
    }

    fn fake_editor(harness: &FakeHarness) -> Editor {
        Editor::test_fixture(fake_direct, harness.direct_ptr())
    }

    unsafe extern "C" fn fake_direct(
        direct_ptr: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        let shared = unsafe { &*(direct_ptr as *const Mutex<FakeState>) };
        let mut state = shared.lock().unwrap();
        state.messages.push((message, wparam, lparam));
        let read = |pointer: isize| {
            unsafe { std::ffi::CStr::from_ptr(pointer as *const std::ffi::c_char) }
                .to_string_lossy()
                .into_owned()
        };
        match message {
            SCI_SETKEYWORDS => {
                let words = read(lparam);
                state.strings.push((message, wparam.to_string(), words));
            }
            SCI_SETPROPERTY => {
                let pair = (read(wparam as isize), read(lparam));
                state.strings.push((message, pair.0, pair.1));
            }
            _ => {}
        }
        0
    }
}
