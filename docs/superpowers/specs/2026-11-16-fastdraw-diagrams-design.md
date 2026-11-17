# FastDraw diagrams (`.fdraw`)

**Branch:** `feat/fastdraw-diagrams`. One PR.
**Upstream:** [coccor/fastdraw](https://github.com/coccor/fastdraw). FastPad depends on its Rust crate,
`fastdraw-render` (`crates/fastdraw-render`), pinned by git commit (§3).

## 1. Goal

FastPad shows FastDraw diagrams the way the `<fastdraw-diagram>` embed shows them on pz-site: drawn,
with pan, zoom and fit, and a way to open the diagram in the FastDraw editor at fastdraw.dev.

- A `.fdraw` file opens in a tab with the Source / Split / Preview control SVG tabs have.
- `![alt](path/to/diagram.fdraw)` in a Markdown note draws the diagram in the Preview and Live views.
- **Open in FastDraw** opens the tab's diagram in `https://fastdraw.dev/edit/` in the default browser.

Diagrams are drawn on the user's machine, offline, the way Mermaid diagrams are: a Rust renderer writes
the SVG and resvg rasterizes it. Nothing is fetched; the only thing that leaves the machine is the
editor link the user asks for, and the diagram travels after its `#`, which no server sees.

## 2. Decisions

| Question | Decision |
|---|---|
| How a diagram becomes a picture | `fastdraw-render` (§3) writes the SVG; resvg rasterizes it on the image worker thread, as for `.svg` files and Mermaid. |
| Who keeps the renderer in step with FastDraw | The fastdraw repository: the crate sits next to the TypeScript renderer, and fastdraw's CI fails when their outputs differ. FastPad pins a commit. |
| Fidelity | Full: every bundled look, `.look` files with `icondir` and `fontdir`, sketch, charts, all Phosphor icons, FastDraw's fonts. |
| Look files named by a network or absolute path | Refused (§3.3): reading one opens an SMB connection, which a downloaded diagram could use to leak Windows credentials. |
| How notes show diagrams | Image links to `.fdraw` files. Fenced ```` ```fdraw ```` blocks are not part of this version. |
| Pan and zoom | The existing image view (`src/image_view`), unchanged. |
| Dark themes | On a dark FastPad theme, a diagram is drawn in its look's dark counterpart, as the embed does on a dark page. |
| Open in FastDraw | The crate's share link, opened with the shell. |
| Not in this version | Syntax highlighting for `.fdraw`, ```` ```fdraw ```` blocks, exports, editing diagrams in FastPad, `fastdraw check`'s layout warnings, redrawing when a look file changes outside FastPad. |

## 3. `fastdraw-render`

### 3.1 Dependency

```toml
fastdraw-render = { git = "https://github.com/coccor/fastdraw", rev = "f0d3905" }
```

The crate is not on crates.io (the fastdraw repository keeps `publish = false`). Crate version 0.2.2,
`rust-version` 1.88. `tools/audit-dependencies.ps1` allows it as a root, and its dependencies
(`miniz_oxide`, `indexmap`, `ryu-js`, `regex`, `libm`, with `adler2`, `aho-corasick`, `memchr`,
`regex-automata`, `regex-syntax`, `hashbrown`, `equivalent`) as it allows other transitive crates.
Moving to a newer FastDraw means moving `rev`.

### 3.2 Interface FastPad uses

```rust
pub struct Source<'a> { pub text: &'a str, pub file: Option<&'a Path>, pub dark: bool }
pub struct Drawing { pub svg: String, pub paper: [u8; 3], pub bounds: [f64; 4],
                     pub issues: Vec<Issue>, pub font_files: Vec<PathBuf>, pub look: Option<LookFile> }
pub struct Issue { pub line: u32, pub level: Level, pub message: String }
pub enum Level { Error, Warning }
pub struct LookFile { pub path: String, pub text: String }
pub fn render(source: &Source) -> Drawing;          // never fails; problems are issues
pub fn look_path(text: &str) -> Option<String>;     // the diagram's look line, as written
pub fn editor_link(name: &str, text: &str, look: Option<&LookFile>) -> String;
pub fn fonts() -> &'static [(&'static str, Vec<u8>)];
pub const VERSION: &str;
```

Issues are FastDraw's `loadScene` errors and warnings, with the user's line numbers, also in dark mode.
With `file: None` nothing is read from disk: the diagram draws in the built-in look and a look line that
names a file gives the error `look file not found: <path>` on that line.

### 3.3 FastPad's look-path rule

Before `render`, FastPad checks `look_path(text)`. A path that is absolute, starts with `\\` or `//`, or
has a drive prefix (`C:x` included) is refused: FastPad renders with `file: None`, so nothing is read,
and replaces the crate's `look file not found` error on that line with the error
`FastPad reads look files only by a relative path; "<path>" was not read`. That error does not hide the
drawing (§4.4). Bundled look names and relative paths are passed through unchanged.

## 4. `.fdraw` tabs

### 4.1 Language

`src/languages/registry.rs` maps the `fdraw` extension to a new `Language::FastDraw` (name "FastDraw", no
lexer, plain text in Scintilla). The file icon is the image kind, like SVG.

### 4.2 View modes

FastDraw tabs get `ViewMode::SVG` (Source, Split, Preview) wherever SVG tabs do: `preview_host.rs`
(`buttons_visible`, `available`, `zoom_svg`, the SVG view's loading) and `editor_toolbar.rs::segments_for`.
A FastDraw tab with a file and some text opens in **Preview**; an empty or untitled one in **Source**.
The choice is remembered per tab, as for SVG.

### 4.3 Drawing

The preview is the SVG tab's image view with a new decode source,
`decode::Source::FastDraw { text: Arc<str>, file: Option<PathBuf>, dark: bool, width: u32 }`. On the
decode thread FastPad applies §3.3, calls `render`, registers fonts (§4.5), then uses the existing SVG
parse-and-rasterize path. Fit, zoom steps, Ctrl+wheel, drag to pan, `+` / `-` / `0` and double-click work
as for SVG tabs.

- The text drawn is the editor's text, so Split shows unsaved edits. Redraws use the preview's existing
  debounce, and a redraw keeps the zoom and pan. The look file is read again on every redraw.
- The view's background is the drawing's paper colour, without the checkerboard.
- `render` runs inside `catch_unwind`; a panic is shown as the error "FastDraw could not draw this
  diagram".

### 4.4 Diagrams with errors

The SVG view already keeps the last good picture and shows a band "Can't render this SVG" along its top
when an edit breaks the SVG. FastDraw tabs use that band with their own text:

- When `render` returns `Error` issues other than the §3.3 refusal, the drawing is not shown. The band
  reads `Line 4: there is no shape called "dbb"`, plus ` (and 2 more)` when there are more errors, over the
  last good drawing. With no good drawing yet, the view shows the failed state titled "FastPad can't draw
  this diagram" with the same text.
- When the only error is the §3.3 refusal, the drawing is shown (in the built-in look) with the band
  giving that error.
- Warnings are not shown.

### 4.5 Fonts

`src/preview/svg.rs` owns the shared font database. The first FastDraw drawing adds
`fastdraw_render::fonts()` to it once. A drawing's `font_files` are added the first time each path is
seen. Other SVGs and Mermaid diagrams do not pay for the FastDraw fonts until a FastDraw diagram is drawn.

### 4.6 Open in FastDraw

- A command **FastDraw: Open in FastDraw**, available on FastDraw tabs: in the palette, in the tab's
  right-click menu, and as a button on the editor toolbar to the left of the view-mode control (an
  "open in new window" glyph from the toolbar's icon font, tooltip "Open in FastDraw (fastdraw.dev)").
- It opens `editor_link(name, text, look)` with the tab's file name (`diagram.fdraw` when untitled), the
  editor's text, and the look file as `render` read it (none when refused), through the existing
  `preview_host::shell_open`. A failure gives the notice "FastPad could not open fastdraw.dev.", as
  `about.rs` does for its links.
- A link longer than 2 MB is not opened; a notice says "This diagram is too large to open by link."

## 5. Diagrams in Markdown notes

- An image whose target is a local path ending in `.fdraw` (any case), resolved as other image paths are,
  draws as a diagram, in `![]()` and `<img src>`, in the Preview and Live views. Web addresses stay as
  they are: nothing is fetched.
- `preview::images::decode_image` routes such paths to `preview::fastdraw::decode_file(path, dark,
  max_width)`: read the file (at most 1 MB), apply §3.3, `render` with its path, register fonts,
  rasterize at the requested width with the SVG path.
- `ImageCache` learns the preview's darkness (`set_dark`); a change forgets its `.fdraw` entries.
- A `.fdraw` entry remembers the modification times of the diagram and of the look file it read. When a
  request finds either changed, it decodes again. Requests happen when the note is laid out (opening it,
  editing it, resizing the pane).
- A diagram that cannot be read or has errors shows as a broken image does today, with its alt text.

## 6. Testing (FastPad side)

The renderer's accuracy is tested in the fastdraw repository. FastPad tests its use of it:

- §3.3: `//server/x.look`, `\\server\x.look`, `C:/x.look`, `C:x.look` and `/x.look` are refused, the
  drawing is still produced and the issue reads as §3.3 says; `./x.look`, `x.look` and `night` pass.
- Error text: one error, three errors (` (and 2 more)`), and a refusal plus a real error.
- Fonts: after the first FastDraw drawing, a diagram's text rasterizes with ink in its label area.
- `.fdraw` maps to `Language::FastDraw`, gets Source / Split / Preview and opens in Preview; typing in
  Split redraws; a typing mistake keeps the last drawing and shows the band with the line number.
- On a dark theme a diagram with no look line draws on `night`'s paper colour.
- Open in FastDraw passes the expected link to the shell (recorded in tests).
- `![x](d.fdraw)` decodes to an image with the drawing's proportions; a broken diagram is a failed image;
  changing `d.fdraw` and requesting again decodes it again; changing darkness decodes it again.

## 7. Docs and licences

- `docs/features.md` gains a **FastDraw diagrams** section: `.fdraw` tabs, view modes, pan and zoom, image
  links in notes, dark themes, look files only by relative path, and Open in FastDraw (the diagram travels
  after the `#`). `docs/reference.md` lists the command.
- `LICENSES.md` and `licenses/` gain FastDraw (MIT), Phosphor Icons (MIT) and the fonts the crate ships
  (SIL Open Font License 1.1).
