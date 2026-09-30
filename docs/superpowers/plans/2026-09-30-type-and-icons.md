# Type and Icons (first visible step) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Amended after the preview builds:** sidebar icons shipped at 12px (`metrics::SIDEBAR_ICON`) and headers in sentence case; only the activity bar uses `ICON = 16`. See the spec's Amendments section. Task 2 below is the original 16px plan.

**Goal:** Move FastPad's chrome text to 13px and its sidebar icons to 16px, fix the one layout that breaks (Search results at 144%), and produce a runnable build.

**Architecture:** The four body-text styles change in the one table in `design/type_ramp.rs`; `DialogBody` folds into `Body`. A new `metrics::ICON = 16` sizes the sidebar and activity-bar glyph fonts. The Search result row grows so two 13px lines fit at every DPI. Tests that pin the old sizes change with them.

**Tech Stack:** Rust 2024, `windows-sys` (GDI), in-crate unit tests with `cargo test --lib`.

**Spec:** `docs/superpowers/specs/2026-09-30-type-and-icons-design.md`

## Global Constraints

- **Branch:** work on `feat/type-and-icons` (stacked on `feat/ui-quality-improvements`, PR #44). Never switch branches in this checkout, and never touch `main` or `feat/ui-quality-improvements`.
- **Sizes (verbatim from the spec):** `Body`, `BodyItalic`, `BodyBold` and `PanelHeader` are all 13px; `Heading` 14 and `Title` 18 stay; `DialogBody` is removed and its two call sites use `Body`; `metrics::ICON = 16` for the sidebar `glyph` and activity-bar `bar_glyph` fonts; Search results `ROW_LINE_AT_96_DPI` 18 to 19 and `ROW_AT_96_DPI` 42 to 46.
- **Not changed:** status bar (22), inline rename box, every other height, the editor, Markdown preview, image view, tooltips, native menus, the titlebar caption glyph (10px), the dialog glyph (11px), the preview-button glyph (17px), the underlined link fonts (already 13px), colors, radii.
- **Startup:** only font sizes change. The same fonts are created at the same times; nothing new on the startup path or in paint.
- **Commits:** no attribution or co-author lines in commit messages.
- **Formatting and checks:** run `cargo fmt` before every commit. Compile check: `cargo clippy --all-targets -- -D warnings`.
- **Testing practice:** run only targeted tests until Task 4. Window unit tests are not parallel-safe: add `-- --test-threads=1` to any `window::` filter. `tests/windows/` integration tests run only in Task 4.

## Review Focus

Failure modes the spec implies that no feature test would otherwise exercise.

1. **Search rows at every DPI.** Two 13px lines must fit the row at 96, 120, 144 and 192 DPI, for both the normal and bold font. (Task 3: the updated row test.)
2. **Fonts at unusual DPIs.** The body font at DPI 0, 96, 144 and 192 is 13, 13, 20 and 26px. (Task 1.)
3. **Icon fonts follow the icon size at every DPI.** Both glyph fonts scale from `ICON`, and the sidebar text font is 13px at each DPI. (Task 2.)
4. **Dialogs do not change.** `Body` must equal the old `DialogBody` (13 regular), so Settings and About stay identical. (Task 1: the style-table test.)
5. **A hard-coded row height.** Other tests may assume the old 42px result row. (Task 3: run the Search tests and fix any literal 42.)

---

### Task 1: The type ramp at 13px

**Files:**
- Modify: `src/window/design/type_ramp.rs`, `src/window/about.rs:393`, `src/window/settings_dialog.rs:314`
- Test: `src/window/design/type_ramp.rs` (its tests module)

**Interfaces:**
- Consumes: `TextStyle`, `Spec`, `create` from the design tokens step.
- Produces: `TextStyle` without `DialogBody`; `Body`, `BodyItalic`, `BodyBold`, `PanelHeader` specs at 13px.

- [ ] **Step 1: Update the tests first**

In `src/window/design/type_ramp.rs`, replace the body of `every_style_keeps_the_size_and_weight_its_call_site_used` with:

```rust
    #[test]
    fn every_style_has_the_size_and_weight_the_spec_gives_it() {
        // Break caught: a size or weight drifting from the spec. The four body-text styles are the
        // 13px trial; reverting it means setting Body, BodyItalic and BodyBold back to 12 and
        // PanelHeader back to 11 (semibold) in `spec` above.
        let spec = |style: TextStyle| {
            let s = style.spec();
            (s.px, s.weight, s.italic)
        };
        assert_eq!(spec(TextStyle::Body), (13, FW_NORMAL as i32, false)); // strip, tabs, sidebar, dialogs
        assert_eq!(spec(TextStyle::BodyItalic), (13, FW_NORMAL as i32, true)); // preview tab, notices
        assert_eq!(spec(TextStyle::BodyBold), (13, FW_BOLD as i32, false)); // search match
        assert_eq!(
            spec(TextStyle::PanelHeader),
            (13, FW_SEMIBOLD as i32, false)
        );
        assert_eq!(spec(TextStyle::Heading), (14, FW_SEMIBOLD as i32, false));
        assert_eq!(spec(TextStyle::Title), (18, FW_SEMIBOLD as i32, false));
    }
```

In `fonts_are_created_at_unusual_dpis_and_scale_with_them`, change the expected list to `[(0_u32, 13), (96, 13), (144, 20), (192, 26)]`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --lib window::design::type_ramp`
Expected: FAIL, `Body` is (12, ...) not (13, ...), and the font heights are 12, 12, 18, 24.

- [ ] **Step 3: Change the ramp**

In `src/window/design/type_ramp.rs`:

- Delete the `DialogBody` variant and its doc comment (`/// Text in the Settings and About dialogs.`) from the enum, and delete `Self::DialogBody => (13, FW_NORMAL, false),` from `spec`.
- Change the four arms:

```rust
            Self::Body => (13, FW_NORMAL, false),
            Self::BodyItalic => (13, FW_NORMAL, true),
            Self::BodyBold => (13, FW_BOLD, false),
            Self::PanelHeader => (13, FW_SEMIBOLD, false),
```

- Change the `Body` doc comment to `/// Title strip, tabs, sidebar row text and the Settings and About dialogs' text.`
- Change the `UI_FACE` doc comment to `/// The chrome's text face. Segoe UI Variable with a fallback replaces it in a later step.`

In `src/window/about.rs:393` and `src/window/settings_dialog.rs:314`, change `type_ramp::create(TextStyle::DialogBody, dpi)` to `type_ramp::create(TextStyle::Body, dpi)`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `cargo test --lib window::design::type_ramp`
Expected: 2 passed.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS. (A leftover reference to `DialogBody` is a compile error; fix it by using `Body`.)

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add -A src
git commit -m "feat: 13px body text through the type ramp; dialogs use Body"
```

---

### Task 2: 16px sidebar icons

**Files:**
- Modify: `src/window/design/metrics.rs`, `src/window/side_panel.rs:62-74` and `:98-99`, `src/window/notebook_view/tests.rs` (font literals)
- Test: `src/window/design/metrics.rs` (tests module), `src/window/side_panel.rs` (its tests module)

**Interfaces:**
- Consumes: `metrics::scale`, `UiFonts::create(dpi: u32) -> UiFonts` (private, used by tests in the same module), `type_ramp::create`.
- Produces: `crate::window::design::metrics::ICON: i32 = 16` (`pub(crate) const`).

- [ ] **Step 1: Write the failing tests**

In `src/window/design/metrics.rs`, add `ICON` to the imports in the tests module (`use super::{CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING, GRID, ICON, PANEL_HEADER, SIDEBAR_ROW};`), add `assert_eq!(ICON, 16); // the sidebar and activity bar icon size` to `shared_metrics_keep_the_values_the_old_constants_had`, and add `("ICON", ICON),` to the `sizes` array in the off-grid test (16 is on the grid, so the expected list stays `["PANEL_HEADER", "SIDEBAR_ROW"]`).

Add this test to the `tests` module in `src/window/side_panel.rs`:

```rust
    #[test]
    fn the_sidebar_and_activity_bar_glyph_fonts_are_icon_sized_at_every_dpi() {
        // Break caught: row and header-button icons staying at 12 px after the icon size moved
        // to 16, or the sidebar text not following the 13 px body style.
        use crate::window::design::metrics::{ICON, scale};
        use windows_sys::Win32::Graphics::Gdi::{GetObjectW, LOGFONTW};
        let height = |font| {
            let mut log: LOGFONTW = unsafe { std::mem::zeroed() };
            let written = unsafe {
                GetObjectW(
                    font,
                    std::mem::size_of::<LOGFONTW>() as i32,
                    (&mut log as *mut LOGFONTW).cast(),
                )
            };
            assert!(written > 0);
            log.lfHeight
        };
        for dpi in [96, 120, 144, 192] {
            let fonts = UiFonts::create(dpi);
            assert_eq!(height(fonts.glyph), -scale(ICON, dpi), "glyph at {dpi}");
            assert_eq!(
                height(fonts.bar_glyph),
                -scale(ICON, dpi),
                "bar glyph at {dpi}"
            );
            assert_eq!(height(fonts.text), -scale(13, dpi), "text at {dpi}");
            fonts.delete();
        }
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --lib window::design::metrics`
Expected: compile error, `ICON` does not exist.

- [ ] **Step 3: Add the constant and use it**

In `src/window/design/metrics.rs`, after `SIDEBAR_ROW`:

```rust
/// The size of the sidebar's and the activity bar's icon glyphs.
pub(crate) const ICON: i32 = 16;
```

In `src/window/side_panel.rs`, add `ICON` to the existing `design::metrics` import, then replace the two glyph lines in `UiFonts::create`:

```rust
            glyph: create_ui_font(scale(ICON, dpi), "Segoe MDL2 Assets", normal, false),
            bar_glyph: create_ui_font(scale(ICON, dpi), "Segoe MDL2 Assets", normal, false),
```

Update the field doc comments (lines 62-74) to:

```rust
    /// Row and body text: Segoe UI, 13 px at 96 DPI.
    pub(crate) text: HFONT,
    /// The match in a Search result's snippet: Segoe UI bold, 13 px. Made on the first paint of a
    /// snippet (`Sidebar::text_bold`), not with the others, so it adds nothing before first paint.
    pub(crate) text_bold: HFONT,
    /// Header titles in small capitals: Segoe UI semibold, 13 px.
    pub(crate) bold: HFONT,
    /// Notices inside the list: Segoe UI italic, 13 px.
    pub(crate) italic: HFONT,
    /// Row and header-button icons: Segoe MDL2 Assets, `ICON` px.
    pub(crate) glyph: HFONT,
    /// The activity bar's icons: Segoe MDL2 Assets, `ICON` px.
    pub(crate) bar_glyph: HFONT,
```

- [ ] **Step 4: Keep the tests' own fonts faithful**

The notebook view tests build their own `UiFonts` from literals. They pass with either size (icon comparisons clip to fixed boxes) but should mirror the real fonts:

```bash
cd /d/Projects/FastPad
sed -i 's/create_ui_font(12, "Segoe UI"/create_ui_font(13, "Segoe UI"/g; s/create_ui_font(11, "Segoe UI", FW_SEMIBOLD/create_ui_font(13, "Segoe UI", FW_SEMIBOLD/g; s/create_ui_font(12, "Segoe MDL2 Assets"/create_ui_font(16, "Segoe MDL2 Assets"/g' src/window/notebook_view/tests.rs
grep -n 'create_ui_font(' src/window/notebook_view/tests.rs
```

Expected from the `grep`: every `"Segoe UI"` font is 13 (the bold one is `FW_SEMIBOLD`) and every `"Segoe MDL2 Assets"` font is 16.

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib window::design::metrics && cargo test --lib window::side_panel -- --test-threads=1 && cargo test --lib window::notebook_view -- --test-threads=1`
Expected: PASS.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add -A src
git commit -m "feat: 16px sidebar and activity bar icons through metrics::ICON"
```

---

### Task 3: Search result rows that hold two 13px lines

**Files:**
- Modify: `src/window/search_view.rs:51-54`, `src/window/search_view/tests.rs:83`, and any test with a hard-coded 42px result row (known: `src/window/main_window/tests/search_view.rs:763`)
- Test: `src/window/search_view/tests.rs` (`a_result_row_holds_two_lines_of_the_sidebar_text_at_every_dpi`)

**Interfaces:**
- Consumes: `metrics::scale`; the constants `ROW_AT_96_DPI`, `ROW_LINE_AT_96_DPI`, `ROW_INSET_AT_96_DPI` (private to `search_view`, re-imported by its tests).
- Produces: `ROW_LINE_AT_96_DPI = 19`, `ROW_AT_96_DPI = 46`.

- [ ] **Step 1: Write the failing test change**

In `src/window/search_view/tests.rs`, in `a_result_row_holds_two_lines_of_the_sidebar_text_at_every_dpi`, change the font size from 12 to 13:

```rust
            let font = create_ui_font(scale(13, dpi), "Segoe UI", weight as i32, false);
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test --lib window::search_view -- --test-threads=1`
Expected: FAIL in that test at DPI 144 with a message like `144: 28`, because a 13px line is 28px tall and `scale(18, 144)` is 27.

- [ ] **Step 3: Grow the row**

In `src/window/search_view.rs`, replace lines 51-54 with:

```rust
/// A result: two line slots with a little room above and below. The 13 px Segoe UI line is 17 px
/// tall at 96 DPI and 28 px at 144 DPI; the old one-line row was 26 px.
const ROW_AT_96_DPI: i32 = 46;
const ROW_LINE_AT_96_DPI: i32 = 19;
```

- [ ] **Step 4: Run the Search tests and fix any hard-coded row height**

Run: `cargo test --lib window::search_view -- --test-threads=1`
Expected: PASS (the row test now holds at 96, 120, 144 and 192 DPI).

Run: `cargo test --lib window::main_window::tests::search_view -- --test-threads=1`
Expected: at least one FAIL that compares against a hard-coded 42px row. Look for `scale(42, dpi)` at `src/window/main_window/tests/search_view.rs:763` (and find others with `grep -rn "scale(42" src`). Change each such literal to the named constant or to 46. Re-run until this command passes.

- [ ] **Step 5: Compile check and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS.

```bash
cargo fmt
git add -A src
git commit -m "fix: Search result rows hold two 13px lines at every DPI"
```

---

### Task 4: Verification and the preview build

**Files:**
- No source changes. Produces `dist/` output (git-ignored) and a report.

**Interfaces:**
- Consumes: everything above.
- Produces: a portable folder `dist/step2-preview/` with `FastPad.exe` and the two Scintilla DLLs, and a verification report.

- [ ] **Step 1: Format and compile check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: no output from `fmt`, and Clippy PASS.

- [ ] **Step 2: Full test run, once**

Run: `cargo test --locked -- --test-threads=1`
Expected: all pass. It is long; run it in the background and wait for it. On any failure, find out whether it comes from this branch's changes (compare against `feat/ui-quality-improvements` using a temporary git worktree in a scratch directory, never by switching branches here) and report precisely. Do not fix unrelated pre-existing failures.

- [ ] **Step 3: Startup benchmark, as a confirmation**

Only font sizes changed, so this should show nothing. This machine is not the reference machine: do not use `-EnforceReference`. From the repository root, run `./tools/benchmark.ps1 -Runs 30 -Warmup 5 -Output <scratch dir>/candidate.jsonl`. For a baseline, create a temporary git worktree at `782efe1` in a scratch directory (copy `native/out` DLLs into the same relative place), run the same command there into `baseline.jsonl`, then `cargo run --release --bin fastpad-bench -- compare baseline.jsonl candidate.jsonl`. Remove the temporary worktree afterwards (`git worktree remove --force`). Write benchmark output only under the scratch directory and never commit it.

- [ ] **Step 4: Build the preview**

Run, from the repository root in PowerShell: `./tools/package.ps1`
Expected: it rebuilds the pinned native DLLs, builds the release binary with the `release-package` feature, and writes `dist/FastPad-<version>-windows-x64.zip` plus a `.sha256`. Extract the zip to `dist/step2-preview/`:

```powershell
Remove-Item -Recurse -Force dist/step2-preview -ErrorAction SilentlyContinue
Expand-Archive -Path (Get-ChildItem dist/FastPad-*-windows-x64.zip | Select-Object -First 1).FullName -DestinationPath dist/step2-preview
Get-ChildItem dist/step2-preview | Select-Object Name, Length
```

Expected: `FastPad.exe`, `Scintilla.dll` and `Lexilla.dll` are listed in the folder (beside the README and license files). Do not launch the app: running it rewrites the user's `fastpad.ini`, and the user will do the visual check.

- [ ] **Step 5: Report**

Write the report with: the fmt and Clippy results, the full-suite counts, the benchmark compare output, the folder path and file list of the build, and its SHA256. Do not commit anything in this task.
