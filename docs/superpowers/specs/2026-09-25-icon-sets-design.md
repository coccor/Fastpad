# File icon sets in the Notebook tree: design

- Status: approved in conversation on 2026-09-25.
- Branch: `feat/icon-sets`, stacked on `feat/inline-naming` (PR #16).
- It replaces the notebook folders spec's note-type icons (§5.1): those glyphs become the Minimal set.

## 1. Goal

- **Better icons:** the Notebook tree's note and folder icons come from Material Icon Theme (MIT), the set many VS Code users know.
- **Switchable sets:** FastPad ships more than one icon set, and you switch between them from the palette.
- **Only what we need:** only the icons the tree actually draws are bundled.

## 2. Decisions

| Question | Decision |
|---|---|
| What "configurable" means | Sets built into FastPad, switched from the palette and saved in `fastpad.ini`. No sets loaded from disk. |
| The sets | **Material** (the default) and **Minimal**, which is today's Segoe glyphs and Catppuccin colours, unchanged. |
| How Material is drawn | Rasterized at build time by a dev-only generator into pixels embedded in the exe, and drawn with `AlphaBlend`. No SVG parsing or Direct2D at runtime. |
| Which Material icons | 12: one per note type, plus the closed and open folder (§3). |
| High contrast | Always draws the Minimal glyphs in system colours, whichever set is chosen. |

## 3. The icons

### 3.1 Mapping

| Note type | Extensions | Material | Minimal (today) |
|---|---|---|---|
| Markdown | `md`, `markdown` | `markdown` | document glyph, blue |
| JSON | `json` | `json` | `{}`, yellow |
| YAML | `yaml`, `yml` | `yaml` | settings glyph, peach |
| TOML | `toml` | `toml`, or `toml_light` in a light theme | settings glyph, peach |
| INI and config | `ini`, `cfg`, `conf` | `settings` | settings glyph, peach |
| CSV | `csv` | `table` | grid glyph, green |
| XML | `xml` | `xml` | code glyph, maroon |
| Text | `txt`, `text`, no extension, any other | `document` | document glyph, overlay |
| Log | `log` | `log` | document glyph, overlay |
| Folder, collapsed | | `folder` | folder glyph, yellow |
| Folder, expanded | | `folder-open` | folder glyph, yellow |

- Extensions match ignoring case, as they do today.
- **Light themes:** Light and Catppuccin Latte. In either one, TOML uses `toml_light`. Every other Material icon looks the same in every theme.
- **Minimal is unchanged.** It has no open-folder glyph: an expanded folder keeps the folder glyph.
- **The Material names are the ones in Material Icon Theme's `icons/` folder.** If a release has renamed one, the generator uses the new file and the Rust table keeps FastPad's own names.

### 3.2 Where the set applies

- **The set draws:**
  - the icon on note rows;
  - the icon on folder rows;
  - the icon on the draft row while a name is typed (inline naming spec §3.1). It follows the extension typed so far.
- **Segoe glyphs in every set:**
  - the chevrons;
  - the pin;
  - the header buttons (+, New folder, star, more);
  - the note glyph on unsaved rows.
- **Selected and hovered rows** keep the icon's own colours, as the coloured glyphs do today.
- **High contrast:**
  - Every set draws the Minimal glyphs in the muted system colour pair, which is today's high-contrast look.
  - Fixed colours can't follow a high-contrast scheme.
- **Out of the tree:** icons appear nowhere else. Tabs, Ctrl+P and Favorites don't change.

### 3.3 Source and licence

- **Where the SVGs come from:**
  - one pinned release of the `material-icon-theme` npm package (github.com/material-extensions/vscode-material-icon-theme);
  - the latest release when this is built.
- **Only the 12 files are copied,** into `assets/icons/material/svg/`, unchanged.
- **`assets/icons/material/SOURCE.md`** records:
  - the package name and version;
  - the upstream path of each file.
- **The licence:**
  - Its text is in `licenses/material-icon-theme.txt`.
  - `LICENSES.md` gains a section for bundled artwork: the set, its version, MIT, and the licence file.
  - The packaging scripts that copy `licenses/` pick it up.

## 4. Choosing a set

- **The setting:**
  - `file_icons = material | minimal` in `fastpad.ini`.
  - The default is `material`.
  - Values are read ignoring case.
  - An unknown value gets the usual settings warning, and the default is used.
- **In the palette:**
  - two rows next to the Theme rows: "File icons: Material" and "File icons: Minimal";
  - new commands `CommandId::FileIconsMaterial` and `CommandId::FileIconsMinimal`;
  - the current set is marked, as the current theme is.
- **Choosing one:**
  - saves the key with `config::save_setting("file_icons", …)`, as `theme` is saved;
  - updates the setting in every FastPad window;
  - repaints each window's tree.
  - No rescan and no restart.
- **A failed save** shows the notice `theme` uses when its save fails. The chosen set still applies for this session.

## 5. The icon data

### 5.1 The generator

- **`tools/generate-file-icons.ps1`:**
  - runs a dev-only generator;
  - that generator is an ignored test or a `src/bin/` tool, and it isn't shipped in the package.
- **For each SVG in `assets/icons/material/svg/`, it:**
  - renders the file with the existing Direct2D SVG code (`preview::svg`);
  - renders at **16, 20, 24, 32 and 48 px** square, the sizes for 100%, 125%, 150%, 200% and 300% scaling;
  - produces premultiplied BGRA, with the SVG's viewBox fitted to the square.
- **It writes two files:**
  - `assets/icons/material/icons.bin`: every icon at every size, back to back;
  - `src/window/icon_sets/material_table.rs`: for each FastPad icon name and size, its offset in `icons.bin`, plus the list of SVG files it was made from.
- **Both outputs are committed.** Building FastPad never runs the generator.
- **Size:**
  - about 150–250 KB of pixels for 12 icons;
  - accepted in exchange for no runtime rasterizing.

### 5.2 In the exe

- `icons.bin` is embedded with `include_bytes!`, so reading it involves no file access.
- **`src/window/icon_sets.rs`:**
  - `IconSet { Material, Minimal }`;
  - the lookup: `(set, note type or folder state, light theme) → TreeIcon`;
  - `TreeIcon` is either `Glyph(FileIcon)` (today's type) or `Image(MaterialIcon)`.
- **`file_icons::type_name`** (screen readers) doesn't change and doesn't depend on the set.
- **`file_icons::file_icon` and `FOLDER_ICON`** stay: they are the Minimal set, and they're also the fallback.

## 6. Drawing

- **The icon box:**
  - It stays `scale(16, dpi)` px square (`GLYPH_BOX`), in the same place in the row.
- **Picking a size:**
  - If the box's pixel size is one of the stored sizes, that one is used as is.
  - Otherwise, the next stored size up is scaled down into the box, with the `HALFTONE` stretch mode set on the DC. For example, 175% is 28 px, drawn from 32.
  - Above 48 px, 48 is scaled up.
- **The bitmaps:**
  - The first time an icon is drawn at a pixel size, one 32-bit top-down DIB section is created from the blob's pixels.
  - It's kept in the Notebook view's paint state, keyed by (icon, stored size).
- **When the cache is dropped:**
  - on a DPI change;
  - on a set change;
  - when the view is destroyed.
- **Blending:** `AlphaBlend` with `AC_SRC_ALPHA`, onto the row's background, which is already painted. Selected and hover backgrounds show through the transparent parts.
- **If creating a DIB fails:**
  - That row draws the Minimal glyph for the same type.
  - The failure is logged once per session.
  - The next paint tries again. Nothing is shown to the user.

## 7. Latency

- **Nothing new before first paint:**
  - The blob is compile-time data, and no bitmaps are created until a row using that icon is painted.
  - The setting is read with the rest of `fastpad.ini`.
- **No file I/O,** at startup or while drawing.
- **The cost:**
  - One `AlphaBlend` per row replaces one `DrawText`. After the first paint, the only extra work is up to 12 DIB sections per DPI.
  - A test of a 1,000-row tree paint records Material against Minimal, and Material must not be slower by more than 10%.
  - `tree_build_ms` doesn't change, because icons are chosen at paint time.

## 8. Screen readers

- **Row names don't change,** for example "budget.csv, CSV", because icons aren't announced.
- **The two palette rows** are ordinary palette rows. The current set's mark is spoken the way the current theme's is.

## 9. Testing

- **Pure:**
  - **Lookup:** every note extension, in any letter case, and both folder states map to the right icon in each set. This includes `toml_light` in Light and Catppuccin Latte, and `toml` in the other themes.
  - **Size choice:**
    - an exact match;
    - the next stored size up in between (for example 28 → 32);
    - 48 above 48 px.
  - **The setting:**
    - `file_icons` parsing, including letter case and an unknown value;
    - saving.
- **Data:**
  - The generated table lists exactly the SVG files in `assets/icons/material/svg/`, one to one. A regeneration that was forgotten fails this test.
  - Every entry's offset plus its length lies inside `icons.bin`, and the entries don't overlap.
  - At every size, every icon has visible pixels, and each pixel's colour channels are ≤ its alpha.
- **Window** (temporary profile, `--test-threads=1`):
  - Choosing "File icons: Minimal" saves `file_icons = minimal` and repaints the tree with the glyphs. Choosing Material brings the images back.
  - **Pixel checks:**
    - a Markdown row under Material shows the icon's colour in the icon box;
    - a Markdown row under Minimal shows the glyph colour;
    - an expanded folder under Material shows `folder-open`, not `folder`.
  - **High contrast:** the tree draws glyphs under Material.
  - The palette marks the current set.
- **Perf:** the 1,000-row paint comparison in §7.

## 10. Out of scope

- icon sets loaded from disk, or a format for them;
- Material's special folder icons (`folder-src`, `folder-docs`…) and file-name icons (`README.md`, `package.json`);
- icons for files that aren't notes;
- icons in tabs, Ctrl+P or Favorites;
- replacing the Segoe chevrons, pin or header buttons.
