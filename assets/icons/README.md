# The Notebook tree's icon sets

FastPad has three icon sets, and you pick one from the palette (**File icons: …**) or with
`file_icons=` in `fastpad.ini`:

| Set | Folder | What it is |
|---|---|---|
| Material (the default) | `material/svg/` | Material Icon Theme's icons, copied unchanged and drawn in their own colours (`material/SOURCE.md`). |
| Minimal | `minimal/svg/` | FastPad's own outlines, one colour each. |
| Solid | `solid/svg/` | FastPad's own filled shapes, one colour each. |

High contrast always draws Minimal in the system colour.

## Editing Minimal or Solid

Each set has the same nine files: `folder`, `folder-open`, `markdown` (Markdown), `braces` (JSON),
`settings` (YAML, TOML, INI and config), `table` (CSV), `code` (XML), `document` (text and any
other extension) and `log`.

Draw only the shape. FastPad applies the colour when it draws: blue for Markdown, yellow for
folders and JSON, and so on, taken from the theme.

- **Canvas:** `viewBox="0 0 24 24"`, with no `width` or `height` on the root. Keep about 2 units
  clear at each edge.
- **Colour:** black only (`#000`). Other colours are ignored.
- **Minimal:** use strokes, not fills. That means `fill="none" stroke="#000" stroke-width="1.5"
  stroke-linecap="round" stroke-linejoin="round"`. A 1.5 stroke is about 1 px at 16 px.
- **Solid:** use filled paths. Cut holes with `fill-rule="evenodd"`.
- **Don't use:** text, gradients, images, filters, masks or `<style>`. The renderer is Direct2D's
  SVG support, and these either don't draw or don't read at 16 px.
- **Check it at 16 px.** That's the size at 100% scaling. It's also drawn at 20, 24, 32 and 48 px.

After changing any SVG, in any set, run:

```powershell
tools\generate-file-icons.ps1
```

The script renders every set into its `icons.bin` and records the sources' hash in
`icons.source-hash`. Commit all three together: the SVG, `icons.bin` and `icons.source-hash`. The
tests fail if an SVG changed without a regeneration, or if an icon renders blank or fills its
whole square.

Adding a tenth icon is a code change: add it to `MaskIcon` in `src/window/icon_sets/masks.rs`,
together with the note types that use it.
