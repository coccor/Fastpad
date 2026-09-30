# Type and icons, first visible step: design

- Status: approved in conversation on 2026-09-30 (scope only). Written spec awaiting review.
- Branch: `feat/type-and-icons`, stacked on `feat/ui-quality-improvements` (PR #44, the design tokens).
- Step 2 of the Windows design alignment, taken in small pieces. This piece is the first one you can see.

## 1. Goal

Make the chrome's text and sidebar icons read more like Windows 11 apps, and give the 13px body-text trial a one-place revert.

- **13px chrome text** in place of 12px, for the title strip and tabs, status bar, menu band, find bar, name box, command palette, and the sidebar's rows and headers.
- **16px sidebar icons** in place of 12px, for the tree, Open Editors, favorites and search rows and buttons.
- **A build you can run** at the end, so the change is judged by eye.

## 2. Decisions

| Question | Decision |
|---|---|
| Text sizes | `Body`, `BodyItalic`, `BodyBold` and `PanelHeader` all become 13px. The preview-tab label, the sidebar notice row and the Search match then stay in step with the body text beside them. |
| Dialog text | `DialogBody` is removed. The Settings and About dialogs use `Body`, which is 13px regular, so they do not change. The dialogs' underlined link fonts already use 13px and stay as they are. |
| Reverting the trial | The four styles above are four rows of the one table in `type_ramp.rs`. Setting them back to 12, 12, 12 and 11 reverts the text. The icon size and the search row change are separate and stay. |
| Icon size | A new `metrics::ICON = 16`. The sidebar glyph font and the activity bar glyph font both use it. Icons stay Segoe MDL2 Assets. |
| Tight 16px boxes | The tree chevron, the recent-folder icon and the section chevron boxes are exactly 16px, so the 16px glyph has no slack. `GLYPH_BOX` is also the file-icon size, so it does not change. The first build decides by eye whether any of the three looks clipped. |
| Search results | The result row grows so two 13px lines fit at every DPI: `ROW_LINE_AT_96_DPI` 18 to 19 and `ROW_AT_96_DPI` 42 to 46. Without this, the 13px line is 28px tall at 144% scaling in a 27px slot and clips, and at 144% two 29px lines plus two 5px insets need 68px, which a 44px row (66px) does not give; 46px gives 69px. |
| Not changed | Row, header and bar heights other than Search results. The status bar stays 22px (a 17px line fits with 2 to 3px spare) and the inline rename box stays as it is (its text height is clamped to the frame). Step 3 normalises heights to the 4px grid. |
| Not in this piece | Segoe UI Variable, Segoe Fluent Icons, and following the Windows text-size setting. They are the next piece. |
| Deliverable | A portable folder with `FastPad.exe` and the two Scintilla DLLs, built in release mode, for you to run. |

## 3. Changes

### 3.1 Type ramp (`design/type_ramp.rs`)

| Style | Before | After |
|---|---|---|
| `Body` | 12 regular | 13 regular |
| `BodyItalic` | 12 italic | 13 italic |
| `BodyBold` | 12 bold | 13 bold |
| `PanelHeader` | 11 semibold | 13 semibold |
| `DialogBody` | 13 regular | removed; its two call sites use `Body` |
| `Heading`, `Title` | 14 and 18 semibold | unchanged |

The tests that pin the ramp change with it: the style table test, and the font-height test, whose expected heights at DPI 0, 96, 144 and 192 become 13, 13, 20 and 26.

### 3.2 Icon size (`design/metrics.rs`, `side_panel.rs`)

- Add `pub(crate) const ICON: i32 = 16;`.
- In `UiFonts::create`, the `glyph` font uses `scale(ICON, dpi)` and `bar_glyph` uses the same constant. The doc comments above the fields (12 and 16 px) are updated.
- No layout measures these glyphs: every site draws centred in a fixed box, so only the drawing size changes. All boxes hold a 16px glyph, three of them with no slack, as noted above.

### 3.3 Search results (`search_view.rs`)

- `ROW_LINE_AT_96_DPI` 18 to 19 and `ROW_AT_96_DPI` 42 to 46. The comment that says the 12px line is 16px tall is corrected.

### 3.4 Tests that change

- `design/type_ramp.rs`: the two tests above.
- `search_view/tests.rs` (`a_result_row_holds_two_lines_of_the_sidebar_text_at_every_dpi`): it builds fonts at 12px; it builds them at 13px instead, and keeps asserting the row holds two lines at 96, 120, 144 and 192 DPI.
- `notebook_view/tests.rs` (tests that build their own fonts with 12, 11, 12 and 12px literals): updated to 13, 13, 13 and 16px so they match the real fonts. They pass either way, because the icon comparisons clip to fixed boxes. The find bar, name box, command palette and inline rename tests that pass a text height of 16 by hand are left alone: they never measure a font.

## 4. Non-goals

- No change to the editor font, the Markdown preview, the image view, tooltips or native menus.
- No change to colors, radii or any metric beyond the Search row.
- No new settings.

## 5. Risks and checks

- **Crowding at 96 DPI.** Measured on this machine, 13px text is 17px tall (12px is 15) and 9 to 12% wider. Everything fits at 96 DPI, but the tab label area (76px at its minimum width) shows about 10% fewer characters, and the status bar and inline rename box have only 2 to 3px of slack. The first build is for looking at exactly these places.
- **Other DPIs.** The Search row is the only layout found to fail (144%). The row test covers 96, 120, 144 and 192.
- **Startup.** Only font sizes change: the same fonts are created at the same times. The benchmark gate in `benchmarks/README.md` is re-run as a confirmation.
- **Verification:** Clippy, the targeted tests for the changed modules, the full suite once, then the build. Before the build, the settings file `fastpad.ini` is backed up, because the app rewrites it.

## 6. Next pieces (out of scope here)

1. Segoe UI Variable for text and Segoe Fluent Icons on Windows 11, keeping today's fonts on Windows 10, and following the Windows text-size setting.
2. Geometry and states: rounded tabs and row backplates, the 8px command palette, one focus ring, heights on the 4px grid.
3. Color, dialogs and copy.
