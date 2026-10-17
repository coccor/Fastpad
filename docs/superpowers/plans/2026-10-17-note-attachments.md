# Attachments in Notes (PR 1f) Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In a Markdown note, Ctrl+V of a screenshot or of files copied in Explorer, Ctrl+drop of files, and Notes: Insert attachment… save the files into the attachments folder and link them at the caret. Links to attachments keep working when notes or images move.

**Spec:** `docs/superpowers/specs/2026-10-16-second-brain-phase-1-design.md` §10.5.

## Architecture

- **Shared with quick capture:**
  - `capture_file::store` writes a PNG or copies a file into a folder, never over another file.
  - `capture::attachments_folder` (with Obsidian's setting) and `capture::link_lines` place and link them.
- **`window::attachments`:**
  - `attach` saves, lists the new files in the notebook, and inserts the links as one undo step;
  - `paste` handles Ctrl+V and Shift+Insert through the editor's `key_down` hook, and the Paste command;
  - `dropped` handles Ctrl+drop, carried in `EditorDrop.attach`;
  - two commands: Notes: Paste image as attachment and Notes: Insert attachment….
- **Link updates:** `links::edits_for_moves_with` also fixes a moved note's own relative links to files the notebook doesn't list (PDFs, logs) that are still where the link pointed. Images are listed, so links to them already follow image moves through the 1c prompt.

## Deliberate deviations from the spec

- **Renames of unlisted attachments outside FastPad** (a PDF renamed in Explorer) are not detected. Only listed files (notes and images) are paired across scans. A note's own links to them still follow the note's moves.
- **Insert attachment… takes one file** (the Open dialog's single choice).
- **Scintilla's right-click Paste** pastes text as before. Ctrl+V, Shift+Insert, Edit > Paste and the palette attach.
- **A file over 20 MB asks first** here too, as quick capture does.
- **A Ctrl+drop on a group's strips** (not the editor) links the files at the caret.

## Parts

1. [x] Shared `store`; unlisted links in `edits_for_moves_with`, with tests.
2. [x] `window::attachments`: paste, drop, Insert attachment, Paste image as attachment.
3. [x] Ctrl+drop through `EditorDrop`.
4. [x] Window tests, README, spec §17, full test run.
