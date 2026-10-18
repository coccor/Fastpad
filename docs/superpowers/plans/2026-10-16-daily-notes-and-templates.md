# Daily Notes, Templates and notebook.ini (PR 1d) Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A notebook keeps its conventions in `.fastpad\notebook.ini`. Ctrl+Alt+D opens today's note, made from a template the first time, and Previous/Next daily note step through the journal. Notes: New note from template… makes a note of any page type from the templates folder, with placeholders filled in.

**Spec:** `docs/superpowers/specs/2026-10-16-second-brain-phase-1-design.md` §9.

## Architecture

- **Pure** (`library::dates`, `library::template`, `library::notebook_ini`):
  - `Date`/`Time`, the format tokens (`YYYY MM DD D MMM MMMM ddd dddd`), `format`, `is_daily_format`, and `parse` to read a daily note's name back;
  - `template::fill` for `{{date}}`, `{{date:FORMAT}}`, `{{time}}`, `{{title}}` and `{{cursor}}`, and `default_daily`;
  - `notebook_ini::parse` (defaults for bad values, warnings with line numbers) and `starter_text`.
- **Platform:** `platform::clock::local_now` wraps `GetLocalTime`. Tests fix "now" with `library_host::set_now`.
- **Window** (`library_host::daily`):
  - `LibraryHost.notebook` holds the settings; it is re-read after every load and rescan, and when the watcher reports `notebook.ini`. Warnings are shown once per change of the file's text.
  - `open_today`, `step_daily`, `edit_notebook_settings`;
  - `new_note_from_template` opens a `PickerKind::Template` picker. The chosen template opens the inline name field with `Purpose::NewNoteFrom`, whose commit writes the filled template and places the caret.
- **Links:** the Links view takes the daily folder, inbox and templates folder from `notebook.ini`. Notes in the templates folder are left out of resolution, completion, missing notes and unlinked notes.
- **Commands:** `NotesToday` (Ctrl+Alt+D), `NotesPreviousDaily`, `NotesNextDaily`, `NotesNewFromTemplate`, `NotesEditNotebookSettings`. They are in the palette, in the Notebook header's `…` menu (Today's note, New note from template…) and in the folder context menu (New note from template…).

## Deliberate deviations from the spec

- **No "This notebook" section in Settings yet** (§9.1). The dialog is at 802 of its 810 pixels and has no text field. **Notes: Edit notebook settings** opens `notebook.ini` instead, made first with every key commented out at its default, and the watcher applies saved changes.
- **The template picker uses the palette's own matching**, not the quick-open matcher (§9.4); template names are short.
- **Obsidian's `attachmentFolderPath`** is left for PR 1e, which is the first to use the attachments folder.
- **Unknown keys in `notebook.ini` get a notice**, as unknown `fastpad.ini` keys do.
- **A daily template that doesn't exist** gets a notice only when `notebook.ini` names it; the default path is optional.

## Parts

1. [x] Pure dates, placeholders and `notebook.ini`, with tests.
2. [x] Loading `notebook.ini` in the window; Edit notebook settings.
3. [x] Open today's note; Previous and Next daily note.
4. [x] New note from template: picker, inline name field, filled note.
5. [x] Links view and resolver use the notebook's conventions.
6. [x] Window tests, README, spec §17, full test run.
