# FastDraw diagrams (`.fdraw`)

**Branch:** `feat/fastdraw-diagrams`. One PR.
**Upstream:** [coccor/fastdraw](https://github.com/coccor/fastdraw), pinned at **0.2.2** (commit `5ec6789`).

## 1. Goal

FastPad shows FastDraw diagrams the way the `<fastdraw-diagram>` embed shows them on pz-site: drawn,
with pan, zoom and fit, and a way to open the diagram in the FastDraw editor at fastdraw.dev.

- A `.fdraw` file opens in a tab with the Source / Split / Preview control SVG tabs have.
- `![alt](path/to/diagram.fdraw)` in a Markdown note draws the diagram in the Preview and Live views.
- **Open in FastDraw** opens the tab's diagram in `https://fastdraw.dev/edit/` in the default browser.

The drawing is FastDraw's own, produced by a native Rust port of FastDraw's renderer. It needs no Node,
no browser and no network. The SVG FastPad produces is identical, character for character, to the SVG
FastDraw 0.2.2 produces for the same diagram and look (§7).

## 2. Decisions

| Question | Decision |
|---|---|
| How a diagram becomes a picture | A Rust port of FastDraw's parser, scene builder, router and SVG renderer writes the SVG; resvg rasterizes it on the image worker thread, as it does for `.svg` files. |
| Fidelity | Full: every bundled look, `.look` files (including `icondir` and `fontdir`), the sketch style, charts, the full Phosphor icon set and FastDraw's fonts. |
| How notes show diagrams | Image links to `.fdraw` files. Fenced ```` ```fdraw ```` blocks are not part of this version. |
| Pan and zoom | The existing image view (`src/image_view`), unchanged. |
| Dark themes | On a dark FastPad theme, a diagram is drawn in its look's dark counterpart, as the embed does on a dark page (§5.3). |
| Open in FastDraw | A link built exactly as FastDraw's `editor/src/embed/share.ts` builds it, opened with the shell. The diagram travels after the `#`. |
| Where the data lives | Generated Rust sources and compressed blobs compiled into the exe; fonts and the extended icons unpack on first use. |
| Keeping up with FastDraw | A `tools/` script regenerates the data and the reference SVGs from a FastDraw checkout; the golden tests show what changed. |
| Not in this version | Syntax highlighting for `.fdraw`, ```` ```fdraw ```` blocks, writing `.svg`/`.png` exports, editing diagrams inside FastPad, `fastdraw check`'s layout warnings. |

## 3. The port: `src/fastdraw/`

A new module with no Win32 code, so it builds and tests on its own. Its files follow FastDraw's, one for
one, so a change upstream maps to one place here:

| FastDraw (`src/`) | FastPad (`src/fastdraw/`) | What it does |
|---|---|---|
| `parse.ts` | `parse.rs` | Statements and tokens, with line numbers. |
| `look.ts`, `defaultLook.ts` | `look.rs` | `.look` parsing, the built-in look, bundled looks, colours, fonts, `dark`. |
| `scene.ts`, `sizes.ts`, `fontWidths.ts` | `scene.rs` | Shapes, connections, texts, legend, extent, text measuring from the width tables, issues. |
| `router.ts` | `router.rs` | Routing lines around boxes. |
| `render.ts`, `chart.ts`, `sketch.ts` | `render.rs`, `chart.rs`, `sketch.rs` | The SVG. The sketch wobble stays seeded from the path text, so output is repeatable. |
| `icons.ts`, `iconStyle.ts`, `iconSet.ts`, `extendedIcons.ts`, `iconfiles.ts` | `icons.rs` | Built-in and extended Phosphor icons; `icondir` SVG files as `data:` URIs. |
| `node.ts` (`loadScene`, `readLook`, `lookFolder`, `plainFiles`, `lookFonts`) | `load.rs` | Reading a diagram's look file, icon folder and font folder from disk, with FastDraw's limits. |
| `editor/src/embed/share.ts` | `share.rs` | The `#fdraw=` link. |
| `editor/src/embed/theme.ts` (`darkCounterpart`) | `look.rs` | Which look to draw in on a dark theme. |

### 3.1 Public interface

```rust
pub struct Source<'a> {
    pub text: &'a str,            // the diagram, possibly unsaved editor text
    pub file: Option<&'a Path>,   // where it lives, for resolving its look; None for an untitled tab
    pub dark: bool,               // draw in the look's dark counterpart when it has one
}

pub struct Drawing {
    pub svg: String,              // standalone SVG, fonts referenced by family name
    pub paper: Color,             // the look's paper colour, for the view's background
    pub bounds: (f32, f32, f32, f32), // x, y, w, h in diagram units
    pub issues: Vec<Issue>,       // { line, level: Error | Warn, msg }
    pub font_files: Vec<PathBuf>, // TTF/OTF files from the look's fontdir
}

pub fn draw(source: &Source) -> Drawing;
pub fn share_link(editor: &str, name: &str, text: &str, look: Option<(&str, &str)>) -> String;
```

`draw` never fails: a missing or unreadable look, a refused path or a parse error is an issue, exactly
as FastDraw reports it. A drawing with any `Error` issue is not shown (§4.4, §5.4).

### 3.2 Matching JavaScript exactly

Byte-identical output needs the port to reproduce JavaScript's arithmetic and formatting:

- **Numbers in the SVG** are formatted as JavaScript's `Number.prototype.toString` formats them
  (shortest round-trip, `1e21` thresholds, `-0` printed as `0`), via the `ryu-js` crate.
- **Rounding** uses JavaScript's `Math.round` (halves round towards +∞), not Rust's `f64::round`.
- **Numbers parsed from text** follow JavaScript's `+v` / `isNaN(+v)` rules (`num` in `render.ts`):
  leading and trailing whitespace allowed, `""` is not a number, hex and `Infinity` accepted.
- **Strings** are measured and sliced by UTF-16 code unit wherever FastDraw uses `.length` or
  `.slice`, so non-BMP text lays out the same.
- **Iteration order** of maps follows insertion order wherever FastDraw iterates a `Map` or object
  (`indexmap`).
- **The sketch generator** reproduces `seeded()` in `sketch.ts` bit for bit, including 32-bit integer
  wrapping (`Math.imul`, `>>> 0`).

### 3.3 Generated data

`tools/fastdraw-data.mjs <fastdraw checkout>` writes:

- `src/fastdraw/data/looks.rs`: the 12 bundled `.look` sources as `&str`.
- `src/fastdraw/data/widths.rs`: the font-width tables from `fontWidths.ts`.
- `src/fastdraw/data/icons_builtin.rs`: the built-in icon set (~290 kB of path data), compiled in.
- `assets/fastdraw/icons-extended.bin`: the rest of Phosphor (~3.4 MB of path data), deflate-compressed,
  included with `include_bytes!` and inflated the first time a diagram names an icon outside the built-in
  set, as FastDraw's `loadExtendedIcons` does.
- `assets/fastdraw/fonts.bin`: FastDraw's TTFs from `fonts/` (~2 MB), deflate-compressed, included with
  `include_bytes!` and inflated the first time a diagram is drawn.
- `tests/fastdraw/upstream.txt`: the FastDraw version and commit the data came from.

The script is the only way these files change. It refuses to run on a FastDraw version that is not the
one in `upstream.txt` unless given `--update`, which also regenerates the reference SVGs (§7.1).

Licences: Phosphor Icons (MIT) and the fonts (SIL Open Font License 1.1: Geist, Geist Mono, Instrument
Serif, Libre Franklin, Newsreader, Sketch Hand) are added to `LICENSES.md` and `licenses/`, and FastDraw
itself (MIT) is credited.

### 3.4 Fonts in resvg

`src/preview/svg.rs` owns the shared `fontdb`. The first `.fdraw` drawing adds FastDraw's fonts to it
once. A look's `fontdir` files (`Drawing::font_files`) are added the first time they are seen, keyed by
path and modification time, within FastDraw's limits (20 MB per file, 200 files). Text in a family with no
font falls back as resvg does today.

## 4. `.fdraw` tabs

### 4.1 Language

`src/languages/registry.rs` maps the `fdraw` extension to a new `Language::FastDraw` (name "FastDraw",
plain text in Scintilla, no lexer). The file icon is the image kind, like SVG.

### 4.2 View modes

FastDraw tabs get `ViewMode::SVG` (Source, Split, Preview) everywhere SVG tabs do: `preview_host.rs`
(`is_previewable`, the mode checks), `editor_toolbar.rs::segments_for`, and the view-mode commands and
menu. A new `.fdraw` tab opens in **Preview**; an empty or untitled one opens in **Source**. The tab's
mode is remembered per tab as for SVG.

### 4.3 Drawing and redrawing

The preview is the existing image view with a new source, `decode::Source::FastDraw { text, file, dark,
width }`. On the image worker thread it runs `fastdraw::draw`, registers any font files, then
`decode_svg_source` on the SVG. Fit, zoom steps, Ctrl+wheel, drag to pan, scroll bars, `+` / `-` / `0`
and double-click to fit behave as for SVG tabs.

- The text drawn is the editor's text, not the file on disk, so Split shows unsaved edits. Redraws are
  debounced as the SVG preview's are, and a redraw keeps the current zoom and pan.
- A save or an outside change to the diagram's `.look` file (or its icon or font folder) redraws the tab.
- The view's background is the drawing's paper colour.

### 4.4 Diagrams with errors

When `draw` returns an `Error` issue, the view keeps the last good drawing, if there is one, and shows a
bar along its bottom: `Line 4: there is no shape called "dbb"`, plus "and 2 more" when there are more.
With no good drawing yet, the view shows the same text centred. Warnings are not shown.

### 4.5 Open in FastDraw

- A button on the editor toolbar of a FastDraw tab, after the view-mode control, tooltip
  "Open in FastDraw (fastdraw.dev)". Also a command, **FastDraw: Open in FastDraw**, in the palette.
- It builds `share_link("https://fastdraw.dev/edit/", name, text, look)` where `name` is the file name
  (`diagram.fdraw` for an untitled tab), `text` is the editor's text, and `look` is the diagram's own
  `.look` file (its path as written in the diagram, and its text) when it names one that can be read.
  Bundled looks travel by name in the diagram text, as in FastDraw.
- The link is opened in the default browser with `ShellExecuteW`, through a shared `platform::shell::open_url`
  that `about.rs`'s private `open_url` moves into. Nothing else is sent anywhere.
- A link longer than 2 MB is not opened; a notice says the diagram is too large to open by link.

## 5. Diagrams in Markdown notes

### 5.1 Which links

An image whose target is a local path ending in `.fdraw` (any case), resolved as other image paths are,
draws as a diagram. Web URLs stay as they are today: nothing is fetched. Links to `.fdraw` files in
`<img src>` HTML follow the same rule.

### 5.2 Drawing

`preview::images::decode_image` routes `.fdraw` paths to a new `fastdraw::decode` that reads the file
(limit 1 MB, §6), runs `draw` with the file's path and the
preview's darkness, registers font files, and rasterizes with the existing SVG path at the requested
width. The image cache treats it like an SVG: it is redrawn larger on request, and dropped when the
`.fdraw` file, its look file, or the preview's theme changes.

### 5.3 Dark themes

`dark` is `PreviewColors::is_dark()` for notes and the image view's colours for tabs. The look used on
dark follows `darkCounterpart`: the look's `dark` line, else `night` for a diagram with no look line,
else the diagram's own look.

### 5.4 Errors

A diagram that cannot be read or has errors shows as a broken image does today, with the alt text, and
its tooltip gives the first error with its line number.

### 5.5 Interaction

The diagram is an image in the preview: no pan or zoom inside the note. Ctrl+Click (or a click in the
Preview) on it opens the `.fdraw` file in a tab, as a link to a local file does.

## 6. Error handling and limits

- Diagram files over 1 MB and look files over 1 MB are not read; that is an error issue.
- Only files ending in `.look` are read as looks (FastDraw's rule). `icondir` and `fontdir` must be
  relative and resolve inside the look file's folder (`lookFolder`); others are ignored with a warning.
- Icon SVG files over 300 kB or not starting with `<svg` are skipped, as in `iconfiles.ts`.
- The rasterized size is capped by the existing `MAX_IMAGE_PIXELS` and `svg_render_size`.
- Inflating the font or icon blob happens once per process; a failure (it cannot fail with a valid
  build) is reported as an error issue rather than a panic.

## 7. Testing

### 7.1 Golden SVGs

`tools/fastdraw-data.mjs --update` bundles FastDraw's `src/node.ts` with esbuild and runs its own
`loadScene` + `renderSvg` and writes, for every diagram in `examples/kinds`, `examples/showcase`, `examples/look-test`,
`examples/whiteboard`, `assets/*.fdraw`, and each of those drawn in every bundled look and in its dark
counterpart, a reference to `tests/fastdraw/golden/<name>[.<look>].svg`, with the diagram and any look
files beside it under `tests/fastdraw/cases/`.

`tests/fastdraw_golden.rs` draws every case with `fastdraw::draw` and asserts the SVG equals the
reference exactly. On a mismatch it prints the first differing line and writes the actual output to
`target/fastdraw-diff/` for inspection.

Issues (line, level, message) are compared the same way against `<name>.issues.json`, so error text
matches FastDraw's.

### 7.2 Unit tests

- `share.rs`: links for a diagram with and without a look decode back (inflate, base64url) to the same
  JSON FastDraw's `decodeShared` accepts; a fixed case matches a link produced by FastDraw's
  `encodeShared` byte for byte.
- `parse.rs`, `look.rs`, `icons.rs`: the cases in FastDraw's `test/core.test.ts`, `test/looks.test.ts`,
  `test/ligatures.test.ts` and `test/icons-extended.test.ts` that check parsing, looks, text measuring
  and icons (not the CLI, server or editor ones), ported as Rust tests.
- JavaScript number formatting and `Math.round` against a table of edge cases.
- `load.rs`: look paths not ending in `.look` are refused; `icondir` outside the look's folder is
  ignored; size limits.

### 7.3 FastPad integration tests

- `.fdraw` maps to `Language::FastDraw`, gets Source / Split / Preview, and opens in Preview.
- Typing in Split redraws from the editor text; an error keeps the last drawing and shows the bar.
- A note with `![x](d.fdraw)` decodes to an image of the diagram's proportions; a broken diagram shows
  as a broken image; changing `d.fdraw` on disk redraws it.
- On a dark theme, a diagram with no look draws with `night`'s paper colour.
- Open in FastDraw produces the expected link for a known diagram (the shell call is stubbed).

## 8. Docs

`docs/features.md` gains a **FastDraw diagrams** section: opening `.fdraw` files, view modes, pan and
zoom, image links in notes, dark themes, and Open in FastDraw (with the privacy note that the diagram
travels after the `#`). `docs/reference.md` lists the new command.
