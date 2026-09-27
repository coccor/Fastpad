# Image preview — design

Date: 2026-09-27. Status: approved in chat; spec awaiting review.

## 1. Goal

Opening a common image file in FastPad shows the image in a tab instead of the "unsupported text
encoding" notice. This works from every place a file can be opened: File › Open, the command line,
single-instance forwarding, Explorer drops, the notebook tree, quick open, Markdown preview links
and session restore. The notebook tree lists image files next to notes. SVG stays a text file,
and the Markdown-style preview toggle renders it as an image.

Viewing only. FastPad does not edit, convert or save images.

### User decisions (2026-09-27)

- An image tab is a real tab kind, not a text tab in disguise, so a save can never overwrite an image.
- SVG opens as text, and Split/Full preview renders the live buffer (option "a").
- The notebook tree, quick open and Open Editors show image files. Text search and replace skip them.

## 2. What exists today

- Every disk open goes through `open_path_placed` (`src/window/main_window.rs`). It reads the
  whole file, and anything that isn't a BOM or strict UTF-8, or that holds a NUL, fails with
  `FastPadError::UnsupportedEncoding`. So PNG/JPEG/BMP/ICO are refused, and SVG opens as plain text.
- `Document` always owns a Scintilla `EditorDocument`.
- `preview::images::decode_image` decodes frame 0 through WIC and scales it down only. It refuses
  images over 64 MP, and dispatches `.svg` to `preview::svg::decode_svg` (Direct2D SVG,
  Windows 10 1703+, 8 MB cap).
- `preview::dwrite::Graphics` loads d2d1/dwrite at runtime, and the import-table guard tests
  forbid static imports of d2d1, dwrite, windowscodecs and shlwapi.
- `library::title::NOTE_EXTENSIONS` (14 text types) filters the notebook scan, the index merge,
  `add_note`, copy results and the file-type icons.

## 3. Performance contract (unchanged rules, applied to images)

- Nothing new runs before the first editable frame. No D2D, DWrite or WIC loads until an image
  tab or an SVG preview is shown. This is extended with a runtime guard: launching with a `.txt`
  file loads none of d2d1, dwrite or windowscodecs.
- The UI thread never reads or decodes image bytes. Opening an image tab only takes a metadata
  stamp (size and mtime), and then the tab paints "Loading…" until the worker posts the result.
- Decoding uses one short-lived worker thread per request, as `ImageCache` does today. There is
  no thread pool.
- Switching to a text tab frees the render target and the D2D bitmap. The decoded pixels of the
  most recently shown image tab are kept, so switching back is instant. Any other image tab
  decodes again when it is shown.

## 4. Which files are images

`library::title` gains `IMAGE_EXTENSIONS` and `is_image_extension`:

`png, jpg, jpeg, jpe, jfif, gif, bmp, dib, ico, tif, tiff, webp, heic, heif, avif, svg`.

- **Raster extensions** (everything except `svg`) open as an image tab. No text read is attempted.
- **`svg`** opens as a text tab (`Language::Svg`, plain lexing), and gets the preview toggle (§8).
- **Sniffing as a fallback.** When a text open fails with `UnsupportedEncoding`, the first 16
  bytes are checked for PNG, JPEG, GIF, BMP, TIFF, ICO or RIFF/WEBP signatures. A match opens an
  image tab instead of the notice. This covers extensionless or misnamed images, and it's cheap
  because the bytes are already in memory.
- An extension listed for a codec that isn't installed (for example HEIC with no Store codec)
  opens an image tab that shows the failure state (§6.4).

`is_listed_extension(ext) = is_note_extension(ext) || is_image_extension(ext)` replaces
`is_note_extension` wherever the question is "does the notebook list this file" (§9).

## 5. Tab model

`Document` changes from owning `handle: EditorDocument` to owning a content enum:

```rust
pub enum Content {
    Text(TextContent),   // today's handle, language, encoding, recovery generations, …
    Image(ImageContent), // path-derived format, disk stamp, decode state
}
```

Text-only fields move into `TextContent`, so the compiler lists every place that assumes text.
The rules for image tabs:

- **Never dirty, never autosaved, never snapshotted.** `needs_snapshot` is false, and closing
  one never prompts.
- **Unavailable commands.** Save, Save As, find and replace, Go to line, undo and redo, cut,
  copy, paste, select all, the JSON commands and language changes are all unavailable. Menus
  gray them out, the palette hides them, and dispatch ignores them. Dispatch treats "no tab"
  and "image tab" alike for editor commands. `needs_document()` gets a sibling
  `needs_text_document()`.
- **What stays the same.** An image tab still gets a `RecoveryId`, because tabs are keyed by it.
  Preview (italic) tabs from the notebook work unchanged, and `replace_preview` and `promote`
  are kind-agnostic.
- **The editor while an image tab is active.** `activate_document` hides the editor HWND and
  shows the image view. The editor holds the existing hidden placeholder document, the same
  state it uses when no tab is open.
- **Title and status bar.** The title is the file name. The status bar shows
  `1920 × 1080 · PNG · 245 KB · 50%` (pixels, WIC container format, file size, zoom) instead
  of caret, language and encoding.
- **Session and recovery.** The session writes `File(path)` for a clean tab with a path, so an
  image tab restores through `open_path` with no new keys. Zoom and pan are not saved. Recovery
  never sees image tabs.

## 6. Image view (`src/image_view/`)

A new module, independent of the Markdown `ViewState`.

### 6.1 Window

- A lazily created `WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP` child of class `FastPadImageView`,
  with its state boxed in `GWLP_USERDATA`. It uses the same pattern as `PreviewView`.
- It shares the window's `Rc<Graphics>` with the preview host. `Graphics` moves out of
  `PreviewHost` into a small shared slot on `App`, so whichever view is shown first loads it.
- `layout_editor_and_find_bar` gives it the content area whenever the active tab is an image tab.
- Focus goes to the image view when an image tab is activated (`content_focus_target`).

### 6.2 Decoding

- **`images::decode_image_full(path, max_side) -> Result<DecodedImage>`.** This is a sibling
  of `decode_image` that shares its WIC path:
  - frame 0, except for `.ico`/`.cur`, where the largest frame is used;
  - JPEG and TIFF EXIF orientation (`System.Photo.Orientation`) is applied with
    `IWICBitmapFlipRotator`;
  - the 64 MP cap is kept;
  - when either side exceeds `max_side`, the image is downscaled with Fant, keeping its aspect
    ratio. `max_side` is the render target's `GetMaximumBitmapSize()`, cached after the first
    target is created, with a default of 16384.
- **Results.** A decode returns the pixels, the size after orientation, the natural size and
  the WIC container format name. Decodes are keyed by `(path, size, mtime)`, and a stale
  result, from a tab that has since closed or changed, is dropped.
- **Animated GIFs.** Only the first frame is shown, as in the Markdown preview. Playback is
  out of scope.

### 6.3 Viewing

- **Initial scale.** The image first shows at "fit": it is scaled to fit the view, never above
  100%, and centered.
- **Zoom.** Zoom In/Out (Ctrl + / Ctrl − and Ctrl + wheel) step through
  `10, 25, 50, 67, 100, 150, 200, 300, 400, 800, 1600`% starting from the current scale. Wheel
  zoom is anchored at the pointer, and key zoom at the view center.
- **Zoom Reset (Ctrl+0)** returns to fit, and double-click toggles between fit and 100%. These
  are the existing `ZoomIn/ZoomOut/ZoomReset` commands, dispatched to the image view when an
  image tab is active. No new `CommandId`.
- **Panning.** When the image is larger than the view, you can pan by dragging with the left
  button (grab cursor), with the scroll bars, with the wheel (vertical) and Shift + wheel
  (horizontal), and with the arrow keys, PgUp/PgDn and Home/End. Panning is clamped so the image
  can't be scrolled off the view.
- **Resampling.** Linear interpolation below 100% and nearest-neighbor above 100%, so pixel
  art stays crisp.
- **Background.** The view uses the editor background, and transparent pixels sit on a subtle
  8-DIP checkerboard made from `PreviewColors` background and border colors. In high contrast
  there is no checkerboard, just the system window color.
- **DPI and device loss.** Rendering is DPI aware, so 100% means one image pixel per device
  pixel. On `D2DERR_RECREATE_TARGET`, the bitmap is rebuilt from the cached pixels.

### 6.4 States

- **Loading:** a centered muted "Loading…" line, shown after 150 ms so fast decodes don't flash.
- **Failed:** a centered message, "FastPad can't display this image", followed by the reason:
  - "The file is larger than 64 megapixels.";
  - "Windows has no decoder for this format." (WINCODEC_ERR_COMPONENTNOTFOUND);
  - "The file is damaged or not an image.";
  - the OS error text for read failures.

  The tab stays open, and there is no notice.
- **D2D load failure:** the same failure state, reading "Image display isn't available on this
  system."

### 6.5 Disk changes

- **On activation or focus.** Text tabs get their disk-change check here, and image tabs now
  get one too: when the stamp differs, the image is decoded again, keeping zoom unless the
  pixel size changed.
- **Deleted files.** A file deleted while its tab is open shows the failed state with "The file
  no longer exists."

### 6.6 Accessibility

- The view answers `WM_GETOBJECT` with an MSAA root of `ROLE_SYSTEM_GRAPHIC`. Its name is
  `"<file name>, image, 1920 by 1080 pixels"`, and its value is the zoom
  (`"Zoom 50 percent"`), which changes with zoom and fires `EVENT_OBJECT_VALUECHANGE`. The
  failed and loading states are the name's suffix.
- The hand-written vtable pattern from `preview::accessible` is reused, but as a separate small
  object with no children.
- Keyboard: every action in §6.3 has a key, and Tab or F6 leave the view as they leave the editor.

## 7. Markdown preview links

`follow_link` already calls `open_path`, so a link to a local image opens an image tab with no
extra work. There's a test for it.

## 8. SVG preview (option a)

- **Opening.** `.svg` opens as text with `Language::Svg`. The preview buttons and
  `MarkdownPreviewCycle` (Ctrl+Shift+V) become available for Svg as well as Markdown. The
  palette and menu labels stay "Markdown preview…". Renaming them is out of scope.
- **Rendering.**
  - In Split or Full mode on an Svg tab, the preview slot hosts an image view, not the
    Markdown `PreviewView`. The image view gets its source from the buffer text, not from disk.
  - `svg::decode_svg` is split into `decode_svg_text(wic, text, base_dir, max_width)` and a
    path wrapper.
  - After the same 120 ms idle delay as the Markdown preview, the buffer text (up to the 8 MB
    cap) is copied and rasterized on a worker.
  - Rasterizing happens at the view's pixel size for fit, and at natural size × zoom otherwise.
    So when the zoom changes, the SVG is rasterized again, not the bitmap stretched.
- **Failure and scrolling.** A parse failure keeps the last good image and shows a thin
  "Can't render this SVG" bar at the top; a first failure shows the failed state. The editor
  and preview scroll independently, with no scroll sync.

## 9. Notebook

- **Listed alongside notes.** `scan.rs`, `merge_notes`, `add_note` and `copy_host`'s "listed
  vs hidden" split use `is_listed_extension`, so images appear in the tree, the index and
  quick open (Ctrl+P). Opening one from the tree (single-click preview tab, or double-click)
  goes through `open_note` → `open_path_placed`, and you get an image tab. The copy notice
  "was copied but isn't shown" now only applies to types that are neither notes nor images.
- **Text search and replace** build their note lists from `is_note_extension` only, so images
  are never read as text.
- **Inline naming.**
  - New note still offers note extensions only.
  - Renaming an image keeps its extension when the typed name has none. This extends the rule
    that already applies to note extensions, in `inline_name.rs` and `title`.
  - A rename that changes the extension between an image type and a note type is allowed. It's
  just a file rename, and the open tab reopens with the right kind.
- **Icons.**
  - `file_icons` gains `NoteKind::Image`, with the Catppuccin role `Green` and the type name
    "image" for screen readers. An SVG shows the image icon too.
  - Material takes `image.svg` from Material Icon Theme 5.38.1 (MIT, recorded in
    `SOURCE.md`). Minimal and Solid get a new outline and filled "picture" mask, drawn to the
    rules in `assets/icons/README.md`.
  - `icons.bin` for each set is regenerated with `tools/generate-file-icons.ps1`.
- **Open Editors rows** show the image icon. Dragging an image row onto the tree copies it,
  as it does for any file.

## 10. Errors summary

| Situation | Result |
|---|---|
| Raster extension, decodes | image tab |
| Raster extension, bad data / no codec / > 64 MP | image tab in failed state, no notice |
| No image extension, not text, image signature | image tab |
| No image extension, not text, no signature | existing "unsupported text encoding" notice |
| SVG | text tab; preview toggle renders it |
| D2D unavailable | image tab failed state; SVG preview stays Off with the existing notice |

## 11. Testing

In-process window tests (`tests/windows/image_preview.rs`, new, `--test-threads=1`):

1. **Opening.**
   - A PNG opens from `open_path`, the Open dialog, an Explorer drop, the notebook tree
     (preview and permanent), quick open and a Markdown link. Each gives one image tab with
     the editor hidden and the view shown, and the status bar reads the right size.
   - A PNG with a `.dat` extension opens by sniffing, and a random binary still gets the
     notice.
2. **Safety.** Ctrl+S, Save As, autosave and closing on an image tab leave the file bytes
   identical. The commands are grayed out and hidden in the palette. Closing never prompts.
3. **Failure states.** A corrupt `.png`, a `.heic` with the codec forced missing (test hook),
   and a deleted file each show the failed state with no notice.
4. **Zoom.**
   - Fit scale for a large image, 100% for a small one.
   - Zoom In/Out steps and Ctrl+0.
   - Double-click toggles between fit and 100%.
   - Panning is clamped.
   - These are checked through a `cfg(test)` `ImageView::stats()` (scale, offset, state).
5. **Reload and resources.**
   - Changing the file on disk and reactivating the tab decodes it again.
   - Switching to a text tab frees the render target, and switching back doesn't decode again.
6. **Session.** An image tab restores and stays active.
7. **SVG.**
   - SVG opens as text, and Ctrl+Shift+V shows the image view in the preview slot.
   - Typing updates the render after the delay, and bad SVG keeps the last image and shows
     the bar.
8. **Notebook.**
   - The scan lists `a.png`, and text search over a notebook with images reads no image bytes
     (skip counters).
   - Renaming `a.png` to `b` gives `b.png`.
   - The icon type is Image.
9. **Accessibility.** The view's accessible name and value, and the value change on zoom.
10. **Guards.** The import-table guard is unchanged, and launching with `.txt` loads no
    graphics DLLs.

Unit tests cover the extension and signature tables, `is_listed_extension`, the zoom-step and
fit math, pan clamping, ICO largest-frame choice and EXIF orientation mapping (all pure).

Run discipline follows memory: Clippy plus targeted tests while working, the full suite once at
final review, and fastpad.ini backed up around any live-app run.

## 12. Out of scope

Editing, rotating, saving or converting images; animated GIF/WebP playback; copying an image to
the clipboard; image thumbnails in the tree; remote images; persisting zoom; slideshow or
next/previous image in folder; renaming the "Markdown preview" commands; XML syntax
highlighting for SVG.
