# FastPad Editor Toolbar Design

Status: Approved in conversation (placement D of the view control mock-ups)  
Date: 22 October 2026  
Builds on: `2026-10-22-markdown-view-modes-design.md`, `2026-09-28-split-editors-design.md`  
Replaces: the floating view control at the top-right of an editor group's content.

## 1. Purpose

The floating view control covers the end of the first line of text. It now has four segments, so
it is twice as wide as the old preview buttons. A thin toolbar row under the tab strip takes its
place. The row shows the file's location on the left and the view control on the right. Nothing floats over the text any more.

## 2. Decisions

| Topic | Decision |
|---|---|
| Where | One row per editor group, directly under its tab strip, above the find bar. The find bar, editor, Live view, preview and image view all sit below it. |
| When | On every tab, text or image, and in an empty group (blank), so neither switching tabs nor opening the first one moves the text. |
| Size | 32px tall at 96 DPI, growing with the Windows text size (`scale_text`). |
| Look | `editor-background`, with a 1px `stroke` line under it. |
| Left: location | The file's place, its parts joined by `›`. A file inside the open notebook shows the notebook folder › its subfolders › the file. Any other file shows its folder › the file. An untitled tab shows its title. The file name is in `editor-foreground`, the rest in `muted-foreground`. Too long: the folders are cut with an ellipsis first, then the name. Read-only. |
| Right: view control | On Markdown and SVG tabs only: a segmented control of icons, Source (Code, E943) · Live (Edit, E70F) · Split (F57C) · Preview (E7B3), no Live on SVG; the tooltip names each and its key. (Text labels were tried first; icons were chosen for a quieter row.) Each segment is 24 × 22px. Each glyph is sized by its ink to 11px tall (text-scaled, narrower if it would not fit), and its ink is centred, since the icon font fills its cells unevenly; on the row itself with no track, 8px from the row's right edge. The current mode gets a soft `hover-background` fill (4px corner, no border) and an `editor-foreground` glyph. The others are `muted-foreground`, turning `hover-foreground` under the pointer, with the fill while pressed. |
| High contrast | The current mode is the `Highlight` pair; the segment under the pointer gets a 1px border in the text colour. Square corners. |
| Clicks | A segment selects its mode for that group's tab, activating the group first. Clicking the current one does nothing. |
| Painting | Through an off-screen bitmap, and only when the hovered or pressed segment changes, so moving the pointer over the row never flickers. |
| Tooltips | As before: the mode and its key, or the cycle key. |
| Accessibility | Unchanged: the group strip's provider lists the segments as radio buttons, with the current one checked. Their rectangles come from the row's geometry, computed from the window alone (the provider may run on another thread). |
| Keyboard | As before, the row is not a tab stop; every mode has a command and a key. |

## 3. Not in this design

- Clickable location segments (opening the folder, revealing the note in the tree).
- A keyboard tab stop with arrow keys between segments.
- Exposing the location to screen readers.
