# Markdown View Modes Implementation Plan

Spec: `docs/superpowers/specs/2026-10-22-markdown-view-modes-design.md`

**Goal:** one per-tab view mode (Source, Live, Split, Preview) replaces the per-group preview mode
and the per-tab Live toggle.

## Decisions taken while planning

1. **State lives on `PreviewHost`.** `PreviewHost.mode` becomes `modes: HashMap<DocumentId,
   ViewMode>` and `last_editing: HashMap<DocumentId, ViewMode>`; `LiveHost.choices` goes.
   `preview_host::mode(hwnd)` resolves the target group's active tab (entry, else the default
   for its language, narrowed to what it can show). `live_host::wants_live` becomes
   `mode == Live`.
2. **Command ids keep their numbers.** `MarkdownPreviewCycle` (152) → `MarkdownViewCycle`,
   `MarkdownPreviewSide` (153) → `MarkdownViewSplit`, `MarkdownPreviewFull` (154) →
   `MarkdownViewPreview`, `MarkdownPreviewClose` (155) → `MarkdownViewSource`,
   `MarkdownToggleLive` (282) → `MarkdownViewLive`. Old keymap names are parsed as aliases.
3. **The preview window is destroyed** when the active tab is set to Source or Live, as
   closing the preview did. A tab switch only hides and releases it.
4. **Keyboard focus inside the segmented control is deferred.** The control stays a
   pointer surface, as the two buttons were. Screen readers reach it through the group strip's
   provider, as now: four (three on SVG) radio buttons, the current one checked, with a
   default action that clicks it. A tab stop with arrow keys is follow-up work.
5. **The mode change notice is not added.** The checked radio button and the menu radio item
   already report it; a notice on every `Ctrl+Shift+V` would be noise.
6. **Diagnostic selector 0** reports Source 0, Split 1, Preview 2, Live 3. Source, Split and
   Preview keep the numbers Off, Split and Full had.

## Tasks

1. `ViewMode` in `src/preview/mod.rs` with `token`/`parse`/`label`, `shows_preview`,
   `hides_editor`, `next` (cycle with skips). Unit tests.
2. Settings: `markdown_view` (default `source`); `markdown_live_default` read as a fallback.
   Tests.
3. Commands, keymap (renames, aliases, `Ctrl+Alt+V` → `markdown.viewLive`), palette entries,
   shortcuts model.
4. `preview_host`: per-tab state, `mode`/`set_mode`/`run_command`/`escape`, `sync_visibility`,
   `layout`, `diagnostic`; drop Live ↔ preview scroll sync. `live_host`: read the mode, drop
   `choices`, `toggle`, `set_live`, `follow_preview`; `leave` sets Source.
5. Moving or copying a tab to another group carries its mode (`place_view`,
   `split_active_group`).
6. Floating segmented control in `preview_buttons.rs`; accessibility children.
7. View → Markdown view radio submenu; `menus::set_markdown_view`.
8. Update unit and window tests; README.
