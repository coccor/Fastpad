# Syntax Highlighting Design

## Goal

Make syntax highlighting good enough to read real files: rich JSON and Markdown, working XML (and SVG),
plus config, web and script/code languages. Today only JSON and Markdown are lexed, each with three
styles, and every other extension (including `.xml`) is plain text.

## What the user asked for

- JSON is "minimal" → colour every meaningful JSON token, not three styles.
- XML "doesn't work" → detect and lex XML; SVG gets XML colours as well as its preview.
- Add config formats (YAML, TOML, INI/cfg/conf, properties, env), web (HTML, CSS, JavaScript,
  TypeScript) and scripts & code (PowerShell, Bash, Batch, Python, C/C++, C#, Rust, SQL).

## Constraints

- `Lexilla.dll` already contains every stock lexer; no native or DLL changes. Lexilla stays lazily
  loaded, and a plain-text session never loads it.
- Style tables stay compile-time `static`s (latency rules still apply).
- Language is not persisted; it is re-detected from the extension, so there is no migration.
- No new `windows` crate features are expected; if one is needed, update `tools/audit-dependencies.ps1`.

## Design

### 1. Semantic colour roles

`SyntaxColors` becomes a full role palette, defined for all six themes:

`background, text, comment, keyword, string, number, operator, key, tag, attribute, type, function,
preprocessor, variable, escape, heading, emphasis, link, code, code_background, error`.

- Light / Dark: VS Code Light+ / Dark+ values. The existing `string` and `number` values are kept as
  they are, and `text`/`background` are unchanged.
- Catppuccin: the official style guide. Keywords mauve, comments overlay2, strings green, numbers
  peach, operators sky, keys/properties blue, tags blue, attributes yellow, types yellow, functions
  blue, preprocessor pink, variables flamingo, escapes pink, headings red, links rosewater, errors red.

`LexerStyle` gains `italic: bool` (comments and emphasis use it), applied via `SCI_STYLESETITALIC`.
A small `const fn style(id, role_fg, colors) -> LexerStyle` helper keeps the per-language mapping
tables one line per style.

### 2. Language registry

The `Language` enum grows these variants: `Xml, Html, Css, JavaScript, TypeScript, Yaml, Toml, Ini,
Properties, Env, PowerShell, Bash, Batch, Python, C, Cpp, CSharp, Rust, Sql` (alongside the existing
`PlainText, Json, Markdown, Svg`).

A single `const LANGUAGES: &[LanguageSpec]` table in `src/languages/registry.rs` drives everything:

```rust
pub(crate) struct LanguageSpec {
    pub language: Language,
    pub display_name: &'static str,        // status bar, menu, palette
    pub extensions: &'static [&'static str], // lower-case, no dot; first = default save extension
    pub file_names: &'static [&'static str], // exact names, e.g. ".env", "Dockerfile"-style cases
    pub lexer: Option<&'static str>,        // Lexilla name; None = null lexer (plain text)
    pub keywords: &'static [&'static str],  // SCI_SETKEYWORDS sets, index = keyword-set number
    pub properties: &'static [(&'static str, &'static str)], // SCI_SETPROPERTY pairs
    pub styles: fn(Theme) -> &'static [LexerStyle],
}
```

Lexer mapping:

| Language | Lexilla lexer | Extensions (first is default) |
|---|---|---|
| JSON | `json` | json, jsonc, json5, jsonl, geojson, webmanifest, code-workspace |
| Markdown | `markdown` | md, markdown, mdown, mkd |
| XML | `xml` | xml, xaml, xsd, xsl, xslt, csproj, vbproj, fsproj, props, targets, config, resx, nuspec, manifest, plist, rss, atom |
| SVG | `xml` | svg |
| HTML | `hypertext` | html, htm, xhtml |
| CSS | `css` | css |
| JavaScript | `cpp` | js, mjs, cjs, jsx |
| TypeScript | `cpp` | ts, mts, cts, tsx |
| YAML | `yaml` | yaml, yml |
| TOML | `toml` | toml (also `Cargo.lock`) |
| INI | `props` | ini, cfg, conf, inf, reg, editorconfig, gitconfig |
| Properties | `props` | properties |
| Env | `props` | env (also `.env`, `.env.*` names) |
| PowerShell | `powershell` | ps1, psm1, psd1 |
| Bash | `bash` | sh, bash, zsh (also `.bashrc`, `.zshrc`, `.profile`) |
| Batch | `batch` | bat, cmd |
| Python | `python` | py, pyw, pyi |
| C | `cpp` | c, h |
| C++ | `cpp` | cpp, cc, cxx, hpp, hh, hxx, inl |
| C# | `cpp` | cs, csx |
| Rust | `rust` | rs |
| SQL | `sql` | sql |

Detection checks exact file names first, then the extension, case-insensitively. Lexers that share
`cpp` differ only in keyword sets (JS/TS keywords, C, C++, C# keywords). The `json` lexer gets
`lexer.json.allow.comments=1` and `lexer.json.escape.sequence=1`.

`detect_language`, `status.rs`'s display name, `title.rs`'s `default_extension`, and the menu and
palette all read the registry instead of matching on the enum. `Svg` keeps its preview special case in
`preview_host.rs`. `Markdown`'s preview is unchanged.

### 3. Per-language style maps

Each language module (`json.rs`, `markdown.rs`, and new `xml.rs` (also used by HTML), `css.rs`,
`cpp.rs` (C/C++/C#/JS/TS), `yaml.rs`, `toml.rs`, `props.rs`, `powershell.rs`, `bash.rs`, `batch.rs`,
`python.rs`, `rust.rs`, `sql.rs`) maps its `SCE_*` styles onto roles through `per_theme_styles!`.
Highlights:

- JSON: property names → `key`, string values → `string`, numbers, `true/false/null` → `keyword`,
  operators, escape sequences, URIs → `link`, comments, compact-IRI/LD keywords, errors.
- Markdown: H1–H6 → `heading` (bold), strong → bold, em → italic, links, list/quote markers
  → `operator`, inline code and fenced code → `code` on `code_background`, horizontal rules.
- XML/HTML: tags, unknown tags, attributes, attribute values, entities, comments, CDATA, processing
  instructions, doctype; HTML also maps embedded-JS (`SCE_HJ_*`) and CSS-in-style styles.

Every mapped language styles `STYLE_DEFAULT`-equivalent text to `text`/`background`, so switching
themes never leaves an unstyled token.

### 4. Applying a language

`LanguageManager::apply` looks up the spec, calls `set_lexer`, applies `properties` (`SCI_SETPROPERTY`),
applies `keywords` (`SCI_SETKEYWORDS`), clears styles, applies the table, then `SCI_COLOURISE(0, -1)`,
the same flow as today. A `None` lexer installs the null lexer without touching Lexilla.

`tools/generate-scintilla-constants.ps1` gains `SCI_SETKEYWORDS`, `SCI_SETPROPERTY`,
`SCI_STYLESETITALIC`, `SCI_COLOURISE` (if not present), and the `SCE_*` prefixes for the lexers
above (`SCE_H_`, `SCE_HJ_`, `SCE_CSS_`, `SCE_C_`, `SCE_YAML_`, `SCE_TOML_`, `SCE_PROPS_`,
`SCE_POWERSHELL_`, `SCE_SH_`, `SCE_BAT_`, `SCE_P_`, `SCE_RUST_`, `SCE_SQL_`). The constants file is
regenerated, not hand-edited.

### 5. Picker UI

- **View → Language ▸** is a submenu with Plain text followed by every registry language in display
  order; the active language is radio-checked.
- The command palette lists one "Language: <name>" command per language.
- The per-language `CommandId`s are generated from the registry index (`CommandId::Language(u8)` or a
  contiguous id range), so adding a language is a one-row registry change.

## Error handling

Unchanged: a Lexilla load or `CreateLexer` failure leaves the editor untouched and surfaces the
existing notification. An unknown lexer name is a programming error, caught by a unit test over the
whole registry.

## Testing

- Unit: every registry lexer name is accepted by the real `Lexilla.dll`; no extension or file name
  appears twice; every language × theme table styles its default style with `text`/`background`;
  the Light/Dark JSON string and number colours are unchanged; detection spot-checks per language
  (including `.env`, `Cargo.lock`, and upper-case extensions); SVG detection is still `Svg`.
- Apply: the fake-editor harness asserts `SCI_SETKEYWORDS` and `SCI_SETPROPERTY` are sent for C#
  and JSON, and the null lexer for plain text.
- Window: opening an `.xml` file loads Lexilla and installs a non-null lexer (mirrors the JSON test).
- Per the project's preference, run only targeted tests while developing and the full suite once at
  the end.

## Out of scope

Folding, a user-editable colour configuration, content sniffing (shebangs, `<?xml` headers), and new
lexers beyond the table.
