# Windows 11 faces and the Windows text-size setting: design

- Status: approved in conversation on 2026-10-01. Written spec awaiting review.
- Branch: `feat/fonts-text-size`, stacked on `feat/focus-ring` (PR #48), which is stacked on #47, #46, #45 and #44.
- Step 4 of the Windows design alignment. It has two parts that share one font-creation path: the faces (part A) and the text-size setting (part B).

## 1. Goal

- **Part A, faces:** on Windows 11, the chrome uses Segoe UI Variable for text and Segoe Fluent Icons for glyphs. On Windows 10 nothing changes.
- **Part B, text size:** the chrome's text follows Settings > Accessibility > Text size (100% to 225%), with the text-tied row and bar heights growing with it so nothing clips.
- **A build you can run** at the end. The development machine is Windows 10 (build 19045): part B is visible on it, part A is not. Part A is verified by unit tests of its pure choice function and by a runtime safety net (section 3.2), not by eye.

## 2. Decisions

| Question | Decision |
|---|---|
| When faces are chosen | Lazily, at the first font creation, and cached for the process. No registry read, no font enumeration and no DLL load at startup. |
| Windows 11 detection | The OS build from `RtlGetVersion` (in `ntdll`, always loaded). Build 22000 or later means Windows 11. |
| Safety net | On Windows 11 only, probe once that GDI really maps the requested face (`GetTextFaceW` on a font created with it). If it does not, use today's faces for the rest of the process. Windows 10 never probes. |
| Text face | "Segoe UI Variable Text" for every text style, and "Segoe UI Variable Display" for `Title`. Other builds keep "Segoe UI". |
| Icon face | "Segoe Fluent Icons" on Windows 11, "Segoe MDL2 Assets" otherwise. Segoe Fluent Icons keeps MDL2's codepoints, so no glyph or layout code changes. |
| Text size source | `HKCU\Software\Microsoft\Accessibility\TextScaleFactor`, a DWORD. Missing, unreadable or out of range means 100. The value is clamped to 100..=225. Read lazily with the font cache, never at startup. |
| What the factor scales | Text style font heights, and the text-tied heights listed in 3.4. Icon fonts, the activity bar, the tab strip height and every other metric stay on DPI scaling only. |
| When it refreshes | On `WM_SETTINGCHANGE` whose parameter is "TextScaleFactor" (a change to DPI keeps its own path). Open About and Settings dialogs keep their fonts until reopened. |
| Not in this piece | The editor font and the Markdown preview (user settings), the dialogs following a text-size change while open, and any new setting. |

## 3. Design

### 3.1 Face choice (`design/type_ramp.rs`, a new `design/faces.rs`)

- `faces::Faces { text: &'static str, display: &'static str, icons: &'static str }` and a pure `faces::choose(build: Option<u32>, probe: impl Fn(&str) -> bool) -> Faces`: build 22000 or later and `probe` accepting "Segoe UI Variable Text" gives the Windows 11 set; anything else gives `("Segoe UI", "Segoe UI", "Segoe MDL2 Assets")`.
- `faces::current() -> Faces` computes it once (a `OnceLock`) from `RtlGetVersion` and the real `GetTextFaceW` probe, and caches it.
- `UI_FACE` becomes `type_ramp::face(style)`, returning `display` for `Title` and `text` for the rest. `create` uses it. The literal "Segoe MDL2 Assets" at `side_panel.rs`, `titlebar.rs`, `preview_buttons.rs` and `soft_paint.rs` (`GLYPH_FONT`) becomes `faces::current().icons`. The underlined link fonts (`about.rs`, `settings_dialog.rs`) use the text face, and the two DirectWrite uses (`image_view/paint.rs`, `preview_host.rs`) use the text face name. The user-chosen preview font (`DEFAULT_PREVIEW_FONT`) is a setting and stays.
- Variable-font weights: the semibold styles keep `FW_SEMIBOLD`; GDI selects the named instance. The probe also confirms the face name only, not the weight, so a Windows 11 machine whose family lacks a weight shows GDI's nearest weight, as it does today for "Segoe UI".

### 3.2 The safety net

- The probe creates a font with the requested face at a nominal size, selects it into the screen DC, reads `GetTextFaceW`, and compares it with the requested name ignoring case. It runs at most once per process and only on Windows 11 builds. A mismatch (the face is missing or substituted) returns the Windows 10 set for the whole process.
- A failure of any API in the probe also returns the Windows 10 set. The worst case is today's look.

### 3.3 The text-size factor (`design/text_scale.rs`)

- `parse_factor(raw: Option<u32>) -> u32`: `None` gives 100 and any other value is clamped to 100..=225 (so 0 and 90 give 100 and 300 gives 225).
- `factor() -> u32` returns the cached percentage; the first call reads the registry value with `RegGetValueW` (the same call shape as `platform/theme.rs`'s dark-mode read). `refresh() -> bool` re-reads it, stores it and returns whether it changed. The cache is an `AtomicU32`.
- `scale_text(value: i32, dpi: u32) -> i32`: `scale(value, dpi)` multiplied by `factor()` percent, rounded half up. At 100 it equals `scale`.
- Text style fonts: `type_ramp::create` uses `scale_text(spec.px, dpi)`. The title strip, side panel, About and Settings font creation already go through `create`; the underlined link fonts and the 13px literals in `about.rs` and `settings_dialog.rs` use `scale_text` too.

### 3.4 Text-tied heights

These constants grow with the factor through `scale_text`, in place of `scale`, and their tests are parameterised on the factor:

- `metrics::SIDEBAR_ROW` (26) and `metrics::PANEL_HEADER` (38), and the Search view's `ROW_AT_96_DPI` (46) and `ROW_LINE_AT_96_DPI` (19);
- `status::STATUS_HEIGHT_AT_96_DPI` (22);
- the command palette's `ROW_HEIGHT_AT_96_DPI` (26).

At 100% every value is unchanged (the existing tests are the guard). The implementation plan lists each call site it changes; a height that is not on this list stays on `scale`.

### 3.5 Refresh (`main_window/wndproc.rs`)

- `WM_SETTINGCHANGE` already calls `refresh_theme`. It additionally checks whether the changed-setting parameter is the string "TextScaleFactor"; if so it calls `text_scale::refresh()`, and when that returns true it drops `title_fonts`, invalidates the sidebar font cache (the cache key becomes the DPI plus the factor), and calls the same relayout the DPI change calls (`layout_editor_and_find_bar`) followed by `InvalidateRect`.
- `ui_fonts` and `TitleFonts` compare `(dpi, factor)` instead of `dpi`.

## 4. Tests

- `faces::choose`: Windows 10 build gives the Segoe UI set; build 22000 with an accepting probe gives the Windows 11 set; build 22000 with a rejecting probe gives the Segoe UI set; no build (the call failed) gives the Segoe UI set.
- `text_scale::parse_factor`: `None`, 0, 90, 100, 150, 225 and 300.
- `scale_text`: at factor 100 equals `scale` for several values and DPIs; at 150 and 225 it is `scale` times the factor, rounded half up; DPI 0 still counts as 96.
- Font height tests (`type_ramp`, `side_panel`): the lfHeight of the body font at DPI 96 is 13, 20 and 29 at factors 100, 150 and 225, and the icon fonts do not change.
- Row-height tests: Search rows, sidebar rows, panel header and the palette row at factors 100 and 225; the Search result row still holds two text lines at 96, 120, 144 and 192 DPI at factor 225.
- The existing tests that pin 26, 38, 46, 19, 22 and the palette row are the 100% guard and must pass unchanged.
- A test that `WM_SETTINGCHANGE`'s text-size branch is skipped for other parameters, where the existing wndproc tests allow it.

## 5. Risks and checks

- **Clipping at 225%.** Any text-tied height missing from 3.4 clips. The build is checked at 150% and 225% on Windows 10 for every sidebar view, the status bar, the Search view and the command palette. A height found clipping is added to the list.
- **Windows 11 cannot be checked here.** The pure choice function and the probe's fallback are tested, but the Windows 11 look is not seen by the author. The safety net means the failure mode is today's fonts, not broken text.
- **Startup.** The first font creation computes the face set: one `RtlGetVersion` call on Windows 10, plus one probe on Windows 11. The registry read happens at the same lazy point. Nothing is added to the paths before the first editable frame; `benchmarks/README.md` gates are re-run as a confirmation.
- **Font-cache invalidation.** A text-size change while the window is open must rebuild every cached font and relayout; stale fonts with new heights (or the reverse) would clip. The refresh path in 3.5 is the single place that does it.
- **Verification:** Clippy, the targeted tests for the changed modules, the full suite once (see the note on the environment-dependent always-on-top test in PR #48), then the build. Before the build, `fastpad.ini` is backed up and the build is checked at three text sizes.

## 6. After this piece

Color and accent (consuming `accent` for selection, the five known low-contrast theme pairs, high-contrast focus visibility), then the dialogs and copy pass.
