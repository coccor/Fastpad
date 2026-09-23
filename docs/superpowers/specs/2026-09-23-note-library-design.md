# Note Library Design

Status: Approved design  
Date: 23 September 2026

## 1. Purpose

FastPad is getting notebooks, tags, Favorites, Recent, search and note links, following the product spec "FastPad Left Sidebar and Notebook Model" (draft, 22 September 2026). That work is split into four sub-projects, each with its own spec, plan and implementation:

1. **Note library and metadata store** (this spec).
2. Sidebar and note list.
3. Search.
4. Links and backlinks.

Each depends on the one before it. This spec covers the foundation: where notes live, how FastPad knows about them, how organizational metadata is stored and kept attached to the right file, and how new notes are named and saved.

The guiding principle is that **notes and files are the same thing viewed from a different angle**. A note is a plain text file. Notebooks, tags, favorites and pins are a metadata view over files, never a separate storage kind. Any file FastPad opens can be treated as a note.

FastPad is past its MVP, so the MVP spec's deferral of sidebars and projects no longer applies. Its lasting rules still do: nothing blocks the first paint or the first input, there is one UI thread, dependencies stay minimal, and Scintilla owns all live text.

## 2. Goals and non-goals

### Goals

- Open a folder as the note library as easily as VS Code opens a folder.
- Treat every text file in that folder, including subfolders, as a note, without writing anything into the folder until the user organizes something.
- Store notebooks, tags, favorite and pin state keyed by stable identifiers that survive renames and moves made inside FastPad, in Explorer, or by a sync client.
- Name and save new notes the way VS Code does: first-line tab labels, and a name confirmed once on the first save.
- Autosave files inside the open folder.
- Leave startup performance unchanged.
- Never let metadata problems block access to note content.
- Provide a setting that turns the whole feature off.

### Non-goals

- The sidebar, the note list pane, and drag and drop onto notebooks (sub-project 2).
- Search and its index (sub-project 3).
- `[[links]]` and backlinks (sub-project 4).
- More than one open folder at a time.
- A file-system watcher (`ReadDirectoryChangesW`). Rescans on activation are used instead.
- A per-folder ignore file. The built-in skip rules are used instead.
- Importing subfolders as notebooks.
- Detecting that a file changed on disk for tabs outside the open folder.

## 3. Differences from the product spec

These decisions change or narrow the product spec and were agreed during design:

| Product spec | This design |
|---|---|
| §7.1: a new note saves automatically as the user types. | Ctrl+N creates an untitled tab, as VS Code does. It is kept by session restore and recovery, and written into the folder only when first saved through the inline name box (§8.2). Autosave starts after that. |
| Acceptance criterion 2: a new note appears in Notes immediately. | Sub-project 2 lists open untitled tabs at the top of Notes, labeled from their first line and marked *unsaved*. |
| §7.2: an explicit title, then the first line, then `Untitled`. | A saved note's title is its filename without the extension. Only untitled tabs take their label from the first line. Renaming a note renames its file. |
| §14: a note record for every note. | Records are sparse. Only notes with metadata get a record (§6). |
| §13: `Ctrl+Shift+F` toggles favorite. | `Ctrl+Shift+F` is Format JSON. Sub-project 2 must choose another binding. |

## 4. Setting

- New `fastpad.ini` key: `notes_mode`. It accepts the existing boolean spellings (`parse_bool`) and defaults to `true`.
- It gets a field on `Settings` and `SettingsDelta`, handling in `apply_delta` and `apply_line`, a `DEFAULT_NOTES_MODE` constant, and an entry in the `parse` doc comment.
- New command `CommandId::ToggleNotesMode`, labeled "Notes: Toggle notes mode" in the palette. It goes through `change_setting` and posts a notice, "Notes mode is on" or "Notes mode is off".
- **With `notes_mode=false`, FastPad behaves exactly as it does today:** no library step at startup, "Untitled" tab labels, today's save flow, no autosave, and no `.fastpad\` or `libraries\` files written. Turning the setting off during a session unloads the library, stops autosave, and restores "Untitled" labels. Turning it on loads the last folder.

## 5. The open folder

### One folder at a time

A single folder is open at any time, as in a VS Code window. Notes, Favorites, Recent, notebooks, tags and (later) search all cover that folder. Each folder keeps its own notebooks and tags.

### Opening a folder

- **File → Open Folder…**, shortcut `Ctrl+Shift+O`, and the palette command "File: Open folder…". These use the system folder picker.
- Palette command "File: Open recent folder", listing `folders.ini` (§6.3).
- Dropping a folder on the window. Dropped files keep today's behavior.
- A command-line argument that names a directory: `fastpad D:\Notes`. `bootstrap` treats a directory argument as a deferred folder request, the same way it treats a file request.
- The IPC protocol gains `IpcRequest::OpenFolder(PathBuf)` with its own command byte. A second launch with a folder argument forwards it to the primary window.
- Sub-project 2 adds the folder switcher in the sidebar header. It uses the same commands.

### Default and startup

- At startup FastPad opens the first entry of `folders.ini`. It does this regardless of `restore_session`, because the folder is the library, not part of the tab session.
- On first run, or if the list is empty, it uses `Documents\FastPad` (resolved with `SHGetKnownFolderPath(FOLDERID_Documents)`) and creates that folder only when a note is first saved into it.
- If the remembered folder no longer exists (for example, an unplugged drive), FastPad falls back to the default and shows a notice naming the missing folder. The entry stays in the recent list.

### Switching

Opening another folder keeps every open tab. Autosave then applies according to the new folder (§8.3). Pending metadata writes for the old folder are flushed before the switch.

### Which files are notes

- Every file with an extension FastPad recognizes as text (the existing `languages` detection list, plus `.txt`, `.md`, `.log`, `.ini` and `.yaml`), in the folder and its subfolders.
- Skipped: hidden or system entries, any directory whose name starts with `.`, and directories named `node_modules`, `target`, `bin` or `obj`.
- Scanning stops at 10,000 notes, and a notice says the folder is too large to index fully.
- **Subfolders are storage only.** The index is flat, notebook membership lives only in metadata, and new notes are saved to the folder root.

## 6. Data model and files

### 6.1 Records are sparse

Every note in the folder is present in the in-memory index. Only notes that carry metadata get a stable ID and a record in `library.ini`. A note carries metadata if it is in a notebook, has a tag, or is a favorite or pinned. Opening a folder therefore writes nothing into it. `.fastpad\` is created the first time the user organizes something.

A file outside the open folder can also be organized (favorited, tagged, or put in a notebook). It gets a record with an absolute path and is never moved.

### 6.2 `.fastpad\library.ini` (shared)

This file travels with the folder through copies, backups and sync.

```
version=1
notebook=<id>|<sort>|<color or ->|<created unix>|<modified unix>|<name>
tag=<id>|<name>
note=<id>|<notebook id or ->|<flags>|<tag ids>|<size>|<hash>|<path>
```

- IDs are 32 hex digits from the generator `RecoveryId` already uses.
- `<flags>` is any combination of `f` (favorite), `p` (pinned) and `d` (deleted, §8.4), or `-` for none.
- `<tag ids>` is a comma-separated list, or `-` for none.
- `<size>|<hash>` is the content fingerprint used for sync matching (§7.3). The hash is 64-bit FNV-1a over the file bytes, written as 16 hex digits. It is updated whenever FastPad saves the file, and whenever a scan sees the size or mtime change for a file that has a record.
- Names escape `%`, `|`, CR and LF as `%25`, `%7C`, `%0D` and `%0A`.
- The path is always the last field and is not escaped, the same convention as `session.ini`. It is relative to the folder for notes inside it, and absolute otherwise. Windows paths cannot contain `|`, so splitting on the first six `|` characters of a `note` line is unambiguous.
- Notes with no notebook belong to the built-in Notes collection, which is not stored.
- A tag with no notes is dropped when the file is next written.
- The notebook `<color>` is one of a fixed palette of names (`red`, `orange`, `yellow`, `green`, `teal`, `blue`, `purple`, `pink`), or `-`. Colors are decorative (product spec §16).
- Unknown keys and malformed lines are ignored. A `note` line naming an unknown notebook falls back to Notes; unknown tag IDs are dropped.
- **An unknown `version` makes the file unreadable.** FastPad never overwrites it, and organizing is disabled for that folder with a notice. A newer FastPad's metadata can therefore never be destroyed by an older one.
- It is written with the existing atomic saver (`file::saver`), as UTF-8 with CRLF line endings.

### 6.3 Local files (this PC only)

**`%LOCALAPPDATA%\FastPad\libraries\<folder key>.ini`**, where `<folder key>` is the FNV-1a hash of the folder's full path, lower-cased, as 16 hex digits:

```
version=1
folder=<absolute path>
autosave=true
recent=<unix time>|<path>
missing=<unix time>|<note id>
file=<volume serial>|<file id>|<mtime>|<size>|<path>
```

- `folder` guards against hash collisions. A mismatch makes the file be treated as absent.
- `autosave` is the per-folder autosave switch (§8.3), default `true`.
- `recent` lines record when each note was last opened, keyed by path, capped at 200 entries. Reconciliation renames (§7.3) update them. Opening a note never creates a shared record.
- `missing` records when a note ID first went missing or was deleted. It drives the 30-day purge (§8.4).
- `file` lines are the scan cache. File IDs are only meaningful on one volume on one PC, which is why they are stored here and not in the shared file.

**`%LOCALAPPDATA%\FastPad\folders.ini`**:

```
version=1
folder=<absolute path>
```

The recent folders, most recent first, at most 10. The first entry is the folder opened at startup.

Both local files use the same atomic write. Errors writing them produce no notice, because they only hold caches and conveniences.

## 7. Loading, scanning and reconciliation

### 7.1 Startup

- A new deferred step, `WM_FASTPAD_OPEN_LIBRARY`, goes after `RESTORE_SESSION`:
  `LOAD_SETTINGS → RESTORE_SESSION → OPEN_LIBRARY → OPEN_REQUEST → APPLY_LANGUAGE → RECOVERY → START_IPC → BUILD_CHROME`
  `messages.rs` and its order test are updated.
- The step returns at once if `notes_mode` is off. Otherwise it reads `folders.ini` (a few hundred bytes), resolves the folder, stores the folder path in `App`, and starts a **one-off worker thread**, the same pattern as the preview image decoder. It then posts the next step without waiting.
- The worker reads `library.ini` and the local file, scans, reconciles, and posts a finished `LibraryState` back with `WM_FASTPAD_LIBRARY_READY`, boxed in the `LPARAM`. The UI thread only swaps it in.
- Each load carries a generation number. A `LIBRARY_READY` for a folder that is no longer open is dropped.
- Until the state arrives, typing, Ctrl+N and saving all work, because the inline name box needs only the folder path. Library commands post the notice "Loading folder…".

### 7.2 Scanning

- Each directory is opened with `FILE_LIST_DIRECTORY` and `FILE_FLAG_BACKUP_SEMANTICS`, and enumerated with `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)`. That returns the name, size, mtime and file ID for every entry in bulk, without opening each file.
- The volume serial number comes from `GetFileInformationByHandle` on the folder root.
- Reparse points (junctions and symbolic links) are not followed, so a scan cannot loop.
- Skip rules and the 10,000-note limit are applied as in §5.

### 7.3 Reconciliation

Scan results are matched against the records in this order:

1. **Path**: a record whose path exists in the scan is that note.
2. **File ID**: a record whose path is gone, where a new file has the record's old volume serial and file ID in the scan cache. The note was renamed or moved outside FastPad. The record's path is updated.
3. **Fingerprint**: a record that is still unmatched, where a new file has the same size. Only those candidate files are hashed. An equal hash means the note arrived by copy or sync. The record's path is updated.
4. **Missing**: any record still unmatched is marked missing and kept. It is never deleted automatically, except by the purge in §8.4.

A `missing` entry is written to the local file the first time a record goes missing, and removed if it is matched again. Every path change updates the record, the matching `recent` entry, and the path of any open tab for that file.

### 7.4 Rescans

- When a folder opens.
- When the window becomes active (`WM_ACTIVATEAPP`) after being inactive for at least 5 seconds. This catches changes made in Explorer.
- FastPad's own saves, renames, deletions and new notes update the index directly and do not trigger a rescan.
- A rescan uses the same worker path as §7.1. At most one scan runs at a time. A request during a scan is remembered, and one more scan runs afterwards.

### 7.5 Writing `library.ini`

- Metadata operations change the in-memory `Library` on the UI thread and are appended to a list of **pending operations** (for example "favorite note X" or "create notebook Y"). The file is written about 500 ms after the last change, and immediately before a folder switch or window close.
- Before writing, FastPad compares the file's current mtime and size with the values from its last read or write. The file may have changed through sync from another PC, or from a second `--new-window` instance on the same folder.
- If it changed, FastPad re-reads it, **replays the pending operations** on top, and then writes. An operation that no longer applies (its note or notebook was removed on the other side) is dropped.
- After a successful write, the pending list is cleared.
- A failed write keeps the pending list, shows a notice, and is retried on the next change or on close.

### 7.6 Metadata that cannot be read

If `library.ini` is corrupt, has an unknown version, or cannot be read, the index still lists every file and editing is unaffected. Organizing commands are disabled and show a notice. The file is never overwritten.

## 8. Note lifecycle in the editor

All of this section applies only with `notes_mode` on.

### 8.1 New notes and labels

- Ctrl+N works as it does today: an untitled tab kept by session restore and recovery.
- An untitled tab's label is its first non-empty line, trimmed, with leading `#` characters and the following space stripped, and cut at 40 characters (with `…`). If there is no such line, the label is "Untitled".
- The label is recomputed only when an edit touches a line at or before the first non-empty line, so typing further down costs nothing.
- `Document::title` gets the label through a new `untitled_label: Option<String>` field that the window keeps up to date. The dirty marker and recovered prefixes behave as today.

### 8.2 First save: the inline name box

- Ctrl+S or Ctrl+Shift+S on an untitled tab opens an inline name box, a `panel.rs` child laid out like the find bar:
  `Name [Meeting notes].md · in FastPad · Save · Browse…`
- The name is prefilled from the tab's label and sanitized:
  - `<>:"/\|?*` and control characters are removed;
  - trailing dots and spaces are removed;
  - reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`) get a `_` suffix;
  - an empty result becomes `Untitled`.
- The extension comes from the tab's language: `.json` for JSON, and `.md` for Markdown and plain text. Typing an extension in the box overrides it.
- **Enter** saves into the folder root, creating the folder if needed. If the name already exists, the box shows an inline error and suggests the first free `Name 2`, `Name 3` and so on. Nothing is overwritten.
- **Esc** closes the box without saving.
- **Browse…** opens today's Save As dialog.
- Ctrl+Shift+S on a tab that already has a path opens today's Save As dialog, starting in the file's folder.
- The close-tab prompt keeps today's modal flow. Its Save As dialog starts in the open folder with the sanitized name prefilled.
- The box is keyboard-operable and has accessible names, following the find bar.

### 8.3 Autosave

- Applies to a tab whose file is inside the open folder, while that folder's `autosave` is `true`. It does not apply to any other tab.
- Palette command "Notes: Toggle autosave for this folder" flips the per-folder switch.
- Triggers: 1 second without an edit after a change, a tab switch away from the tab, window deactivation, and close. It uses the existing save path and atomic saver.
- Tabs that autosave never prompt on close. If the close-time save fails, FastPad falls back to today's prompt for that tab.
- **Disk-change guard:** each document remembers the file's mtime and size from its last load or save. Before an autosave, FastPad compares them with the file on disk. If they differ, autosave pauses for that tab, and a notice offers **Reload** (discard the tab's edits and reload from disk) or **Keep mine** (save the tab over the file and resume autosave). A manual Ctrl+S behaves like Keep mine.
- **Failure:** a notice in the error style names the file. The tab stays dirty, the next trigger retries, and the recovery snapshot keeps the text. A failed save is never shown as saved.

### 8.4 Rename and delete

These are palette commands until sub-project 2 adds UI for them.

- **"Note: Rename"** reuses the inline name box, prefilled with the current name. It renames the file with `MoveFileExW`, then updates the record, the `recent` entry, the scan cache and the tab. A name clash is handled as in §8.2. A failed rename leaves everything unchanged and shows a notice.
- **"Note: Delete"** asks for confirmation, then sends the file to the Recycle Bin (`IFileOperation` with `FOFX_RECYCLEONDELETE`) and closes its tab. A record, if there is one, is kept, flagged `d`, and hidden from views. If the file comes back from the Recycle Bin, path reconciliation clears the flag and its notebook and tags return. A record that has been deleted or missing for 30 days (from the `missing` time in the local file) is dropped during reconciliation. If sending the file to the Recycle Bin fails, nothing changes and a notice is shown.

### 8.5 Temporary organizing commands

Until sub-project 2 builds the sidebar, these palette commands make the model usable and testable. Sub-project 2 binds its UI to the same `CommandId`s.

- Note: Toggle favorite
- Note: Toggle pin
- Note: Move to notebook… (a picker listing notebooks, with "Notes" first and "New notebook…" last)
- Note: Add tag… and Note: Remove tag…
- Notebook: New…, Rename…, Change color…, and Delete…. Deleting moves its notes to Notes, after a confirmation that says so.
- Tag: Rename… and Tag: Remove from all notes…

They act on the active tab's file. For a file outside the open folder, they create a record with an absolute path. Names are validated by `library::model`. Errors show inline in the picker or as a notice.

## 9. Components

### `src/library/` (no window dependencies)

- `model.rs`: `NoteId`, `NotebookId`, `TagId`, `NoteRecord`, `Notebook`, `Tag`, and `Library`. It holds the notebooks, tags and records, and implements the operations and name validation (trimmed, not empty, unique ignoring case).
- `ops.rs`: `PendingOp` and replaying operations onto a freshly read `Library` (§7.5).
- `store.rs`: encoding and parsing of `library.ini`, and the escaping rules.
- `local.rs`: the per-folder local file and `folders.ini`.
- `scan.rs`: the directory walk, skip rules and limit, returning `ScanEntry { path, file_id, volume_serial, size, mtime }`.
- `reconcile.rs`: matching scan results against records (§7.3). It takes a hashing function as an argument so tests need no files.
- `title.rs`: the untitled label, filename sanitizing and clash numbering.
- `mod.rs`: `LibraryState` (the folder, the `Library`, the index and the local state) and the worker entry point that loads and scans.

### `src/window/library_host.rs`

`main_window.rs` is already about 6,600 lines, so the window code goes here instead:

- the `OPEN_LIBRARY` and `LIBRARY_READY` handlers, rescans on activation, and debounced metadata writes;
- the folder commands and the folder drop;
- the inline name box;
- the autosave timer and the disk-change guard;
- the temporary organizing commands.

`main_window.rs` only routes messages and commands to it.

### Other changes

- `config`: the `notes_mode` setting.
- `ipc/protocol.rs`: `OpenFolder`.
- `bootstrap`: directory arguments become folder requests.
- `document.rs`: `untitled_label`, and the on-disk mtime and size used by the guard.
- `window/commands.rs` and `menus.rs`: the new commands and the File menu item.
- `messages.rs`: the new deferred step and `LIBRARY_READY`.
- `platform`: the folder picker, the Documents known-folder lookup, and the Recycle Bin wrapper.

## 10. Error handling summary

| Situation | Behavior |
|---|---|
| `library.ini` corrupt, unreadable, or a newer version | Every file is listed and editable. Organizing is disabled with a notice. The file is never overwritten. |
| `library.ini` changed on disk before a write | Re-read, pending operations replayed, then written. |
| `library.ini` write fails | Pending operations kept, notice shown, retried later. |
| Local file or `folders.ini` unreadable | Treated as absent. It is a cache. |
| Remembered folder missing at startup | Fall back to `Documents\FastPad` with a notice. |
| A record's file cannot be found | Marked missing and kept, dropped after 30 days. |
| Autosave fails | Error notice, the tab stays dirty, retry on the next trigger, the recovery snapshot keeps the text. |
| File changed on disk under an autosaving tab | Autosave pauses, notice offers Reload or Keep mine. |
| Rename, delete or move fails | Nothing changes, notice shown. |
| Library worker fails | Notice shown. Plain editing continues. |

## 11. Performance

- Nothing new runs before the first paint or the first input. `OPEN_LIBRARY` reads only `folders.ini` on the UI thread.
- Scanning and reconciliation run on the worker thread. The target is under 500 ms warm for 10,000 notes on the reference i5-4590.
- Handling `LIBRARY_READY` on the UI thread should take under 5 ms.
- The index for 10,000 notes should take about 1 to 2 MB.
- Recomputing the untitled label reads at most the first few lines, and only when those lines change.
- `fastpad-bench` gains a fixture folder with 10,000 files. It checks that warm TTI and first paint stay within noise of the current numbers with that folder open, and reports scan and reconcile times (cold and warm), `LIBRARY_READY` handling time, and idle memory.

## 12. Testing

### Unit tests (`src/library/`)

- `store` and `local`:
  - encode/parse round trips;
  - escaping of `%`, `|`, CR and LF, and a path containing `|`;
  - unknown keys, malformed lines, unknown notebook and tag references;
  - an unknown version makes the file unreadable;
  - absolute paths for external notes;
  - a `folder` mismatch in the local file.
- `model`:
  - name validation (trimmed, empty, duplicate ignoring case);
  - deleting a notebook moves its notes to Notes;
  - a tag with no notes is dropped.
- `ops`: two diverged copies merge by replay without losing either side's changes, and an operation whose target was removed is dropped.
- `reconcile`, with a fake scan and hasher:
  - a path match;
  - a rename by file ID;
  - a sync match by size and hash;
  - the same size with a different hash;
  - a record goes missing, and comes back;
  - a deleted record returns from the Recycle Bin;
  - the 30-day purge.
- `title`:
  - labels (a leading `#` is stripped, blank lines are skipped, cut at 40 characters);
  - sanitizing (invalid characters, trailing dots, reserved names, an empty name);
  - clash numbering.
- `scan`: skip rules, not following reparse points, and the note limit, on a real temp folder.
- `config/persisted.rs`: `notes_mode` with each boolean spelling, default `true`.
- `messages.rs`: the deferred order includes `OPEN_LIBRARY`.
- `ipc/protocol.rs`: `OpenFolder` round trip.

### Windows integration tests (new `tests/windows/library.rs`, with a `[[test]]` entry)

These use a scratch `LOCALAPPDATA` and a temp folder, and run with `--test-threads=1` like the other window tests.

- Open a folder with a subfolder tree. The index follows the skip rules.
- Rename a file with `MoveFileExW` outside FastPad, then reactivate the window. The note keeps its notebook and tags.
- Ctrl+N and type. The tab label follows the first line. Ctrl+S prefills the inline box, and Enter creates the file in the folder root. A clashing name is refused with a suggestion.
- Autosave writes after the idle delay. An external write in between trips the disk-change guard, and the file is not overwritten.
- A second launch with a folder argument switches the running window through IPC.
- A corrupt `library.ini` leaves every file openable and editable, and the file is byte-for-byte unchanged afterwards.
- `notes_mode=false`: the existing save and session tests pass unchanged, and no `.fastpad\` or `libraries\` files are created.

## 13. Open items for later sub-projects

- The sidebar's folder switcher, note list, and the unsaved-note entries in Notes (sub-project 2).
- A shortcut for "toggle favorite" that does not clash with Format JSON (sub-project 2).
- A per-folder ignore file, a file-system watcher, and merging OneDrive conflict copies of `library.ini`.

## 14. Implementation notes

Decisions made while building this design, not anticipated by the sections above.

- **Deleting to the Recycle Bin** (§8.4) uses `SHFileOperationW` with `FOF_ALLOWUNDO` instead of `IFileOperation`. It gives the same behavior with no COM vtable. `recycle` takes only an absolute path to a file; a relative path or a directory is refused before anything is sent to the shell. The shell still warns before a permanent delete, for example when there is no Recycle Bin to send to (a network share, the bin disabled or over quota).
- **Note extensions are an explicit list** (§5): `md`, `markdown`, `txt`, `text`, `json`, `log`, `ini`, `cfg`, `conf`, `yaml`, `yml`, `toml`, `csv`, `xml`. `detect_language`, which drives syntax highlighting, knows only `json` and `md`; every other note extension is plain text.
- **OneDrive online-only files** (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` or `OFFLINE`) are listed by the scanner but never hashed, so reconciliation's fingerprint pass (§7.3) never downloads them.
- **The disk-change notice** (§8.3) offers *Reload* and *Keep mine* as the palette commands "Note: Reload from disk" and "Note: Keep my version", since notices have no buttons yet. "Note: Keep my version" on a tab that has never been saved does nothing.
- **Dropping files on the window opens them as tabs** (§5). FastPad handled no drops before this design, so "dropped files keep today's behavior" had nothing to keep. Both files and folders dropped on the text area work; FastPad wraps Scintilla's drop target, and text drag-and-drop inside the editor still goes to Scintilla.
- **Reconciliation** (§7.3) runs the file-ID pass to completion before the fingerprint pass, and both passes only ever consider files whose path is new since the previous scan's cache — on a folder's first scan on a given PC, every file counts as new. A truncated scan (over the 10,000-note limit) never marks records missing or purges them, since it did not see the whole folder.
- **Rescan merges** (§7.5): changes flushed to `library.ini` while a scan is running are kept; if another PC changed `library.ini` during the interval, one stat of the file decides which side is current; notes created or renamed while a scan is running stay in the index.
- **Opening a folder is refused**, with the current folder left open and a notice shown, if the current library's pending metadata operations can't be flushed to disk first.
- **The first save through the inline name box** (§8.2) never replaces an existing file, even one created after the name was checked; the save uses an exclusive create and reports `SaveOutcome::NameTaken` rather than overwriting.
- **Save As** behaves differently depending on the setting: with notes mode off, it suggests "Untitled.txt" exactly as before this design; with notes mode on, Save As of a file that already has a path starts in today's dialog start folder, not the open folder.
- **Autosave** (§8.3) waits until the folder has finished loading, and pauses instead of writing when FastPad has no known on-disk stamp for the file yet or the file has vanished. A failed autosave shows one notice naming the file.
- **Organizing commands** (§8.5) work on files outside the open folder by creating a record with an absolute path, as §8.5 describes. They are refused outright — not just left with nothing to act on — while notes mode is off, the folder is still loading, or `library.ini` is unreadable.
- **Two parts of this spec are deferred beyond what §13 lists:** reordering notebooks exists in the model (`move_notebook`) but has no command yet; sub-project 2's sidebar will drive it. The inline name box shows a name clash as text next to the field rather than as a separate notice.
- **Tests:** in-process tests use a per-process scratch profile instead of the real one; real-exe tests in `tests/windows/` seed a scratch `folders.ini`; the library end-to-end tests refuse to run while any FastPad window is already open, to avoid colliding with a real session.
