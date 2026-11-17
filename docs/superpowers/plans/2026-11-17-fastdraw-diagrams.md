# FastDraw diagrams Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `.fdraw` files open in tabs with Source / Split / Preview, pan and zoom; `![](x.fdraw)` draws in notes; Open in FastDraw sends the diagram to fastdraw.dev's editor.

**Architecture:** The `fastdraw-render` crate (git-pinned) turns diagram text into SVG. A new `src/preview/fastdraw.rs` wraps it with FastPad's rules (look-path refusal, error text, fonts, panic guard) and rasterizes through the existing `preview::svg` path. The SVG tab's image view, the Markdown `ImageCache` and the editor toolbar learn a FastDraw case alongside SVG.

**Tech Stack:** Rust 2024, Win32 (`windows`, `windows-sys`), resvg 0.48.1, `fastdraw-render` at `f0d3905`.

**Spec:** `docs/superpowers/specs/2026-11-16-fastdraw-diagrams-design.md`

## Global Constraints

- Dependency, exactly: `fastdraw-render = { git = "https://github.com/coccor/fastdraw", rev = "f0d3905" }`.
- Refused look paths: absolute, starting `\\` or `//`, or with a drive prefix (`C:x` included). Refusal error text: `FastPad reads look files only by a relative path; "<path>" was not read`.
- Error band text: `Line <n>: <message>`, plus ` (and <k> more)` when k more errors follow.
- Failed-state title for diagrams: `FastPad can't draw this diagram`. Panic text: `FastDraw could not draw this diagram`.
- `.fdraw` files over 1 MB are not read. Editor links over 32,000 characters are not opened: notice `This diagram is too large to open by link.`; shell failure notice `FastPad could not open fastdraw.dev.`
- Command palette label `FastDraw: Open in FastDraw`; toolbar tooltip `Open in FastDraw (fastdraw.dev)`; glyph `\u{E8A7}`.
- Nothing is fetched from the network. No commit adds `Co-Authored-By` trailers.
- Before launching a dev build by hand, copy `native/out/x64` DLLs into `target/debug` (no highlighting otherwise). CI runs `cargo fmt --check`, `cargo clippy --all-targets --all-features -D warnings`, `cargo test -- --test-threads=1`, and `tools/audit-dependencies.ps1`; every task leaves all four passing.

## Review Focus

1. **A diagram that panics or loops in the renderer** must not take FastPad down: `render` runs in `catch_unwind` on the decode thread (Task 1 test with a forced panic hook).
2. **Typing in Split through invalid states** (half-typed lines) must keep the last good drawing and never blank the view (Task 3 integration test types a broken line).
3. **A look file given as `C:x.look` or `\\?\C:\x.look`** must be refused, not read (Task 1 cases).
4. **Dark theme switch with a note open**: `.fdraw` images in the note redraw in the dark look instead of keeping the light bitmap (Task 5 `set_dark` test).
5. **A `.fdraw` image link in a note whose file is over 1 MB or not UTF-8** shows as a failed image, no panic (Task 5 tests).

---

### Task 1: `preview::fastdraw` — drawing with FastPad's rules, and fonts

**Files:**
- Modify: `Cargo.toml` (dependency), `tools/audit-dependencies.ps1` (allowed root `fastdraw-render` 0.2.2, git source), `Cargo.lock`
- Create: `src/preview/fastdraw.rs`
- Modify: `src/preview/mod.rs` (`pub mod fastdraw;`), `src/preview/svg.rs` (font database)

**Interfaces:**
- Produces:
  - `pub struct Drawn { pub svg: String, pub paper: u32 /* 0x00RRGGBB */, pub errors: Vec<String> /* "Line n: msg" */, pub show: bool, pub look: Option<fastdraw_render::LookFile> }` — `show` is false when any error other than the refusal exists.
  - `pub fn draw(text: &str, file: Option<&Path>, dark: bool) -> Drawn` — applies the refusal rule, runs `render` in `catch_unwind`, registers fonts.
  - `pub fn band_text(errors: &[String]) -> Option<String>` — first error plus ` (and k more)`.
  - `pub fn refused(look: &str) -> bool`.
  - `pub fn rasterize(drawn: &Drawn, max_width: u32, scale: f32) -> Result<DecodedImage, String>` — via `svg::parse` + `svg::natural_size` + `mermaid::pixel_size` + `svg::rasterize`.
  - In `svg.rs`: `pub(crate) fn add_fastdraw_fonts()` (once) and `pub(crate) fn add_font_files(&[PathBuf])` (each path once). `system_fonts()` becomes a `RwLock<Arc<fontdb::Database>>`; adding clones the database, loads, and swaps the `Arc`.

- [ ] **Step 1: Write the failing tests** in `src/preview/fastdraw.rs`:

```rust
#[test] fn network_absolute_and_drive_look_paths_are_refused() {
    for p in ["//server/x.look", r"\\server\x.look", "C:/x.look", "C:x.look", "/x.look", r"\\?\C:\x.look"] { assert!(refused(p), "{p}"); }
    for p in ["./x.look", "x.look", "looks/x.look", "night"] { assert!(!refused(p), "{p}"); }
}
#[test] fn a_refused_look_draws_in_the_built_in_look_with_its_own_error() {
    let d = draw("look \"//server/x.look\"\na: node \"A\" at 0,0\n", Some(Path::new(r"C:\t\d.fdraw")), false);
    assert!(d.show && !d.svg.is_empty() && d.look.is_none());
    assert_eq!(d.errors, [r#"Line 1: FastPad reads look files only by a relative path; "//server/x.look" was not read"#]);
}
#[test] fn band_text_counts_the_rest() {
    assert_eq!(band_text(&[]), None);
    assert_eq!(band_text(&["Line 4: x".into()]).unwrap(), "Line 4: x");
    assert_eq!(band_text(&["Line 4: x".into(), "Line 5: y".into(), "Line 9: z".into()]).unwrap(), "Line 4: x (and 2 more)");
}
#[test] fn a_real_error_hides_the_drawing() { let d = draw("a: nosuchkind \"A\" at 0,0\n", None, false); assert!(!d.show); assert!(d.errors[0].starts_with("Line 1: ")); }
#[test] fn a_dark_diagram_uses_night_paper() { assert_ne!(draw("a: node \"A\" at 0,0\n", None, true).paper, draw("a: node \"A\" at 0,0\n", None, false).paper); }
#[test] fn text_rasterizes_with_ink_after_fonts_are_added() { /* draw "a: node \"Hello\" at 0,0", rasterize at natural size, assert some pixel inside the label's box differs from the paper colour */ }
#[test] fn a_panicking_render_becomes_an_error() { /* call the internal `guarded(|| panic!())` helper that `draw` uses; assert errors == ["FastDraw could not draw this diagram"], show == false */ }
```

- [ ] **Step 2: Run** `cargo test --lib preview::fastdraw` — Expected: FAIL (module missing).
- [ ] **Step 3: Add the dependency and audit entry; implement** `refused` (use `Path::new(p).is_absolute() || p.starts_with(['/', '\\']) || p.get(1..2) == Some(":")`), `draw` (if `look_path` is refused, render with `file: None` and replace the `look file not found` error on that line with the refusal text), `band_text`, `rasterize`, and the font functions. Error lines are only `Level::Error` issues, in order.
- [ ] **Step 4: Run** `cargo test --lib preview::fastdraw preview::svg` and `./tools/audit-dependencies.ps1` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(fastdraw): draw .fdraw diagrams with fastdraw-render`.

### Task 2: Image view — FastDraw source, paper background, error band

**Files:**
- Modify: `src/image_view/decode.rs`, `src/image_view/mod.rs`, `src/image_view/paint.rs`

**Interfaces:**
- Consumes: `preview::fastdraw::{draw, rasterize, band_text, Drawn}`.
- Produces:
  - `decode::Source::FastDraw { text: Arc<str>, file: Option<PathBuf>, dark: bool, width: u32 }`.
  - `FullImage` gains `pub diagram: Option<DiagramFacts>`; `pub struct DiagramFacts { pub paper: u32, pub band: Option<String> }`.
  - `ImageError::Diagram(String)` — `message()` is the string; new `ImageError::title()` returns `FastPad can't draw this diagram` for it and `FastPad can't display this image` otherwise.
  - `ImageView::show_fastdraw(&self, text: Arc<str>, file: Option<PathBuf>, name: &str, new_document: bool)` — like `show_svg`, darkness from the view's colours (`is_dark()`).
  - `ViewStats` gains `band: Option<String>` (the text shown in the band; `svg_error` stays).

- [ ] **Step 1: Write failing tests** in `decode.rs` tests: `decode_fastdraw` of `"a: node \"A\" at 0,0\n"` returns `Ok` with `diagram.paper` set and `band == None`; of a refused-look diagram returns `Ok` with `band == Some(refusal line)`; of `"a: nosuchkind \"A\" at 0,0\n"` returns `Err(ImageError::Diagram(text))` with `text` starting `Line 1: `. In `paint.rs`/`mod.rs` tests (or a `ViewState` unit test): an `Ok` with a band sets `svg_error` and `band`; an `Err(Diagram)` with a previous image keeps the image and sets the band to the error text; without one sets `error`.
- [ ] **Step 2: Run** `cargo test --lib image_view` — Expected: FAIL.
- [ ] **Step 3: Implement.** Decode: `draw` → if `!show` → `Err(Diagram(band_text(errors)))`, else `rasterize` with the existing `svg_render_size` sizing. View: FastDraw shares the SVG zoom-keeping rules (treat it as `svg` in the decoded-result handler). Paint: clear with `diagram.paper` when present and skip the checkerboard; the band draws `band` text when set, else "Can't render this SVG"; the failed state uses `error.title()`.
- [ ] **Step 4: Run** `cargo test --lib image_view` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(fastdraw): the image view draws FastDraw diagrams`.

### Task 3: `.fdraw` tabs

**Files:**
- Modify: `src/document.rs` (`Language::FastDraw`), `src/languages/registry.rs` (spec: name "FastDraw", extensions `["fdraw"]`, `lexer: None`, plain-text properties/styles as `PlainText` uses), `src/window/file_icons.rs` (`("fdraw", NoteKind::Image)`, `Language::FastDraw => NoteKind::Image`), `src/window/preview_host.rs`, `src/window/editor_toolbar.rs`
- Test: `tests/windows/image_preview.rs`, registry unit tests

**Interfaces:**
- Consumes: `ImageView::show_fastdraw` (Task 2).
- Produces: `preview_host::active_is_fastdraw(hwnd) -> bool` (used by Task 4); every `Language::Svg` check in `preview_host.rs` that concerns the SVG view (`available`, `buttons_visible`, `active_is_svg` callers, `zoom_svg`, the loading in `load_svg`) also accepts `Language::FastDraw`; `load_svg` calls `show_fastdraw` for FastDraw tabs with the tab's path; `group_mode`'s default is `Preview` for a FastDraw tab with a path and non-empty text, else `Source`; `segments_for` returns `&ViewMode::SVG` for FastDraw.

- [ ] **Step 1: Write failing tests.** Registry: `language_for_path(r"C:\x\Flow.FDRAW") == Language::FastDraw`, `default_extension(Language::FastDraw) == "fdraw"`. Integration, `a_fdraw_opens_in_preview_draws_and_keeps_the_last_drawing_on_a_mistake` modelled on `an_svg_opens_as_text_and_the_preview_renders_and_follows_edits`: write `d.fdraw` with `a: node "A" at 0,0`, open it, assert `buttons_visible`, mode is Preview without a command, wait for `Phase::Ready`; `set_text` adding `b: node "B" at 200,0`, wait for a wider `status().size`; `set_text` a third line `c: nosuchkind "C" at 0,0`, wait for `stats().band` to start with `Line 3: `, and assert the size did not change.
- [ ] **Step 2: Run** `cargo test --lib languages` and `cargo test --test image_preview a_fdraw -- --test-threads=1` — Expected: FAIL.
- [ ] **Step 3: Implement** the language and the preview_host / toolbar changes above.
- [ ] **Step 4: Run** the two commands again plus `cargo test --lib window::editor_toolbar` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(fastdraw): .fdraw tabs with Source, Split and Preview`.

### Task 4: Open in FastDraw

**Files:**
- Modify: `src/window/commands.rs` (`OpenInFastDraw = 299`, its id table and keymap name `fastdraw.openInFastDraw` in `src/window/keymap.rs`), `src/window/command_palette.rs` (entry `FastDraw: Open in FastDraw`), `src/window/menus.rs` (`show_tab_menu` adds `Open in &FastDraw` after a separator when the tab is FastDraw), `src/window/main_window/command_dispatch.rs`, `src/window/preview_host.rs` (`open_in_fastdraw(hwnd)`; a `#[cfg(test)]` recorder like `about.rs`'s `OPENED`), `src/window/editor_toolbar.rs` (button)
- Test: `src/window/main_window/tests/preview_tabs_and_folders.rs`

**Interfaces:**
- Consumes: `preview::fastdraw::draw` (for the look as read), `fastdraw_render::editor_link`, `preview_host::shell_open`, `active_is_fastdraw`.
- Produces: `preview_host::open_in_fastdraw(hwnd: HWND)`; `#[cfg(test)] preview_host::take_opened() -> Vec<String>`.

- [ ] **Step 1: Write failing tests.** `open_in_fastdraw_sends_the_tab_to_the_editor`: open a FastDraw tab `flow.fdraw` with `a: node "A" at 0,0`, run `CommandId::OpenInFastDraw`, assert one opened URL equal to `fastdraw_render::editor_link("flow.fdraw", <text>, None)` and starting `https://fastdraw.dev/edit/#fdraw=`. `the_command_does_nothing_on_other_tabs`: on a `.txt` tab nothing is opened. `the_tab_menu_offers_open_in_fastdraw_only_on_fastdraw_tabs` (menu entries built by a helper `tab_menu_entries(hwnd)` that tests read). Toolbar: `segment_at`-style hit test `button_at` returns the button for a point inside its rect on a FastDraw tab, and the toolbar has no button on an SVG tab.
- [ ] **Step 2: Run** `cargo test --lib open_in_fastdraw the_tab_menu window::editor_toolbar -- --test-threads=1` — Expected: FAIL.
- [ ] **Step 3: Implement.** The look passed is `draw(text, path, false).look`. Over 32,000 characters → notice; `shell_open` false → notice. Toolbar: one 24-px button with glyph `\u{E8A7}` left of the track, separated by `TRACK_PADDING * 2`, sharing the hover/press painting and the tooltip tool list (tool index `segments.len()`); a click dispatches `CommandId::OpenInFastDraw`.
- [ ] **Step 4: Run** the same command — Expected: PASS.
- [ ] **Step 5: Commit** `feat(fastdraw): Open in FastDraw`.

### Task 5: `.fdraw` image links in notes

**Files:**
- Modify: `src/preview/fastdraw.rs` (`decode_file`), `src/preview/images.rs`, `src/preview/view.rs` and `src/live/view.rs` (pass darkness), wherever each view builds its `ImageCache` / sets colours

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub fn decode_file(path: &Path, dark: bool, max_width: u32) -> Result<(DecodedImage, Stamp), String>`; `pub struct Stamp { diagram: Option<SystemTime>, look: Option<(PathBuf, Option<SystemTime>)> }` with `pub fn current(&self, path: &Path) -> bool` (re-stats both).
  - `ImageCache::set_dark(&mut self, dark: bool)` — on change, removes entries whose path ends in `.fdraw` (any case).
  - `ImageCache::request` decodes `.fdraw` paths through `decode_file` with the cache's darkness, and re-queues an entry whose `Stamp` is no longer `current`.

- [ ] **Step 1: Write failing tests** in `images.rs`: `a_fdraw_link_decodes_to_the_drawing` (write `d.fdraw`, request, drain, `size(path)` has the drawing's proportions — compare with `fastdraw::draw` bounds ratio within 1 px); `a_broken_fdraw_is_failed` (`a: nosuchkind ...` → `is_failed`); `an_oversized_or_non_utf8_fdraw_is_failed` (1 MB + 1 bytes; bytes `[0xff, 0xfe, 0x00]` → failed, no panic); `changing_the_fdraw_decodes_it_again` (rewrite with a wider diagram, bump mtime with `File::set_modified`, request, drain, size changed); `switching_darkness_decodes_it_again` (`set_dark(true)` → entry gone, request → decoded again).
- [ ] **Step 2: Run** `cargo test --lib preview::images` — Expected: FAIL.
- [ ] **Step 3: Implement.** `decode_file` reads at most 1 MB (`MAX_FDRAW_BYTES = 1 << 20`), `String::from_utf8` else failed, `draw(text, Some(path), dark)`, `!show` → `Err(band_text)`, else `rasterize`; the stamp's look path is the diagram folder joined with `look.path` when `drawn.look` is `Some`. Views call `set_dark(colors.is_dark())` wherever they apply colours.
- [ ] **Step 4: Run** `cargo test --lib preview live` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(fastdraw): draw .fdraw image links in notes`.

### Task 6: Docs, licences, final check

**Files:**
- Modify: `docs/features.md`, `docs/reference.md`, `LICENSES.md`, `licenses/` (FastDraw MIT, Phosphor Icons MIT, SIL OFL 1.1 for Geist, Geist Mono, Instrument Serif, Libre Franklin, Newsreader, Sketch Hand — texts from the fastdraw repo's `licenses/` and `fonts/OFL-*.txt` at `f0d3905`)

- [ ] **Step 1:** Write the **FastDraw diagrams** section in `docs/features.md` covering spec §7's list, and the command row in `docs/reference.md`.
- [ ] **Step 2:** Add the licences; run `./tools/rust-crate-licenses.ps1` if it regenerates `LICENSES.md` and check it lists `fastdraw-render` and its dependencies.
- [ ] **Step 3: Run** `cargo fmt --all -- --check`, `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings`, `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1`, `./tools/audit-dependencies.ps1` — Expected: all pass.
- [ ] **Step 4:** Launch the debug build (copy `native/out/x64` DLLs first), open `C:\Projects\fastdraw\examples\kinds\architecture-delta.fdraw`, check Preview, zoom, pan, Split typing, a dark theme, Open in FastDraw, and a note with `![](architecture-delta.fdraw)`.
- [ ] **Step 5: Commit** `docs: FastDraw diagrams`, then clean build artefacts (`cargo clean`, remove `native/src`; keep `native/out` and `native/cache`).
