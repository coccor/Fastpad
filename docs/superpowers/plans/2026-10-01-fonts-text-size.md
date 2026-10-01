# Windows 11 Faces and Text Size Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Use Segoe UI Variable and Segoe Fluent Icons on Windows 11 (with a runtime safety net), and make the chrome's text and its text-tied row and bar heights follow the Windows text-size setting.

**Architecture:** Two small token modules, `design/faces.rs` (pure face choice plus a cached, lazily computed current set) and `design/text_scale.rs` (the 100-225 factor, `scale_text`). `type_ramp::create` and the icon-font sites use them; the font caches key on `(dpi, factor)`; `WM_SETTINGCHANGE` refreshes the factor and re-runs the metrics refresh. Row and bar heights go through named helpers in `design/metrics.rs`.

**Tech Stack:** Rust 2024, windows-sys (GDI, Registry, LibraryLoader), no new crate features.

**Spec:** `docs/superpowers/specs/2026-10-01-fonts-text-size-design.md`

## Global Constraints

- Nothing new before the first editable frame: faces and factor are computed lazily at first font creation and cached; no registry read, font enumeration or DLL load at startup. `ntdll` is always loaded, so `RtlGetVersion` is reached with `GetModuleHandleW` + `GetProcAddress`, with a locally defined `repr(C)` version struct (no new windows-sys features).
- At text factor 100 and on Windows 10, behaviour is byte-for-byte today's: every existing test that pins a size, height or face name must pass unchanged.
- Icon fonts, the activity bar, the tab strip height, the editor and preview fonts stay on DPI scaling only; the user's preview font setting is untouched.
- The factor is clamped to 100..=225 and defaults to 100 when the value is missing or unreadable.
- High contrast, DPI scaling (`design::metrics::scale`, DPI 0 counts as 96) and every other metric are untouched.
- No attribution lines in commit messages; never skip hooks.
- Testing practice: compile with `cargo clippy --all-targets -- -D warnings`, run only targeted tests with `-- --test-threads=1`; the controller runs the full suite once at the end. A known environment-dependent failure, `always_on_top_pins_the_window_and_saves_only_its_own_line`, may fail in the full run on this desktop session and is not caused by this work.
- Tests that touch the process-wide cached factor must set it through the test-only setter introduced in Task 1 and restore it, and must run with `--test-threads=1` like the other window tests.

## Review Focus

- The factor at the edges: missing, 0, 90, 100, 225, 300, and a registry read that fails.
- Rounding at 150% and 225% for each text-tied height, at DPIs 96, 120, 144 and 192.
- The Search result row holding two text lines at factor 225 (the earlier 144% DPI bug class).
- A stale font or height after the setting changes while the window is open (cache keys, relayout).
- The Windows 11 probe failing or the face being substituted: the whole process must fall back to today's faces.
- Windows 10 never runs the probe and never calls anything new on the startup path.

---

### Task 1: The text-size factor

**Files:**
- Create: `src/window/design/text_scale.rs`
- Modify: `src/window/design/mod.rs` (add `pub(crate) mod text_scale;`)

**Interfaces:**
- Consumes: `design::metrics::scale`, `crate::platform::wide_null`.
- Produces:
  - `text_scale::parse_factor(raw: Option<u32>) -> u32`
  - `text_scale::factor() -> u32` (percent, 100..=225, cached; first call reads the registry)
  - `text_scale::refresh() -> bool` (re-reads, stores, returns whether it changed)
  - `text_scale::scale_text(value: i32, dpi: u32) -> i32`
  - `text_scale::set_factor_for_test(percent: u32)` (`#[cfg(test)]`)

- [ ] **Step 1: Write the failing tests**

Create `src/window/design/text_scale.rs` containing only this test module (and `use super::*;`), and register the module in `design/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::design::metrics::scale;

    #[test]
    fn the_factor_defaults_to_100_and_clamps_to_the_supported_range() {
        // Break caught: a missing value scaling text to 0, or a hostile value blowing up the UI.
        assert_eq!(parse_factor(None), 100);
        assert_eq!(parse_factor(Some(0)), 100);
        assert_eq!(parse_factor(Some(90)), 100);
        assert_eq!(parse_factor(Some(100)), 100);
        assert_eq!(parse_factor(Some(150)), 150);
        assert_eq!(parse_factor(Some(225)), 225);
        assert_eq!(parse_factor(Some(300)), 225);
        assert_eq!(parse_factor(Some(u32::MAX)), 225);
    }

    #[test]
    fn scale_text_equals_scale_at_100_percent() {
        set_factor_for_test(100);
        for dpi in [0, 96, 120, 144, 192] {
            for value in [1, 13, 22, 26, 38, 46] {
                assert_eq!(scale_text(value, dpi), scale(value, dpi), "{value} at {dpi}");
            }
        }
    }

    #[test]
    fn scale_text_multiplies_the_dpi_scaled_value_by_the_factor_rounding_half_up() {
        set_factor_for_test(150);
        assert_eq!(scale_text(13, 96), 20); // 13 * 1.5 = 19.5
        assert_eq!(scale_text(26, 96), 39);
        set_factor_for_test(225);
        assert_eq!(scale_text(13, 96), 29); // 13 * 2.25 = 29.25
        assert_eq!(scale_text(26, 96), 59); // 58.5
        assert_eq!(scale_text(13, 192), 59); // 26 * 2.25 = 58.5
        set_factor_for_test(100);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib window::design::text_scale -- --test-threads=1`
Expected: compile errors (`parse_factor`, `scale_text`, `set_factor_for_test` not found).

- [ ] **Step 3: Implement**

Above the tests:

```rust
//! Windows' text-size setting (Settings > Accessibility > Text size), 100 to 225 percent, and the
//! helper that applies it. The value is read lazily the first time something asks for it, never
//! at startup, and cached; `refresh` re-reads it when Windows says settings changed.

use std::sync::atomic::{AtomicU32, Ordering};

use super::metrics::scale;

const MIN: u32 = 100;
const MAX: u32 = 225;
/// `0` means "not read yet".
static FACTOR: AtomicU32 = AtomicU32::new(0);

/// The percentage for a raw registry value: missing is 100, anything else is clamped to 100..=225.
pub(crate) fn parse_factor(raw: Option<u32>) -> u32 {
    raw.map_or(MIN, |value| value.clamp(MIN, MAX))
}

fn read_registry() -> u32 {
    use std::ffi::c_void;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let subkey = crate::platform::wide_null(r"Software\Microsoft\Accessibility");
    let value_name = crate::platform::wide_null("TextScaleFactor");
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut data).cast::<c_void>(),
            &mut size,
        )
    };
    parse_factor((status == 0).then_some(data))
}

/// The text-size percentage, 100 to 225. The first call reads the registry.
pub(crate) fn factor() -> u32 {
    match FACTOR.load(Ordering::Relaxed) {
        0 => {
            let value = read_registry();
            FACTOR.store(value, Ordering::Relaxed);
            value
        }
        value => value,
    }
}

/// Re-reads the setting and returns whether it changed.
pub(crate) fn refresh() -> bool {
    let before = factor();
    let after = read_registry();
    FACTOR.store(after, Ordering::Relaxed);
    before != after
}

/// `value` (pixels at 96 DPI) scaled to `dpi`, then by the text-size factor, rounded half up.
/// At 100 percent it equals `scale`.
pub(crate) fn scale_text(value: i32, dpi: u32) -> i32 {
    let scaled = i64::from(scale(value, dpi));
    ((scaled * i64::from(factor()) + 50) / 100) as i32
}

/// Sets the cached factor, for tests.
#[cfg(test)]
pub(crate) fn set_factor_for_test(percent: u32) {
    FACTOR.store(percent.clamp(MIN, MAX), Ordering::Relaxed);
}
```

`scale_text(26, 96)` at 150 is `(26*150+50)/100 = 39`; at 225 `(26*225+50)/100 = 59` (58.5 rounds up); `(13*225+50)/100 = 29`; `scale(13,192)=26`, `(26*225+50)/100 = 59`. If the real `scale` rounds differently from these hand values at some DPI, recompute the expected value from the formula and keep the test's intent.

Remove the `allow` need: the three functions and the setter are unused until later tasks, so put `#[allow(dead_code, reason = "used by the font and height sites in the next tasks")]` on `factor`, `refresh` and `scale_text`, and remove them in Task 4/5 as each is used.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --lib window::design::text_scale -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add src/window/design
git commit -m "feat: read the Windows text-size factor and add scale_text"
```

---

### Task 2: The face choice

**Files:**
- Create: `src/window/design/faces.rs`
- Modify: `src/window/design/mod.rs` (add `pub(crate) mod faces;`)

**Interfaces:**
- Consumes: `crate::window::titlebar::create_ui_font`, `crate::platform::wide_null`.
- Produces:
  - `faces::Faces { text: &'static str, display: &'static str, icons: &'static str }` (`Copy`)
  - `faces::choose(build: Option<u32>, probe: impl Fn(&str) -> bool) -> Faces`
  - `faces::current() -> Faces` (cached in a `OnceLock`)

- [ ] **Step 1: Write the failing tests**

Create `src/window/design/faces.rs` with only this test module (and `use super::*;`), register it in `design/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: Faces = Faces {
        text: "Segoe UI",
        display: "Segoe UI",
        icons: "Segoe MDL2 Assets",
    };
    const WINDOWS_11: Faces = Faces {
        text: "Segoe UI Variable Text",
        display: "Segoe UI Variable Display",
        icons: "Segoe Fluent Icons",
    };

    #[test]
    fn windows_10_keeps_the_legacy_faces_and_never_probes() {
        // Break caught: Windows 10 running the probe (startup cost) or getting Windows 11 faces.
        let probe = |_: &str| -> bool { panic!("Windows 10 must not probe") };
        assert_eq!(choose(Some(19045), probe), LEGACY);
        assert_eq!(choose(Some(21999), probe), LEGACY);
    }

    #[test]
    fn windows_11_uses_the_variable_faces_when_the_probe_accepts() {
        assert_eq!(choose(Some(22000), |_| true), WINDOWS_11);
        assert_eq!(choose(Some(26100), |_| true), WINDOWS_11);
    }

    #[test]
    fn a_rejected_probe_falls_back_to_the_legacy_faces() {
        // Break caught: a Windows 11 machine without the face rendering a substituted font.
        assert_eq!(choose(Some(22621), |_| false), LEGACY);
    }

    #[test]
    fn an_unknown_build_falls_back_to_the_legacy_faces() {
        let probe = |_: &str| -> bool { panic!("an unknown build must not probe") };
        assert_eq!(choose(None, probe), LEGACY);
    }

    #[test]
    fn the_current_faces_are_a_valid_set_and_stable() {
        let first = current();
        assert!(!first.text.is_empty() && !first.display.is_empty() && !first.icons.is_empty());
        assert_eq!(current(), first);
    }
}
```

`Faces` must derive `PartialEq, Eq, Debug` for these assertions.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib window::design::faces -- --test-threads=1`
Expected: compile errors.

- [ ] **Step 3: Implement**

Above the tests:

```rust
//! The chrome's font faces. Windows 11 gets Segoe UI Variable and Segoe Fluent Icons (the icon face
//! keeps Segoe MDL2 Assets' codepoints); everything else keeps Segoe UI and Segoe MDL2 Assets.
//! The set is chosen lazily the first time a font is made and cached, so startup does no work.

use std::sync::OnceLock;

/// The faces the chrome draws with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Faces {
    /// Every text style except `Title`.
    pub(crate) text: &'static str,
    /// `Title`.
    pub(crate) display: &'static str,
    /// Glyph fonts.
    pub(crate) icons: &'static str,
}

const LEGACY: Faces = Faces {
    text: "Segoe UI",
    display: "Segoe UI",
    icons: "Segoe MDL2 Assets",
};
const WINDOWS_11: Faces = Faces {
    text: "Segoe UI Variable Text",
    display: "Segoe UI Variable Display",
    icons: "Segoe Fluent Icons",
};
/// The first Windows 11 build.
const WINDOWS_11_BUILD: u32 = 22000;

/// Pure: the Windows 11 faces on build 22000 or later when `probe` accepts the text face, the
/// legacy faces otherwise. `probe` is only called on a Windows 11 build.
pub(crate) fn choose(build: Option<u32>, probe: impl Fn(&str) -> bool) -> Faces {
    match build {
        Some(build) if build >= WINDOWS_11_BUILD && probe(WINDOWS_11.text) => WINDOWS_11,
        _ => LEGACY,
    }
}

/// The set for this process, computed on first use.
pub(crate) fn current() -> Faces {
    static CURRENT: OnceLock<Faces> = OnceLock::new();
    *CURRENT.get_or_init(|| choose(os_build(), face_is_mapped))
}

/// The OS build from `RtlGetVersion` (in ntdll, which every process has loaded), or `None` when it
/// cannot be reached.
fn os_build() -> Option<u32> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    #[repr(C)]
    struct VersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    type RtlGetVersion = unsafe extern "system" fn(*mut VersionInfo) -> i32;
    unsafe {
        let ntdll = GetModuleHandleW(crate::platform::wide_null("ntdll.dll").as_ptr());
        if ntdll.is_null() {
            return None;
        }
        let function = GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast())?;
        let function: RtlGetVersion = std::mem::transmute(function);
        let mut info: VersionInfo = std::mem::zeroed();
        info.size = std::mem::size_of::<VersionInfo>() as u32;
        (function(&mut info) == 0).then_some(info.build)
    }
}

/// Whether GDI maps `face` to itself: a font made with it, selected into the screen DC, reports
/// the same face name. A missing face is substituted by GDI and reports another name.
fn face_is_mapped(face: &str) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, FW_NORMAL, GetDC, GetTextFaceW, ReleaseDC, SelectObject,
    };
    let font = crate::window::titlebar::create_ui_font(13, face, FW_NORMAL as i32, false);
    if font.is_null() {
        return false;
    }
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            DeleteObject(font);
            return false;
        }
        let previous = SelectObject(dc, font);
        let mut name = [0u16; 64];
        let written = GetTextFaceW(dc, name.len() as i32, name.as_mut_ptr());
        SelectObject(dc, previous);
        ReleaseDC(std::ptr::null_mut(), dc);
        DeleteObject(font);
        let length = usize::try_from(written).unwrap_or(0).saturating_sub(1);
        length > 0 && String::from_utf16_lossy(&name[..length.min(name.len())]).eq_ignore_ascii_case(face)
    }
}
```

If `GetProcAddress`'s signature in this windows-sys version takes a different pointer type for the name (`*const u8`), adapt the cast; if `Win32_System_LibraryLoader` is not an enabled feature, check `Cargo.toml` for how other code loads libraries (`preview::dwrite::load_system_library` exists) and reuse that path rather than adding a feature. Put `#[allow(dead_code, reason = "used by the font sites in the next tasks")]` on `current` and `Faces` fields as clippy requires; remove in Task 3.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --lib window::design::faces -- --test-threads=1`
Expected: PASS (on this Windows 10 machine `current()` is the legacy set).

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add src/window/design
git commit -m "feat: choose Windows 11 faces behind a build check and a probe"
```

---

### Task 3: Use the chosen faces everywhere

**Files:**
- Modify: `src/window/design/type_ramp.rs`, `src/window/titlebar.rs` (`TitleFonts::create`, ~line 351), `src/window/side_panel.rs` (`UiFonts::create`, ~lines 98-99), `src/window/preview_buttons.rs` (~line 354), `src/window/soft_paint.rs` (`GLYPH_FONT`, ~line 40), `src/window/about.rs` (`create_underlined_font`, ~line 501), `src/window/settings_dialog.rs` (~line 575), `src/window/image_view/paint.rs` (~line 54), `src/window/preview_host.rs` (~line 545), `src/window/design/faces.rs` (remove the temporary allows)
- Test: `src/window/design/type_ramp.rs` tests

**Interfaces:**
- Consumes: `faces::current()`.
- Produces: `type_ramp::face(style: TextStyle) -> &'static str` (replaces the `UI_FACE` constant).

- [ ] **Step 1: Write the failing tests**

In `type_ramp.rs` tests add:

```rust
    #[test]
    fn the_title_uses_the_display_face_and_every_other_style_the_text_face() {
        // Break caught: Title losing the display face, or a style bypassing the chosen faces.
        let faces = crate::window::design::faces::current();
        assert_eq!(face(TextStyle::Title), faces.display);
        for style in [
            TextStyle::Body,
            TextStyle::BodyItalic,
            TextStyle::BodyBold,
            TextStyle::PanelHeader,
            TextStyle::Heading,
        ] {
            assert_eq!(face(style), faces.text, "{style:?}");
        }
    }
```

Add a test in `side_panel.rs` tests that `UiFonts::create(96)` makes its `glyph` font with the icon face: select it into a DC and compare `GetTextFaceW` to `faces::current().icons` (copy the DC pattern from the existing glyph-font height test). Run both and see them fail to compile (`face` does not exist).

- [ ] **Step 2: Implement**

`type_ramp.rs`: delete `UI_FACE`; add

```rust
/// The face for `style`: the display face for `Title`, the text face for the rest.
pub(crate) fn face(style: TextStyle) -> &'static str {
    let faces = super::faces::current();
    if style == TextStyle::Title { faces.display } else { faces.text }
}
```

and make `create` use `face(style)`. Replace every other use of `UI_FACE` (grep) with `faces::current().text`. Replace the literal "Segoe MDL2 Assets" in the non-test, non-comment sites listed above with `faces::current().icons` (for `soft_paint::GLYPH_FONT`, a `const`, change it to a function `glyph_font() -> &'static str` and update its users). The underlined link fonts use `faces::current().text`. The two DirectWrite `"Segoe UI"` uses (`image_view/paint.rs`, `preview_host.rs`) use `faces::current().text`; if the DirectWrite font collection cannot resolve "Segoe UI Variable Text" it falls back inside DirectWrite, so no extra guard is needed. Leave `DEFAULT_PREVIEW_FONT`, the editor and preview font settings, tests that name "Segoe UI" explicitly for a font they build themselves, and comments untouched (update a comment only if it states the face as fact). Remove the temporary `allow(dead_code)` attributes from `faces.rs`.

- [ ] **Step 3: Run**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::design -- --test-threads=1`, `window::side_panel`, `window::titlebar`, `window::about`, `window::settings_dialog`, `window::preview_buttons` (each `-- --test-threads=1`). Expected: PASS on this Windows 10 machine with the legacy faces unchanged.

- [ ] **Step 4: Commit**

```bash
git add src
git commit -m "feat: chrome fonts use the chosen faces"
```

---

### Task 4: The text factor in fonts, caches and the refresh path

**Files:**
- Modify: `src/window/design/type_ramp.rs` (`create`), `src/window/about.rs` and `src/window/settings_dialog.rs` (the underlined link fonts and the 13px literals), `src/window/titlebar.rs` (`TitleFonts` dpi key), `src/window/side_panel.rs` (`Sidebar::fonts` cache key), `src/window/main_window/chrome.rs` (the `title_chrome` font compare, ~lines 205-222), `src/window/main_window/wndproc.rs` (`WM_SETTINGCHANGE`, ~line 452), `src/window/design/text_scale.rs` (remove `allow(dead_code)` on `factor` and `refresh`), `docs/superpowers/specs/2026-10-01-fonts-text-size-design.md` (one sentence, below)
- Test: `src/window/design/type_ramp.rs`, `src/window/side_panel.rs`

**Interfaces:**
- Consumes: `text_scale::{factor, refresh, scale_text, set_factor_for_test}`.
- Produces: fonts whose heights follow the factor; font caches keyed on `(dpi, factor)`; a `refresh_metrics(hwnd)` function in `main_window` that the DPI path and the text-size path share.

- [ ] **Step 1: Write the failing tests**

- `type_ramp`: with `set_factor_for_test(150)`, `create(TextStyle::Body, 96)` has `lfHeight == -20`; at 225 it is `-29`; at 100 it is `-13` (restore 100 after). Title (18) at 225 is `-41` (40.5 rounds up).
- `side_panel`: with the factor at 225, `UiFonts::create(96)`: `height(fonts.text) == -29`, while `height(fonts.glyph) == -scale(SIDEBAR_ICON, 96)` and `height(fonts.bar_glyph) == -scale(ICON, 96)` (icons unchanged). Extend the existing glyph-font height test's pattern; restore the factor to 100.
- `side_panel`: `Sidebar::fonts` returns a new font set when only the factor changes (create at 100, set 150, call again with the same dpi, and assert the returned `text` handle is not the old one); use the existing way tests build a `Sidebar`.

Run them; they fail (heights unchanged by the factor).

- [ ] **Step 2: Implement**

- `type_ramp::create`: `create_ui_font(scale_text(spec.px, dpi), face(style), ...)` (import `text_scale::scale_text`).
- `about.rs` and `settings_dialog.rs`: the underlined link fonts and any remaining literal text pixel heights (13) use `scale_text(13, dpi)` instead of `scale(13, dpi)`. Do not touch glyph-font sizes (`scale(11)` in About).
- Caches: `Sidebar::fonts: Option<(u32, UiFonts)>` becomes `Option<((u32, u32), UiFonts)>` keyed `(dpi, text_scale::factor())`; compare both. `TitleFonts` gets a `factor: u32` next to its `dpi` and `title_chrome` rebuilds when either differs.
- `refresh_metrics(hwnd)` in `main_window` (new, in `chrome.rs` or `wndproc.rs` beside the DPI handling): sets `title_fonts = None` (as `WM_DPICHANGED` does), then runs the same relayout the DPI path runs (`layout_editor_and_find_bar`, the editor padding and folding reset, and the row-height refresh described in Task 5) and `InvalidateRect`. Make `WM_DPICHANGED` call the shared pieces rather than duplicating them, keeping its `SetWindowPos`.
- `WM_SETTINGCHANGE`: call `text_scale::refresh()` on every such message (a cheap registry read; the parameter string is not matched, because it is not reliable across Windows versions), and when it returns true call `refresh_metrics(hwnd)`. Keep the existing `refresh_theme` and `InvalidateRect` behaviour for the other settings.
- Remove the `allow(dead_code)` attributes on `factor` and `refresh`.
- In the spec, replace "On `WM_SETTINGCHANGE` whose parameter is \"TextScaleFactor\" (a change to DPI keeps its own path)." with "On every `WM_SETTINGCHANGE`, by re-reading the value and acting only when it changed (the message's parameter string is not matched; a change to DPI keeps its own path)." and make the same change to the sentence in section 3.5 that says "checks whether the changed-setting parameter is the string".

- [ ] **Step 3: Run**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::design -- --test-threads=1`, `window::side_panel`, `window::titlebar`, `window::main_window::tests::sidebar_layout` and `window::about` (each `-- --test-threads=1`). Expected: PASS; at factor 100 every existing test is unchanged.

- [ ] **Step 4: Commit**

```bash
git add src docs
git commit -m "feat: chrome fonts follow the Windows text-size setting"
```

---

### Task 5: Text-tied heights follow the factor

**Files:**
- Modify: `src/window/design/metrics.rs` (new helpers), and every call site of `SIDEBAR_ROW`, `PANEL_HEADER`, the Search view's `ROW_AT_96_DPI`, `ROW_LINE_AT_96_DPI` and `REPLACE_ROW_AT_96_DPI`, the command palette's `ROW_HEIGHT_AT_96_DPI` and `status::STATUS_HEIGHT_AT_96_DPI`:
  `favorites_view.rs` (~124, 151, 170, 183, 185, 194, 204, 237), `notebook_layout.rs` (~42, 44), `notebook_view/paint.rs` (~645-646), `notebook_view.rs` (~422, 443), `search_view/edits.rs` (~338), `search_view/geometry.rs` (~17, 76, 114), `search_view/paint.rs` (~325), `search_view.rs` (~291), `side_panel.rs` (~1109), `command_palette.rs` (~497, 793), `status.rs` (~15)
- Test: `src/window/design/metrics.rs`, `src/window/search_view/tests.rs`, and the nearest tests of each changed layout

**Interfaces:**
- Consumes: `text_scale::scale_text`.
- Produces: in `design::metrics`: `sidebar_row(dpi: u32) -> i32` (`scale_text(SIDEBAR_ROW, dpi)`) and `panel_header(dpi: u32) -> i32` (`scale_text(PANEL_HEADER, dpi)`). The Search, palette and status heights get the same treatment through their own local helper or by calling `scale_text` directly.

- [ ] **Step 1: Write the failing tests**

- `metrics`: with the factor at 100, `sidebar_row(dpi) == scale(SIDEBAR_ROW, dpi)` and `panel_header(dpi) == scale(PANEL_HEADER, dpi)` for DPIs 96, 120, 144, 192; at 225, `sidebar_row(96) == 59` and `panel_header(96) == 86`. Restore the factor to 100.
- Search view: `a_result_row_holds_two_lines_of_the_sidebar_text_at_every_dpi` gains a loop over factors `[100, 150, 225]`: at each DPI and factor, build the body font with `create(TextStyle::Body, dpi)` (which now follows the factor), and assert the row height `scale_text(ROW_AT_96_DPI, dpi)` is at least two text lines (`tmHeight` of that font, using `ROW_LINE_AT_96_DPI` scaled the same way) plus the existing insets. Restore the factor to 100.
- Status bar: `status::status_height(dpi)` at factor 225 is `scale_text(22, dpi)`-based and at 100 unchanged.
- Command palette: the measured list height at factor 225 is `rows * scale_text(26, dpi)`.

Run them; they fail (heights ignore the factor).

- [ ] **Step 2: Implement**

Add `sidebar_row` and `panel_header` to `metrics.rs` and replace every `scale(SIDEBAR_ROW, dpi)` and `scale(PANEL_HEADER, dpi)` in the listed files. Replace `scale(ROW_AT_96_DPI, dpi)`, `scale(ROW_LINE_AT_96_DPI, dpi)`, `scale(REPLACE_ROW_AT_96_DPI, dpi)` in the Search view, `scale(ROW_HEIGHT_AT_96_DPI, dpi)` in the command palette and the arithmetic in `status.rs` with `scale_text` (status keeps its `dpi.max(96)`). Leave the 100% values and constants themselves unchanged.

Where a row height is stored in state (`RowListState::new(...)`, `list.row_height = ...`, `OpenEditors::new(...)`), find where the DPI change updates it and make `refresh_metrics` (Task 4) trigger the same update, so a changed factor refreshes every stored row height; the notebook view sets its row heights on every paint (`paint.rs` ~645), but favorites (~151) and the Search view (`edits.rs` ~338) set theirs in layout, so confirm each is re-run by `refresh_metrics`.

Also check, and report in your task report without changing them unless the same fix is needed to avoid clipping text at factor 225: `find_bar::find_bar_height`, the name box height, the tab strip's tab height and `titlebar::strip_height`. If one derives from a text constant that clips at 225%, apply `scale_text` to it the same way and add a test; otherwise leave it and say why.

At factor 100 every value must equal today's: the existing layout tests that pin 26, 38, 46, 19, 22, 34 and the palette row are the guard and must pass unchanged.

- [ ] **Step 3: Run**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::design -- --test-threads=1`, `window::search_view`, `window::favorites_view`, `window::notebook_view`, `window::notebook_layout`, `window::command_palette`, `window::status`, `window::side_panel` and `window::main_window::tests::sidebar_layout` (each `-- --test-threads=1`). Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src
git commit -m "feat: sidebar, Search, palette and status heights follow the text size"
```
