# FastDraw diagrams (`.fdraw`)

**Branch:** `feat/fastdraw-diagrams`. One PR, after the crate in §3 is published.
**Upstream:** [coccor/fastdraw](https://github.com/coccor/fastdraw). FastPad depends on a Rust crate,
`fastdraw-render`, that lives in that repository (§3).

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
| Who keeps the renderer in step with FastDraw | The fastdraw repository: the crate sits next to the TypeScript renderer, and fastdraw's CI fails when their SVGs differ. FastPad pins an exact version, like `mermaid-rs-renderer`. |
| Fidelity | Full: every bundled look, `.look` files with `icondir` and `fontdir`, sketch, charts, all Phosphor icons, FastDraw's fonts. |
| How notes show diagrams | Image links to `.fdraw` files. Fenced ```` ```fdraw ```` blocks are not part of this version. |
| Pan and zoom | The existing image view (`src/image_view`), unchanged. |
| Dark themes | On a dark FastPad theme, a diagram is drawn in its look's dark counterpart, as the embed does on a dark page. |
| Open in FastDraw | The crate's share link, opened with the shell. |
| Not in this version | Syntax highlighting for `.fdraw`, ```` ```fdraw ```` blocks, exports, editing diagrams in FastPad, `fastdraw check`'s layout warnings. |

## 3. What FastPad needs from `fastdraw-render`

This section is the contract. The crate's own design (how it ports FastDraw, its tests, its release
process) is specified and built in the fastdraw repository; FastPad relies only on what is here.

### 3.1 Properties

- Pure Rust, `#![forbid(unsafe_code)]`, no network, no threads, no global mutable state beyond
  lazily-initialised read-only data. Builds on the FastPad toolchain (`rust-toolchain.toml`, edition 2024,
  `x86_64-pc-windows-msvc`).
- Dependencies limited to small, widely used crates (for example `miniz_oxide`, `indexmap`, `ryu-js`);
  FastPad's `tools/audit-dependencies.ps1` adds the crate as an allowed root at its pinned version.
- For every diagram and look in fastdraw's `examples/` and `assets/`, in every bundled look and its dark
  counterpart, `render` returns the same SVG, character for character, and the same issues as FastDraw's
  TypeScript `loadScene` + `renderSvg` at the same fastdraw version. fastdraw's CI checks this.
- Published to crates.io with the same version number as the npm package it matches.

### 3.2 Interface

```rust
/// A diagram and what it reads from disk. `file` is None for an untitled tab: no look file, icon
/// folder or font folder is read, and a look line naming a file is an issue.
pub struct Source<'a> { pub text: &'a str, pub file: Option<&'a Path>, pub dark: bool }

pub struct Drawing {
    pub svg: String,                  // standalone SVG; text names font families, no @font-face
    pub paper: [u8; 3],               // the look's paper colour (sRGB)
    pub bounds: [f64; 4],             // x, y, w, h in diagram units
    pub issues: Vec<Issue>,           // what `fastdraw check` reports as errors and warnings
    pub font_files: Vec<PathBuf>,     // TTF/OTF files from the look's fontdir
    pub look: Option<LookFile>,       // the diagram's own .look file, as read (for the share link)
}
pub struct Issue { pub line: u32, pub level: Level, pub message: String }
pub enum Level { Error, Warning }
pub struct LookFile { pub path: String, pub text: String } // path as written in the diagram

/// Never fails: unreadable files, refused paths and parse errors are issues, worded as FastDraw words them.
pub fn render(source: &Source) -> Drawing;

/// `https://fastdraw.dev/edit/#fdraw=<data>`, encoded as fastdraw's `editor/src/embed/share.ts` encodes it.
pub fn editor_link(name: &str, text: &str, look: Option<&LookFile>) -> String;

/// FastDraw's bundled fonts, (file name, TTF bytes). Inflated on first call.
pub fn fonts() -> &'static [(&'static str, Vec<u8>)];

pub const VERSION: &str; // the fastdraw version this crate matches
```

- With `dark`, the crate does what the embed's `draw` does: the look's `dark` line, else `night` for a
  diagram in the built-in look; a dark look that is a file is read relative to the look file.
- The extended icon set is loaded when a diagram names an icon outside the built-in set, as
  `loadExtendedIcons` does; the caller does not see this.
- Disk reads follow FastDraw's `node.ts`: only `*.look` files are read as looks; `icondir` and `fontdir`
  must resolve inside the look file's folder; its file count and size limits apply. Diagram and look files
  over 1 MB are not read.

## 4. `.fdraw` tabs

### 4.1 Language

`src/languages/registry.rs` maps the `fdraw` extension to a new `Language::FastDraw` (name "FastDraw", no
lexer, plain text in Scintilla). The file icon is the image kind, like SVG.

### 4.2 View modes

FastDraw tabs get `ViewMode::SVG` (Source, Split, Preview) wherever SVG tabs do: `preview_host.rs`
(`buttons_visible`, the per-language mode check, `zoom_svg`, `load_svg`) and
`editor_toolbar.rs::segments_for`. A new `.fdraw` tab opens in **Preview**; an empty or untitled one in
**Source**. The mode is remembered per tab, as for SVG.

### 4.3 Drawing

The preview is the SVG tab's image view with a new decode source,
`decode::Source::FastDraw { text: Arc<str>, file: Option<PathBuf>, dark: bool, width: u32 }`. On the
decode thread it calls `fastdraw_render::render`, registers fonts (§4.5), then the existing SVG
parse-and-rasterize path. Fit, zoom steps, Ctrl+wheel, drag to pan, `+` / `-` / `0` and double-click work
as for SVG tabs.

- The text drawn is the editor's text, so Split shows unsaved edits. Redraws use the preview's existing
  120 ms debounce, and a redraw keeps the zoom and pan.
- Saving or changing the diagram's look file outside FastPad redraws the tab.
- The view's background is the drawing's paper colour.

### 4.4 Diagrams with errors

When `render` returns an `Error` issue, the view keeps the last good drawing, if there is one, and shows a
bar along its bottom: `Line 4: there is no shape called "dbb"`, plus " (and 2 more)" when there are more.
With no good drawing yet, the view shows the same text centred. Warnings are not shown.

### 4.5 Fonts

`src/preview/svg.rs` owns the shared font database. The first FastDraw drawing adds `fastdraw_render::
fonts()` to it once. A drawing's `font_files` are added the first time each path is seen. Text in a family
with no font falls back as resvg does today.

### 4.6 Open in FastDraw

- A button on a FastDraw tab's editor toolbar after the view-mode control, tooltip
  "Open in FastDraw (fastdraw.dev)", and a command **FastDraw: Open in FastDraw** in the palette.
- It opens `editor_link(name, text, look)` with the tab's file name (`diagram.fdraw` when untitled), the
  editor's text and the last drawing's `look`, through a shared `platform::shell::open_url` that
  `about.rs`'s private `open_url` moves into.
- A link longer than 2 MB is not opened; a notice says the diagram is too large to open by link.

## 5. Diagrams in Markdown notes

- An image whose target is a local path ending in `.fdraw` (any case), resolved as other image paths are,
  draws as a diagram, in both `![]()` and `<img src>`. Web addresses stay as they are: nothing is fetched.
- `preview::images::decode_image` routes such paths to a new `preview::fastdraw::decode(path, dark,
  max_width)`: read the file, `render` with its path, register fonts, rasterize at the requested width.
- The image cache stamps `.fdraw` entries with the modification times of the diagram and its look file and
  with `dark`; a request whose stamp differs decodes again.
- A diagram that cannot be read or has errors shows as a broken image does today, with the alt text; its
  tooltip is the first error with its line number.
- Ctrl+Click (or a click in the Preview) on the diagram opens the `.fdraw` file in a tab, as a link to a
  local file does.

## 6. Testing (FastPad side)

The renderer's correctness is tested in the fastdraw repository (§3.1). FastPad tests its use of it:

- `.fdraw` maps to `Language::FastDraw` and gets Source / Split / Preview, opening in Preview.
- Typing in Split redraws from the editor text; a diagram with an error keeps the last drawing and shows
  the bar with the line number; an untitled diagram draws.
- On a dark theme, a diagram with no look line draws with `night`'s paper colour.
- The FastDraw fonts are in the font database after the first drawing, and a diagram's text renders with
  glyphs (the rasterized text area is not blank).
- `![x](d.fdraw)` decodes to an image with the drawing's proportions; a broken diagram is a broken image
  whose tooltip names the line; changing `d.fdraw` on disk decodes it again.
- Open in FastDraw passes the expected link to `open_url` (recorded in tests, as `reveal_in_explorer` is).

## 7. Docs and licences

- `docs/features.md` gains a **FastDraw diagrams** section: `.fdraw` tabs, view modes, pan and zoom, image
  links in notes, dark themes, and Open in FastDraw (noting that the diagram travels after the `#`).
  `docs/reference.md` lists the command.
- `LICENSES.md` and `licenses/` gain FastDraw (MIT), Phosphor Icons (MIT) and the fonts the crate ships
  (SIL Open Font License 1.1), via `tools/rust-crate-licenses.ps1` where it covers them.

## 8. Order of work

1. In the fastdraw repository: the `fastdraw-render` crate, its parity test in CI, and a crates.io release.
2. In FastPad: §4–§7 against that release. Until it is published, FastPad's branch can use a `path` or
   `git` dependency at a fixed commit, replaced by the exact version before merging.
