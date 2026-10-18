# Quick Capture (PR 1e) Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ctrl+Alt+N anywhere opens a small capture box. Ctrl+Enter files what you typed, plus pasted, dropped or snipped images and files, into the notebook's inbox, today's note or a file of its own. `fastpad --capture` does the same from scripts and launchers, with or without a running FastPad.

**Spec:** `docs/superpowers/specs/2026-10-16-second-brain-phase-1-design.md` §10.1–§10.4.

## Architecture

- **Pure** (`library::capture`): entry and note text, file names (`YYYY-MM-DD HHmm <first words>`), unique names, link lines relative to the note, and Obsidian's `attachmentFolderPath`.
- **Disk, no window** (`library::capture_file`):
  - `file_capture` copies the attachments first, then appends to the inbox or daily note (stamp-checked, keeping the file's encoding and most common line ending) or writes a one-file capture. It is all or nothing.
  - An `OpenTabs` trait lets the running FastPad take the append into an open tab.
  - `capture_root` picks the notebook when no window has one open.
- **Platform:**
  - `clipboard::{has_text, has_image, has_files, files, image_png}` read the clipboard;
  - `png::{dib_to_pixels, encode_png}` turn a DIB into PNG through WIC;
  - `clock::local_now`.
- **Command line and IPC:**
  - `LaunchRequest::Capture`, parsing `--capture ["text" | -]` and `--attach <path>`;
  - `IpcRequest::Capture` (command 5), with length-prefixed UTF-16 text and paths;
  - `client::forward_if_running`, which never keeps the mutex.
- **Window:**
  - `capture_box`: the top-most box, with an edit, an attachment strip, Snip, drafts and file drops. A `Sink` trait does the filing.
  - `capture_host`: the running FastPad's sink, the `capture_hotkey` registration and `WM_HOTKEY`, and forwarded captures.
  - `capture_only`: the process that runs for `--capture` when no FastPad is running.
  - `translate_accelerator` leaves the box's keys alone.
- **Settings and installer:** `capture_hotkey` and `capture_source` in `fastpad.ini`, and an optional "Quick capture shortcut" installer task that makes a Start menu shortcut with Ctrl+Alt+N.

## Deliberate deviations from the spec

- **Attachments show as named chips**, "Image 1" or the file name, not thumbnails (§10.2).
- **`capture_source` records the window title** only, not the process's file description (§10.4). Browser and editor titles already end with the app's name.
- **The 20 MB question is FastPad's prompt** over the box, not text inside it.
- **Ctrl+Shift+Enter in a capture-only box** starts FastPad with the note, without placing the caret on the entry.
- **A second `--capture` while a capture-only box is open** brings that box forward (or cycles its target when it is in front) instead of opening another one.
- **`capture_hotkey` is read once, at startup.** The Settings dialog has no row for it; it is full.

## Parts

1. [x] Pure capture text, then filing on disk, with tests.
2. [x] `--capture` parsing, the Capture IPC frame and forwarding.
3. [x] Clipboard images and files; PNG encoding through WIC.
4. [x] The capture box, the main window's sink, the hotkey and forwarded captures.
5. [x] The capture-only process.
6. [x] Settings, installer task, README, spec §17, window and process tests.
