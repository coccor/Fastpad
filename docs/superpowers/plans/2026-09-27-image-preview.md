# Image Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (recommended here: the user asked to minimise test runs and reviews) or superpowers:subagent-driven-development to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open common image files in an image tab (zoom, pan, themed, accessible), render SVG tabs in the Split/Full preview, and list images in the notebook tree.

**Architecture:**
- **Tab kind.** `Document` gains `content: Content { Text(EditorDocument), Image }`.
- **Image view.** A new lazily created Direct2D child window, `image_view::ImageView`. It decodes on per-request worker threads, using WIC for raster images and the existing Direct2D SVG rasterizer for SVG. It is driven by:
  - `window::image_host` for image tabs;
  - `window::preview_host` for SVG tabs in Split/Full.
- **One branch point.** `open_path_placed` is the only place opens branch. Every entry point (Open dialog, launch, IPC, drops, tree, quick open, links, session) reaches it.

**Tech Stack:** Rust 2024, windows 0.62.2 (Direct2D, DirectWrite, WIC), windows-sys 0.61.2, Scintilla (unchanged).

**Spec:** `docs/superpowers/specs/2026-09-27-image-preview-design.md`. Read §3–§11 before starting. §13 is added by Part 0 of this plan.

## How this plan runs (user request: few test runs, few reviews)

- **One implementer, in order.** Parts 0–9 are done by one implementer in order. Each part ends with `cargo clippy --all-targets --all-features -- -D warnings` and a commit.
- **Targeted tests run only at the three marked checkpoints:**
  - **A**, after Part 3: unit tests;
  - **B**, after Part 6: the new `image_preview` target;
  - **C**, after Part 8: notebook and icon unit tests.
- **Rerun only what failed.** Don't rerun passing targets.
- **One final pass.** One whole-branch review and one full serial suite, at Part 9 only.
- **Before any test in this worktree**, copy `native/out/x64` from the main checkout (`D:\Projects\FastPad\native\out\x64` → `native\out\x64`). Window tests need `-- --test-threads=1`.
- **Live-app checks:** back up `%LocalAppData%\FastPad\fastpad.ini` first and restore it afterwards.

## Global Constraints

- No static imports of d2d1, dwrite, windowscodecs or shlwapi. Load them with `preview::dwrite::load_system_library` / `Graphics::load`. The existing `binary_does_not_statically_import_preview_graphics_libraries` test must stay green.
- Nothing new runs before the first editable frame. No D2D, DWrite or WIC loads until an image tab or an SVG preview is shown.
- The UI thread never reads or decodes image bytes. It may only read metadata (`library::disk_stamp`) and the 16-byte sniff on the text-open failure path.
- There is one short-lived worker thread per decode request, and no thread pool. Results are posted as a `Box` that the receiver frees. A failed `PostMessageW` frees the box on the worker.
- Raster extensions: `png, jpg, jpeg, jpe, jfif, gif, bmp, dib, ico, tif, tiff, webp, heic, heif, avif`. Plus `svg`, which is an image for the notebook but opens as text.
- Existing caps: `images::MAX_IMAGE_PIXELS = 64_000_000` and `svg::MAX_SVG_BYTES = 8 MiB`.
- Zoom steps: `10, 25, 50, 67, 100, 150, 200, 300, 400, 800, 1600` %. Fit never goes above 100%.
- Messages are `WM_APP + 0x58` (image decoded, sent to the view) and `WM_APP + 0x59` (image status changed, sent to the main window).
- No new `CommandId`. Zoom In/Out/Reset are reused.
- Status bar, right side: `1920 × 1080 · PNG · 245 KB · 50%`. Uses U+00D7 and U+00B7.
- Failed-state copy, verbatim:
  - "FastPad can't display this image"
  - "The file is larger than 64 megapixels."
  - "Windows has no decoder for this format."
  - "The file is damaged or not an image."
  - "The file no longer exists."
  - "The SVG is larger than 8 MB."
  - SVG error bar: "Can't render this SVG"
- No backward compatibility. `session.ini` gets no new keys.

## Review Focus

Each line below has a test added in the part that owns the code.

1. **Ctrl+S / Ctrl+Shift+S / autosave on an active image tab** must never write the file. The editor then holds an empty placeholder document, so a missed guard writes 0 bytes over the image. → Part 6 test `saving_an_image_tab_never_writes_the_file`.
2. **Switching between an image tab and a dirty text tab** must not mark the image tab dirty or lose the text tab's edits. Scintilla save-point notifications come from the placeholder document. → Part 2 unit test `set_active_dirty_ignores_an_image_tab`, and Part 6 test `switching_between_image_and_text_tabs_keeps_each_tab_intact`.
3. **An empty start tab** is replaced by the first image opened, as it is for text files. The strip must not end up with "Untitled" plus the image. → Part 6 test `an_image_replaces_the_empty_start_tab`.
4. **Opening the same image twice** (another spelling of the path, or tree then Ctrl+P) activates the existing tab rather than failing. → Part 6 test `opening_an_open_image_again_activates_its_tab`.
5. **A huge or corrupt file** must not freeze the window. A 64+ MP header or random bytes with `.png` shows the failed state quickly, with no notice. → Part 6 test `a_corrupt_png_shows_the_failed_state_without_a_notice`, and Part 3 unit test `classify_maps_decoder_errors_to_messages`.

---

### Part 0: Spec amendments found while planning

**Files:**
- Modify: `docs/superpowers/specs/2026-09-27-image-preview-design.md` (append §13)

- [ ] **Step 1: Append this section to the spec, verbatim**

```markdown
## 13. Amendments from planning (2026-09-27)

These are code facts found while writing the plan, and they refine the sections above:

1. **§5 tab model.** Only `handle` moves into `Content`. Only 8 sites read it. The ~120 reads of
   `language`/`encoding`/`generation` stay on `Document` and are simply unused by image tabs.
   Save safety does not rely on the type system: every save path refuses an image tab
   explicitly (`save_command`, `save_as_command`, `save_active_document`,
   `save_active_document_as`, `save_active_to`), and a test checks the bytes are unchanged.
2. **§6.5 disk changes.** Text tabs have no disk-change check on activation today, so only
   image tabs gain one. It runs on tab activation and on `WM_ACTIVATEAPP` (active). Text tabs
   are unchanged.
3. **§5 menus.** Today the Edit and Search menus are never grayed. A new
   `menus::set_text_commands_enabled` grays `CommandId::TEXT_COMMANDS` in every menu while an
   image tab is active.
4. **§6.4 D2D load failure.** When `Graphics::load` fails there is no Direct2D view to draw the
   failed state in. The image tab stays open and blank, and a notice appears once: "FastPad
   could not display images: {error}".
5. **§9 rename across kinds.** An open tab keeps its kind when its file is renamed to the other
   kind. It opens with the right kind the next time it is opened. No automatic reopen.
6. **§9 inline rename.** Typing an image extension on a renamed image is kept. `pic.png` → `x.jpg`
   gives `x.jpg`, not `x.jpg.png`, because `title::rename_parts` accepts listed extensions.
7. **§7 Markdown links.** Covered by the shared `open_path`. There is no dedicated end-to-end
   test, because `follow_link` needs a posted payload. The open path itself is tested.
8. **§6.3 grab cursor.** Windows has no grab cursor. `IDC_SIZEALL` shows while the image is
   larger than the view.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-09-27-image-preview-design.md
git commit -m "docs(image-preview): record plan-time amendments in the spec"
```

---

### Part 1: Extension tables, signature sniffing, `Language::Svg`

**Files:**
- Modify: `src/library/title.rs` (after `is_note_extension`, line ~19)
- Create: `src/file/sniff.rs`; Modify: `src/file/mod.rs` (add `pub mod sniff;`)
- Modify: `src/document.rs:49` (`Language`), `src/languages/mod.rs:125,160`, `src/window/status.rs:83`, `src/library/title.rs:~99` (`default_extension`)

**Interfaces:**
- Produces:
  - `title::IMAGE_EXTENSIONS: [&str; 16]`
  - `title::is_image_extension(&str) -> bool`
  - `title::is_raster_image_extension(&str) -> bool`
  - `title::is_listed_extension(&str) -> bool`
  - `title::is_raster_image_path(&Path) -> bool`
  - `file::sniff::looks_like_image(&[u8]) -> bool`
  - `file::sniff::file_looks_like_image(&Path) -> bool`
  - `Language::Svg`

- [ ] **Step 1: Add the tables to `src/library/title.rs`**

```rust
/// Image files the notebook lists and FastPad shows (image preview spec §4). SVG is listed but
/// opens as text, so `is_raster_image_extension` leaves it out.
pub const IMAGE_EXTENSIONS: [&str; 16] = [
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "bmp", "dib", "ico", "tif", "tiff", "webp", "heic",
    "heif", "avif", "svg",
];

pub fn is_image_extension(extension: &str) -> bool {
    IMAGE_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

/// An image that opens in an image tab rather than as text.
pub fn is_raster_image_extension(extension: &str) -> bool {
    is_image_extension(extension) && !extension.eq_ignore_ascii_case("svg")
}

/// A file the notebook lists: a note or an image.
pub fn is_listed_extension(extension: &str) -> bool {
    is_note_extension(extension) || is_image_extension(extension)
}

pub fn is_raster_image_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| is_raster_image_extension(&extension.to_string_lossy()))
}
```

Add a unit test to `title.rs`'s `tests` module:

```rust
#[test]
fn images_are_listed_but_only_raster_images_open_as_images() {
    // Break caught: an SVG opening in the image view instead of as text (user decision "a"), or
    // a PNG missing from the notebook tree.
    assert!(is_listed_extension("PNG") && is_listed_extension("md") && is_listed_extension("svg"));
    assert!(!is_listed_extension("exe"));
    assert!(is_raster_image_extension("JPeG") && !is_raster_image_extension("svg"));
    assert!(is_raster_image_path(Path::new(r"C:\a\b.webp")));
    assert!(!is_raster_image_path(Path::new(r"C:\a\b.svg")) && !is_raster_image_path(Path::new("png")));
}
```

- [ ] **Step 2: Create `src/file/sniff.rs`**

```rust
//! Image file signatures, checked only after a file failed to open as text (image preview
//! spec §4), so an extensionless or misnamed image still opens in an image tab.

use std::io::Read;
use std::path::Path;

const SIGNATURES: [&[u8]; 8] = [
    b"\x89PNG\r\n\x1a\n",
    b"\xFF\xD8\xFF",
    b"GIF87a",
    b"GIF89a",
    b"BM",
    b"II*\0",
    b"MM\0*",
    b"\0\0\x01\0",
];

pub fn looks_like_image(head: &[u8]) -> bool {
    SIGNATURES.iter().any(|signature| head.starts_with(signature))
        || (head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP")
}

/// Reads at most the first 16 bytes of `path`.
pub fn file_looks_like_image(path: &Path) -> bool {
    let mut head = [0_u8; 16];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(read) => filled += read,
        }
    }
    looks_like_image(&head[..filled])
}

#[cfg(test)]
mod tests {
    use super::looks_like_image;

    #[test]
    fn known_signatures_match_and_text_or_short_input_does_not() {
        // Break caught: a PNG saved as .dat getting the "unsupported text encoding" notice, or a
        // four-byte RIFF file indexing past its end.
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\n\0\0"));
        assert!(looks_like_image(b"\xFF\xD8\xFF\xE0"));
        assert!(looks_like_image(b"RIFF\x10\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"\0\0\x01\0\x01\0"));
        assert!(!looks_like_image(b"RIFF"));
        assert!(!looks_like_image(b"RIFF\x10\0\0\0WAVEfmt "));
        assert!(!looks_like_image(b"\x00\x01\x02\x03"));
        assert!(!looks_like_image(b""));
    }
}
```

Add `pub mod sniff;` to `src/file/mod.rs`.

- [ ] **Step 3: Add `Language::Svg` and fix each exhaustive match**

The compiler lists the matches. Make these changes:
- `src/document.rs:49`: add `Svg,` after `Markdown,`.
- `src/languages/mod.rs` `detect_language`: map `"svg"` to `Language::Svg`, next to the `"md"` arm.
- `LanguageManager::apply`: `Language::PlainText | Language::Svg => { editor.set_lexer(0)?; Ok(()) }`.
- `src/window/status.rs` `language_name`: `Language::Svg => "SVG"`.
- `src/library/title.rs` `default_extension`: `Language::Svg => "svg"`.

Add to the `languages` tests:

```rust
#[test]
fn svg_files_are_detected_as_svg() {
    // Break caught: an SVG tab without the preview buttons because it was detected as plain text.
    assert_eq!(detect_language(std::path::Path::new(r"C:\x\Logo.SVG")), Language::Svg);
}
```

- [ ] **Step 4: Clippy, then commit**

Run: `cargo clippy --all-targets --all-features -- -D warnings`. Expected: clean.

```bash
git add src/library/title.rs src/file/sniff.rs src/file/mod.rs src/document.rs src/languages/mod.rs src/window/status.rs
git commit -m "feat(image-preview): image extension tables, signature sniffing and Language::Svg"
```

---

### Part 2: Tab model: `Content::Text | Content::Image`

**Files:**
- Modify: `src/document.rs:66-167`
- Modify: `src/window/tabs.rs:459` (`set_active_dirty`), `:508` (`note_active_text_change`), `:596` (`active_handle`)
- Modify: `src/window/main_window.rs`, the eight `.handle` sites: 3571, 3806, 3881, 4062 (already uses `active_handle`), 4512, 4636, 4672, 4751, 5349

**Interfaces:**
- Produces:
  - `document::Content`
  - `Document::image(DocumentId, RecoveryId, PathBuf) -> Document`
  - `Document::text_handle(&self) -> Option<&EditorDocument>`
  - `Document::expect_text(&self) -> crate::Result<&EditorDocument>`
  - `Document::is_image(&self) -> bool`
  - `#[cfg(test)] Document::image_fixture(DocumentId, PathBuf)`

- [ ] **Step 1: Change `Document`**

Replace the `pub handle: EditorDocument,` field with `pub content: Content,`. Add this above `pub struct Document`:

```rust
/// What a tab shows: text in a Scintilla document, or an image file (image preview spec §5).
/// An image tab has no text, so nothing can be typed into it or saved over it.
#[derive(Debug)]
pub enum Content {
    Text(EditorDocument),
    Image,
}
```

Rewrite the constructors in `impl Document`:

```rust
pub fn untitled(id: DocumentId, recovery_id: RecoveryId, handle: EditorDocument) -> Self {
    Self::with_content(id, recovery_id, Content::Text(handle))
}

/// An image tab for `path`. It is never dirty and never snapshotted.
pub fn image(id: DocumentId, recovery_id: RecoveryId, path: PathBuf) -> Self {
    let mut document = Self::with_content(id, recovery_id, Content::Image);
    document.path = Some(path);
    document
}

fn with_content(id: DocumentId, recovery_id: RecoveryId, content: Content) -> Self {
    Self {
        id,
        content,
        path: None,
        // … every other field exactly as `untitled` sets it today …
    }
}

pub fn text_handle(&self) -> Option<&EditorDocument> {
    match &self.content {
        Content::Text(handle) => Some(handle),
        Content::Image => None,
    }
}

pub fn expect_text(&self) -> crate::Result<&EditorDocument> {
    self.text_handle()
        .ok_or(crate::FastPadError::Invariant("the tab shows an image, not text"))
}

pub fn is_image(&self) -> bool {
    matches!(self.content, Content::Image)
}

#[cfg(test)]
pub fn image_fixture(id: DocumentId, path: PathBuf) -> Self {
    Self::image(id, RecoveryId(u128::from(id.0)), path)
}
```

- [ ] **Step 2: Make `Tabs` ignore image tabs for text state (`src/window/tabs.rs`)**

- `active_handle`: `self.active().and_then(Document::text_handle)`.
- `set_active_dirty` (line 459): as the first line after the active document is resolved, add `if document.is_image() { return false; }`. Put it before any field write, using the function's existing binding name for the active document.
- `note_active_text_change` (line 508): add the same early `return false` for an image tab.

Add to `src/document.rs` tests:

```rust
#[test]
fn set_active_dirty_ignores_an_image_tab() {
    // Break caught: a save-point notification from the editor's placeholder document marking the
    // active image tab dirty, which would make closing it ask to save an image.
    let mut tabs = Tabs::with_document(Document::image_fixture(
        DocumentId(1),
        std::path::PathBuf::from(r"C:\pictures\a.png"),
    ));
    assert!(!tabs.set_active_dirty(true));
    assert!(!tabs.active().unwrap().dirty);
    assert!(tabs.active_handle().is_none());
    assert_eq!(tabs.active().unwrap().title(), "a.png");
}
```

- [ ] **Step 3: Fix the `.handle` sites in `src/window/main_window.rs`**

The compiler lists them.
- **3571 (`open_path_placed`), 3806 (new untitled), 5349 (`open_snapshot_tab`):** replace `editor.use_document(&document.handle)` with `document.expect_text().and_then(|handle| editor.use_document(handle))`.
- **4512, 4636, 4672, 4751** (inactive-document swaps; text tabs only): replace each `X.handle.clone()` with `X.text_handle()?.clone()` inside the existing `Option` closure. Where the code is `.then(|| (document.handle.clone(), active.handle.clone()))`, write:

  ```rust
  .then_some(())
  .and_then(|()| Some((document.text_handle()?.clone(), active.text_handle()?.clone())))
  ```

  An image target or an image active tab then makes these helpers return `None`, which their callers already treat as "nothing to read". Only dirty tabs and search overlays use them, and image tabs are never either.
- **`activate_document` (3854)**: an image tab has no handle, so the editor gets a blank placeholder. Replace the `target` block and the `use_document` call with:

  ```rust
  let target = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
      let app = unsafe { app.as_mut() };
      let editor = app.editor.clone()?;
      app.tabs.activate(id).ok()?;
      Some((editor, app.tabs.active_handle().cloned()))
  });
  let Some((editor, handle)) = target else {
      return false;
  };
  // An image tab has no text: the hidden editor holds an empty placeholder, as with no tab open.
  let handle = match handle {
      Some(handle) => handle,
      None => match editor.create_document() {
          Ok(blank) => blank,
          Err(_) => return false,
      },
  };
  if editor.use_document(&handle).is_err() || !identity.is_live_for(hwnd) {
      return false;
  }
  refresh_tabs(hwnd);
  crate::window::image_host::check_disk(hwnd); // added in Part 5; leave this line for Part 5
  true
  ```

  Leave the `check_disk` line out until Part 5 creates `image_host`, then add it.

- [ ] **Step 4: Clippy, then commit**

```bash
git add src/document.rs src/window/tabs.rs src/window/main_window.rs
git commit -m "feat(image-preview): a tab holds text or an image"
```

---

### Part 3: Decoding (`src/image_view/decode.rs`) and SVG from text

**Files:**
- Create: `src/image_view/mod.rs` (for now only `pub mod decode; pub mod zoom;`), `src/image_view/decode.rs`, `src/image_view/zoom.rs`
- Modify: `src/lib.rs` (add `pub mod image_view;` next to `pub mod preview;`)
- Modify: `src/preview/svg.rs` (split out `rasterize_source`)
- Modify: `src/preview/images.rs` (make the `ComScope` struct and its `enter` `pub(crate)`; move `PNG_2X2` to module level as `#[cfg(test)] pub(crate) const PNG_2X2`, and make the tests use `super::PNG_2X2`)
- Modify: `Cargo.toml`: add `"Win32_System_Com_StructuredStorage"` and `"Win32_System_Variant"` to the **`windows`** crate features, for `PROPVARIANT`

**Interfaces:**
- Produces (decode):
  - `ImageError { TooLarge, NoCodec, Damaged, Missing, SvgTooLarge, Read(String) }`, with `message(&self) -> String`
  - `FullImage { width, height, natural_width, natural_height: u32, format: &'static str, pixels: Vec<u8> }`
  - `decode_file(&Path, max_side: u32) -> Result<FullImage, ImageError>`
  - `decode_svg_source(&str, target_width: u32, max_side: u32) -> Result<FullImage, ImageError>`
  - `Source { File(PathBuf), Svg { text: Arc<str>, width: u32 } }`
  - `spawn(generation: u64, source: Source, max_side: u32, notify: HWND, message: u32)`
  - `Decoded { generation: u64, result: Result<FullImage, ImageError> }` (boxed in `lparam`)
- Produces (zoom):
  - `STEPS`
  - `Zoom { Fit, Scale(f32) }`
  - `fit_scale((f32,f32),(f32,f32)) -> f32`
  - `step_in(f32) -> f32` and `step_out(f32) -> f32`
  - `clamp_axis(offset, image_len, view_len) -> f32`
  - `zoom_about(offset:(f32,f32), old:f32, new:f32, anchor:(f32,f32)) -> (f32,f32)`

- [ ] **Step 1: Split the SVG rasterizer (`src/preview/svg.rs`)**

Move the lines from `let document = rewrite_for_direct2d(&source);` through `drop(module);` into:

```rust
/// Rasterizes SVG `source` to `width`×`height` premultiplied BGRA pixels, scaling its natural
/// size to fit exactly. Runs on a worker thread.
pub(crate) fn rasterize_source(
    wic: &IWICImagingFactory,
    source: &str,
    natural: (f32, f32),
    (width, height): (u32, u32),
) -> Result<Vec<u8>> {
    let document = rewrite_for_direct2d(source);
    let module = load_system_library("d2d1.dll")?;
    let pixels = {
        let factory = create_d2d_factory(&module, D2D1_FACTORY_TYPE_MULTI_THREADED)?;
        rasterize(wic, &factory, document.as_bytes(), natural, (width, height))?
    };
    drop(module);
    Ok(pixels)
}
```

`decode_svg` then calls `let pixels = rasterize_source(wic, &source, natural, (width, height))?;`. Its behaviour doesn't change, and the existing SVG tests cover it.

- [ ] **Step 2: Write `src/image_view/zoom.rs` (pure) with its tests**

```rust
//! Zoom and pan arithmetic for the image view (image preview spec §6.3). Scales are image pixels
//! to device pixels: 1.0 shows one image pixel per screen pixel. Offsets are the image's top-left
//! corner in the view's client pixels.

pub const STEPS: [f32; 11] = [0.10, 0.25, 0.50, 0.67, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0, 16.0];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// Fit the view, never above 100%.
    Fit,
    Scale(f32),
}

pub fn fit_scale(image: (f32, f32), view: (f32, f32)) -> f32 {
    if image.0 <= 0.0 || image.1 <= 0.0 {
        return 1.0;
    }
    (view.0 / image.0).min(view.1 / image.1).clamp(STEPS[0] / 100.0, 1.0)
}

pub fn step_in(current: f32) -> f32 {
    STEPS.iter().copied().find(|step| *step > current * 1.001).unwrap_or(STEPS[STEPS.len() - 1])
}

pub fn step_out(current: f32) -> f32 {
    STEPS.iter().rev().copied().find(|step| *step < current * 0.999).unwrap_or(STEPS[0])
}

/// Centres an image smaller than the view; otherwise keeps it covering the view.
pub fn clamp_axis(offset: f32, image_len: f32, view_len: f32) -> f32 {
    if image_len <= view_len {
        ((view_len - image_len) / 2.0).round()
    } else {
        offset.clamp(view_len - image_len, 0.0)
    }
}

/// The offset that keeps the image point under `anchor` fixed while the scale goes old → new.
pub fn zoom_about(offset: (f32, f32), old: f32, new: f32, anchor: (f32, f32)) -> (f32, f32) {
    let ratio = new / old;
    (
        anchor.0 - (anchor.0 - offset.0) * ratio,
        anchor.1 - (anchor.1 - offset.1) * ratio,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shrinks_large_images_and_never_enlarges_small_ones() {
        // Break caught: a 100×50 icon blown up to fill the window, or a photo opening at 100%.
        assert_eq!(fit_scale((1600.0, 1200.0), (800.0, 900.0)), 0.5);
        assert_eq!(fit_scale((100.0, 50.0), (800.0, 600.0)), 1.0);
    }

    #[test]
    fn steps_move_to_the_next_listed_scale_from_any_scale() {
        // Break caught: zooming in from a fit scale of 0.37 jumping to 1.0 or staying put.
        assert_eq!(step_in(0.37), 0.50);
        assert_eq!(step_in(1.0), 1.5);
        assert_eq!(step_in(16.0), 16.0);
        assert_eq!(step_out(0.37), 0.25);
        assert_eq!(step_out(0.10), 0.10);
    }

    #[test]
    fn panning_is_clamped_and_small_images_stay_centred() {
        // Break caught: dragging a zoomed image off screen, or a small image stuck top-left.
        assert_eq!(clamp_axis(-5000.0, 2000.0, 800.0), -1200.0);
        assert_eq!(clamp_axis(300.0, 2000.0, 800.0), 0.0);
        assert_eq!(clamp_axis(-40.0, 200.0, 800.0), 300.0);
    }

    #[test]
    fn zooming_about_a_point_keeps_that_point_still() {
        // Break caught: Ctrl+wheel zoom drifting away from the pointer.
        let (x, _) = zoom_about((0.0, 0.0), 1.0, 2.0, (100.0, 100.0));
        assert_eq!(x, -100.0);
    }
}
```

- [ ] **Step 3: Write `src/image_view/decode.rs`**

```rust
//! Full-resolution decoding for the image view (image preview spec §6.2), on one short-lived
//! worker thread per request. Raster files go through WIC: the largest ICO frame, EXIF orientation
//! applied, downscaled only past the render target's maximum bitmap size. SVG source is rasterized
//! with Direct2D at the width the view asks for.

use crate::preview::images::{ComScope, MAX_IMAGE_PIXELS};
use crate::preview::svg::{MAX_SVG_BYTES, natural_size, rasterize_source};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::{GUID, Interface, PCWSTR, w};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// WINCODEC_ERR_COMPONENTNOTFOUND: no installed decoder accepts the bytes.
const COMPONENT_NOT_FOUND: i32 = 0x8898_2F50_u32 as i32;
/// Formats whose decoder is an optional Store extension.
const OPTIONAL_CODECS: [&str; 4] = ["webp", "heic", "heif", "avif"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageError {
    TooLarge,
    NoCodec,
    Damaged,
    Missing,
    SvgTooLarge,
    Read(String),
}

impl ImageError {
    pub fn message(&self) -> String {
        match self {
            Self::TooLarge => "The file is larger than 64 megapixels.".to_owned(),
            Self::NoCodec => "Windows has no decoder for this format.".to_owned(),
            Self::Damaged => "The file is damaged or not an image.".to_owned(),
            Self::Missing => "The file no longer exists.".to_owned(),
            Self::SvgTooLarge => "The SVG is larger than 8 MB.".to_owned(),
            Self::Read(text) => text.clone(),
        }
    }
}

pub struct FullImage {
    /// Decoded pixels (after orientation; smaller than natural only past `max_side`).
    pub width: u32,
    pub height: u32,
    /// Display size in image pixels, after orientation.
    pub natural_width: u32,
    pub natural_height: u32,
    pub format: &'static str,
    pub pixels: Vec<u8>,
}

pub enum Source {
    File(PathBuf),
    Svg { text: Arc<str>, width: u32 },
}

pub struct Decoded {
    pub generation: u64,
    pub result: Result<FullImage, ImageError>,
}

/// Decodes `source` on a new thread and posts `message` to `notify` with a `Box<Decoded>` in
/// `lparam`, which the receiver frees.
pub fn spawn(generation: u64, source: Source, max_side: u32, notify: HWND, message: u32) {
    let notify = notify as isize;
    std::thread::spawn(move || {
        let result = match &source {
            Source::File(path) => decode_file(path, max_side),
            Source::Svg { text, width } => decode_svg_source(text, *width, max_side),
        };
        let boxed = Box::into_raw(Box::new(Decoded { generation, result }));
        if unsafe { PostMessageW(notify as HWND, message, 0, boxed as isize) } == 0 {
            drop(unsafe { Box::from_raw(boxed) });
        }
    });
}

pub(crate) fn classify(code: i32, extension: Option<&str>) -> ImageError {
    let optional = extension.is_some_and(|extension| {
        OPTIONAL_CODECS.iter().any(|known| known.eq_ignore_ascii_case(extension))
    });
    match code {
        COMPONENT_NOT_FOUND if optional => ImageError::NoCodec,
        // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND as HRESULTs.
        code if code == 0x8007_0002_u32 as i32 || code == 0x8007_0003_u32 as i32 => {
            ImageError::Missing
        }
        code if (code as u32) & 0xFFFF_0000 == 0x8007_0000 => {
            ImageError::Read(windows::core::Error::from_hresult(windows::core::HRESULT(code)).message())
        }
        _ => ImageError::Damaged,
    }
}

/// The EXIF orientation (1–8) as a WIC transform, and whether it swaps width and height.
pub(crate) fn orientation_transform(value: u16) -> (WICBitmapTransformOptions, bool) {
    match value {
        2 => (WICBitmapTransformFlipHorizontal, false),
        3 => (WICBitmapTransformRotate180, false),
        4 => (WICBitmapTransformFlipVertical, false),
        5 => (WICBitmapTransformOptions(WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0), true),
        6 => (WICBitmapTransformRotate90, true),
        7 => (WICBitmapTransformOptions(WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0), true),
        8 => (WICBitmapTransformRotate270, true),
        _ => (WICBitmapTransformRotate0, false),
    }
}

/// Index of the frame with the most pixels (the first on ties).
pub(crate) fn largest_frame(sizes: &[(u32, u32)]) -> usize {
    sizes
        .iter()
        .enumerate()
        .fold((0, 0_u64), |best, (index, &(w, h))| {
            let area = u64::from(w) * u64::from(h);
            if area > best.1 { (index, area) } else { best }
        })
        .0
}

/// `(width, height)` scaled down to fit `max_side` on both axes, keeping the aspect ratio.
pub(crate) fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    if max_side == 0 || (width <= max_side && height <= max_side) {
        return (width, height);
    }
    let scale = f64::from(max_side) / f64::from(width.max(height));
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

pub(crate) fn container_name(guid: &GUID) -> Option<&'static str> {
    [
        (GUID_ContainerFormatPng, "PNG"),
        (GUID_ContainerFormatJpeg, "JPEG"),
        (GUID_ContainerFormatGif, "GIF"),
        (GUID_ContainerFormatBmp, "BMP"),
        (GUID_ContainerFormatIco, "ICO"),
        (GUID_ContainerFormatTiff, "TIFF"),
        (GUID_ContainerFormatWebp, "WebP"),
        (GUID_ContainerFormatHeif, "HEIF"),
    ]
    .iter()
    .find(|(known, _)| known == guid)
    .map(|(_, name)| *name)
}

fn wic_factory() -> Result<IWICImagingFactory, ImageError> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| ImageError::Read(error.message()))
}

pub fn decode_file(path: &Path, max_side: u32) -> Result<FullImage, ImageError> {
    let _com = ComScope::enter();
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned());
    let fail = |error: windows::core::Error| classify(error.code().0, extension.as_deref());
    if !path.exists() {
        return Err(ImageError::Missing);
    }
    let factory = wic_factory()?;
    let wide = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    unsafe {
        let decoder = factory
            .CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(fail)?;
        let format = decoder
            .GetContainerFormat()
            .ok()
            .and_then(|guid| container_name(&guid))
            .unwrap_or("Image");
        let count = decoder.GetFrameCount().map_err(fail)?.max(1);
        let index = if format == "ICO" {
            let sizes = (0..count)
                .map(|index| {
                    let (mut w, mut h) = (0, 0);
                    if let Ok(frame) = decoder.GetFrame(index) {
                        let _ = frame.GetSize(&mut w, &mut h);
                    }
                    (w, h)
                })
                .collect::<Vec<_>>();
            largest_frame(&sizes) as u32
        } else {
            0
        };
        let frame = decoder.GetFrame(index).map_err(fail)?;
        let (mut width, mut height) = (0_u32, 0_u32);
        frame.GetSize(&mut width, &mut height).map_err(fail)?;
        if width == 0 || height == 0 {
            return Err(ImageError::Damaged);
        }
        if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(ImageError::TooLarge);
        }
        let mut source: IWICBitmapSource = frame.cast().map_err(fail)?;
        let (transform, swaps) = orientation_transform(read_orientation(&frame));
        if transform != WICBitmapTransformRotate0 {
            let rotator = factory.CreateBitmapFlipRotator().map_err(fail)?;
            rotator.Initialize(&source, transform).map_err(fail)?;
            source = rotator.cast().map_err(fail)?;
            if swaps {
                (width, height) = (height, width);
            }
        }
        let (natural_width, natural_height) = (width, height);
        let (scaled_width, scaled_height) = fit_within(width, height, max_side);
        if (scaled_width, scaled_height) != (width, height) {
            let scaler = factory.CreateBitmapScaler().map_err(fail)?;
            scaler
                .Initialize(&source, scaled_width, scaled_height, WICBitmapInterpolationModeFant)
                .map_err(fail)?;
            source = scaler.cast().map_err(fail)?;
            (width, height) = (scaled_width, scaled_height);
        }
        let converter = factory.CreateFormatConverter().map_err(fail)?;
        converter
            .Initialize(&source, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(fail)?;
        let mut pixels = vec![0_u8; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels).map_err(fail)?;
        Ok(FullImage { width, height, natural_width, natural_height, format, pixels })
    }
}

/// EXIF orientation from `System.Photo.Orientation`, or 1 when the frame has none.
fn read_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    unsafe {
        let Ok(reader) = frame.GetMetadataQueryReader() else {
            return 1;
        };
        let mut value = windows::core::PROPVARIANT::default();
        if reader.GetMetadataByName(w!("System.Photo.Orientation"), Some(&mut value)).is_err() {
            return 1;
        }
        // windows-core converts a VT_UI2 variant; anything else reads as "no orientation".
        u16::try_from(&value).unwrap_or(1)
    }
}

pub fn decode_svg_source(text: &str, target_width: u32, max_side: u32) -> Result<FullImage, ImageError> {
    if text.len() as u64 > MAX_SVG_BYTES {
        return Err(ImageError::SvgTooLarge);
    }
    let natural = natural_size(text);
    let (natural_width, natural_height) = (natural.0.ceil() as u32, natural.1.ceil() as u32);
    if natural_width == 0 || natural_height == 0 {
        return Err(ImageError::Damaged);
    }
    let width = if target_width == 0 { natural_width } else { target_width };
    let height = ((u64::from(natural_height) * u64::from(width)) / u64::from(natural_width)).max(1) as u32;
    let (width, height) = fit_within(width, height, max_side);
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let _com = ComScope::enter();
    let factory = wic_factory()?;
    let pixels = rasterize_source(&factory, text, natural, (width, height)).map_err(|_| ImageError::Damaged)?;
    Ok(FullImage { width, height, natural_width, natural_height, format: "SVG", pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_maps_decoder_errors_to_messages() {
        // Break caught: a HEIC with no Store codec called "damaged", or a PNG that WIC can't
        // read called "no decoder".
        assert_eq!(classify(COMPONENT_NOT_FOUND, Some("HEIC")), ImageError::NoCodec);
        assert_eq!(classify(COMPONENT_NOT_FOUND, Some("png")), ImageError::Damaged);
        assert_eq!(classify(0x8007_0002_u32 as i32, Some("png")), ImageError::Missing);
        assert!(matches!(classify(0x8007_0005_u32 as i32, None), ImageError::Read(_)));
    }

    #[test]
    fn orientation_rotates_and_swaps_for_the_quarter_turns() {
        // Break caught: a portrait phone photo shown sideways, or stretched to landscape.
        assert_eq!(orientation_transform(6), (WICBitmapTransformRotate90, true));
        assert_eq!(orientation_transform(8), (WICBitmapTransformRotate270, true));
        assert_eq!(orientation_transform(3), (WICBitmapTransformRotate180, false));
        assert_eq!(orientation_transform(0), (WICBitmapTransformRotate0, false));
        assert!(orientation_transform(5).1 && orientation_transform(7).1);
    }

    #[test]
    fn the_largest_icon_frame_wins_and_huge_images_fit_the_maximum_bitmap() {
        // Break caught: an .ico shown as its 16×16 frame, or a 20000-px panorama failing to create
        // a Direct2D bitmap.
        assert_eq!(largest_frame(&[(16, 16), (256, 256), (32, 32)]), 1);
        assert_eq!(largest_frame(&[]), 0);
        assert_eq!(fit_within(20_000, 5_000, 16_384), (16_384, 4_096));
        assert_eq!(fit_within(800, 600, 16_384), (800, 600));
    }

    #[test]
    fn decoding_a_png_and_an_svg_gives_premultiplied_pixels_and_their_format() {
        // Break caught: the viewer receiving a scaled or unformatted decode.
        let path = std::env::temp_dir().join(format!("fastpad-full-{}.png", std::process::id()));
        std::fs::write(&path, crate::preview::images::PNG_2X2).unwrap();
        let image = decode_file(&path, 16_384).unwrap();
        assert_eq!((image.width, image.height, image.format), (2, 2, "PNG"));
        assert_eq!(image.pixels.len(), 16);
        std::fs::remove_file(&path).unwrap();
        let svg = decode_svg_source(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20"/></svg>"#,
            80,
            16_384,
        )
        .unwrap();
        assert_eq!((svg.width, svg.height, svg.natural_width), (80, 40, 40));
        assert_eq!(decode_file(&std::env::temp_dir().join("fastpad-missing.png"), 0).err(), Some(ImageError::Missing));
    }
}
```

If `u16::try_from(&value)` doesn't compile against windows-core 0.62, read the raw variant instead: `let raw = value.as_raw(); if raw.Anonymous.Anonymous.vt == 18 /* VT_UI2 */ { raw.Anonymous.Anonymous.Anonymous.uiVal } else { 1 }`. `glob` imports from `Imaging::*` may trip `clippy::wildcard_imports`; list the names explicitly if so.

- [ ] **Step 4: Clippy, then CHECKPOINT A (targeted tests, one run)**

Run:
```
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- image_view:: library::title:: file::sniff:: languages:: document:: preview::svg:: preview::images::
```
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/lib.rs src/image_view src/preview/svg.rs src/preview/images.rs
git commit -m "feat(image-preview): full-resolution decoding, orientation, zoom arithmetic"
```

---

### Part 4: The image view window (`src/image_view/mod.rs`, `paint.rs`, `accessible.rs`)

**Files:**
- Modify: `src/image_view/mod.rs` (becomes the window)
- Create: `src/image_view/paint.rs`, `src/image_view/accessible.rs`
- Modify: `src/window/messages.rs` (add two constants), `src/window/mod.rs` (re-export them)

**Interfaces:**
- Consumes: `decode::{spawn, Source, Decoded, FullImage, ImageError}`, `zoom::*`, `preview::dwrite::Graphics`, `preview::render::{create_hwnd_target, color_f}`, `preview::colors::PreviewColors`
- Produces:
  - `ImageView` (Copy handle), with:
    - `create(parent: HWND, graphics: Rc<Graphics>, colors: PreviewColors, high_contrast: bool) -> Result<Self>`
    - `hwnd()` and `destroy()`
    - `show_file(&self, path: &Path, stamp: Option<DiskStamp>, name: &str)`
    - `show_svg(&self, text: Arc<str>, name: &str)`
    - `show_error(&self, error: ImageError)`
    - `release(&self)`
    - `zoom_in()`, `zoom_out()`, `zoom_reset()`, `toggle_actual_size()`
    - `set_appearance(colors, high_contrast)`
    - `status(&self) -> ImageViewStatus`
    - `stats(&self) -> ImageStats`
  - `ImageViewStatus { size: Option<(u32,u32)>, format: Option<&'static str>, zoom_percent: Option<u32>, failed: bool }`
  - `ImageStats { phase: Phase, scale: f32, offset: (f32,f32), decodes: u32, has_target: bool, svg_error: bool }`
  - `Phase { Empty, Loading, Ready, Failed }`
  - `#[cfg(test)] accessible_text(&self) -> (String, String)`
  - `window::WM_FASTPAD_IMAGE_DECODED = WM_APP + 0x58` (to the view) and `window::WM_FASTPAD_IMAGE_STATUS = WM_APP + 0x59` (posted to the parent when size, zoom or state changes)

- [ ] **Step 1: Add the messages**

In `src/window/messages.rs`, after `WM_FASTPAD_PREVIEW_ACTIVATE`:

```rust
/// A finished image decode, sent to the image view; `lparam` is a `Box<image_view::decode::Decoded>`
/// the receiver frees. Not part of the deferred chain.
pub const WM_FASTPAD_IMAGE_DECODED: u32 = WM_APP + 0x58;
/// Posted to the main window when an image view's size, zoom or state changes, so the status bar
/// repaints. Not part of the deferred chain.
pub const WM_FASTPAD_IMAGE_STATUS: u32 = WM_APP + 0x59;
```

Add both to the `pub use messages::{…}` list in `src/window/mod.rs`, and to the uniqueness test's list in `messages.rs` if it enumerates values.

- [ ] **Step 2: Write the view state and window (`src/image_view/mod.rs`)**

Follow the `preview::view` structure, and keep `graphics` as the last field so it drops last:

```rust
//! The image view (image preview spec §6): a lazily created Direct2D child window showing one
//! image with fit, zoom and pan. Image tabs (`window::image_host`) and the SVG preview
//! (`window::preview_host`) each own one.

pub mod accessible;
pub mod decode;
mod paint;
pub mod zoom;

use decode::{Decoded, FullImage, ImageError, Source};
use zoom::Zoom;
// … windows-sys imports as preview::view uses them …

const CLASS_NAME: &str = "FastPadImageView";
const LOADING_TIMER: usize = 1;
const SVG_TIMER: usize = 2;
const LOADING_DELAY_MS: u32 = 150;
const DEFAULT_MAX_SIDE: u32 = 16_384;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Phase { #[default] Empty, Loading, Ready, Failed }

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImageStats {
    pub phase: Phase,
    pub scale: f32,
    pub offset: (f32, f32),
    pub decodes: u32,
    pub has_target: bool,
    pub svg_error: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageViewStatus {
    pub size: Option<(u32, u32)>,
    pub format: Option<&'static str>,
    pub zoom_percent: Option<u32>,
    pub failed: bool,
}

/// What the view was last asked to show, so repeating the request does not decode again.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    File { path: PathBuf, stamp: Option<crate::library::DiskStamp> },
    Svg(Arc<str>),
}

struct ViewState {
    hwnd: HWND,
    target: Option<ID2D1HwndRenderTarget>,
    bitmap: Option<ID2D1Bitmap>,
    checker: Option<ID2D1BitmapBrush>,
    colors: PreviewColors,
    high_contrast: bool,
    name: String,
    shown: Option<Shown>,
    generation: u64,
    image: Option<FullImage>,
    error: Option<ImageError>,
    /// The last SVG render failed while an earlier image is still shown.
    svg_error: bool,
    loading: bool,
    loading_visible: bool,
    zoom: Zoom,
    scale: f32,
    offset: (f32, f32),
    drag: Option<((i32, i32), (f32, f32))>,
    max_side: u32,
    decodes: u32,
    accessible: Arc<RwLock<(String, String)>>,
    graphics: Rc<Graphics>,
}
```

Behaviour to implement:
- **`create`.**
  - Register the class with `CS_DBLCLKS`, `hCursor: IDC_ARROW` and `preview`'s `ERROR_CLASS_ALREADY_EXISTS` tolerance.
  - Create the window as `WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP | WS_HSCROLL | WS_VSCROLL`, hidden.
  - Box the state into `GWLP_USERDATA`.
- **`show_file(path, stamp, name)`.**
  - If `shown == Some(File{path, stamp})` and `image.is_some()`, return without decoding.
  - Otherwise, when the path differs (a new file), reset `zoom = Zoom::Fit`. When only the stamp changed, keep `zoom`.
  - Then:
    - `generation += 1`;
    - `error = None`;
    - `loading = true`;
    - `SetTimer(hwnd, LOADING_TIMER, LOADING_DELAY_MS)`;
    - `decode::spawn(generation, Source::File(path), max_side, hwnd, WM_FASTPAD_IMAGE_DECODED)`.
  - Update `name` and the accessible text, and post the status message.
  - Keep the old `image` until the new result lands, so a reload doesn't flash.
- **`show_svg(text, name)`.**
  - If `shown == Some(Svg(text))` with an equal `Arc` string, return.
  - Otherwise `generation += 1`, then spawn `Source::Svg { text, width: svg_width(state) }`.
  - `svg_width` is `(natural_width * scale).round()` once an image exists. Before the first render it is 0 (natural size).
  - Keep the old image while rendering.
- **`WM_FASTPAD_IMAGE_DECODED`.**
  - `let decoded = unsafe { Box::from_raw(lparam as *mut Decoded) };`, then ignore the result unless `decoded.generation == generation`.
  - Kill `LOADING_TIMER` and set `loading = false`, `decodes += 1`.
  - `Ok(image)`: set `image`, `bitmap = None`, `error = None`, `svg_error = false`, and recompute scale and offset.
  - `Err(e)` for an SVG source while an image exists: set `svg_error = true`.
  - `Err(e)` otherwise: set `error = Some(e)` and `image = None`.
  - Invalidate, update the accessible text, and post `WM_FASTPAD_IMAGE_STATUS` to `GetParent(hwnd)`.
- **`show_error(e)`.** Sets the failed state directly, used for "SVG too large" and missing files.
- **Scale.**
  - `recompute(state)` sets `scale = match zoom { Fit => fit_scale(natural, client), Scale(s) => s }`.
  - It then clamps `offset` on each axis with `clamp_axis(offset, natural * scale, client)`, and updates the scroll bars.
  - Update the scroll bars with `SetScrollInfo(SB_HORZ/SB_VERT, SIF_RANGE | SIF_PAGE | SIF_POS)`:
    - `nMin = 0`;
    - `nMax = image_len - 1`;
    - `nPage = client_len`;
    - `nPos = -offset`.

    A zero range hides the bar.
- **Zoom.**
  - `zoom_in`/`zoom_out` do `Zoom::Scale(step_in/step_out(scale))`, anchored at the client centre with `zoom_about`.
  - `zoom_reset` does `Zoom::Fit`.
  - `toggle_actual_size(anchor)` switches `Fit` → `Scale(1.0)` about `anchor`, and anything else → `Fit`.
  - After any zoom on an SVG source, `SetTimer(hwnd, SVG_TIMER, PREVIEW_UPDATE_DELAY_MS)`. `WM_TIMER(SVG_TIMER)` kills the timer and, if `svg_width` differs from the bitmap's width by more than 1%, spawns a new SVG render.
  - Post the status message after every zoom.
- **`WM_TIMER(LOADING_TIMER)`:** kill it, and if still `loading`, set `loading_visible = true` and invalidate.
- **`WM_MOUSEWHEEL`:**
  - With `MK_CONTROL`: zoom about the pointer (`ScreenToClient` on `lparam`), stepping in for `delta > 0` and out otherwise.
  - With `MK_SHIFT`: pan horizontally by `delta / 120 * 48 * dpi/96`.
  - Otherwise: pan vertically by the same amount.
- **`WM_MOUSEHWHEEL`:** pan horizontally.
- **`WM_LBUTTONDOWN`:** `SetFocus(hwnd)`. If the image is larger than the view on any axis, `SetCapture` and record the drag start.
- **`WM_MOUSEMOVE`:** while dragging, set `offset = start + delta`, clamp and invalidate.
- **`WM_LBUTTONUP` / `WM_CAPTURECHANGED`:** end the drag and `ReleaseCapture`.
- **`WM_LBUTTONDBLCLK`:** `toggle_actual_size(pointer)`.
- **`WM_SETCURSOR`** with `HTCLIENT`: `IDC_SIZEALL` when draggable, else `IDC_ARROW`. Return 1.
- **`WM_KEYDOWN`:**
  - arrows pan by 48 px (DPI-scaled);
  - `VK_PRIOR`/`VK_NEXT` pan by 90% of the client height;
  - `VK_HOME`/`VK_END` go to the top/bottom.
- **`WM_GETDLGCODE`:** return `DLGC_WANTARROWS`.
- **`WM_HSCROLL` / `WM_VSCROLL`:** handle `SB_LINE*` (48 px), `SB_PAGE*` (client), and `SB_THUMBTRACK`/`SB_THUMBPOSITION` via `GetScrollInfo(SIF_TRACKPOS)`.
- **`WM_SIZE`:** `target.Resize` (drop device resources on error), then `recompute` and invalidate.
- **`WM_DPICHANGED_AFTERPARENT`:** `recompute` and invalidate.
- **`WM_SETFOCUS` / `WM_KILLFOCUS`:** invalidate (focus ring).
- **`WM_ERASEBKGND`:** return 1.
- **`WM_PAINT`:** as in `preview::view`, which paints and then calls `ValidateRect`, retrying once on failure. Call `paint::paint(state)`. On `D2DERR_RECREATE_TARGET`, set `target = None; bitmap = None; checker = None` and invalidate.
- **`WM_GETOBJECT` with `OBJID_CLIENT`:** clone the `Arc` inside the borrow, then call `accessible::object_result(hwnd, text, wparam)`.
- **`WM_NCDESTROY`:** free the box, as in `preview::view`.
- **`release`:** `target = None; bitmap = None; checker = None`, and `KillTimer` for both timers. Keep `image` and `shown`, so re-showing the same tab paints without decoding (spec §3).
- **`stats`:** `has_target = target.is_some()`.
- **`status`:** `size` from `image` (`natural_width`, `natural_height`), `format`, `zoom_percent = (scale * 100).round()` while Ready, and `failed`.

The render target is created with **dpi = 96**, so DIPs equal device pixels. Text sizes are then scaled by `GetDpiForWindow(hwnd) / 96`.

- [ ] **Step 3: Write `src/image_view/paint.rs`**

```rust
//! Painting: background, checkerboard under the image's rectangle, the bitmap (linear below
//! 100%, nearest-neighbor above), a focus ring, and the loading, failed and SVG-error text.
```

Steps inside `pub(super) fn paint(state: &mut ViewState) -> windows::core::Result<()>`:

1. Get the client size. If `target` is `None`:
   - create it with `create_hwnd_target(&state.graphics, hwnd, w, h, 96)`;
   - on the first creation, set `state.max_side = target.GetMaximumBitmapSize().min(DEFAULT_MAX_SIDE)`. If it shrinks below the current image's decoded size, request `show_file` again.
2. `BeginDraw`, `SetTransform(identity)`, then `Clear(color_f(colors.background))`.
3. If the image exists:
   - Compute `dest = RectF::new(offset.0, offset.1, offset.0 + natural_w * scale, offset.1 + natural_h * scale)`.
   - Unless `high_contrast`, fill `dest ∩ client` with the checker brush:
     - lazily created from a 2×2 BGRA bitmap `[background, border; border, background]`;
     - `CreateBitmapBrush` with `D2D1_EXTEND_MODE_WRAP` on both axes and `D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR`;
     - brush transform `Matrix3x2::scale(cell, cell)`, where `cell = 8 * dpi / 96`.
   - Lazily create `bitmap` with `CreateBitmap` from the pixels, as `ImageCache::bitmap` does: B8G8R8A8 premultiplied, 96 DPI.
   - Call `DrawBitmap(bitmap, Some(&dest.to_d2d()), 1.0, mode, None)`. The mode is `NEAREST_NEIGHBOR` when `dest.width() > image.width as f32` (enlarged), else `LINEAR`.
4. If `error` is set, draw two centred lines with DWrite (`graphics.text_format("Segoe UI", 15 * dpi/96, SEMI_BOLD, NORMAL)`, then `NORMAL` weight for the second): "FastPad can't display this image", then `error.message()`. Use the `text` and `muted` colours.
5. Else if `loading_visible && image.is_none()`: draw centred "Loading…" in `muted`.
6. If `svg_error`: fill a band `28 * dpi/96` px tall at the top in `code_background` and draw "Can't render this SVG" in `text`.
7. If the view has focus: draw a 1-px `focus` rectangle inset by 1 px.
8. `EndDraw`, returning its error so `WM_PAINT` can handle `D2DERR_RECREATE_TARGET`.

Create the solid brushes per paint with `CreateSolidColorBrush`, as a handful of calls. The DWrite formats are cached in `ViewState` as `Option<IDWriteTextFormat>` fields (add `title_format`, `body_format`) and dropped in `release`.

- [ ] **Step 4: Write `src/image_view/accessible.rs`**

This is a root-only MSAA object. Copy the struct layout, `query_interface`, `add_ref`, `release`, `create_provider` and `object_result` from `src/preview/accessible.rs`. Replace the `links` field with `text: Arc<RwLock<(String, String)>>` (name, value). Build `IMAGE_VTABLE: AccessibleVtable` with these entries:
- `get_acc_name`: the root returns `text.0`, via `allocate_bstr`.
- `get_acc_value`: the root returns `text.1`.
- `get_acc_role`: the root returns `ROLE_SYSTEM_GRAPHIC` (0x28).
- `get_acc_state`: `STATE_SYSTEM_READONLY | STATE_SYSTEM_FOCUSABLE`, plus `STATE_SYSTEM_FOCUSED` when `GetFocus() == hwnd`.
- `get_acc_child_count`: 0.
- `get_acc_child`: `E_INVALIDARG`.
- `acc_location`: the client rect in screen coordinates (as preview's root does).
- `acc_hit_test`: `CHILDID_SELF` when inside the rect.
- `acc_navigate`: `S_FALSE`.
- `get_acc_focus`: `CHILDID_SELF` when focused, else empty.
- `get_acc_selection` and `get_acc_default_action`: empty.
- `acc_select` and `acc_do_default_action`: `DISP_E_MEMBERNOTFOUND`.
- Description, help and keyboard shortcut: `empty_text`.
- `put_acc_name` / `put_acc_value`: `put_text` (`E_NOTIMPL`).
- The `IDispatch` entries use the stock helpers from `window::accessibility`.

Any child id other than `CHILDID_SELF` returns `E_INVALIDARG`.

Text rules, applied by `ViewState::update_accessible`:
- **Name.**
  - Ready: `"{name}, image, {w} by {h} pixels"`.
  - Loading: `"{name}, image, loading"`.
  - Failed: `"{name}, image, can't display: {message}"`.
- **Value.** Ready: `"Zoom {percent} percent"`. Otherwise empty.
- **Events.** When the value changes, `NotifyWinEvent(EVENT_OBJECT_VALUECHANGE, hwnd, OBJID_CLIENT, CHILDID_SELF)`. When the name changes, `EVENT_OBJECT_NAMECHANGE`.

- [ ] **Step 5: Clippy, then commit** (no test run; Part 6's target covers the view)

```bash
git add src/image_view src/window/messages.rs src/window/mod.rs
git commit -m "feat(image-preview): the image view window"
```

---

### Part 5: `window::image_host`: image tabs in the main window

**Files:**
- Create: `src/window/image_host.rs`; Modify: `src/window/mod.rs` (`pub(crate) mod image_host;`)
- Modify: `src/app.rs` (field `pub(crate) image: crate::window::image_host::ImageHost`, placed **before** `preview`; initialise with `Default::default()` in `App::new`)
- Modify: `src/window/preview_host.rs` (extract `pub(crate) fn shared_graphics(hwnd) -> Result<Rc<Graphics>>` from `ensure_view`, and use it there)
- Modify: `src/window/main_window.rs`, as listed in the steps

**Interfaces:**
- Consumes: `ImageView`, `preview_host::shared_graphics`, `preview_colors`, `Document::is_image`
- Produces:
  - `active_is_image(HWND) -> bool`
  - `shown_view_hwnd(HWND) -> Option<HWND>`
  - `sync(HWND)`
  - `layout(HWND, RECT)`
  - `zoom(HWND, CommandId) -> bool`
  - `check_disk(HWND)`
  - `status(HWND) -> Option<ImageStatus>`
  - `refresh_appearance(HWND)`
  - `open_image_placed(HWND, &Path, preview: bool) -> Result<()>` (in main_window.rs)

- [ ] **Step 1: Write `src/window/image_host.rs`**

```rust
//! Image tabs (image preview spec §5–§6): owns the image view, shows it for the active image tab,
//! and hides it and frees its device resources for text tabs.

use crate::image_view::ImageView;
use crate::window::commands::CommandId;
use crate::window::host_window;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{MoveWindow, SW_HIDE, SW_SHOWNA, ShowWindow};

#[derive(Debug, Default)]
pub(crate) struct ImageHost {
    pub(crate) view: Option<ImageView>,
    /// Set once Direct2D failed to load, so the notice appears once.
    unavailable: bool,
}

/// The status-bar facts of the active image tab.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImageStatus {
    pub size: Option<(u32, u32)>,
    pub format: Option<&'static str>,
    pub bytes: Option<u64>,
    pub zoom_percent: Option<u32>,
}

fn with_host<R>(hwnd: HWND, action: impl FnOnce(&mut ImageHost) -> R) -> Option<R> {
    unsafe { host_window::app_ptr(hwnd) }.map(|mut app| action(&mut unsafe { app.as_mut() }.image))
}

fn view(hwnd: HWND) -> Option<ImageView> {
    with_host(hwnd, |host| host.view).flatten()
}

/// The active tab's path, disk stamp and file name, when it is an image tab.
fn active_image(hwnd: HWND) -> Option<(std::path::PathBuf, Option<crate::library::DiskStamp>)> {
    let app = unsafe { host_window::app_ptr(hwnd) }?;
    let document = unsafe { app.as_ref() }.tabs.active()?;
    document.is_image().then(|| (document.path.clone(), document.disk_stamp)).and_then(|(path, stamp)| Some((path?, stamp)))
}

pub(crate) fn active_is_image(hwnd: HWND) -> bool {
    active_image(hwnd).is_some()
}

pub(crate) fn shown_view_hwnd(hwnd: HWND) -> Option<HWND> {
    active_is_image(hwnd).then(|| view(hwnd).map(|view| view.hwnd())).flatten()
}

fn ensure_view(hwnd: HWND) -> Option<ImageView> {
    if let Some(view) = view(hwnd) {
        return Some(view);
    }
    if with_host(hwnd, |host| host.unavailable).unwrap_or(true) {
        return None;
    }
    let created = crate::window::preview_host::shared_graphics(hwnd).and_then(|graphics| {
        let (colors, high_contrast) = crate::window::preview_host::image_colors(hwnd);
        ImageView::create(hwnd, graphics, colors, high_contrast)
    });
    match created {
        Ok(view) => {
            with_host(hwnd, |host| host.view = Some(view));
            Some(view)
        }
        Err(error) => {
            with_host(hwnd, |host| host.unavailable = true);
            host_window::push_notice(hwnd, format!("FastPad could not display images: {error}"));
            None
        }
    }
}

/// Follows tab changes: shows the view for an image tab and moves the keyboard focus onto it,
/// or hides it and frees its render target for a text tab (or no tab).
pub(crate) fn sync(hwnd: HWND) {
    let Some((path, stamp)) = active_image(hwnd) else {
        if let Some(view) = view(hwnd) {
            let had_focus = unsafe { GetFocus() } == view.hwnd();
            unsafe { ShowWindow(view.hwnd(), SW_HIDE) };
            view.release();
            if had_focus && let Some(editor) = unsafe { host_window::editor_hwnd(hwnd) } {
                unsafe { SetFocus(editor) };
            }
        }
        return;
    };
    let Some(view) = ensure_view(hwnd) else {
        return;
    };
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    view.show_file(&path, stamp, &name);
    host_window::layout_editor_and_find_bar(hwnd);
    unsafe { ShowWindow(view.hwnd(), SW_SHOWNA) };
    let focus = unsafe { GetFocus() };
    if focus == hwnd || unsafe { host_window::editor_hwnd(hwnd) } == Some(focus) {
        unsafe { SetFocus(view.hwnd()) };
    }
}

pub(crate) fn layout(hwnd: HWND, area: RECT) {
    if let Some(view) = view(hwnd).filter(|_| active_is_image(hwnd)) {
        unsafe {
            MoveWindow(view.hwnd(), area.left, area.top, area.right - area.left, area.bottom - area.top, 1)
        };
    }
}

/// Runs a zoom command on the active image tab; false when the active tab is not an image.
pub(crate) fn zoom(hwnd: HWND, command: CommandId) -> bool {
    let Some(view) = shown_view_hwnd(hwnd).and_then(|_| view(hwnd)) else {
        return false;
    };
    match command {
        CommandId::ZoomIn => view.zoom_in(),
        CommandId::ZoomOut => view.zoom_out(),
        CommandId::ZoomReset => view.zoom_reset(),
        _ => return false,
    }
    true
}

/// Re-reads the active image tab's disk stamp. A changed file is decoded again; a deleted one
/// shows "The file no longer exists."
pub(crate) fn check_disk(hwnd: HWND) {
    let Some((path, known)) = active_image(hwnd) else {
        return;
    };
    let now = crate::library::disk_stamp(&path);
    if now == known {
        return;
    }
    if let Some(mut app) = unsafe { host_window::app_ptr(hwnd) }
        && let Some(document) = unsafe { app.as_mut() }.tabs.active_mut()
    {
        document.disk_stamp = now;
    }
    match (now, view(hwnd)) {
        (None, Some(view)) => view.show_error(crate::image_view::decode::ImageError::Missing),
        (Some(_), Some(_)) => sync(hwnd),
        _ => {}
    }
    host_window::invalidate_status_bar(hwnd);
}

pub(crate) fn status(hwnd: HWND) -> Option<ImageStatus> {
    let (_, stamp) = active_image(hwnd)?;
    let view_status = view(hwnd).map(|view| view.status()).unwrap_or_default();
    Some(ImageStatus {
        size: view_status.size,
        format: view_status.format,
        bytes: stamp.map(|stamp| stamp.size),
        zoom_percent: view_status.zoom_percent,
    })
}

pub(crate) fn refresh_appearance(hwnd: HWND) {
    if let Some(view) = view(hwnd) {
        let (colors, high_contrast) = crate::window::preview_host::image_colors(hwnd);
        view.set_appearance(colors, high_contrast);
    }
}
```

`Tabs` has no `active_mut` today. Add `pub(crate) fn active_mut(&mut self) -> Option<&mut Document>` next to `active()` in `src/window/tabs.rs`, mirroring it. `DiskStamp { size, modified }` is at `src/library/mod.rs:207`.

In `preview_host.rs`, add:

```rust
/// Loads Direct2D and DirectWrite once per window; the Markdown preview, the SVG preview and image
/// tabs share them.
pub(crate) fn shared_graphics(hwnd: HWND) -> Result<Rc<Graphics>> {
    if let Some(graphics) = with_host(hwnd, |host| host.graphics.clone()).flatten() {
        return Ok(graphics);
    }
    let graphics = Rc::new(Graphics::load()?);
    with_host(hwnd, |host| host.graphics = Some(Rc::clone(&graphics)));
    Ok(graphics)
}

/// The image view's colours: the preview palette for the current theme, and high contrast.
pub(crate) fn image_colors(hwnd: HWND) -> (PreviewColors, bool) {
    let (colors, ..) = appearance(hwnd);
    let high_contrast = unsafe { host_window::app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.theme.is_some_and(|theme| theme.high_contrast));
    (colors, high_contrast)
}
```

`ensure_view` then uses `let graphics = shared_graphics(hwnd)?;`.

- [ ] **Step 2: Branch `open_path_placed` and add `open_image_placed` (`src/window/main_window.rs`)**

After the `existing` block (line ~3503), before the autosave:

```rust
if crate::library::title::is_raster_image_path(path) {
    return open_image_placed(hwnd, path, preview);
}
```

Replace the load and NUL check:

```rust
let loaded = match crate::file::loader::load(path) {
    Err(crate::FastPadError::UnsupportedEncoding) if crate::file::sniff::file_looks_like_image(path) => {
        return open_image_placed(hwnd, path, preview);
    }
    loaded => loaded?,
};
// A NUL byte cannot round-trip through Scintilla's UTF-8 buffer: the file is unsupported.
if std::ffi::CString::new(loaded.text.as_str()).is_err() {
    return if crate::file::sniff::file_looks_like_image(path) {
        open_image_placed(hwnd, path, preview)
    } else {
        Err(crate::FastPadError::UnsupportedEncoding)
    };
}
```

Add after `open_path_placed`:

```rust
/// Opens `path` in an image tab (image preview spec §5). Like a text open it reuses an empty start
/// tab or the preview tab, but reads no bytes: the image view decodes on a worker.
fn open_image_placed(hwnd: HWND, path: &std::path::Path, preview: bool) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    if !path.is_file() {
        return Err(crate::FastPadError::Io(std::io::Error::from(std::io::ErrorKind::NotFound)));
    }
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant("main window was destroyed during file open"));
    }
    let stamp = crate::library::disk_stamp(path);
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app.editor.clone().ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let replace_preview = preview && app.tabs.preview_id().is_some();
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| !replace_preview && !active.is_image() && !active.dirty && active.path.is_none())
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
    let reused_ids = match candidate_ids {
        Some(ids) if editor.text()?.is_empty() => Some(ids),
        _ => None,
    };
    let reuse = reused_ids.is_some();
    let (id, recovery_id) = match reused_ids {
        Some(ids) => ids,
        None => {
            let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
                "main window app state was not available",
            ))?;
            unsafe { app.as_mut() }.allocate_document_identity()
        }
    };
    let mut document = Document::image(id, recovery_id, path.to_path_buf());
    document.preview = preview;
    let (commit, retired) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        if replace_preview {
            (Ok(()), app.tabs.replace_preview(document))
        } else if reuse {
            let retired = app.tabs.replace_active_untitled(document);
            let commit = if retired.is_some() {
                Ok(())
            } else {
                Err(crate::FastPadError::Invariant("the reused tab closed during file open"))
            };
            (commit, retired)
        } else {
            (
                app.tabs.push(document).map_err(|_| crate::FastPadError::Invariant("duplicate document path")),
                None,
            )
        }
    };
    commit?;
    // The retired tab's text document leaves the editor for an empty placeholder.
    let blank = editor.create_document()?;
    editor.use_document(&blank)?;
    drop(retired);
    unsafe {
        let _ = record_milestone(hwnd, Milestone::FileLoaded);
    }
    refresh_tabs(hwnd);
    crate::window::library_host::document_loaded(hwnd, stamp);
    Ok(())
}
```

- [ ] **Step 3: Wire the rest of `main_window.rs`**

1. **`refresh_tabs` (2373).**
   - Replace the editor show/hide conditions with `let text_active = count > 0 && !crate::window::image_host::active_is_image(hwnd);`.
   - Hide when `!text_active && visible`, and show when `text_active && !visible`. Keep the existing focus moves.
   - Call `crate::window::image_host::sync(hwnd);` right before `preview_host::sync_visibility(hwnd)`.
   - Only call `close_find_bar(hwnd)` when `count == 0 || image`.
2. **`preview_host::sync_visibility`** (the last editor `ShowWindow`): add `&& !crate::window::image_host::active_is_image(hwnd)` to the `tab_count(hwnd) > 0` condition.
3. **`layout_editor_and_find_bar` (1127).** After `let rects = crate::window::preview_host::layout(hwnd, area, dpi);`, add `crate::window::image_host::layout(hwnd, area);`.
4. **`content_focus_target` (1444).** Change it to:

   ```rust
   crate::window::image_host::shown_view_hwnd(hwnd)
       .or_else(|| crate::window::preview_host::full_view_hwnd(hwnd))
       .or_else(|| unsafe { editor_hwnd(hwnd) })
   ```

5. **`activate_document`.** Add the `crate::window::image_host::check_disk(hwnd);` line from Part 2 Step 3.
6. **`WM_ACTIVATEAPP` (296).** After `library_host::activation_changed(...)`, add `if wparam != 0 { crate::window::image_host::check_disk(hwnd); }`.
7. **wndproc.** Add `crate::window::WM_FASTPAD_IMAGE_STATUS => { invalidate_status_bar(hwnd); 0 }`.
8. **Theme refresh (2995, 3177).** After each `preview_host::refresh_appearance(hwnd);`, add `crate::window::image_host::refresh_appearance(hwnd);`.
9. **`apply_detected_language` (2854).** Start with `if crate::window::image_host::active_is_image(hwnd) { return; }`.
10. **`current_status_bar` (3269).** Before borrowing `app`, add:

    ```rust
    if let Some(image) = crate::window::image_host::status(hwnd) {
        let app = unsafe { app_ptr(hwnd) }?;
        let app = unsafe { app.as_ref() };
        app.status.as_ref()?;
        return Some(crate::window::status::image_status_bar_text(&app.notifications, &image));
    }
    ```

11. **`build_session` (5520).** Make it `pub(crate)` for the test. Only read Scintilla state when the active tab is text: add `&& !document.is_image()` for the active document, using the loop's binding.

- [ ] **Step 4: Status text (`src/window/status.rs`)**

```rust
/// The status bar of an image tab: notices on the left; size, format, file size and zoom on the
/// right (image preview spec §5).
pub fn image_status_bar_text(
    notifications: &NotificationCenter,
    image: &crate::window::image_host::ImageStatus,
) -> StatusBarText {
    let mut parts = Vec::new();
    if let Some((width, height)) = image.size {
        parts.push(format!("{width} \u{d7} {height}"));
    }
    if let Some(format) = image.format {
        parts.push(format.to_owned());
    }
    if let Some(bytes) = image.bytes {
        parts.push(file_size_text(bytes));
    }
    if let Some(zoom) = image.zoom_percent {
        parts.push(format!("{zoom}%"));
    }
    StatusBarText {
        left: status_text(notifications).unwrap_or_default(),
        right: parts.join(" \u{b7} "),
    }
}

pub fn file_size_text(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{} KB", (bytes + 512) / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}
```

`status_text(notifications)` is the existing left-side helper at status.rs:63. If its signature differs, use whatever `status_bar_text` calls for its left side. Add a unit test:

```rust
#[test]
fn an_image_status_lists_size_format_file_size_and_zoom() {
    // Break caught: an image tab showing a caret position and "Plain Text UTF-8".
    let image = crate::window::image_host::ImageStatus {
        size: Some((1920, 1080)),
        format: Some("PNG"),
        bytes: Some(250_880),
        zoom_percent: Some(50),
    };
    let text = image_status_bar_text(&NotificationCenter::default(), &image);
    assert_eq!(text.right, "1920 \u{d7} 1080 \u{b7} PNG \u{b7} 245 KB \u{b7} 50%");
    assert_eq!(file_size_text(900), "900 bytes");
    assert_eq!(file_size_text(3_250_000), "3.1 MB");
}
```

- [ ] **Step 5: Commands, menus, palette and save guards**

1. **`src/window/commands.rs`.** Add:

   ```rust
   /// Commands that read or change a tab's text; an image tab has none (image preview spec §5).
   pub const TEXT_COMMANDS: [CommandId; 18] = [
       CommandId::Save, CommandId::SaveAs, CommandId::Undo, CommandId::Redo, CommandId::Cut,
       CommandId::Copy, CommandId::Paste, CommandId::Find, CommandId::FindNext,
       CommandId::FindPrevious, CommandId::Replace, CommandId::ValidateJson, CommandId::FormatJson,
       CommandId::LanguagePlainText, CommandId::LanguageJson, CommandId::LanguageMarkdown,
       CommandId::NoteReloadFromDisk, CommandId::NoteKeepMine,
   ];

   pub fn needs_text(self) -> bool {
       TEXT_COMMANDS.contains(&self)
   }
   ```

2. **Dispatcher (`execute_command_with_note`, 2460).** After the `needs_document` guard, add:

   ```rust
   if command.needs_text() && tree_note.is_none() && crate::window::image_host::active_is_image(hwnd) {
       return;
   }
   ```

   Before the existing zoom arms, add:

   ```rust
   CommandId::ZoomIn | CommandId::ZoomOut | CommandId::ZoomReset
       if crate::window::image_host::zoom(hwnd, command)
           || crate::window::preview_host::zoom_svg(hwnd, command) => {}
   ```

   `zoom_svg` is added in Part 6. Until then, use only the `image_host::zoom` guard.
3. **Palette filter (1855).** Add `let image = crate::window::image_host::active_is_image(hwnd);` and the clause `&& (!image || !command.needs_text())`.
4. **`src/window/menus.rs`.** Add:

   ```rust
   /// Grays the commands that need text while an image tab is active.
   pub(crate) fn set_text_commands_enabled(menu: HMENU, enabled: bool) {
       let state = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
       for command in crate::window::commands::TEXT_COMMANDS {
           unsafe { EnableMenuItem(menu, command as u32, state) };
       }
   }
   ```

   In `open_menu` (5780), for every menu index, before it is tracked, call `menus::set_text_commands_enabled(menu, !crate::window::image_host::active_is_image(hwnd));`.
5. **Save guards.** Each of these returns early when `crate::window::image_host::active_is_image(hwnd)`:
   - `library_host::save_command` and `save_as_command`: return at the top.
   - `save_active_document` and `save_active_document_as`: `return false;`.
   - `save_active_to`: `return SaveOutcome::Failed;` as its very first statement.

- [ ] **Step 6: Clippy, then commit**

```bash
git add src/app.rs src/window
git commit -m "feat(image-preview): open images in image tabs"
```

---

### Part 6: SVG preview, plus the integration test target (CHECKPOINT B)

**Files:**
- Modify: `src/window/preview_host.rs`
- Create: `tests/windows/image_preview.rs`; Modify: `Cargo.toml` (add a `[[test]] name = "image_preview"` entry)

**Interfaces:**
- Produces: `preview_host::zoom_svg(HWND, CommandId) -> bool`, and `PreviewHost.svg_view: Option<ImageView>` (declare it after `view`, before `graphics`)

- [ ] **Step 1: The SVG path in `preview_host.rs`**

1. **Fields.** Add to `PreviewHost`:
   - `pub(crate) svg_view: Option<ImageView>`;
   - `pub(crate) svg_document: Option<DocumentId>`.

   Initialise both to `None`, and add them to the `Debug` impl.
2. **`buttons_visible`.** `matches!(language, Language::Markdown | Language::Svg)`.
3. **SVG helpers.** Add:

   ```rust
   fn active_is_svg(hwnd: HWND) -> bool {
       active_document(hwnd).is_some_and(|(_, language, _)| language == Language::Svg)
   }

   fn svg_view(hwnd: HWND) -> Option<ImageView> {
       with_host(hwnd, |host| host.svg_view).flatten()
   }

   fn ensure_svg_view(hwnd: HWND) -> Result<ImageView> {
       if let Some(view) = svg_view(hwnd) {
           return Ok(view);
       }
       let graphics = shared_graphics(hwnd)?;
       let (colors, high_contrast) = image_colors(hwnd);
       let view = ImageView::create(hwnd, graphics, colors, high_contrast)?;
       with_host(hwnd, |host| {
           host.svg_view = Some(view);
           host.svg_document = None;
       });
       Ok(view)
   }

   /// Sends the SVG tab's current text to the SVG preview.
   fn load_svg(hwnd: HWND) {
       let (Some(view), Some(editor), Some((id, ..))) = (svg_view(hwnd), editor(hwnd), active_document(hwnd)) else {
           return;
       };
       with_host(hwnd, |host| host.svg_document = Some(id));
       if editor.length().unwrap_or(0) as u64 > crate::preview::svg::MAX_SVG_BYTES {
           view.show_error(crate::image_view::decode::ImageError::SvgTooLarge);
           return;
       }
       let name = host_window::active_title(hwnd).unwrap_or_default();
       if let Ok(text) = editor.text() {
           view.show_svg(std::sync::Arc::from(text), &name);
       }
   }

   pub(crate) fn zoom_svg(hwnd: HWND, command: CommandId) -> bool {
       let Some(view) = svg_view(hwnd).filter(|view| {
           active_is_svg(hwnd) && mode(hwnd) != PreviewMode::Off && unsafe { GetFocus() } == view.hwnd()
       }) else {
           return false;
       };
       match command {
           CommandId::ZoomIn => view.zoom_in(),
           CommandId::ZoomOut => view.zoom_out(),
           CommandId::ZoomReset => view.zoom_reset(),
           _ => return false,
       }
       true
   }
   ```

   `host_window::active_title` is whatever gives the active tab's file name. Use `tabs.active()?.title()` directly through `app_ptr` if there's no such helper.
4. **`sync_visibility`.**
   - Split the flag: `markdown` stays `language == Markdown`, and add `let svg = active_is_svg(hwnd);`.
   - Call `set_preview_buttons(markdown || svg)`.
   - After the Markdown block, before `let editor_hwnd`, add:

     ```rust
     let mut shown_svg = None;
     if wanted != PreviewMode::Off && svg {
         match ensure_svg_view(hwnd) {
             Ok(view) => shown_svg = Some(view),
             Err(error) => {
                 with_host(hwnd, |host| host.mode = PreviewMode::Off);
                 host_window::push_notice(hwnd, format!("FastPad could not open the SVG preview: {error}"));
             }
         }
     }
     if let Some(view) = svg_view(hwnd) {
         if shown_svg.is_some() {
             unsafe { ShowWindow(view.hwnd(), SW_SHOWNA) };
             let active = active_document(hwnd).map(|(id, ..)| id);
             if with_host(hwnd, |host| host.svg_document).flatten() != active {
                 load_svg(hwnd);
             }
         } else {
             with_host(hwnd, |host| host.svg_document = None);
             unsafe { ShowWindow(view.hwnd(), SW_HIDE) };
             view.release();
         }
     }
     ```

   - Then `let hide_editor = (shown || shown_svg.is_some()) && wanted == PreviewMode::Full;`. The focus move uses the SVG view's `hwnd` when it's the one shown.
5. **Shared view helpers.**
   - `preview_shown`: `mode(hwnd) != PreviewMode::Off && buttons_visible(hwnd) && (view(hwnd).is_some() || svg_view(hwnd).is_some())`.
   - `full_view_hwnd`: return the SVG view's hwnd when `active_is_svg`, else the Markdown view's.
   - `layout`: move `svg_view` to `rects.preview` when `active_is_svg`, else `view`.
   - `set_mode`'s focus target: `full_view_hwnd(hwnd)` after `sync_visibility`.
6. **`record_edit`.** At the start of the closure: `if host.svg_document.is_some() { return true; }`. This arms the timer without recording Markdown edits.
7. **`flush`.** Right after `KillTimer` and the `input_pending` check: `if active_is_svg(hwnd) { if with_host(hwnd, |h| h.svg_document).flatten().is_some() { load_svg(hwnd); } return; }`.
8. **`close_view`.** Also take and destroy `svg_view`, and set `svg_document = None`.
9. **`refresh_appearance`.** Also call `svg_view.set_appearance(image_colors(hwnd))`.
10. **`NOT_MARKDOWN_NOTICE`.** Change "for Markdown documents" to "for Markdown and SVG documents". Keep the rest of the string unchanged.
11. **Zoom dispatch.** Add the `preview_host::zoom_svg` guard to the zoom arm now (Part 5 Step 5.2).

- [ ] **Step 2: Write `tests/windows/image_preview.rs`**

Add to `Cargo.toml`:

```toml
[[test]]
name = "image_preview"
path = "tests/windows/image_preview.rs"
```

Copy `TestMain`, `pump_until`, `pump_pending` and `visible` verbatim from `tests/windows/markdown_preview.rs` (lines 26–180), keeping the same `#![cfg(windows)]`, `mod support;` and `include!("../../src/lib.rs");` header. Then add:

```rust
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fastpad-image-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a 24-bit bottom-up BMP of `width`×`height` grey pixels.
fn write_bmp(path: &std::path::Path, width: u32, height: u32) {
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut bytes = Vec::with_capacity(size as usize);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&(width as i32).to_le_bytes());
    bytes.extend_from_slice(&(height as i32).to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    bytes.resize(size as usize, 0x80);
    std::fs::write(path, bytes).unwrap();
}

impl TestMain {
    fn open(&self, path: &std::path::Path) -> crate::Result<()> {
        window::main_window::open_path(self.hwnd, path)
    }
    fn image_view(&self) -> image_view::ImageView {
        self.with_app(|app| app.image.view).expect("image view")
    }
    fn wait_ready(&self) -> image_view::ImageStats {
        let view = self.image_view();
        pump_until("image decoded", Duration::from_secs(5), || {
            matches!(view.stats().phase, image_view::Phase::Ready | image_view::Phase::Failed)
        });
        view.stats()
    }
}
```

Tests. Each starts with `let _scintilla = support::win32::WindowHarness::new().unwrap(); let main = TestMain::new();`.

```rust
#[test]
fn a_bmp_opens_in_an_image_tab_that_fits_the_window() {
    // Break caught: images refused with "unsupported text encoding", shown in the editor, or opened
    // at 100% overflowing the window.
    let dir = scratch("fit");
    let path = dir.join("big.bmp");
    write_bmp(&path, 4000, 3000);
    main.open(&path).unwrap();
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
    assert!(!visible(main.editor));
    let view = main.image_view();
    assert!(visible(view.hwnd()));
    let stats = main.wait_ready();
    assert_eq!(stats.phase, image_view::Phase::Ready);
    assert!(stats.scale < 1.0);
    assert_eq!(view.status().size, Some((4000, 3000)));
    assert_eq!(view.status().format, Some("BMP"));
    assert!(main.notices().is_empty());
}

#[test]
fn a_small_image_shows_at_100_percent_and_zoom_commands_step_and_reset() {
    // Break caught: small images enlarged to fit, Ctrl+plus zooming the hidden editor, or Ctrl+0
    // not returning to fit.
    let dir = scratch("zoom");
    let path = dir.join("small.bmp");
    write_bmp(&path, 64, 32);
    main.open(&path).unwrap();
    let view = main.image_view();
    assert_eq!(main.wait_ready().scale, 1.0);
    main.command(CommandId::ZoomIn);
    assert_eq!(view.stats().scale, 1.5);
    main.command(CommandId::ZoomOut);
    main.command(CommandId::ZoomOut);
    assert_eq!(view.stats().scale, 0.67);
    main.command(CommandId::ZoomReset);
    assert_eq!(view.stats().scale, 1.0);
    unsafe { SendMessageW(view.hwnd(), WM_LBUTTONDBLCLK, 0, 0) };
    assert_eq!(view.stats().scale, 1.0); // fit is already 100% for a small image
}

#[test]
fn saving_an_image_tab_never_writes_the_file() {
    // Break caught: Ctrl+S writing the empty placeholder document over the image.
    let dir = scratch("save");
    let path = dir.join("keep.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    main.wait_ready();
    for command in [CommandId::Save, CommandId::SaveAs, CommandId::Undo, CommandId::Paste, CommandId::Find] {
        main.command(command);
    }
    assert!(platform::dialogs::take_dialog_events().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), preview::images::PNG_2X2);
    assert!(!main.with_app(|app| app.tabs.active().unwrap().dirty));
    assert!(main.with_app(|app| app.find_bar.is_none()));
    main.command(CommandId::CloseTab);
    assert_eq!(main.with_app(|app| app.tabs.len()), 0);
    assert_eq!(std::fs::read(&path).unwrap(), preview::images::PNG_2X2);
}

#[test]
fn a_corrupt_png_shows_the_failed_state_without_a_notice() {
    // Break caught: a damaged image freezing the window, raising a notice, or closing its tab.
    let dir = scratch("corrupt");
    let path = dir.join("broken.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot really").unwrap();
    main.open(&path).unwrap();
    assert_eq!(main.wait_ready().phase, image_view::Phase::Failed);
    assert!(main.notices().is_empty());
    assert!(main.image_view().accessible_text().0.contains("can't display"));
}

#[test]
fn a_misnamed_image_opens_by_signature_and_other_binary_still_gets_the_notice() {
    // Break caught: a PNG saved as .dat refused, or every binary file opening as a broken image.
    let dir = scratch("sniff");
    let png = dir.join("photo.dat");
    std::fs::write(&png, preview::images::PNG_2X2).unwrap();
    main.open(&png).unwrap();
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
    let other = dir.join("blob.bin");
    std::fs::write(&other, [0xFF_u8, 0xFE, 0x00, 0xC3, 0x28]).unwrap();
    assert!(matches!(main.open(&other), Err(FastPadError::UnsupportedEncoding)));
}

#[test]
fn an_image_replaces_the_empty_start_tab() {
    // Break caught: "Untitled" left beside the first image opened.
    main.command(CommandId::New);
    let dir = scratch("reuse");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    assert_eq!(main.with_app(|app| app.tabs.len()), 1);
}

#[test]
fn opening_an_open_image_again_activates_its_tab() {
    // Break caught: a second open failing with "duplicate document path".
    let dir = scratch("twice");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    main.command(CommandId::New);
    main.open(&dir.join(".").join("a.png")).unwrap();
    assert_eq!(main.with_app(|app| app.tabs.len()), 2);
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
}

#[test]
fn switching_between_image_and_text_tabs_keeps_each_tab_intact() {
    // Break caught: the image tab turning dirty, the text tab losing its edits, the view keeping
    // its render target while hidden, or switching back decoding again.
    let dir = scratch("switch");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.command(CommandId::New);
    main.set_text("draft");
    main.open(&path).unwrap();
    let view = main.image_view();
    main.wait_ready();
    unsafe { SendMessageW(view.hwnd(), WM_PAINT, 0, 0) };
    assert!(view.stats().has_target);
    main.command(CommandId::PreviousTab);
    assert!(visible(main.editor) && !visible(view.hwnd()));
    assert!(!view.stats().has_target);
    assert_eq!(support::win32::scintilla_text(main.editor).unwrap(), "draft");
    let decodes = view.stats().decodes;
    main.command(CommandId::NextTab);
    assert!(visible(view.hwnd()));
    assert_eq!(view.stats().decodes, decodes);
    assert!(!main.with_app(|app| app.tabs.active().unwrap().dirty));
}

#[test]
fn a_changed_or_deleted_file_is_noticed_when_its_tab_is_activated() {
    // Break caught: a stale image after the file was edited elsewhere, or a deleted file still shown.
    let dir = scratch("disk");
    let path = dir.join("a.bmp");
    write_bmp(&path, 10, 10);
    main.open(&path).unwrap();
    let view = main.image_view();
    main.wait_ready();
    main.command(CommandId::New);
    write_bmp(&path, 20, 10);
    main.command(CommandId::PreviousTab);
    pump_until("redecode", Duration::from_secs(5), || view.status().size == Some((20, 10)));
    main.command(CommandId::NextTab);
    std::fs::remove_file(&path).unwrap();
    main.command(CommandId::PreviousTab);
    assert_eq!(view.stats().phase, image_view::Phase::Failed);
}

#[test]
fn an_image_tab_restores_from_the_session_as_its_file() {
    // Break caught: an image tab dropped from the session, or saved as a snapshot of empty text.
    let dir = scratch("session");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    let session = window::main_window::build_session(main.hwnd, &dir).unwrap();
    assert!(matches!(&session.entries[0].source, session::SessionSource::File(file) if file.ends_with("a.png")));
}

#[test]
fn an_svg_opens_as_text_and_the_preview_renders_and_follows_edits() {
    // Break caught: SVG opening in the image view (user chose text), the preview not rendering
    // it, or a typing mistake blanking the last good render.
    let dir = scratch("svg");
    let path = dir.join("logo.svg");
    std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20"/></svg>"#).unwrap();
    main.open(&path).unwrap();
    pump_pending();
    assert!(!main.with_app(|app| app.tabs.active().unwrap().is_image()));
    assert!(window::preview_host::buttons_visible(main.hwnd));
    main.command(CommandId::MarkdownPreviewCycle);
    let view = main.with_app(|app| app.preview.svg_view).expect("svg view");
    pump_until("svg rendered", Duration::from_secs(5), || view.stats().phase == image_view::Phase::Ready);
    assert_eq!(view.status().size, Some((40, 20)));
    main.set_text(r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="20"><rect width="80" height="20"/></svg>"#);
    pump_until("svg re-rendered", Duration::from_secs(5), || view.status().size == Some((80, 20)));
    main.set_text("<svg");
    pump_until("svg error bar", Duration::from_secs(5), || view.stats().svg_error);
    assert_eq!(view.status().size, Some((80, 20)));
}

#[test]
fn the_image_view_names_itself_and_its_zoom_for_screen_readers() {
    // Break caught: Narrator reading "Markdown preview" or nothing for an image.
    let dir = scratch("a11y");
    let path = dir.join("shot.bmp");
    write_bmp(&path, 64, 32);
    main.open(&path).unwrap();
    main.wait_ready();
    let (name, value) = main.image_view().accessible_text();
    assert_eq!(name, "shot.bmp, image, 64 by 32 pixels");
    assert_eq!(value, "Zoom 100 percent");
    main.command(CommandId::ZoomIn);
    assert_eq!(main.image_view().accessible_text().1, "Zoom 150 percent");
}

#[test]
fn launching_with_a_text_file_loads_no_preview_graphics_library() {
    // Break caught: image support loading Direct2D, DirectWrite or WIC at startup.
    // Body: copy `launching_with_a_markdown_file_loads_no_preview_graphics_library` from
    // tests/windows/markdown_preview.rs:876 verbatim, changing only the file name to "startup.txt"
    // and its contents to "plain text\n".
}
```

The last test's body is a verbatim copy with two literal changes, and the implementer has the source file.

`CloseTab`, `NextTab`, `PreviousTab` and `platform::dialogs::take_dialog_events` exist under these names (checked while planning).

- [ ] **Step 3: Clippy, then CHECKPOINT B (one run)**

```
cargo clippy --all-targets --all-features -- -D warnings
cargo build --bin fastpad
cargo test --test image_preview -- --test-threads=1
cargo test --lib -- window::status:: document::
```

Expected: all pass. Fix only what fails, and rerun only the failing test by name.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/window/preview_host.rs src/window/main_window.rs tests/windows/image_preview.rs
git commit -m "feat(image-preview): SVG preview and image tab integration tests"
```

---

### Part 7: The notebook lists images

**Files:**
- Modify: `src/library/scan.rs:151`, `src/library/mod.rs:368,532`, `src/window/copy_host.rs:422`: switch `is_note_extension` to `is_listed_extension`
- Modify: `src/library/title.rs`, `rename_parts`: its second branch uses `is_listed_extension`
- Modify: `src/window/inline_name.rs:161` (`draft_icon`): filter with `is_listed_extension`
- Modify: `src/window/text_search_host.rs:228-243` and `src/bin/fastpad-bench.rs:618`: search only notes, through a new pure function
- Modify: `src/library/text_search.rs` (add `search_notes`)

- [ ] **Step 1: Swap the filters**

- In each listed call, replace `title::is_note_extension(` (or `super::title::…` / `library::title::…`) with the matching `is_listed_extension(` path.
- In `copy_host.rs`, rename the local `is_note` to `is_listed`.
- The notice for types that are still hidden keeps its wording.

- [ ] **Step 2: Search reads notes only (`src/library/text_search.rs`)**

```rust
/// The notes a search reads: every listed text note, never an image (image preview spec §9).
pub fn search_notes<'a>(notes: impl IntoIterator<Item = &'a super::NoteEntry>) -> Vec<SearchNote> {
    notes
        .into_iter()
        .filter(|note| {
            note.path
                .extension()
                .is_some_and(|extension| super::title::is_note_extension(&extension.to_string_lossy()))
        })
        .map(SearchNote::from)
        .collect()
}
```

In `text_search_host.rs`, keep the `keep` filter and replace `.map(SearchNote::from).collect::<Vec<_>>()` with a call to `text_search::search_notes(...)` on the filtered iterator. Do the same in `fastpad-bench.rs`. Add a test in `text_search.rs`:

```rust
#[test]
fn search_never_reads_images_listed_in_the_notebook() {
    // Break caught: Search opening every PNG as text and reporting binary noise as matches.
    let entry = |path: &str| super::super::NoteEntry { path: path.into(), size: 1, mtime: 0, online_only: false };
    let notes = [entry("a.md"), entry(r"pics\b.png"), entry("c.svg"), entry("d.txt")];
    let paths = search_notes(&notes).into_iter().map(|note| note.path).collect::<Vec<_>>();
    assert_eq!(paths, [std::path::PathBuf::from("a.md"), "d.txt".into()]);
}
```

`NoteEntry`'s fields are `path, size, mtime, online_only`. If more exist, fill them with defaults.

- [ ] **Step 3: Scan and rename tests**

In `src/library/scan.rs` tests:

```rust
#[test]
fn images_are_listed_next_to_notes_and_other_files_are_not() {
    // Break caught: images missing from the tree, or executables and archives appearing in it.
    let scratch = Scratch::new("images");
    scratch.file("a.md", "a");
    scratch.file(r"pics\b.PNG", "x");
    scratch.file("c.svg", "<svg/>");
    scratch.file("d.exe", "x");
    let scan = scan(&scratch.0, NOTE_LIMIT).unwrap();
    assert_eq!(paths(&scan), ["a.md", "c.svg", r"pics\b.PNG"]);
}
```

`paths()` may not sort. If so, sort both sides.

In `src/library/title.rs` tests:

```rust
#[test]
fn renaming_an_image_keeps_or_changes_its_image_extension() {
    // Break caught: "x.jpg" typed on pic.png becoming "x.jpg.png", or a bare name losing ".png".
    assert_eq!(renamed_note_name("x", Some("png")).as_deref(), Some("x.png"));
    assert_eq!(renamed_note_name("x.jpg", Some("png")).as_deref(), Some("x.jpg"));
    assert_eq!(new_note_name("x.png").as_deref(), Some("x.png.md"));
}
```

Check the last assertion against `new_note_name`'s existing behaviour for an unknown extension. `file_name(stem, "md")` gives `x.png.md` only if `clean_stem` keeps dots. If the existing tests show otherwise, assert the existing behaviour. The point of that line is that New note never creates an image.

- [ ] **Step 4: Clippy, then commit**

```bash
git add src/library src/window/copy_host.rs src/window/inline_name.rs src/window/text_search_host.rs src/bin/fastpad-bench.rs
git commit -m "feat(notebook): list images in the tree and quick open; search skips them"
```

---

### Part 8: Image icons in all three sets (CHECKPOINT C)

**Files:**
- Modify: `src/window/file_icons.rs`, `src/window/icon_sets/mod.rs`, `src/window/icon_sets/material.rs`, `src/window/icon_sets/masks.rs`
- Create: `assets/icons/material/svg/image.svg` (upstream), `assets/icons/minimal/svg/image.svg`, `assets/icons/solid/svg/image.svg`
- Modify: `assets/icons/material/SOURCE.md`, `assets/icons/README.md`
- Regenerate: `assets/icons/{material,minimal,solid}/icons.bin` and `icons.source-hash`

- [ ] **Step 1: `file_icons.rs`**

- Add `Image` to `NoteKind`.
- Rename `NOTE_KINDS` to `FILE_KINDS: [(&str, NoteKind); 30]`: the 14 existing entries plus all 16 `IMAGE_EXTENSIONS` mapped to `NoteKind::Image`.
- `note_kind` reads `FILE_KINDS`.
- `type_name` gets `NoteKind::Image => "image"`.
- The test iterates `NOTE_EXTENSIONS.iter().chain(IMAGE_EXTENSIONS.iter())`, and its fixed table adds `("png", "image")` and `("svg", "image")`.

- [ ] **Step 2: Icon mappings**

- `material.rs`:
  - Add `Image` **after `FolderOpen`**, so every existing slot keeps its discriminant.
  - `ALL: [Self; 13]` ends with `Self::Image`.
  - `file_name` gets `Self::Image => "image.svg"`.
- `icon_sets/mod.rs`: `NoteKind::Image => MaterialIcon::Image`.
- `masks.rs`:
  - Add `Image` after `Log`.
  - `ALL: [Self; 10]`.
  - `file_name` gets `"image.svg"`.
  - `mask_icon` gets `NoteKind::Image => (MaskIcon::Image, IconColor::Green)`.
- In the tests' extension tables (`icon_sets/mod.rs:91`, `masks.rs:227`), add `("png", MaterialIcon::Image)` and `("png", MaskIcon::Image, IconColor::Green)` in their row shapes. Add `MaskIcon::Image` to `minimal_outlines_are_lighter_than_solid_shapes`'s list.

- [ ] **Step 3: The SVGs**

Material, copied unchanged from upstream:

```
curl -sL https://cdn.jsdelivr.net/npm/material-icon-theme@5.38.1/icons/image.svg -o assets/icons/material/svg/image.svg
```

Check the root element has no `width`/`height`, because the generator asserts that. If the download fails, stop and report. Don't hand-draw a Material icon.

Add a row to `assets/icons/material/SOURCE.md`'s table: `| image.svg | icons/image.svg |`, in the same column format as the others.

`assets/icons/minimal/svg/image.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" d="M5.25 3.75h13.5c.8 0 1.5.7 1.5 1.5v13.5c0 .8-.7 1.5-1.5 1.5H5.25c-.8 0-1.5-.7-1.5-1.5V5.25c0-.8.7-1.5 1.5-1.5ZM3.75 16.5l4.5-4.5 3.5 3.5 2.5-2.5 6 6M15.5 6.75a1.75 1.75 0 1 0 0 3.5 1.75 1.75 0 0 0 0-3.5Z"/></svg>
```

`assets/icons/solid/svg/image.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000" fill-rule="evenodd" d="M5.5 3h13A2.5 2.5 0 0 1 21 5.5v13a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 18.5v-13A2.5 2.5 0 0 1 5.5 3ZM5 5v8.4l4-4 4.5 4.5 2.5-2.5 3 3V5ZM15.5 6.2a1.6 1.6 0 1 0 0 3.2 1.6 1.6 0 0 0 0-3.2Z"/></svg>
```

In `assets/icons/README.md`:
- Change "the same nine files" to "the same ten files".
- Add `image.svg`: images (png, jpg, gif, bmp, ico, tiff, webp, heic, avif, svg) to its list of which types use each file.

- [ ] **Step 4: Regenerate and CHECKPOINT C (one run)**

```
powershell -File tools/generate-file-icons.ps1
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- window::icon_sets:: window::file_icons:: library:: window::inline_name::
```

Expected: all pass. The coverage-range test in `masks.rs` checks that each mask covers between 1/20 and 3/4 of its square; the SVGs above are drawn to fit. If it fails, adjust the SVG, never the test.

- [ ] **Step 5: Commit**

```bash
git add assets/icons src/window/file_icons.rs src/window/icon_sets
git commit -m "feat(notebook): an image icon in the Material, Minimal and Solid sets"
```

---

### Part 9: Docs, final review, full suite

**Files:**
- Modify: `README.md` (next to the encodings list at ~line 199)

- [ ] **Step 1: README**

Add under the file-support section:

```markdown
**Images.** PNG, JPEG, GIF (first frame), BMP, ICO, TIFF, and WebP/HEIC/AVIF when Windows has the
codec open in an image tab: it fits the window, Ctrl + wheel or Ctrl +/− zooms, Ctrl+0 fits
again, double-click toggles 100%, and drag or the arrow keys pan. SVG files open as text; press
Ctrl+Shift+V to see them rendered beside or instead of the source. The notebook lists images next
to notes; Search reads notes only.
```

Commit: `git commit -am "docs: image preview"`.

- [ ] **Step 2: Run the whole-branch review once**

Invoke `superpowers:requesting-code-review` once for `main..feat/image-preview`. Point the reviewer at the spec (§13 included) and at this plan's **Review Focus** list. Fix what it confirms in one pass.

- [ ] **Step 3: Run the full suite once**

```
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets -- --test-threads=1
```

This must pass with 0 failed. If the user's FastPad is running, use a separate `CARGO_TARGET_DIR`, because the running exe locks `target/debug/fastpad.exe`. Rerun only a failing test by name, and don't rerun the whole suite unless a fix touched shared code.

- [ ] **Step 4: Push and open a draft PR**

```bash
git -c credential.helper= -c "credential.helper=!gh auth git-credential" push origin feat/image-preview
gh pr create --draft --base main --title "Image preview" --body "Implements docs/superpowers/specs/2026-09-27-image-preview-design.md. Manual checks still needed: Narrator on an image tab, high contrast, 150–200% DPI, a portrait phone JPEG (EXIF 6/8), HEIC with and without the Store codec, an Explorer drop onto the image view."
```
