# FastPad Markdown View Modes Design

Status: Draft design, pending review  
Date: 22 October 2026  
Builds on: `2026-09-16-markdown-preview-design.md`, `2026-09-27-image-preview-design.md` (SVG preview), `2026-09-28-split-editors-design.md`, `2026-10-18-markdown-live-editor-design.md`  
Replaces: the preview mode of the preview spec (Off / Split / Full, per group) and the Live toggle of the Live editor spec (per tab).

## 1. Purpose

A Markdown tab is shown by two controls that combine. The **preview mode** (Off / Split / Full) is
set per editor group with `Ctrl+Shift+V`, two floating buttons and three View menu entries. **Live**
(on / off) is set per tab with `Ctrl+Alt+V`, View → Live Markdown and `markdown_live_default`.
Together they give five visible states (Source, Live, Source + preview, Live + preview, Preview)
and one hidden one (Live under the Full preview), behind two shortcuts and two scopes.

This design replaces both with one **view mode** per tab:

| Mode | Shows |
|---|---|
| **Source** | The Scintilla editor. |
| **Live** | The Live view. |
| **Split** | The Scintilla editor with the preview beside it. |
| **Preview** | The preview, full width and centred. |

Success: one shortcut, one floating control and one setting decide how a Markdown tab is shown, and
switching tabs shows each tab the way it was left.

## 2. Decisions

| Topic | Decision |
|---|---|
| Modes | Source, Live, Split, Preview. **Live with the preview beside it is dropped**: Live already renders in place. Split always shows the source. |
| Scope | **Per tab**: each (group, document) remembers its mode. A tab with no choice yet uses the default (§5). The divider ratio stays per group. |
| Moving a tab | A tab moved or copied to another group takes its mode with it. |
| SVG tabs | Same mechanism, without Live: Source, Split, Preview. Default Source. |
| Other tabs | No view mode; the floating control is hidden, as the preview buttons are today. |
| Cycle | `Ctrl+Shift+V` (`markdown.cycleView`): Source → Live → Split → Preview → Source. Modes the tab can't show are skipped (Live on SVG tabs and on Markdown over `LIVE_MAX_BYTES`). |
| Direct commands | `markdown.viewSource`, `markdown.viewLive` (`Ctrl+Alt+V`), `markdown.viewSplit`, `markdown.viewPreview`, all in the palette. |
| Escape | Escape in Preview goes back to the tab's last editing mode (Source, Live or Split), Source if it had none. Today it goes to Split. |
| Floating control | The two preview buttons become one **segmented control** of four icons (three on SVG tabs), the current mode highlighted. A click selects that mode. Clicking the current mode does nothing; Source is the "off". Tooltips name the mode and its shortcut. |
| View menu | A **Markdown view** submenu with Source / Live / Split / Preview as radio items, the tab's mode checked, grayed off Markdown and SVG tabs (Live grayed on SVG). It replaces the four entries there today. |
| Setting | `markdown_view = source \| live \| split \| preview`, default `source`. `markdown_live_default=true` is still read as `markdown_view=live` when `markdown_view` is absent; Settings writes only `markdown_view`. |
| Persistence | Not saved in the session, as today for both controls. |
| Source-only commands | Multiple cursors, column selection and folding switch a Live tab to Source with today's notice. Run from Preview, they switch to Source the same way. |
| Old command ids | `markdown.cyclePreview`, `markdown.previewSide`, `markdown.previewFull`, `markdown.closePreview` and `markdown.toggleLive` keep working in `key.<command-id>=` lines as aliases of `cycleView`, `viewSplit`, `viewPreview`, `viewSource` and `viewLive`. |

### Rejected

- **Five modes, keeping Live + Split.** It keeps a combination that needs two-way Live/preview
  scroll sync, for little gain over Live alone.
- **Per-group mode.** The preview works this way today, but then a group's mode means nothing for a
  plain-text tab and changes when the user switches to a Markdown tab. Per tab matches Live.
- **Live replacing Source.** The source view stays a mode the user picks.

## 3. State

- `preview::PreviewMode` becomes `ViewMode { Source, Live, Split, Preview }`, still in
  `src/preview/mod.rs`, with `shows_preview()`, `shows_editor()` and `is_live()` helpers.
- One per-group map, `modes: HashMap<DocumentId, ViewMode>`, plus `last_editing:
  HashMap<DocumentId, ViewMode>` for Escape. It replaces `LiveHost.choices` and `PreviewHost.mode`.
  Entries are pruned when a document leaves the group, as `LiveHost::prune_choices` does now.
- `mode(hwnd, group)` returns the active tab's mode: its entry, or else the default for its kind,
  narrowed to what it can show (Live → Source over the size limit, silently, as today).
- `LiveHost` keeps the Live window and its show/reload state. `PreviewHost` keeps the preview
  windows, the divider ratio, the parse state and sync.

## 4. Showing a mode

`preview_host::sync_visibility` stays the one place that shows and hides windows. For the active
tab's mode:

| Mode | Editor | Live view | Preview | Divider |
|---|---|---|---|---|
| Source | shown | hidden | hidden, released | — |
| Live | hidden | shown | hidden, released | — |
| Split | shown | hidden | shown | shown |
| Preview | hidden | hidden | shown, centred | — |

- Focus moves with the visible window, as today: into the preview when entering Preview, back to
  the text view when leaving it.
- Going from Live to Split or Preview keeps the scroll position: the preview starts at Live's top
  line, and Split's editor scrolls there too, as leaving Live does today.
- The SVG preview follows the same table with Live removed.
- Switching tabs re-runs `sync_visibility`; the preview loads the newly active document only if its
  mode shows the preview.
- Failing to create the preview (graphics stack) sets the tab back to its last editing mode with
  today's notice.

## 5. What goes away

- `PreviewButton::{Side, Full}` and the toggle-off click.
- `CommandId::MarkdownPreviewClose` and `MarkdownToggleLive`, folded into the new commands (§2).
- The Live → preview and preview → Live scroll sync (`2026-10-18` spec, phase 4 "Split"), and Live's
  "with the preview beside it" paths in find and go to line.
- `set_markdown_live` and `set_markdown_preview_enabled` in `menus.rs`, replaced by one
  `set_markdown_view(menu, available, checked)`.

## 6. Accessibility

- The segmented control is one tab stop with arrow keys between segments, exposed as a radio group
  (MSAA role `ROLE_SYSTEM_RADIOBUTTON` per segment, checked state for the current mode).
- The mode change is announced through the existing notification channel ("Live", "Split",
  "Preview", "Source").

## 7. Testing

- Unit: the cycle order with skipped modes; default resolution (`markdown_view`, the
  `markdown_live_default` fallback, the size limit); keymap aliases; settings parse and write.
- `preview_host/tests.rs`: content rects per mode; per-tab mode across tab switches; Escape from
  Preview; a moved tab keeps its mode.
- `tests/windows/markdown_preview.rs` and `markdown_live.rs`: the segmented control's clicks and
  keyboard; the menu radio state; Live → Split → Preview keeps the top line; SVG skips Live.
- Perf scenarios: drop "Live with the Split preview"; the others are unchanged.

## 8. Docs

README: replace "Show the preview side by side or full width" and the Live paragraph's toggle text
with the four modes; the shortcuts table gets "Markdown view (cycle)" `Ctrl+Shift+V` and "Live"
`Ctrl+Alt+V`; the settings table replaces `markdown_live_default` with `markdown_view`.
