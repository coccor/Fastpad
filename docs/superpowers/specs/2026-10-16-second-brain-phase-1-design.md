# Second Brain, Phase 1: Linked Notes Design

Status: Draft design, pending review  
Date: 16 October 2026  
Builds on: `2026-09-23-note-library-design.md` ("the library spec"), `2026-09-23-note-sidebar-design.md` ("the sidebar spec"), `2026-09-24-note-search-design.md`, `2026-09-24-quick-open-design.md`, `2026-09-16-markdown-preview-design.md`  
Followed by: Phase 2, `fastpad-mcp` (a separate sidecar that gives AI agents access to the notebook), designed in its own spec.

## 1. Purpose

FastPad is becoming a second brain: one place where thoughts are captured quickly, connected to each other, and found again, by the user and by AI agents working on the user's behalf.

The plan has four phases. This spec covers the first one. It adds no AI and no network access. It makes the notebook **connected** (links, backlinks, tags) and **easy to write into** (daily notes, quick capture), and it makes FastPad **show changes that other programs make to the files**. Each of these is useful on its own. Together they are what Phase 2 needs: agents that read and write the same plain Markdown files the user edits in FastPad.

The lasting rules still apply:

- Notes are plain files. Everything this spec adds lives in the text of those files, or in a cache FastPad can rebuild at any time.
- Nothing new runs before the first paint or the first input, and there is one UI thread.
- FastPad makes no network access.
- Dependencies stay minimal. This spec adds none: `pulldown-cmark` 0.13.4, already used by the preview, parses wikilinks (`Options::ENABLE_WIKILINKS`) and front matter (`Options::ENABLE_YAML_STYLE_METADATA_BLOCKS`).

Success looks like this: an agent edits a note while the user has it open. The change appears in FastPad straight away and can be undone with Ctrl+Z. The agent's new `[[links]]` show up in the user's backlinks, and its `#tags` show up in the Tags view.

## 2. Decisions

| Topic | Decision |
|---|---|
| Link syntax | Obsidian-compatible: `[[Note]]`, `[[Note\|label]]`, `[[Note#Heading]]`, `[[folder/Note]]`. Standard Markdown links to local `.md` files count as links too. |
| Tag syntax | Inline `#tag` and `#nested/tag` in body text, plus `tags:` in front matter |
| Metadata | YAML front matter at the top of Markdown files, read with a small parser for a subset of YAML. FastPad never rewrites it in Phase 1. |
| Where links and tags are stored | Only in the note text. A per-PC **note index cache** speeds up startup and can be thrown away at any time. |
| Notebook conventions | A new shared file, `.fastpad\notebook.ini`, holds the daily folder, the date format, the inbox file and the templates folder. It is written only when the user changes a default. |
| Backlinks and tags UI | Two new sidebar views: **Links** (for the active note) and **Tags** (for the whole notebook) |
| Reacting to outside changes | A `ReadDirectoryChangesW` watcher on the open notebook, plus stat checks for other tabs. A clean tab reloads as a single undoable edit. A dirty tab enters a conflict state. |
| Capture | A global hotkey while FastPad runs; `fastpad --capture` to open the capture box even when FastPad is closed; `fastpad --capture "text"` to append with no window. FastPad stays without a background service. |
| Delivery | Five PRs (§12). The external-change work comes first, because agents need it most. |

Rejected alternatives:

- **Storing links or tags in `library.ini`.** The sidebar spec removed tags from `library.ini` so that files stay the single source of truth. Tags in the note text keep that rule, and an agent writing a file can tag it without knowing FastPad exists.
- **A full YAML parser.** It would be the first new dependency in a long time, and notes only need flat keys, strings and lists.
- **A database (SQLite) for the index.** Too much binary weight and too much startup risk for a cache the size of a few MB.
- **A FastPad-specific link syntax.** Obsidian's syntax is what agents and other tools already write, and with it an existing Obsidian vault opens as a FastPad notebook with no changes.
- **Watching nothing and polling stats on a timer.** Tabs would show agents' changes several seconds late, and the polling would wake the process even when idle.

## 3. Relation to earlier decisions

- **Tags come back, in the file text this time.** The sidebar spec (§3) removed tags that were stored in `library.ini`. These tags are text in the note, so none of the reasons for removing them apply: nothing breaks when a file is synced, renamed or edited by another program.
- **The library spec's non-goal "a file-system watcher"** (§2, §13) is now a goal for the open notebook. Rescans when the window becomes active stay, as the fallback for folders a watcher cannot watch.
- **The disk-change guard** (library spec §8.3) is kept and widened. Today it checks a file only right before an autosave. After this spec, changes are noticed when they happen, for every tab, with autosave on or off.
- **Library sub-project 4, "links and backlinks"**, is delivered by §7 of this spec.

## 4. Goals and non-goals

### Goals

- Changes made to a file outside FastPad appear in its open tab within about half a second when the file is in the open notebook, and when the window or tab is next activated when it is not.
- Write `[[` and pick a note. Click a link in the preview, or Ctrl+click one in the editor, to open it. A link to a note that doesn't exist yet creates the note.
- See every note that links to the active note, with the line that links to it.
- Renaming or moving a note in FastPad offers to update the links that point to it.
- Tag notes inline or in front matter, and browse the notebook by tag.
- Open today's daily note with one key, created from a template the first time.
- Capture a thought from anywhere in Windows in under a second, without switching windows.

### Non-goals (for Phase 1)

- Any AI, model or network access. Phase 2 is a separate sidecar.
- Embeds (`![[Note]]` showing another note's content). An embed is shown as a plain link.
- "Unlinked mentions" (text that names a note without linking to it).
- A graph view.
- Completing `#heading` inside `[[Note#…]]`. Headings are matched when the link is followed, but not offered as you type.
- General note templates ("New note from template"). Only the daily note has a template.
- Editing front matter from the UI, and queries such as `tag:x` in the Search view.
- Merging conflicting edits. A conflict is resolved by choosing one side, or by comparing the two (§5.4).
- Updating links after a rename made outside FastPad.

## 5. Changes made outside FastPad (PR 1a)

### 5.1 Noticing changes

**Notebook watcher.** While a notebook is open, a dedicated worker thread watches it with `ReadDirectoryChangesW`:

- recursive, filter `FILE_NOTIFY_CHANGE_FILE_NAME | DIR_NAME | LAST_WRITE | SIZE`;
- overlapped I/O with a 64 KB buffer, waiting on the I/O event and a stop event;
- changes are collected for 250 ms after the last event, then posted to the UI thread as one `WM_FASTPAD_FILES_CHANGED` carrying a boxed set of paths and a flag that says whether names changed;
- a buffer overflow (the call completes with zero bytes) posts a "full rescan" instead of paths;
- paths under `.fastpad\` are passed on only for `library.ini` (pins changed on another PC) and `notebook.ini` (§9.1).

The watcher starts when the notebook's first scan finishes, and is stopped and joined when the notebook is closed or switched and when the window closes. If `ReadDirectoryChangesW` fails (some network shares, FAT drives, or an error later on), the watcher quits. A single line goes to the diagnostic log, and FastPad keeps today's behavior: a rescan when the window is activated after 5 seconds.

On the UI thread, `FILES_CHANGED`:

- for each changed path open in a tab, runs the tab check (§5.2);
- for each changed Markdown note, queues an index update (§6.4);
- when names changed, requests a rescan through the existing path (library spec §7.4). That keeps the tree, pins and reconciliation correct with no new code for renames.

FastPad's own writes cause no extra work: after every save, the tab's `disk_stamp` already equals the file, so the tab check finds nothing to do. Other writes FastPad makes (replace across notes, link updates, capture) record the new stamps they write in the same way, and are not treated as outside changes.

**Tabs outside the notebook,** and every tab when no watcher is running, are checked:

- when the window is activated (`WM_ACTIVATEAPP`): every visible tab;
- when a tab becomes visible in its group: that tab.

A check is one `GetFileAttributesExW` for each tab, compared with its `disk_stamp`. This applies with `notes_mode` off too, because it is an editor feature and not a notebook feature.

### 5.2 The tab check

For a tab with a path, FastPad compares the file's current stamp with `document.disk_stamp`:

| Disk | Tab | Result |
|---|---|---|
| Same stamp | any | Nothing |
| Changed | Clean | **Reload** (§5.3), when the tab is visible. A hidden tab is marked `stale` and reloads when it is shown. |
| Changed | Dirty | **Conflict** (§5.4) |
| Missing | Clean | The title shows "(deleted)". The tab stays open with its text. Autosave doesn't re-create the file; Ctrl+S does. |
| Missing | Dirty | Conflict, with the "deleted on disk" wording |
| Missing, then back | any | The marker clears and the check runs again |

A tab restored from a snapshot with no known stamp (library spec §8.3) is treated as dirty, as today.

The setting `reload_changed_files` (`auto` or `ask`, default `auto`) controls whether clean tabs reload by themselves. With `ask`, a clean tab is handled like a conflict.

### 5.3 Reload

A reload replaces the tab's text with the file's text **as an edit, not as a new document**:

1. Read the file with the existing loader (encoding detected, as when opening). A file over 8 MB isn't reloaded automatically; it is treated as a conflict, so a huge log being written can't freeze the UI.
2. Compare the old and new text. Find the common prefix and suffix and replace only the part between them, inside one `SCI_BEGINUNDOACTION` / `SCI_ENDUNDOACTION`.
3. Set a save point. The tab is clean, and `disk_stamp`, `encoding` and `autosave_paused = false` are updated as `reload_from_disk` does today.
4. Scintilla keeps the caret, selection, folds and scroll position for every change that doesn't cover them. A caret inside the replaced range moves to the range's end.
5. The preview, the outline and the recovery snapshot update as they do after typing.

**Ctrl+Z after a reload brings back the text from before the change**, and the tab becomes dirty with that text. That is how the user rejects an agent's edit. The status bar shows "Reloaded: changed outside FastPad" for 4 seconds. Reloads don't post notices, because an agent may change files many times a minute.

The existing "Note: Reload from disk" switches to the same edit-style replacement.

### 5.4 Conflicts

A tab in conflict:

- has `autosave_paused = true` (the flag that exists today);
- shows a conflict mark in its tab: the dirty dot is replaced by a warning glyph in the theme's warning color, and its accessible name gains "changed on disk";
- posts one notice per conflict: "<name> changed on disk while you had unsaved edits." (or "…was deleted on disk…").

Three palette commands resolve it, and work in the conflict tab:

- **Note: Reload from disk** (exists): take the disk version, as an undoable edit (§5.3).
- **Note: Keep my version** (exists): save the tab over the disk version.
- **Note: Compare with disk** (new): opens the disk version in a read-only tab named "<name> (on disk)", in a split to the right of the conflict tab. FastPad has no diff view. The two texts side by side are enough to copy what's needed.

A later outside change to a tab already in conflict changes nothing except the stamp the conflict compares against.

Notices still have no buttons (library spec §14). Adding buttons is a separate change; the palette commands are enough for now.

## 6. The note index (PR 1b)

### 6.1 What is indexed

The index holds facts about every **Markdown note** (`.md`, `.markdown`) in the open notebook that is 2 MB or smaller:

```rust
struct NoteFacts {
    stamp: DiskStamp,
    aliases: Vec<String>,          // front matter `aliases` / `alias`
    tags: Vec<TagUse>,             // name (as written), line, source: FrontMatter | Inline
    links: Vec<LinkUse>,           // target text, heading, label, line, kind: Wiki | Markdown
    properties: Vec<(String, String)>, // other front matter keys, as raw text, for the preview
}
```

Links and tags are found by parsing the text with `pulldown-cmark` using the preview's options plus `ENABLE_WIKILINKS` and `ENABLE_YAML_STYLE_METADATA_BLOCKS`. Text inside code spans, code blocks and HTML is never a link or tag, because those events are skipped.

Notes with other extensions (`.txt`, `.json`, …) are not parsed. They can still be link *targets*.

### 6.2 Front matter

Front matter is recognized only when the first line of the file (after a BOM) is exactly `---`, and a later line is exactly `---` or `...`, within the first 200 lines. Inside it, FastPad reads:

- `key: value`, with the value plain, `'single-quoted'` or `"double-quoted"`;
- `key: [a, b, "c d"]`;
- `key:` followed by lines of `  - item`;
- lines starting with `#` are comments.

Anything else (nested maps, multi-line strings, anchors) is kept as raw text for that key and never causes an error. Keys are matched without regard to case. FastPad uses:

| Key | Use |
|---|---|
| `tags`, `tag` | Tags. A leading `#` is optional. A single string may hold several tags separated by commas or spaces. |
| `aliases`, `alias` | Other names that resolve links to this note (§7.2) |
| anything else | Shown in the preview's properties table only |

**Preview:** front matter is drawn as a compact two-column properties table at the top, in the muted text color, with tags shown like inline tags. Today a front matter block is drawn as a rule followed by a setext heading; enabling the option fixes that too.

**Editor:** no change in Phase 1. Lexilla's Markdown lexer keeps styling front matter as plain text.

### 6.3 Inline tags

An inline tag is `#` followed by one or more letters, digits, `_`, `-` or `/`, with at least one letter. The `#` must be at the start of a text run or follow whitespace or `(`. A trailing `/` isn't part of the tag. So `#project/fastpad`, `#2026-q4` and `#todo` are tags, while `#123`, `C#`, `a#b` and `# Heading` are not.

Tags are matched without regard to case (Unicode lowercase). The Tags view shows the spelling seen most often, with ties going to the first in natural sort order. A nested tag also counts under each of its parents: `#project/fastpad` is listed under `project`.

### 6.4 Building and updating

**Build.** When `LIBRARY_READY` swaps in a new library state, FastPad starts a one-off **index worker**. For each Markdown note in the scan, it reuses the cached facts when the note's stamp matches the cache, and otherwise reads and parses the file. It posts the finished `NoteIndex` back with `WM_FASTPAD_INDEX_READY`, boxed, with the same generation check as `LIBRARY_READY`. Until then, the Links and Tags views show "Indexing notes…", and `[[` completion falls back to the names in the scan.

**Updates.** A save in FastPad, a watcher change (§5.1), or a rescan that finds new stamps queues the affected paths. One update worker runs at a time. Requests that arrive while it runs are merged and run after it, as with rescans. The UI thread applies the result as a delta and refreshes the Links and Tags views only if what they show changed.

The index reflects **saved text**, not unsaved edits. With autosave's 1-second delay that is close enough, and it keeps the UI thread free of parsing.

**Cache.** The facts are written to `%LocalAppData%\FastPad\libraries\<folder key>.notes.cache`, beside the per-PC library file `<folder key>.ini` (library spec §6.3), by the worker, only when they changed. The format is versioned lines in the style of the local file. Any parse error or unknown version discards the whole cache, and the next build reads every note.

**Limits.** The index covers the scan's notes (at most 10,000, library spec §5). Facts for a note are capped at 2,000 links and 500 tags; anything beyond that is dropped and noted in the diagnostic log.

## 7. Links and backlinks (PR 1c)

### 7.1 What counts as a link

- **Wikilinks:** `[[target]]`, `[[target|label]]`, `[[target#heading]]`, `[[target#heading|label]]`, `[[#heading]]` (a heading in the same note). `![[target]]` is treated as a link (embeds are a non-goal).
- **Markdown links** whose destination resolves to a file in the notebook, using the preview's existing `classify_link` rules: relative to the note's folder, never UNC or device paths.

### 7.2 Resolving a wikilink target

From the target text, with any `#heading` removed:

1. **Path form**, when the target contains `/` or `\`: it is resolved relative to the notebook root, then relative to the linking note's folder. Without an extension, `.md` is tried, then `.markdown`.
2. **Name form:** notes whose file stem matches the target (ignoring case). When the target has an extension, the whole file name must match it.
3. **Alias:** notes whose front matter `aliases` match the target (ignoring case).

When more than one note matches at a step, the winner is the note in the linking note's folder, then the one with the fewest path components, then the first in natural sort order. Ambiguity is fine; it is never an error.

Resolution uses the index's name map **when the link is looked up**, not when it is indexed. So creating a note immediately resolves every link that was waiting for it.

### 7.3 Following links

**Preview:**
- A resolved link is drawn in the link color. Clicking it opens the note as today's local-file links do, then scrolls to `#heading` when there is one (matched like the preview's existing anchor slugs).
- An unresolved link is drawn in the muted color with a dashed underline, and its tooltip reads "Create note “target”". Clicking it creates `<target>.md` in the notebook root (named with the library spec §8.2 sanitizing; a path form creates its folders), then opens it. The file starts empty. With no notebook open, clicking shows the notice "Open a notebook to create linked notes."

**Editor (Markdown tabs):**
- Links in the visible lines are marked with a Scintilla indicator (a thin underline in the link color, or dashed for unresolved). Marks are recomputed for the visible range after styling, so a large file costs nothing off-screen.
- Ctrl+click on a marked link follows it as the preview does. The mouse pointer becomes a hand while Ctrl is held over a link.
- **Follow link at caret** (`notes.followLink`, default F12) does the same from the keyboard.

### 7.4 Completion

Typing `[[` in a Markdown tab, while a notebook is open, opens a completion list of notes:

- ranked by the quick-open matcher (`library::quick_open::search`) against what has been typed since `[[`, showing the top 50;
- each item shows the file stem, with the folder in a quieter style when two notes share a stem;
- the list refreshes on every keystroke, and closes on `]]`, `|`, `#`, Esc, or when the caret leaves the brackets;
- Enter or Tab inserts the shortest target that resolves to the chosen note (the stem, or the path when the stem is ambiguous), followed by `]]` unless it's already there. This is one undo step.

The list uses Scintilla's autocompletion (`SCI_AUTOCSHOW`), with FastPad supplying the ranked items each time, so screen readers get Scintilla's existing list accessibility.

### 7.5 The Links view

A new sidebar view, **Links** (`sidebar_view=links`, an activity bar icon, palette "View: Show links", no default shortcut). It follows the active tab:

```
LINKS — Project Atlas
▾ Linked from (3)
    2026-10-14              “…kickoff for [[Project Atlas]] moved to…”
    Meeting notes           “- [[Project Atlas]]: budget approved”
    People/Dana             “Owns [[Project Atlas|Atlas]] frontend”
▾ Links to (2)
    Roadmap
    Vendor list             not created
```

- **Linked from** lists each linking note once, with up to three of its linking lines below it as snippets. Enter or a double-click opens the note at that line.
- **Links to** lists the active note's outgoing links in order, without duplicates. Unresolved ones say "not created"; opening one creates it, as in §7.3.
- Other states: "This file isn't in the open notebook.", "No notebook open.", "Indexing notes…", and "No links yet. Type [[ to link to another note."
- The view uses the shared tree row list, keyboard model and accessibility of the Notebook view (sidebar spec §10).

### 7.6 Updating links on rename and move

When a note is renamed or moved **by FastPad** (the inline name box, a tree drag, "Note: Move to notebook…"), FastPad finds the links that resolved to the note before the change and will no longer resolve to it afterwards. If there are any, a prompt asks:

> Update 7 links in 4 notes to point to “New name”?  
> **Update links** · Leave them

- Wikilinks keep their form: the stem stays a stem if it is unambiguous after the change, and otherwise becomes a path. The label and `#heading` are kept.
- Markdown links get the new relative path, with the original's percent-encoding style.
- The rewrite reuses the replace-across-notes machinery (search spec, 3b): open tabs are edited in their buffers as one undo step each, and closed files are rewritten with the atomic saver, recorded as FastPad's own writes (§5.1).
- A file that fails to write is listed in a notice. The others stay updated.

Renames made outside FastPad don't rewrite links. The links dangle until the user fixes them.

## 8. Tags (PR 1b)

### 8.1 The Tags view

A new sidebar view, **Tags** (`sidebar_view=tags`, an activity bar icon, palette "View: Show tags"):

```
TAGS
▾ project (12)
    ▸ atlas (5)
    ▸ fastpad (7)
▸ todo (9)
▸ waiting (2)
```

- Tags are a tree split on `/`, sorted by natural order. The count is the number of notes with that tag or any tag nested under it.
- Expanding a tag lists its notes (direct ones first), then its child tags.
- Opening a note works like the Notebook tree, including the preview tab (sidebar spec §6.4).
- A filter box at the top narrows the tree as you type.

### 8.2 Tags in the preview

Inline tags are drawn as small rounded chips in the accent color. Clicking a chip shows the Tags view with that tag expanded and selected.

## 9. Daily notes and notebook conventions (PR 1d)

### 9.1 `.fastpad\notebook.ini`

A new shared file, written next to `library.ini`, holds the conventions of a notebook. Agents can read it too (Phase 2).

```ini
version=1
daily_folder=journal
daily_format=YYYY-MM-DD
daily_template=.fastpad/templates/daily.md
inbox=inbox.md
capture_target=inbox
```

| Key | Meaning | Default |
|---|---|---|
| `daily_folder` | Folder for daily notes, relative to the notebook | `journal` |
| `daily_format` | File name pattern: `YYYY`, `MM`, `DD`, `ddd` (Mon), `dddd` (Monday). Anything else is literal. Must contain `YYYY`, `MM` and `DD`. | `YYYY-MM-DD` |
| `daily_template` | Template for a new daily note | `.fastpad/templates/daily.md` |
| `inbox` | Capture inbox file | `inbox.md` |
| `capture_target` | `inbox` or `daily` | `inbox` |

- When the file is missing, the defaults apply. It is created only when the user changes a value, which keeps the library spec rule that nothing is written into a folder until the user organizes something.
- Settings gains a **This notebook** section with these five values. It is disabled when no notebook is open.
- Hand edits keep comments and unknown lines, as `fastpad.ini` does. A bad value falls back to its default, with a notice naming the line.
- A watcher change to the file (§5.1) re-reads it.

### 9.2 Opening today's note

**Notes: Open today's note** (`notes.today`, default Ctrl+Alt+D, also in the palette and in the Notebook header's `…` menu):

1. Today is the local date when the command runs.
2. The path is `<notebook>\<daily_folder>\<daily_format>.md`.
3. If the file exists, it opens (or its tab is activated).
4. Otherwise the folder is created, then the file is written atomically from the template (§9.3), or, without a template, `# <dddd, D MMMM YYYY>` followed by a blank line. It then opens with the caret at `{{cursor}}`, or at the end.

**Notes: Previous daily note** and **Notes: Next daily note** (palette only, bindable) move through the existing files in the daily folder whose names match the format, sorted by date, starting from the active daily note or from today.

With no notebook open, these commands show "Open a notebook to use daily notes."

### 9.3 Templates

A template is a plain text file. The following placeholders are replaced when a note is created from it:

| Placeholder | Value |
|---|---|
| `{{date}}` | Today, in `daily_format` |
| `{{date:FORMAT}}` | Today, in FORMAT (the tokens above, plus `D`, `MMM`, `MMMM`) |
| `{{time}}` | Now, `HH:mm` |
| `{{title}}` | The new file's stem |
| `{{cursor}}` | Removed; the caret starts here |

Unknown placeholders are left as they are. A missing or unreadable template falls back to the default heading, with a notice naming the template path.

## 10. Quick capture (PR 1e)

### 10.1 Ways in

| Entry point | When FastPad is running | When it is not |
|---|---|---|
| Global hotkey `capture_hotkey` (default Ctrl+Alt+N) | Opens the capture box | Not available. FastPad keeps no background service. |
| `fastpad --capture` | Forwarded over IPC; the running instance opens the capture box | A **capture-only** FastPad starts: it shows only the capture box and exits after it closes |
| `fastpad --capture "text"` | Forwarded; appended with no window | The capture-only process appends with no window, then exits |
| `fastpad --capture -` | As above, with the text from stdin | As above |

`fastpad --capture` makes a hotkey possible without a background service: the Start menu shortcut's **Shortcut key** (or any launcher) can run it, and FastPad's fast start makes the box appear in about the time a running app would take. The installer gains an optional task, "Quick capture shortcut (Ctrl+Alt+N)", which creates a Start menu shortcut with that key. When FastPad is running, its own `RegisterHotKey` holds the same key, so the two don't fight.

- `capture_hotkey` is a `fastpad.ini` setting in keymap syntax. Empty turns it off. If `RegisterHotKey` fails (another app holds the key), one notice says so and suggests changing it in Settings.
- The capture-only process skips session restore, recovery, the library scan and the main window. It reads `fastpad.ini` and `folders.ini`, takes the instance mutex only to forward when another instance owns it, and otherwise holds no mutex. If a full FastPad starts while the capture box is open, both run, and the append (§10.3) is safe either way.
- `LaunchRequest` gains `Capture(Option<CaptureText>)`, and `IpcRequest` gains `Capture` (command 5), with the text as UTF-16. Text that doesn't fit the 64 KB frame is refused with exit code 2. A capture-only process exits with code 0 on success, and 1 when the append failed.

### 10.2 The capture box

A small top-most window (about 520 × 180 DIP), centered on the monitor under the mouse, in the current theme:

```
┌ Capture → inbox.md ─────────────────────────────┐
│ Call Dana about the Atlas budget #project/atlas │
│                                                 │
└──────────────── Ctrl+Enter save · Esc close ────┘
```

- A multi-line edit control. Enter inserts a new line.
- **Ctrl+Enter** appends and closes. **Ctrl+Shift+Enter** appends, closes, and opens the target in FastPad with the caret on the new entry (in a capture-only process, this starts FastPad normally with that file).
- **Esc** closes. Text that wasn't saved is kept as a draft in `%LocalAppData%\FastPad\capture-draft.txt` and comes back the next time the box opens. A successful capture deletes the draft.
- The box stays open when it loses focus, so the user can copy something from another window into it.
- The title names the target. Pressing **Ctrl+Alt+N** again inside the box toggles between `inbox` and `daily` for this capture only.
- It has an accessible name ("Quick capture to inbox.md") and is fully keyboard-operable.

### 10.3 Appending

The target is the open notebook in a running instance, or the most recent notebook from `folders.ini` in a capture-only process (falling back to `Documents\FastPad`, as in library spec §7.1). It is `inbox`, or today's daily note created as in §9.2.

The entry is a Markdown list item stamped with the time:

```markdown
- 14:32 Call Dana about the Atlas budget #project/atlas
  second line of the same capture
```

Lines after the first are indented by two spaces so they stay inside the list item. A blank line is added before the entry when the file doesn't already end with a blank line or a list item.

Writing:

- **The target is open in a tab** of the running instance: the entry is inserted at the end of the buffer as one undo step. A tab that was clean is then saved. A dirty tab is saved by autosave when it is on; otherwise it stays dirty and a notice says the capture is in the unsaved tab.
- **Otherwise:** FastPad reads the file with the loader (keeping its encoding and its most common line ending), appends, and writes with the atomic saver. If the stamp changed between the read and the write, it reads again and retries, up to three times. A missing inbox is created with the heading `# Inbox`.

Errors leave the text in the draft file, keep the box open, and show the error in the box itself.

## 11. Performance

- Nothing in this spec runs before the first paint or the first input. The watcher, the index worker and the hotkey registration all start after `LIBRARY_READY` or `BUILD_CHROME`.
- **Watcher:** no CPU use while idle (one blocked thread, about 64 KB of buffer). `FILES_CHANGED` handling on the UI thread should take under 2 ms for 100 paths, not counting reloads.
- **Reload:** under 5 ms for a 100 KB note on the reference i5-4590, including the prefix and suffix comparison.
- **Index:** a cold build of 10,000 notes (about 30 MB of Markdown) under 3 s on the worker, and a warm build from the cache under 300 ms. The in-memory index stays under 8 MB for that fixture.
- **Editor link marks:** only visible lines, at most once per styling pass, under 0.5 ms per screen.
- **Completion:** re-ranking 10,000 names per keystroke under 5 ms (it is the quick-open matcher).
- **Capture-only start:** box visible within 80 ms warm (the main window's budget plus a margin), no Scintilla or Direct2D load.
- `fastpad-bench` gains index build (cold and warm), reload of a 100 KB note, and capture-only time to visible.

## 12. Delivery

Five PRs, in this order. Each one is useful without the ones after it.

| PR | Scope | Depends on |
|---|---|---|
| **1a** | Outside changes: watcher, tab checks, edit-style reload, conflicts, Compare with disk (§5) | none |
| **1b** | Note index, front matter, inline tags, Tags view, front matter in the preview (§6, §8) | 1a (index updates come from the watcher) |
| **1c** | Wikilinks: preview, editor marks and Ctrl+click, completion, Links view, link updates on rename (§7) | 1b |
| **1d** | `notebook.ini`, Settings section, daily notes, templates (§9) | none (can come before 1b) |
| **1e** | Quick capture: hotkey, `--capture`, capture box, appending, installer task (§10) | 1d (for `notebook.ini` and the daily target) |

Each PR updates the README (features, shortcuts, `fastpad.ini` keys) in the style of the existing sections.

## 13. Error handling

| Situation | Behavior |
|---|---|
| Watcher can't start or fails later | Diagnostic log line. Rescans when the window is activated, as today. |
| Watcher buffer overflow | Full rescan, then tab checks for every visible tab |
| File changed under a clean tab, over 8 MB | Handled as a conflict |
| Reload read fails (locked, bad encoding) | The tab is untouched and marked stale. The next check tries again. One notice per file per session. |
| Outside change under a dirty tab | Conflict (§5.4) |
| Front matter that isn't valid YAML | The keys that can be read are used, and the rest is kept as raw text. Never an error. |
| `notes.cache` unreadable or a wrong version | Thrown away and rebuilt |
| A note can't be read while indexing | Left out of the index, and kept in the scan |
| Link update fails for some files | The others stay updated. A notice lists the failures. |
| Unresolved link clicked with no notebook open | Notice; nothing is created |
| Daily folder can't be created, or template unreadable | Notice. A missing template falls back to the default heading. |
| `notebook.ini` bad value | Default used, and a notice names the line |
| `RegisterHotKey` fails | One notice, and capture still works through `--capture` |
| Capture append fails | The box stays open with the error, and the text is kept as a draft |
| IPC capture text too long | Exit code 2. Nothing is appended. |

## 14. Testing

### Unit tests

- **`library::frontmatter`:** the start and end rules (BOM, `...`, no closing line, the 200-line limit); scalars, quoted strings, flow lists and block lists; comments; nested maps kept as raw text; `tags` given as a string with commas or spaces; keys matched without regard to case.
- **Inline tags:** the accepted and rejected examples in §6.3; tags in code spans and blocks ignored; nesting and parent counts; casing chosen by majority.
- **Link extraction:** every wikilink form in §7.1; Markdown links to local files; links in code ignored; line numbers.
- **Resolution:** path form relative to the root, then the note's folder; stem, extension and alias matches; each tie-break; a new note resolving a dangling link.
- **Link rewrite:** stem kept, stem turned into a path when it becomes ambiguous, label and heading kept, Markdown relative paths and percent-encoding.
- **Index cache:** round trip; a stale stamp re-parses; an unknown version discards the cache.
- **Reload:** the prefix and suffix diff, including empty, identical, all-changed and multi-byte text at the edges.
- **Daily notes:** `daily_format` tokens, an invalid format, template placeholders, previous and next ordering with gaps and non-matching files.
- **Capture entry formatting:** single and multi-line entries; blank-line rules; CRLF and LF files; UTF-16 files.
- **`launch.rs`:** `--capture`, `--capture "text"`, `--capture -`, combined with `--new-window`.
- **`ipc/protocol.rs`:** `Capture` round trip and the size limit.
- **`config/persisted.rs`:** `reload_changed_files`, `capture_hotkey`, `sidebar_view=links|tags`.
- **`notebook.ini`:** parse, defaults, keeping comments, bad values.

### Windows integration tests

Run with a scratch `LOCALAPPDATA` and a temp notebook, `--test-threads=1`.

- An outside write to an open clean note reloads it within 1 s. Ctrl+Z restores the old text and makes the tab dirty.
- An outside write to a dirty note gives a conflict. Autosave doesn't overwrite. Compare with disk opens the split.
- An outside delete marks the tab "(deleted)". Ctrl+S re-creates the file.
- An outside change to a tab outside the notebook is noticed when the window is activated.
- Replace across notes and capture don't trigger reloads or conflicts in their own tabs.
- Write a note with `[[Other]]`; the Links view of `Other` lists it after the save. Renaming `Other` in the tree offers to update the link, and Update rewrites it.
- Clicking an unresolved link in the preview creates the note in the notebook root.
- Ctrl+Alt+D creates `journal\<today>.md` from the template, and a second press opens the same file.
- `fastpad --capture "x"` with FastPad running appends to the open inbox tab. With FastPad closed, it appends to the file and exits with 0.

## 15. Open questions

1. **Default capture hotkey.** Ctrl+Alt+N can clash with other apps (and Ctrl+Alt on some keyboards is AltGr). Win+Shift+N is an alternative, but Windows reserves many Win combinations.
2. **Where new linked notes go.** This spec uses the notebook root, which matches first saves (library spec §8.2). The linking note's folder is the alternative, and could become a `notebook.ini` key later.
3. **`#tag` in the editor.** Should inline tags get an editor indicator and Ctrl+click like links? It's left out to keep 1b small.
4. **Index cache as a contract.** `<folder key>.notes.cache` sits beside the per-PC library file. Phase 2's sidecar will want the same facts. Either it reads this cache (making the format a contract) or it builds its own. To be decided in the Phase 2 spec. Until then the format stays private.
