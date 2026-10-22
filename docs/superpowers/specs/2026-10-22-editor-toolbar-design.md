# FastPad Editor Toolbar Design

Status: Approved in conversation (placement D of the view control mock-ups)  
Date: 22 October 2026  
Builds on: `2026-10-22-markdown-view-modes-design.md`, `2026-09-28-split-editors-design.md`  
Replaces: the floating view control at the top-right of an editor group's content.

## 1. Purpose

The floating view control covers the end of the first line of text. It now has four segments, so
it is twice as wide as the old preview buttons. A thin toolbar row under the tab strip takes its
place. The row shows the file's location on the left and the view control, with text labels, on
the right. Nothing floats over the text any more.

## 2. Decisions

| Topic | Decision |
|---|---|
| Where | One row per editor group, directly under its tab strip, above the find bar. The find bar, editor, Live view, preview and image view all sit below it. |
| When | On every tab, text or image, and in an empty group (blank), so neither switching tabs nor opening the first one moves the text. |
| Size | 32px tall at 96 DPI, growing with the Windows text size (`scale_text`). |
| Look | `editor-background`, with a 1px `stroke` line under it. |
| Left: location | The file's place, its parts joined by `›`. A file inside the open notebook shows the notebook folder › its subfolders › the file. Any other file shows its folder › the file. An untitled tab shows its title. The file name is in `editor-foreground`, the rest in `muted-foreground`. Too long: the folders are cut with an ellipsis first, then the name. Read-only. |
| Right: view control | On Markdown and SVG tabs only: a segmented control with text labels, Source · Live · Split · Preview (no Live on SVG). Each segment is 72 × 24px (text-scaled) on a `strip-background` track with a 4px corner and 2px padding, 8px from the row's right edge. The current mode is `editor-background` with a `stroke` border and semibold text. The others are `muted-foreground`, and get `hover-background` with `hover-foreground` under the pointer. |
| High contrast | No track. The current mode is the `Highlight` pair; the segment under the pointer gets a 1px border in the text colour. Square corners. |
| Clicks | A segment selects its mode for that group's tab, activating the group first. Clicking the current one does nothing. |
| Tooltips | As before: the mode and its key, or the cycle key. |
| Accessibility | Unchanged: the group strip's provider lists the segments as radio buttons, with the current one checked. Their rectangles come from the row's geometry, computed from the window alone (the provider may run on another thread). |
| Keyboard | As before, the row is not a tab stop; every mode has a command and a key. |

## 3. Not in this design

- Clickable location segments (opening the folder, revealing the note in the tree).
- A keyboard tab stop with arrow keys between segments.
- Exposing the location to screen readers.
