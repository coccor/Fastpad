# Editor Toolbar Implementation Plan

Spec: `docs/superpowers/specs/2026-10-22-editor-toolbar-design.md`

## Decisions taken while planning

1. **The floating control's module becomes the toolbar.** `window/preview_buttons.rs` is renamed
   `window/editor_toolbar.rs`, `PreviewButtons` becomes `EditorToolbar`, and the group's field is
   `toolbar`. Its child window, tooltip and pointer handling stay.
2. **Geometry is pure.** `segment_rect(toolbar, count, index)` computes a segment from the
   window's client width, its DPI and the text scale. The accessibility provider and the mouse
   share it, and the provider passes the segment count from its snapshot.
3. **The location is a pure function** of the path, the tab title and the notebook folder, with
   unit tests.
4. **The toolbar lays itself out** and returns its height, which `layout_group` adds before the
   find bar.
5. The `GLYPH_VIEW_*` constants go: the segments are text.

## Tasks

1. Rename the module and its references.
2. `location_segments` with tests; `segment_rect`; `layout` returning the row height; painting
   (location, track, segments, high contrast).
3. `layout_group`: the row under the strip, the find bar and content below it.
4. Clicks act on the toolbar's own group.
5. Accessibility rects through `segment_rect`.
6. Tests: the row's place and height, every tab showing it, the editor below it, segment clicks,
   location text; update the floating-control tests.
7. README.
