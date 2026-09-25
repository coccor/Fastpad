# Note Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let FastPad open a folder as a note library (like VS Code opens a folder), keep sparse notebook, tag, favorite and pin metadata attached to files through renames and sync, name new notes VS Code-style, and autosave files inside the folder, all without touching startup latency.

**Architecture:** A new window-free `src/library/` module holds the model, the two file formats, the directory scan, reconciliation, and a replayable operation log. A one-off worker thread loads and scans a folder and posts a finished `LibraryState` to the UI thread. A new `src/window/library_host.rs` does all window wiring: the deferred startup step, rescans, debounced writes, folder commands, the inline name box, autosave, and temporary palette commands for organizing. Those commands use a new picker mode of the command palette.

**Tech Stack:** Rust 2024, `windows-sys` 0.61 (already has `Win32_Storage_FileSystem`, `Win32_UI_Shell` and `Win32_System_Com`), and Scintilla. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-23-note-library-design.md`

## Global Constraints

- Nothing new runs before first paint or first input. `WM_FASTPAD_OPEN_LIBRARY` reads only `folders.ini` on the UI thread. Scanning and reconciliation run on a one-off worker thread.
- No new dependencies. The hash is 64-bit FNV-1a, written by hand.
- `library.ini` lives at `<folder>\.fastpad\library.ini` and is UTF-8 with CRLF line endings. It is written only through `crate::file::saver::save_atomic`. A file with an unknown or missing `version` is never overwritten.
- Local files: `%LOCALAPPDATA%\FastPad\libraries\<16-hex FNV of the lower-cased folder path>.ini` and `%LOCALAPPDATA%\FastPad\folders.ini` (at most 10 folders).
- The setting is `notes_mode` in `fastpad.ini`, and it defaults to `true`. With it off, FastPad behaves exactly as it does today, and no `.fastpad\` or `libraries\` files are written.
- IDs are 32 hex digits built with `RecoveryId::compose(process_start, pid, counter)`.
- Only the `editor` module sends `SCI_*` messages.
- Note limit: 10,000. The fingerprint hash is computed only for files of at most 64 MiB. Records are purged after 30 days missing.
- Autosave runs after 1 s of no edits, and on tab switch, window deactivation, and close, for files inside the open folder when the folder's `autosave` is `true`.
- A rescan runs on activation after at least 5 s inactive. Metadata writes are debounced by 500 ms.
- Never commit `native/out` or files under `target/`. Git commit messages carry no attribution lines.
- Compile with `cargo clippy --all-targets`, and run only each task's targeted tests. The full suite runs once, at the final review. In-process window tests and `tests/windows/*` need `-- --test-threads=1`.

## Review Focus

1. **The folder is on FAT32, exFAT or a network share.** These return file IDs of 0 or unstable ones. Scanning must still work, and a file ID of 0 must never match another file. Pinned in Task 8 (`zero_file_ids_never_match`).
2. **A rename that changes only letter case** (`plan.md` → `Plan.md`) in Explorer. It is the same note, and the record takes the new spelling. Pinned in Task 8 (`a_case_only_rename_keeps_the_note_and_takes_the_new_case`).
3. **A very large file with the same size as a missing record** (a multi-GB log). The worker must not hash it. Files over 64 MiB never fingerprint-match. Pinned in Task 8 (`files_over_the_hash_limit_are_never_hashed`).
4. **A first line that is long, emoji or RTL, only `#` characters, or only whitespace.** The label is cut on `char` boundaries and never panics. A line of only `#` is skipped. Pinned in Task 6 (`labels_cut_on_char_boundaries_and_skip_hash_only_lines`).
5. **A folder that is a drive root, or has a junction loop.** Scanning stops at the limit, never follows reparse points, and skips hidden or system folders such as `$Recycle.Bin`. Pinned in Task 7 (`reparse_points_are_not_followed` and `the_scan_stops_at_the_limit`).

## File Map

| File | Responsibility |
|---|---|
| `src/library/mod.rs` (new) | `LibraryState`, `load`, `flush`, `merge_rescan`, path helpers, `DiskStamp` |
| `src/library/ids.rs` (new) | `NoteId`, `NotebookId`, `TagId`, `IdSource`, FNV-1a, `hash_file`, escaping |
| `src/library/model.rs` (new) | `Library`, `Notebook`, `Tag`, `NoteRecord`, `NoteRef`, name rules, primitive mutations |
| `src/library/ops.rs` (new) | `PendingOp`, `apply`, `replay` |
| `src/library/store.rs` (new) | `library.ini` encode/parse/read/write, `FileStamp` |
| `src/library/local.rs` (new) | Per-folder local file, `folders.ini` |
| `src/library/title.rs` (new) | Untitled labels, filename sanitizing, clash numbering, extensions |
| `src/library/scan.rs` (new) | Directory walk with `FileIdBothDirectoryInfo` |
| `src/library/reconcile.rs` (new) | Path, file-ID and fingerprint matching |
| `src/lib.rs` | `pub mod library;` |
| `src/config/{persisted,defaults}.rs` | `notes_mode` |
| `src/document.rs` | `untitled_label`, `label_watch`, `disk_stamp`, `autosave_paused`; the title rule |
| `src/editor/scintilla.rs` | `line_text` |
| `src/ipc/protocol.rs`, `src/ipc/client.rs` | `IpcRequest::OpenFolder` |
| `src/platform/dialogs.rs` | Folder picker, and Save As starting in a folder |
| `src/platform/paths.rs` | `documents_dir` |
| `src/platform/files.rs` (new) | `rename_no_replace`, `recycle` |
| `src/window/messages.rs` | `WM_FASTPAD_OPEN_LIBRARY`, `WM_FASTPAD_LIBRARY_READY`, chain order |
| `src/window/library_host.rs` (new) | All library window wiring |
| `src/window/name_box.rs` (new) | The inline name box panel |
| `src/window/panel.rs` | `create_child_with_id` |
| `src/window/command_palette.rs` | Picker mode |
| `src/window/commands.rs`, `menus.rs` | New `CommandId`s, File menu item, `Ctrl+Shift+O` |
| `src/window/modal.rs` | `confirm` with a test seam |
| `src/window/tabs.rs` | `rebind_path`, `document_mut` |
| `src/window/main_window.rs` | Routing only, plus in-process tests |
| `src/app.rs` | `library: LibraryHost` |
| `tests/windows/library.rs` (new), `Cargo.toml` | End-to-end tests |
| `src/bin/fastpad-bench.rs` | `library-scan` action and `--notes-folder` |
| `README.md`, the spec | Docs |

## Batches

- **Batch A (Tasks 1–9): the window-free library core.** Pure logic and file formats, all unit tested.
- **Batch B (Tasks 10–13): plumbing.** The setting, platform helpers, IPC and launch, and the palette picker.
- **Batch C (Tasks 14–20): window features.** Loading, rescans and writes; folders and drops; labels; the name box; autosave; organizing commands; delete and rename.
- **Batch D (Tasks 21–22): verification and docs.** End-to-end tests, bench, and README.

---

## Batch A: Library core

### Task 1: Library skeleton, IDs, hashing and escaping

**Files:**
- Create: `src/library/mod.rs`, `src/library/ids.rs`
- Modify: `src/lib.rs` (add `pub mod library;` after `pub mod languages;`)

**Interfaces:**
- Produces:
  - `NoteId`, `NotebookId` and `TagId`: each a `(pub u128)` newtype with `to_hex(self) -> String` and `parse_hex(&str) -> Option<Self>`.
  - `IdSource::new(process_start: u64, pid: u32) -> IdSource` and `IdSource::next(&mut self) -> u128`.
  - `Fnv1a` (`new`, `update`, `finish`), plus `fnv1a(&[u8]) -> u64`.
  - `hash_file(path: &Path, limit: u64) -> Option<u64>`.
  - `escape(&str) -> String` and `unescape(&str) -> String`.

- [ ] **Step 1: Write the failing tests** in `src/library/ids.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_as_32_hex_digits_and_reject_anything_else() {
        // Break caught: an ID written in one width and read back in another, so every record
        // loses its identity on the next launch.
        let id = NoteId(0xabc);
        assert_eq!(id.to_hex(), "00000000000000000000000000000abc");
        assert_eq!(NoteId::parse_hex(&id.to_hex()), Some(id));
        assert_eq!(NoteId::parse_hex("abc"), None);
        assert_eq!(NoteId::parse_hex("zz000000000000000000000000000abc"), None);
        assert_eq!(NotebookId::parse_hex(&"f".repeat(32)), Some(NotebookId(u128::MAX)));
    }

    #[test]
    fn id_sources_never_repeat_within_a_process() {
        let mut ids = IdSource::new(7, 42);
        let first = ids.next();
        let second = ids.next();
        assert_ne!(first, second);
        assert_eq!(first >> 64, 7);
    }

    #[test]
    fn fnv1a_matches_the_published_64_bit_vectors() {
        // Break caught: a wrong prime or offset basis, so another FastPad build (or another PC)
        // computes different fingerprints and sync matching silently stops working.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
        let mut streamed = Fnv1a::new();
        streamed.update(b"foo");
        streamed.update(b"bar");
        assert_eq!(streamed.finish(), fnv1a(b"foobar"));
    }

    #[test]
    fn hash_file_streams_the_bytes_and_refuses_files_over_the_limit() {
        let dir = std::env::temp_dir().join(format!("fastpad-ids-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.md");
        std::fs::write(&path, b"foobar").unwrap();
        assert_eq!(hash_file(&path, 6), Some(fnv1a(b"foobar")));
        assert_eq!(hash_file(&path, 5), None);
        assert_eq!(hash_file(&dir.join("missing.md"), 100), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn escaping_protects_separators_and_line_breaks_and_round_trips() {
        // Break caught: a notebook called "a|b" or a name with a newline splitting one record
        // into two lines, or a literal "%41" being decoded into something else.
        let name = "50% a|b\r\nc %41";
        let escaped = escape(name);
        assert_eq!(escaped, "50%25 a%7Cb%0D%0Ac %2541");
        assert!(!escaped.contains(['|', '\r', '\n']));
        assert_eq!(unescape(&escaped), name);
        assert_eq!(unescape("100%"), "100%");
        assert_eq!(unescape("%zz"), "%zz");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::ids`
Expected: a compile error, because `library` does not exist yet.

- [ ] **Step 3: Write the implementation**

`src/library/mod.rs` (it grows in later tasks):

```rust
//! The note library: a folder of plain text files seen as notes, plus sparse organizational
//! metadata (notebooks, tags, favorites, pins) kept in `.fastpad\library.ini` and attached to
//! files by path, file ID and content fingerprint. Nothing here touches a window.

pub mod ids;
```

`src/library/ids.rs` (put it above the test module):

```rust
//! Stable identifiers, the content fingerprint hash, and the escaping used in library files.

use std::io::Read;
use std::path::Path;

macro_rules! library_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub u128);

        impl $name {
            pub fn to_hex(self) -> String {
                format!("{:032x}", self.0)
            }

            pub fn parse_hex(text: &str) -> Option<Self> {
                if text.len() != 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                u128::from_str_radix(text, 16).ok().map(Self)
            }
        }
    };
}

library_id!(NoteId);
library_id!(NotebookId);
library_id!(TagId);

/// Hands out IDs in the same shape as `RecoveryId`: process start, PID, then a counter.
#[derive(Debug)]
pub struct IdSource {
    process_start: u64,
    pid: u32,
    counter: u64,
}

impl IdSource {
    pub const fn new(process_start: u64, pid: u32) -> Self {
        Self {
            process_start,
            pid,
            counter: 0,
        }
    }

    pub fn next(&mut self) -> u128 {
        self.counter += 1;
        crate::document::RecoveryId::compose(self.process_start, self.pid, self.counter).0
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// 64-bit FNV-1a, the content fingerprint and folder-key hash.
#[derive(Clone, Copy, Debug)]
pub struct Fnv1a(u64);

impl Fnv1a {
    pub const fn new() -> Self {
        Self(FNV_OFFSET)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    pub const fn finish(self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = Fnv1a::new();
    hash.update(bytes);
    hash.finish()
}

/// The fingerprint of a file's bytes, or `None` when it cannot be read or is larger than `limit`.
pub fn hash_file(path: &Path, limit: u64) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut hash = Fnv1a::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hash.finish());
        }
        hash.update(&buffer[..read]);
    }
}

/// Escapes `%`, `|`, CR and LF so a name fits in one `|`-separated field.
pub fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '%' => output.push_str("%25"),
            '|' => output.push_str("%7C"),
            '\r' => output.push_str("%0D"),
            '\n' => output.push_str("%0A"),
            _ => output.push(character),
        }
    }
    output
}

/// Reverses `escape`. Any other `%` sequence is kept literally.
pub fn unescape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('%') {
        output.push_str(&rest[..index]);
        let code = rest.get(index + 1..index + 3);
        let decoded = match code.map(str::to_ascii_uppercase).as_deref() {
            Some("25") => Some('%'),
            Some("7C") => Some('|'),
            Some("0D") => Some('\r'),
            Some("0A") => Some('\n'),
            _ => None,
        };
        match decoded {
            Some(character) => {
                output.push(character);
                rest = &rest[index + 3..];
            }
            None => {
                output.push('%');
                rest = &rest[index + 1..];
            }
        }
    }
    output.push_str(rest);
    output
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::ids`
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/library/mod.rs src/library/ids.rs
git commit -m "feat(library): ids, FNV-1a fingerprints and field escaping"
```

---

### Task 2: The library model

**Files:**
- Create: `src/library/model.rs`
- Modify: `src/library/mod.rs` (add `pub mod model;`)

**Interfaces:**
- Consumes: `NoteId`, `NotebookId` and `TagId` (Task 1).
- Produces (everything is `pub`):
  - `NotebookColor`, with `ALL`, `name()` and `parse()`.
  - The structs `Notebook`, `Tag`, `NoteRecord` and `NoteRef { id: NoteId, path: PathBuf }`, and `Library { notebooks, tags, notes }`.
  - `NameError { Empty, Duplicate }` and `LibraryError { Name(NameError), NotFound }`, both implementing `Display`.
  - Free functions: `normalize_name`, `normalize_tag_name`, `same_name`, `same_path`.
  - `Library` methods:
    - `notebook`, `notebooks_in_order`, `tag`, `tag_by_name`, `note`, `note_by_path`, `find_note_mut`, `resolve_note`;
    - `create_notebook`, `rename_notebook`, `set_notebook_color`, `move_notebook`, `delete_notebook`;
    - `create_tag`, `rename_tag`, `remove_tag_everywhere`, `tag_count`;
    - `prune`.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/model.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn nb(value: u128) -> NotebookId {
        NotebookId(value)
    }

    #[test]
    fn notebook_names_are_trimmed_non_empty_and_unique_ignoring_case() {
        // Break caught: "Work" and " work " becoming two notebooks, or an empty name accepted.
        let mut library = Library::default();
        library.create_notebook(nb(1), "  Work ", 10).unwrap();
        assert_eq!(library.notebook(nb(1)).unwrap().name, "Work");
        assert_eq!(
            library.create_notebook(nb(2), "work", 10),
            Err(LibraryError::Name(NameError::Duplicate))
        );
        assert_eq!(
            library.create_notebook(nb(3), "   ", 10),
            Err(LibraryError::Name(NameError::Empty))
        );
        library.create_notebook(nb(4), "Personal", 10).unwrap();
        assert_eq!(
            library.rename_notebook(nb(4), "WORK", 11),
            Err(LibraryError::Name(NameError::Duplicate))
        );
        // Renaming to a different case of its own name is allowed.
        library.rename_notebook(nb(1), "WORK", 12).unwrap();
        assert_eq!(library.notebook(nb(1)).unwrap().modified, 12);
    }

    #[test]
    fn creating_a_notebook_with_an_existing_id_is_a_no_op() {
        // Break caught: replaying "create notebook" after a merge failing as a duplicate and
        // being dropped, or creating a second copy.
        let mut library = Library::default();
        library.create_notebook(nb(1), "Work", 10).unwrap();
        library.create_notebook(nb(1), "Work", 20).unwrap();
        assert_eq!(library.notebooks.len(), 1);
    }

    #[test]
    fn notebooks_reorder_and_new_ones_go_last() {
        let mut library = Library::default();
        for (id, name) in [(1, "A"), (2, "B"), (3, "C")] {
            library.create_notebook(nb(id), name, 0).unwrap();
        }
        library.move_notebook(nb(3), 0).unwrap();
        let order: Vec<_> = library.notebooks_in_order().iter().map(|n| n.name.as_str()).collect();
        assert_eq!(order, ["C", "A", "B"]);
        library.create_notebook(nb(4), "D", 0).unwrap();
        assert_eq!(library.notebooks_in_order().last().unwrap().name, "D");
        library.move_notebook(nb(1), 99).unwrap();
        assert_eq!(library.notebooks_in_order().last().unwrap().name, "A");
    }

    #[test]
    fn deleting_a_notebook_moves_its_notes_to_notes_and_keeps_them() {
        // Break caught: deleting a notebook deleting its notes' records (and their tags).
        let mut library = Library::default();
        library.create_notebook(nb(1), "Work", 0).unwrap();
        let note = library.resolve_note(&NoteRef { id: NoteId(9), path: "a.md".into() });
        note.notebook = Some(nb(1));
        note.favorite = true;
        assert_eq!(library.delete_notebook(nb(1)), Ok(1));
        let note = library.note(NoteId(9)).unwrap();
        assert_eq!(note.notebook, None);
        assert!(note.favorite);
        assert_eq!(library.delete_notebook(nb(1)), Err(LibraryError::NotFound));
    }

    #[test]
    fn resolve_finds_by_id_then_by_path_ignoring_case_then_creates() {
        let mut library = Library::default();
        library.resolve_note(&NoteRef { id: NoteId(1), path: "Plan.md".into() });
        let by_path = library.resolve_note(&NoteRef { id: NoteId(2), path: "plan.MD".into() });
        assert_eq!(by_path.id, NoteId(1));
        let by_id = library.resolve_note(&NoteRef { id: NoteId(1), path: "other.md".into() });
        assert_eq!(by_id.path, PathBuf::from("Plan.md"));
        library.resolve_note(&NoteRef { id: NoteId(3), path: "new.md".into() });
        assert_eq!(library.notes.len(), 2);
    }

    #[test]
    fn tags_strip_a_leading_hash_reuse_names_and_disappear_when_unused() {
        // Break caught: "#idea" and "idea" becoming two tags, or an unused tag lingering.
        let mut library = Library::default();
        assert_eq!(library.create_tag(TagId(1), "#idea"), Ok(TagId(1)));
        assert_eq!(library.tag(TagId(1)).unwrap().name, "idea");
        assert_eq!(library.create_tag(TagId(2), "IDEA"), Ok(TagId(1)));
        assert_eq!(
            library.create_tag(TagId(3), " # "),
            Err(LibraryError::Name(NameError::Empty))
        );
        library.resolve_note(&NoteRef { id: NoteId(5), path: "a.md".into() }).tags.push(TagId(1));
        library.create_tag(TagId(4), "todo").unwrap();
        library.prune();
        assert!(library.tag(TagId(1)).is_some());
        assert!(library.tag(TagId(4)).is_none());
    }

    #[test]
    fn removing_a_tag_everywhere_counts_notes_and_prune_drops_bare_records() {
        let mut library = Library::default();
        library.create_tag(TagId(1), "todo").unwrap();
        for (id, path) in [(1, "a.md"), (2, "b.md")] {
            library.resolve_note(&NoteRef { id: NoteId(id), path: path.into() }).tags.push(TagId(1));
        }
        library.resolve_note(&NoteRef { id: NoteId(2), path: "b.md".into() }).favorite = true;
        assert_eq!(library.tag_count(TagId(1)), 2);
        assert_eq!(library.remove_tag_everywhere(TagId(1)), Ok(2));
        library.prune();
        assert_eq!(library.notes.len(), 1);
        assert_eq!(library.notes[0].id, NoteId(2));
        assert!(library.tags.is_empty());
    }

    #[test]
    fn paths_compare_ignoring_case_like_ntfs() {
        assert!(same_path(Path::new(r"Sub\Plan.md"), Path::new(r"sub\plan.MD")));
        assert!(!same_path(Path::new("a.md"), Path::new("b.md")));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::model`
Expected: a compile error (the items are not defined).

- [ ] **Step 3: Write the implementation** above the tests in `src/library/model.rs`

```rust
//! Notebooks, tags and note records, and the primitive mutations on them. Every name rule of the
//! spec lives here: trimmed, not empty, unique ignoring case; tags never store a leading `#`.

use super::ids::{NoteId, NotebookId, TagId};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotebookColor {
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Purple,
    Pink,
}

impl NotebookColor {
    pub const ALL: [Self; 8] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Teal,
        Self::Blue,
        Self::Purple,
        Self::Pink,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Teal => "teal",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Pink => "pink",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.name() == text)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notebook {
    pub id: NotebookId,
    pub name: String,
    pub color: Option<NotebookColor>,
    pub sort: u32,
    pub created: u64,
    pub modified: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
}

/// One organized note. `path` is relative to the folder, or absolute for a file outside it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub notebook: Option<NotebookId>,
    pub favorite: bool,
    pub pinned: bool,
    pub deleted: bool,
    pub tags: Vec<TagId>,
    pub size: u64,
    pub hash: u64,
    pub path: PathBuf,
}

impl NoteRecord {
    pub fn new(id: NoteId, path: PathBuf) -> Self {
        Self {
            id,
            notebook: None,
            favorite: false,
            pinned: false,
            deleted: false,
            tags: Vec::new(),
            size: 0,
            hash: 0,
            path,
        }
    }

    pub fn has_metadata(&self) -> bool {
        self.notebook.is_some() || self.favorite || self.pinned || !self.tags.is_empty()
    }
}

/// Names a note for an operation: by stable ID first, then by path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRef {
    pub id: NoteId,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameError {
    Empty,
    Duplicate,
}

impl std::fmt::Display for NameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "A name cannot be empty.",
            Self::Duplicate => "That name is already used.",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryError {
    Name(NameError),
    NotFound,
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name(error) => error.fmt(formatter),
            Self::NotFound => formatter.write_str("That item no longer exists."),
        }
    }
}

pub fn normalize_name(name: &str) -> Result<String, NameError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(NameError::Empty)
    } else {
        Ok(trimmed.to_owned())
    }
}

/// Like `normalize_name`, but the leading `#` is presentation only and never stored.
pub fn normalize_tag_name(name: &str) -> Result<String, NameError> {
    normalize_name(name.trim().trim_start_matches('#'))
}

pub fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

/// NTFS compares names ignoring case, so the library does too.
pub fn same_path(left: &Path, right: &Path) -> bool {
    left.as_os_str().to_string_lossy().to_lowercase()
        == right.as_os_str().to_string_lossy().to_lowercase()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Library {
    pub notebooks: Vec<Notebook>,
    pub tags: Vec<Tag>,
    pub notes: Vec<NoteRecord>,
}

impl Library {
    pub fn notebook(&self, id: NotebookId) -> Option<&Notebook> {
        self.notebooks.iter().find(|notebook| notebook.id == id)
    }

    pub fn notebooks_in_order(&self) -> Vec<&Notebook> {
        let mut ordered: Vec<_> = self.notebooks.iter().collect();
        ordered.sort_by(|a, b| a.sort.cmp(&b.sort).then_with(|| a.name.cmp(&b.name)));
        ordered
    }

    pub fn tag(&self, id: TagId) -> Option<&Tag> {
        self.tags.iter().find(|tag| tag.id == id)
    }

    pub fn tag_by_name(&self, name: &str) -> Option<&Tag> {
        self.tags.iter().find(|tag| same_name(&tag.name, name))
    }

    pub fn note(&self, id: NoteId) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| note.id == id)
    }

    pub fn note_by_path(&self, path: &Path) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| same_path(&note.path, path))
    }

    /// An existing record, by ID and then by path. Never creates one.
    pub fn find_note_mut(&mut self, target: &NoteRef) -> Option<&mut NoteRecord> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| self.notes.iter().position(|note| same_path(&note.path, &target.path)))?;
        self.notes.get_mut(index)
    }

    /// The record for `target`, created with its ID and path when none exists.
    pub fn resolve_note(&mut self, target: &NoteRef) -> &mut NoteRecord {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| self.notes.iter().position(|note| same_path(&note.path, &target.path)));
        let index = index.unwrap_or_else(|| {
            self.notes.push(NoteRecord::new(target.id, target.path.clone()));
            self.notes.len() - 1
        });
        &mut self.notes[index]
    }

    fn check_notebook_name(
        &self,
        name: &str,
        except: Option<NotebookId>,
    ) -> Result<String, LibraryError> {
        let name = normalize_name(name).map_err(LibraryError::Name)?;
        let taken = self
            .notebooks
            .iter()
            .any(|notebook| Some(notebook.id) != except && same_name(&notebook.name, &name));
        if taken {
            Err(LibraryError::Name(NameError::Duplicate))
        } else {
            Ok(name)
        }
    }

    pub fn create_notebook(
        &mut self,
        id: NotebookId,
        name: &str,
        now: u64,
    ) -> Result<(), LibraryError> {
        if self.notebook(id).is_some() {
            return Ok(());
        }
        let name = self.check_notebook_name(name, None)?;
        let sort = self.notebooks.iter().map(|n| n.sort + 1).max().unwrap_or(0);
        self.notebooks.push(Notebook {
            id,
            name,
            color: None,
            sort,
            created: now,
            modified: now,
        });
        Ok(())
    }

    pub fn rename_notebook(
        &mut self,
        id: NotebookId,
        name: &str,
        now: u64,
    ) -> Result<(), LibraryError> {
        let name = self.check_notebook_name(name, Some(id))?;
        let notebook = self
            .notebooks
            .iter_mut()
            .find(|notebook| notebook.id == id)
            .ok_or(LibraryError::NotFound)?;
        notebook.name = name;
        notebook.modified = now;
        Ok(())
    }

    pub fn set_notebook_color(
        &mut self,
        id: NotebookId,
        color: Option<NotebookColor>,
        now: u64,
    ) -> Result<(), LibraryError> {
        let notebook = self
            .notebooks
            .iter_mut()
            .find(|notebook| notebook.id == id)
            .ok_or(LibraryError::NotFound)?;
        notebook.color = color;
        notebook.modified = now;
        Ok(())
    }

    /// Moves the notebook to `index` in display order (clamped) and renumbers every sort key.
    pub fn move_notebook(&mut self, id: NotebookId, index: usize) -> Result<(), LibraryError> {
        let mut order: Vec<NotebookId> = self.notebooks_in_order().iter().map(|n| n.id).collect();
        let from = order.iter().position(|&n| n == id).ok_or(LibraryError::NotFound)?;
        order.remove(from);
        order.insert(index.min(order.len()), id);
        for (sort, id) in order.into_iter().enumerate() {
            if let Some(notebook) = self.notebooks.iter_mut().find(|n| n.id == id) {
                notebook.sort = sort as u32;
            }
        }
        Ok(())
    }

    /// Removes the notebook and moves its notes to Notes. Returns how many notes moved.
    pub fn delete_notebook(&mut self, id: NotebookId) -> Result<usize, LibraryError> {
        let before = self.notebooks.len();
        self.notebooks.retain(|notebook| notebook.id != id);
        if self.notebooks.len() == before {
            return Err(LibraryError::NotFound);
        }
        let mut moved = 0;
        for note in &mut self.notes {
            if note.notebook == Some(id) {
                note.notebook = None;
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Returns the existing tag with this name, or creates one with `id`.
    pub fn create_tag(&mut self, id: TagId, name: &str) -> Result<TagId, LibraryError> {
        let name = normalize_tag_name(name).map_err(LibraryError::Name)?;
        if let Some(existing) = self.tag_by_name(&name) {
            return Ok(existing.id);
        }
        if self.tag(id).is_some() {
            return Ok(id);
        }
        self.tags.push(Tag { id, name });
        Ok(id)
    }

    pub fn rename_tag(&mut self, id: TagId, name: &str) -> Result<(), LibraryError> {
        let name = normalize_tag_name(name).map_err(LibraryError::Name)?;
        if self.tags.iter().any(|tag| tag.id != id && same_name(&tag.name, &name)) {
            return Err(LibraryError::Name(NameError::Duplicate));
        }
        let tag = self.tags.iter_mut().find(|tag| tag.id == id).ok_or(LibraryError::NotFound)?;
        tag.name = name;
        Ok(())
    }

    /// Removes the tag from every note and from the library. Returns how many notes had it.
    pub fn remove_tag_everywhere(&mut self, id: TagId) -> Result<usize, LibraryError> {
        if self.tag(id).is_none() {
            return Err(LibraryError::NotFound);
        }
        let count = self.tag_count(id);
        for note in &mut self.notes {
            note.tags.retain(|&tag| tag != id);
        }
        self.tags.retain(|tag| tag.id != id);
        Ok(count)
    }

    pub fn tag_count(&self, id: TagId) -> usize {
        self.notes
            .iter()
            .filter(|note| !note.deleted && note.tags.contains(&id))
            .count()
    }

    /// Drops records that carry no metadata and tags no record uses. Run before every write.
    pub fn prune(&mut self) {
        self.notes.retain(NoteRecord::has_metadata);
        let notes = &self.notes;
        self.tags.retain(|tag| notes.iter().any(|note| note.tags.contains(&tag.id)));
    }
}
```

Add `pub mod model;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::model`
Expected: all 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/model.rs
git commit -m "feat(library): notebooks, tags, note records and their name rules"
```

---

### Task 3: Replayable operations

**Files:**
- Create: `src/library/ops.rs`
- Modify: `src/library/mod.rs` (add `pub mod ops;`)

**Interfaces:**
- Consumes: `Library`, `NoteRef`, `LibraryError`, `NotebookColor` and the IDs (Tasks 1 and 2).
- Produces:
  - `pub enum PendingOp`, with the variants below.
  - `pub fn apply(library: &mut Library, op: &PendingOp) -> Result<(), LibraryError>`.
  - `pub fn replay(library: &mut Library, ops: &[PendingOp]) -> usize`, which returns how many operations were dropped.

Every operation sets a value; none toggles one. That makes replaying an operation that was already written harmless.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/ops.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::{NoteId, NotebookId, TagId};

    fn note(id: u128, path: &str) -> NoteRef {
        NoteRef { id: NoteId(id), path: path.into() }
    }

    #[test]
    fn two_diverged_copies_merge_by_replay_without_losing_either_side() {
        // Break caught: a sync from another PC overwriting this PC's favorite, or the reverse.
        let mut base = Library::default();
        apply(&mut base, &PendingOp::CreateNotebook { id: NotebookId(1), name: "Work".into(), now: 1 }).unwrap();

        let mut other_pc = base.clone();
        apply(&mut other_pc, &PendingOp::SetNoteNotebook { note: note(7, "a.md"), notebook: Some(NotebookId(1)) }).unwrap();

        let ours = vec![
            PendingOp::SetFavorite { note: note(8, "b.md"), value: true },
            PendingOp::AddTag { note: note(8, "b.md"), tag: TagId(3), name: "idea".into() },
        ];
        let mut merged = other_pc.clone();
        assert_eq!(replay(&mut merged, &ours), 0);
        assert_eq!(merged.note(NoteId(7)).unwrap().notebook, Some(NotebookId(1)));
        let b = merged.note(NoteId(8)).unwrap();
        assert!(b.favorite);
        assert_eq!(b.tags, vec![TagId(3)]);
    }

    #[test]
    fn replaying_an_already_applied_log_changes_nothing() {
        let ops = vec![
            PendingOp::CreateNotebook { id: NotebookId(1), name: "Work".into(), now: 1 },
            PendingOp::SetNoteNotebook { note: note(7, "a.md"), notebook: Some(NotebookId(1)) },
            PendingOp::AddTag { note: note(7, "a.md"), tag: TagId(2), name: "todo".into() },
            PendingOp::SetPinned { note: note(7, "a.md"), value: true },
        ];
        let mut once = Library::default();
        replay(&mut once, &ops);
        let mut twice = once.clone();
        assert_eq!(replay(&mut twice, &ops), 0);
        assert_eq!(once, twice);
    }

    #[test]
    fn operations_whose_target_was_removed_are_dropped() {
        // Break caught: a note moved into a notebook that the other PC deleted pointing at a
        // notebook that no longer exists.
        let mut library = Library::default();
        let ops = vec![
            PendingOp::SetNoteNotebook { note: note(7, "a.md"), notebook: Some(NotebookId(9)) },
            PendingOp::RenameNotebook { id: NotebookId(9), name: "X".into(), now: 1 },
            PendingOp::Relocate { note: note(8, "gone.md"), path: "moved.md".into() },
        ];
        assert_eq!(replay(&mut library, &ops), 3);
        assert!(library.notes.is_empty());
    }

    #[test]
    fn adding_a_tag_reuses_a_same_named_tag_created_elsewhere() {
        let mut library = Library::default();
        library.create_tag(TagId(1), "idea").unwrap();
        apply(&mut library, &PendingOp::AddTag { note: note(7, "a.md"), tag: TagId(2), name: "Idea".into() }).unwrap();
        assert_eq!(library.tags.len(), 1);
        assert_eq!(library.note(NoteId(7)).unwrap().tags, vec![TagId(1)]);
    }

    #[test]
    fn relocation_fingerprints_and_deletion_flags_need_an_existing_record() {
        let mut library = Library::default();
        apply(&mut library, &PendingOp::SetFavorite { note: note(7, "a.md"), value: true }).unwrap();
        apply(&mut library, &PendingOp::Relocate { note: note(7, "a.md"), path: "b.md".into() }).unwrap();
        apply(&mut library, &PendingOp::SetFingerprint { note: note(7, "b.md"), size: 3, hash: 9 }).unwrap();
        apply(&mut library, &PendingOp::SetDeleted { note: note(7, "b.md"), value: true }).unwrap();
        let record = library.note(NoteId(7)).unwrap();
        assert_eq!(record.path, std::path::PathBuf::from("b.md"));
        assert_eq!((record.size, record.hash, record.deleted), (3, 9, true));
        apply(&mut library, &PendingOp::Drop { id: NoteId(7) }).unwrap();
        assert!(library.notes.is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::ops`
Expected: a compile error.

- [ ] **Step 3: Write the implementation**

```rust
//! Every change the UI makes to a library is a `PendingOp`, applied at once to the live library
//! and kept until the next successful write. When `library.ini` changed on disk in between
//! (sync, another instance), the file is re-read and the pending operations are replayed on top,
//! so both sides' changes survive.

use super::ids::{NoteId, NotebookId, TagId};
use super::model::{Library, LibraryError, NoteRef, NotebookColor};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingOp {
    CreateNotebook { id: NotebookId, name: String, now: u64 },
    RenameNotebook { id: NotebookId, name: String, now: u64 },
    SetNotebookColor { id: NotebookId, color: Option<NotebookColor>, now: u64 },
    MoveNotebook { id: NotebookId, index: usize },
    DeleteNotebook { id: NotebookId },
    SetNoteNotebook { note: NoteRef, notebook: Option<NotebookId> },
    SetFavorite { note: NoteRef, value: bool },
    SetPinned { note: NoteRef, value: bool },
    AddTag { note: NoteRef, tag: TagId, name: String },
    RemoveTag { note: NoteRef, tag: TagId },
    RenameTag { id: TagId, name: String },
    RemoveTagEverywhere { id: TagId },
    Relocate { note: NoteRef, path: PathBuf },
    SetFingerprint { note: NoteRef, size: u64, hash: u64 },
    SetDeleted { note: NoteRef, value: bool },
    Drop { id: NoteId },
}

pub fn apply(library: &mut Library, op: &PendingOp) -> Result<(), LibraryError> {
    match op {
        PendingOp::CreateNotebook { id, name, now } => library.create_notebook(*id, name, *now),
        PendingOp::RenameNotebook { id, name, now } => library.rename_notebook(*id, name, *now),
        PendingOp::SetNotebookColor { id, color, now } => {
            library.set_notebook_color(*id, *color, *now)
        }
        PendingOp::MoveNotebook { id, index } => library.move_notebook(*id, *index),
        PendingOp::DeleteNotebook { id } => library.delete_notebook(*id).map(|_| ()),
        PendingOp::SetNoteNotebook { note, notebook } => {
            if let Some(notebook) = notebook
                && library.notebook(*notebook).is_none()
            {
                return Err(LibraryError::NotFound);
            }
            library.resolve_note(note).notebook = *notebook;
            Ok(())
        }
        PendingOp::SetFavorite { note, value } => {
            library.resolve_note(note).favorite = *value;
            Ok(())
        }
        PendingOp::SetPinned { note, value } => {
            library.resolve_note(note).pinned = *value;
            Ok(())
        }
        PendingOp::AddTag { note, tag, name } => {
            let tag = library.create_tag(*tag, name)?;
            let record = library.resolve_note(note);
            if !record.tags.contains(&tag) {
                record.tags.push(tag);
            }
            Ok(())
        }
        PendingOp::RemoveTag { note, tag } => {
            let record = library.find_note_mut(note).ok_or(LibraryError::NotFound)?;
            record.tags.retain(|existing| existing != tag);
            Ok(())
        }
        PendingOp::RenameTag { id, name } => library.rename_tag(*id, name),
        PendingOp::RemoveTagEverywhere { id } => library.remove_tag_everywhere(*id).map(|_| ()),
        PendingOp::Relocate { note, path } => {
            library.find_note_mut(note).ok_or(LibraryError::NotFound)?.path = path.clone();
            Ok(())
        }
        PendingOp::SetFingerprint { note, size, hash } => {
            let record = library.find_note_mut(note).ok_or(LibraryError::NotFound)?;
            record.size = *size;
            record.hash = *hash;
            Ok(())
        }
        PendingOp::SetDeleted { note, value } => {
            library.find_note_mut(note).ok_or(LibraryError::NotFound)?.deleted = *value;
            Ok(())
        }
        PendingOp::Drop { id } => {
            library.notes.retain(|note| note.id != *id);
            Ok(())
        }
    }
}

/// Applies every operation in order. Operations that no longer apply are dropped and counted.
pub fn replay(library: &mut Library, ops: &[PendingOp]) -> usize {
    ops.iter().filter(|op| apply(library, op).is_err()).count()
}
```

Add `pub mod ops;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::ops`
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/ops.rs
git commit -m "feat(library): replayable metadata operations"
```

---

### Task 4: `library.ini` store

**Files:**
- Create: `src/library/store.rs`
- Modify: `src/library/mod.rs` (add `pub mod store;`)

**Interfaces:**
- Consumes: the model and IDs, plus `escape` and `unescape` (Tasks 1 and 2).
- Produces:
  - `pub const LIBRARY_DIR: &str = ".fastpad"`
  - `pub fn library_file(folder: &Path) -> PathBuf`
  - `pub fn encode(&Library) -> String`
  - `pub fn parse(&str) -> Option<Library>` (`None` means unreadable)
  - `pub struct FileStamp { pub size: u64, pub modified: u64 }` (`Copy`, `Eq`)
  - `pub fn stamp(&Path) -> Option<FileStamp>`
  - `pub enum ReadOutcome { Absent, Loaded(Library, FileStamp), Unreadable }`
  - `pub fn read(&Path) -> ReadOutcome`
  - `pub fn write(&Path, &Library) -> Result<FileStamp>`

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/store.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::{NoteId, NotebookId, TagId};
    use crate::library::model::{Notebook, NotebookColor, NoteRecord, Tag};

    fn sample() -> Library {
        let mut note = NoteRecord::new(NoteId(0xa), PathBuf::from(r"sub\a b|c.md"));
        note.notebook = Some(NotebookId(1));
        note.favorite = true;
        note.pinned = true;
        note.tags = vec![TagId(2), TagId(3)];
        note.size = 12;
        note.hash = 0xfeed;
        let mut external = NoteRecord::new(NoteId(0xb), PathBuf::from(r"C:\elsewhere\log.txt"));
        external.deleted = true;
        external.favorite = true;
        Library {
            notebooks: vec![Notebook {
                id: NotebookId(1),
                name: "Work | 50%\nplans".into(),
                color: Some(NotebookColor::Teal),
                sort: 3,
                created: 100,
                modified: 200,
            }],
            tags: vec![
                Tag { id: TagId(2), name: "idea".into() },
                Tag { id: TagId(3), name: "to|do".into() },
            ],
            notes: vec![note, external],
        }
    }

    #[test]
    fn a_library_round_trips_through_its_text_form() {
        // Break caught: a name with `|`, `%` or a newline, a path with `|`, or an absolute path
        // not surviving a write and read.
        let text = encode(&sample());
        assert!(text.starts_with("version=1\r\n"));
        assert!(text.contains(
            "note=0000000000000000000000000000000a|00000000000000000000000000000001|fp|\
             00000000000000000000000000000002,00000000000000000000000000000003|12|000000000000feed|sub\\a b|c.md\r\n"
        ));
        assert!(text.contains("|fd|-|0|0000000000000000|C:\\elsewhere\\log.txt\r\n"));
        assert_eq!(parse(&text), Some(sample()));
    }

    #[test]
    fn only_version_one_files_are_readable() {
        // Break caught: a newer FastPad's file (or a damaged one) read as an empty library and
        // then overwritten, destroying every notebook.
        assert_eq!(parse("notebook=x\r\n"), None);
        assert_eq!(parse("version=2\r\n"), None);
        assert_eq!(parse("\u{feff}version=1\r\n"), Some(Library::default()));
    }

    #[test]
    fn malformed_lines_unknown_keys_and_dangling_references_are_tolerated() {
        let text = "version=1\n\
            future=1\n\
            notebook=bad\n\
            tag=00000000000000000000000000000002|idea\n\
            note=00000000000000000000000000000007|00000000000000000000000000000009|zq|\
            00000000000000000000000000000002,00000000000000000000000000000004|5|0000000000000001|a.md\n\
            note=short\n";
        let library = parse(text).unwrap();
        assert!(library.notebooks.is_empty());
        let note = &library.notes[0];
        assert_eq!(note.notebook, None, "unknown notebook falls back to Notes");
        assert_eq!(note.tags, vec![TagId(2)], "unknown tag IDs are dropped");
        assert!(!note.favorite && !note.pinned && !note.deleted, "unknown flags are ignored");
        assert_eq!(library.notes.len(), 1);
    }

    #[test]
    fn read_distinguishes_absent_loaded_and_unreadable_and_write_returns_the_stamp() {
        let dir = std::env::temp_dir().join(format!("fastpad-store-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = library_file(&dir);
        assert_eq!(path, dir.join(".fastpad").join("library.ini"));
        assert!(matches!(read(&path), ReadOutcome::Absent));
        let stamp = write(&path, &sample()).unwrap();
        assert_eq!(super::stamp(&path), Some(stamp));
        match read(&path) {
            ReadOutcome::Loaded(library, read_stamp) => {
                assert_eq!(library, sample());
                assert_eq!(read_stamp, stamp);
            }
            _ => panic!("expected a loaded library"),
        }
        std::fs::write(&path, "version=9\r\n").unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::store`
Expected: a compile error.

- [ ] **Step 3: Write the implementation**

```rust
//! `.fastpad\library.ini`: the shared metadata that travels with the folder.
//!
//! ```text
//! version=1
//! notebook=<id>|<sort>|<color or ->|<created unix>|<modified unix>|<name>
//! tag=<id>|<name>
//! note=<id>|<notebook id or ->|<flags>|<tag ids or ->|<size>|<hash>|<path>
//! ```
//!
//! Names are escaped (`ids::escape`); the path is last and unescaped, so splitting a `note` line on
//! its first six `|` characters is unambiguous. A missing or unknown `version` makes the whole file
//! unreadable, and an unreadable file is never overwritten.

use super::ids::{NoteId, NotebookId, TagId, escape, unescape};
use super::model::{Library, NoteRecord, Notebook, NotebookColor, Tag};
use crate::Result;
use std::path::{Path, PathBuf};

const VERSION: &str = "1";
pub const LIBRARY_DIR: &str = ".fastpad";
const LIBRARY_FILE: &str = "library.ini";

pub fn library_file(folder: &Path) -> PathBuf {
    folder.join(LIBRARY_DIR).join(LIBRARY_FILE)
}

/// Size and last-write time (nanoseconds since the Unix epoch), used to notice outside changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStamp {
    pub size: u64,
    pub modified: u64,
}

pub fn stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some(FileStamp {
        size: metadata.len(),
        modified,
    })
}

pub fn encode(library: &Library) -> String {
    let mut output = format!("version={VERSION}\r\n");
    for notebook in &library.notebooks {
        output.push_str(&format!(
            "notebook={}|{}|{}|{}|{}|{}\r\n",
            notebook.id.to_hex(),
            notebook.sort,
            notebook.color.map_or("-", NotebookColor::name),
            notebook.created,
            notebook.modified,
            escape(&notebook.name)
        ));
    }
    for tag in &library.tags {
        output.push_str(&format!("tag={}|{}\r\n", tag.id.to_hex(), escape(&tag.name)));
    }
    for note in &library.notes {
        let mut flags = String::new();
        if note.favorite {
            flags.push('f');
        }
        if note.pinned {
            flags.push('p');
        }
        if note.deleted {
            flags.push('d');
        }
        if flags.is_empty() {
            flags.push('-');
        }
        let tags = if note.tags.is_empty() {
            "-".to_owned()
        } else {
            note.tags.iter().map(|tag| tag.to_hex()).collect::<Vec<_>>().join(",")
        };
        output.push_str(&format!(
            "note={}|{}|{flags}|{tags}|{}|{:016x}|{}\r\n",
            note.id.to_hex(),
            note.notebook.map_or_else(|| "-".to_owned(), NotebookId::to_hex),
            note.size,
            note.hash,
            note.path.to_string_lossy()
        ));
    }
    output
}

pub fn parse(source: &str) -> Option<Library> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut version = None;
    let mut library = Library::default();
    for line in source.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "version" => version = Some(value),
            "notebook" => library.notebooks.extend(parse_notebook(value)),
            "tag" => library.tags.extend(parse_tag(value)),
            "note" => library.notes.extend(parse_note(value)),
            _ => {}
        }
    }
    if version != Some(VERSION) {
        return None;
    }
    dedupe_by(&mut library.notebooks, |notebook| notebook.id.0);
    dedupe_by(&mut library.tags, |tag| tag.id.0);
    dedupe_by(&mut library.notes, |note| note.id.0);
    let notebooks: Vec<NotebookId> = library.notebooks.iter().map(|n| n.id).collect();
    let tags: Vec<TagId> = library.tags.iter().map(|t| t.id).collect();
    for note in &mut library.notes {
        if note.notebook.is_some_and(|id| !notebooks.contains(&id)) {
            note.notebook = None;
        }
        note.tags.retain(|tag| tags.contains(tag));
    }
    Some(library)
}

fn dedupe_by<T>(items: &mut Vec<T>, key: impl Fn(&T) -> u128) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(key(item)));
}

fn parse_notebook(value: &str) -> Option<Notebook> {
    let mut fields = value.splitn(6, '|');
    let id = NotebookId::parse_hex(fields.next()?)?;
    let sort = fields.next()?.parse().ok()?;
    let color = match fields.next()? {
        "-" => None,
        name => NotebookColor::parse(name),
    };
    let created = fields.next()?.parse().ok()?;
    let modified = fields.next()?.parse().ok()?;
    let name = unescape(fields.next()?);
    if name.trim().is_empty() {
        return None;
    }
    Some(Notebook { id, name, color, sort, created, modified })
}

fn parse_tag(value: &str) -> Option<Tag> {
    let (id, name) = value.split_once('|')?;
    let name = unescape(name);
    if name.trim().is_empty() {
        return None;
    }
    Some(Tag { id: TagId::parse_hex(id)?, name })
}

fn parse_note(value: &str) -> Option<NoteRecord> {
    let mut fields = value.splitn(7, '|');
    let id = NoteId::parse_hex(fields.next()?)?;
    let notebook = match fields.next()? {
        "-" => None,
        text => Some(NotebookId::parse_hex(text)?),
    };
    let flags = fields.next()?;
    let tags = match fields.next()? {
        "-" => Vec::new(),
        text => text.split(',').filter_map(TagId::parse_hex).collect(),
    };
    let size = fields.next()?.parse().ok()?;
    let hash = u64::from_str_radix(fields.next()?, 16).ok()?;
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    let mut note = NoteRecord::new(id, PathBuf::from(path));
    note.notebook = notebook;
    note.favorite = flags.contains('f');
    note.pinned = flags.contains('p');
    note.deleted = flags.contains('d');
    note.tags = tags;
    note.size = size;
    note.hash = hash;
    Some(note)
}

pub enum ReadOutcome {
    Absent,
    Loaded(Library, FileStamp),
    Unreadable,
}

pub fn read(path: &Path) -> ReadOutcome {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ReadOutcome::Absent,
        Err(_) => return ReadOutcome::Unreadable,
    };
    let Some(stamp) = stamp(path) else {
        return ReadOutcome::Unreadable;
    };
    match std::str::from_utf8(&bytes).ok().and_then(parse) {
        Some(library) => ReadOutcome::Loaded(library, stamp),
        None => ReadOutcome::Unreadable,
    }
}

pub fn write(path: &Path, library: &Library) -> Result<FileStamp> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, encode(library).as_bytes())?;
    stamp(path).ok_or(crate::FastPadError::Invariant(
        "library.ini could not be read back after it was written",
    ))
}
```

Add `pub mod store;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::store`
Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/store.rs
git commit -m "feat(library): library.ini encoding, parsing and atomic writes"
```

---

### Task 5: Local state and recent folders

**Files:**
- Create: `src/library/local.rs`
- Modify: `src/library/mod.rs` (add `pub mod local;`)

**Interfaces:**
- Consumes: `NoteId`, `fnv1a` and `same_path` (Tasks 1 and 2).
- Produces:
  - `pub struct CachedFile { pub volume: u32, pub file_id: u64, pub mtime: u64, pub size: u64, pub path: PathBuf }`
  - `pub struct LocalState { pub folder, pub autosave: bool, pub recent: Vec<(u64, PathBuf)>, pub missing: Vec<(u64, NoteId)>, pub files: Vec<CachedFile> }`, with methods:
    - `LocalState::new(folder)`, `encode`, `parse(source, folder) -> Option<Self>`
    - `note_opened(path, now)`, `rename_path(old, new)`, `merge_recent(&other)`
    - `missing_since(id) -> Option<u64>`, `set_missing(id, now)`, `clear_missing(id)`
  - `pub fn folder_key(folder: &Path) -> String`
  - `pub fn local_file(data_dir: &Path, folder: &Path) -> PathBuf`
  - `pub fn read(path: &Path, folder: &Path) -> LocalState` and `pub fn write(path: &Path, state: &LocalState) -> Result<()>`
  - `pub struct RecentFolders { pub folders: Vec<PathBuf> }`, with `parse`, `encode` and `push`
  - `pub fn folders_file(data_dir: &Path) -> PathBuf`, `pub fn read_folders(path) -> RecentFolders` and `pub fn write_folders(path, &RecentFolders) -> Result<()>`
  - `pub const RECENT_LIMIT: usize = 200` and `pub const FOLDER_LIMIT: usize = 10`

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/local.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LocalState {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.autosave = false;
        state.recent = vec![(20, PathBuf::from("b.md")), (10, PathBuf::from(r"sub\a|x.md"))];
        state.missing = vec![(5, NoteId(9))];
        state.files = vec![CachedFile {
            volume: 0x1234,
            file_id: 77,
            mtime: 133_000,
            size: 42,
            path: PathBuf::from(r"sub\a|x.md"),
        }];
        state
    }

    #[test]
    fn local_state_round_trips_and_rejects_another_folders_file() {
        // Break caught: two folders whose keys collide sharing one cache, or `|` in a path
        // breaking the scan cache.
        let text = sample().encode();
        assert_eq!(LocalState::parse(&text, Path::new(r"D:\Notes")), Some(sample()));
        assert!(LocalState::parse(&text, Path::new(r"d:\notes")).is_some(), "case is ignored");
        assert_eq!(LocalState::parse(&text, Path::new(r"D:\Other")), None);
        assert_eq!(LocalState::parse("version=2\r\n", Path::new(r"D:\Notes")), None);
    }

    #[test]
    fn opening_a_note_moves_it_to_the_front_and_caps_the_list() {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        for index in 0..RECENT_LIMIT + 5 {
            state.note_opened(Path::new(&format!("{index}.md")), index as u64);
        }
        assert_eq!(state.recent.len(), RECENT_LIMIT);
        state.note_opened(Path::new("10.MD"), 999);
        assert_eq!(state.recent[0], (999, PathBuf::from("10.MD")));
        assert_eq!(state.recent.iter().filter(|(_, p)| same_path(p, Path::new("10.md"))).count(), 1);
    }

    #[test]
    fn renames_follow_recent_entries_and_merging_keeps_the_newest_open_time() {
        let mut state = sample();
        state.rename_path(Path::new(r"SUB\A|X.md"), Path::new("moved.md"));
        assert!(state.recent.iter().any(|(_, p)| p == Path::new("moved.md")));
        let mut other = LocalState::new(PathBuf::from(r"D:\Notes"));
        other.recent = vec![(50, PathBuf::from("b.md")), (1, PathBuf::from("c.md"))];
        state.merge_recent(&other);
        assert_eq!(state.recent[0], (50, PathBuf::from("b.md")));
        assert!(state.recent.iter().any(|(t, p)| *t == 1 && p == Path::new("c.md")));
    }

    #[test]
    fn missing_times_are_recorded_once_and_cleared() {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.set_missing(NoteId(1), 100);
        state.set_missing(NoteId(1), 200);
        assert_eq!(state.missing_since(NoteId(1)), Some(100));
        state.clear_missing(NoteId(1));
        assert_eq!(state.missing_since(NoteId(1)), None);
    }

    #[test]
    fn folder_keys_ignore_case_and_name_the_local_file() {
        let data = Path::new(r"C:\Users\u\AppData\Local\FastPad");
        assert_eq!(folder_key(Path::new(r"D:\Notes")), folder_key(Path::new(r"d:\NOTES")));
        assert_eq!(folder_key(Path::new(r"D:\Notes")).len(), 16);
        assert_eq!(
            local_file(data, Path::new(r"D:\Notes")),
            data.join("libraries").join(format!("{}.ini", folder_key(Path::new(r"D:\Notes"))))
        );
    }

    #[test]
    fn recent_folders_dedupe_ignoring_case_put_the_newest_first_and_cap_at_ten() {
        let mut folders = RecentFolders::default();
        for index in 0..12 {
            folders.push(PathBuf::from(format!(r"D:\F{index}")));
        }
        folders.push(PathBuf::from(r"d:\f5"));
        assert_eq!(folders.folders.len(), FOLDER_LIMIT);
        assert_eq!(folders.folders[0], PathBuf::from(r"d:\f5"));
        assert_eq!(RecentFolders::parse(&folders.encode()), folders);
        assert_eq!(RecentFolders::parse("version=7\r\nfolder=D:\\x\r\n"), RecentFolders::default());
    }

    #[test]
    fn missing_or_damaged_local_files_read_as_fresh_state() {
        let dir = std::env::temp_dir().join(format!("fastpad-local-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = Path::new(r"D:\Notes");
        let path = local_file(&dir, folder);
        assert_eq!(read(&path, folder), LocalState::new(folder.to_path_buf()));
        write(&path, &sample()).unwrap();
        assert_eq!(read(&path, folder), sample());
        std::fs::write(&path, [0xff, 0x00]).unwrap();
        assert_eq!(read(&path, folder), LocalState::new(folder.to_path_buf()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::local`
Expected: a compile error.

- [ ] **Step 3: Write the implementation**

```rust
//! Per-PC state that must not travel with the folder: the scan cache (file IDs only mean something
//! on one volume), recently opened notes, when records went missing, the folder's autosave switch,
//! and the list of recent folders. Losing any of it is harmless, so reads never fail.

use super::ids::{NoteId, fnv1a};
use super::model::same_path;
use crate::Result;
use std::path::{Path, PathBuf};

const VERSION: &str = "1";
pub const RECENT_LIMIT: usize = 200;
pub const FOLDER_LIMIT: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedFile {
    pub volume: u32,
    pub file_id: u64,
    pub mtime: u64,
    pub size: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalState {
    pub folder: PathBuf,
    pub autosave: bool,
    /// Most recent first: (unix time opened, path as stored in records).
    pub recent: Vec<(u64, PathBuf)>,
    pub missing: Vec<(u64, NoteId)>,
    pub files: Vec<CachedFile>,
}

impl LocalState {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            autosave: true,
            recent: Vec::new(),
            missing: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn encode(&self) -> String {
        let mut output = format!(
            "version={VERSION}\r\nfolder={}\r\nautosave={}\r\n",
            self.folder.to_string_lossy(),
            self.autosave
        );
        for (time, path) in &self.recent {
            output.push_str(&format!("recent={time}|{}\r\n", path.to_string_lossy()));
        }
        for (time, id) in &self.missing {
            output.push_str(&format!("missing={time}|{}\r\n", id.to_hex()));
        }
        for file in &self.files {
            output.push_str(&format!(
                "file={}|{}|{}|{}|{}\r\n",
                file.volume,
                file.file_id,
                file.mtime,
                file.size,
                file.path.to_string_lossy()
            ));
        }
        output
    }

    /// `None` for anything but a version-1 file written for `folder`.
    pub fn parse(source: &str, folder: &Path) -> Option<Self> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut stored_folder = None;
        let mut state = Self::new(folder.to_path_buf());
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "version" => version = Some(value),
                "folder" => stored_folder = Some(PathBuf::from(value)),
                "autosave" => state.autosave = value != "false",
                "recent" => {
                    if let Some((time, path)) = value.split_once('|')
                        && let Ok(time) = time.parse()
                        && !path.is_empty()
                    {
                        state.recent.push((time, PathBuf::from(path)));
                    }
                }
                "missing" => {
                    if let Some((time, id)) = value.split_once('|')
                        && let (Ok(time), Some(id)) = (time.parse(), NoteId::parse_hex(id))
                    {
                        state.missing.push((time, id));
                    }
                }
                "file" => state.files.extend(parse_file(value)),
                _ => {}
            }
        }
        if version != Some(VERSION) || !stored_folder.is_some_and(|f| same_path(&f, folder)) {
            return None;
        }
        state.folder = folder.to_path_buf();
        Some(state)
    }

    pub fn note_opened(&mut self, path: &Path, now: u64) {
        self.recent.retain(|(_, existing)| !same_path(existing, path));
        self.recent.insert(0, (now, path.to_path_buf()));
        self.recent.truncate(RECENT_LIMIT);
    }

    pub fn rename_path(&mut self, old: &Path, new: &Path) {
        for (_, path) in &mut self.recent {
            if same_path(path, old) {
                *path = new.to_path_buf();
            }
        }
    }

    /// Keeps the newest open time per path from both lists, most recent first.
    pub fn merge_recent(&mut self, other: &LocalState) {
        for (time, path) in &other.recent {
            match self.recent.iter_mut().find(|(_, existing)| same_path(existing, path)) {
                Some(entry) if entry.0 < *time => entry.0 = *time,
                Some(_) => {}
                None => self.recent.push((*time, path.clone())),
            }
        }
        self.recent.sort_by(|a, b| b.0.cmp(&a.0));
        self.recent.truncate(RECENT_LIMIT);
    }

    pub fn missing_since(&self, id: NoteId) -> Option<u64> {
        self.missing.iter().find(|(_, missing)| *missing == id).map(|(time, _)| *time)
    }

    pub fn set_missing(&mut self, id: NoteId, now: u64) {
        if self.missing_since(id).is_none() {
            self.missing.push((now, id));
        }
    }

    pub fn clear_missing(&mut self, id: NoteId) {
        self.missing.retain(|(_, missing)| *missing != id);
    }
}

fn parse_file(value: &str) -> Option<CachedFile> {
    let mut fields = value.splitn(5, '|');
    let volume = fields.next()?.parse().ok()?;
    let file_id = fields.next()?.parse().ok()?;
    let mtime = fields.next()?.parse().ok()?;
    let size = fields.next()?.parse().ok()?;
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    Some(CachedFile { volume, file_id, mtime, size, path: PathBuf::from(path) })
}

pub fn folder_key(folder: &Path) -> String {
    format!("{:016x}", fnv1a(folder.to_string_lossy().to_lowercase().as_bytes()))
}

pub fn local_file(data_dir: &Path, folder: &Path) -> PathBuf {
    data_dir.join("libraries").join(format!("{}.ini", folder_key(folder)))
}

pub fn read(path: &Path, folder: &Path) -> LocalState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|source| LocalState::parse(&source, folder))
        .unwrap_or_else(|| LocalState::new(folder.to_path_buf()))
}

pub fn write(path: &Path, state: &LocalState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, state.encode().as_bytes())
}

/// `folders.ini`: recent folders, most recent first. The first one opens at startup.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecentFolders {
    pub folders: Vec<PathBuf>,
}

impl RecentFolders {
    pub fn parse(source: &str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut folders = Vec::new();
        for line in source.lines() {
            match line.split_once('=') {
                Some(("version", value)) => version = Some(value),
                Some(("folder", value)) if !value.is_empty() => folders.push(PathBuf::from(value)),
                _ => {}
            }
        }
        if version != Some(VERSION) {
            return Self::default();
        }
        folders.truncate(FOLDER_LIMIT);
        Self { folders }
    }

    pub fn encode(&self) -> String {
        let mut output = format!("version={VERSION}\r\n");
        for folder in &self.folders {
            output.push_str(&format!("folder={}\r\n", folder.to_string_lossy()));
        }
        output
    }

    pub fn push(&mut self, folder: PathBuf) {
        self.folders.retain(|existing| !same_path(existing, &folder));
        self.folders.insert(0, folder);
        self.folders.truncate(FOLDER_LIMIT);
    }
}

pub fn folders_file(data_dir: &Path) -> PathBuf {
    data_dir.join("folders.ini")
}

pub fn read_folders(path: &Path) -> RecentFolders {
    std::fs::read_to_string(path)
        .map(|source| RecentFolders::parse(&source))
        .unwrap_or_default()
}

pub fn write_folders(path: &Path, folders: &RecentFolders) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, folders.encode().as_bytes())
}
```

Add `pub mod local;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::local`
Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/local.rs
git commit -m "feat(library): per-PC local state and recent folders"
```

---
### Task 6: Labels, filenames and extensions

**Files:**
- Create: `src/library/title.rs`
- Modify: `src/library/mod.rs` (add `pub mod title;`)

**Interfaces:**
- Consumes: `crate::document::Language`.
- Produces:
  - `pub const LABEL_LIMIT: usize = 40` and `pub const LABEL_SCAN_LINES: usize = 16`
  - `pub struct Label { pub text: Option<String>, pub watch_through: usize }`
  - `pub fn untitled_label<'a>(lines: impl IntoIterator<Item = &'a str>) -> Label`
  - `pub fn sanitize_stem(name: &str) -> String`
  - `pub fn default_extension(language: Language) -> &'static str`
  - `pub fn split_typed_name(input: &str, default_extension: &str) -> (String, String)`, returning (stem, extension)
  - `pub fn free_name(stem: &str, extension: &str, exists: impl Fn(&str) -> bool) -> String`
  - `pub fn note_title(path: &Path) -> String`
  - `pub fn is_note_extension(extension: &str) -> bool` and `pub const NOTE_EXTENSIONS: [&str; 14]`

The spec's "extensions FastPad recognizes as text" is pinned here as an explicit list, because `languages::detect_language` only knows `json` and `md`.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/title.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_is_the_first_non_empty_line_without_heading_marks() {
        let label = untitled_label(["", "   ", "## Meeting notes  ", "body"]);
        assert_eq!(label.text.as_deref(), Some("Meeting notes"));
        assert_eq!(label.watch_through, 2);
        let empty = untitled_label(["", "  "]);
        assert_eq!(empty.text, None);
        assert_eq!(empty.watch_through, LABEL_SCAN_LINES - 1);
    }

    #[test]
    fn labels_cut_on_char_boundaries_and_skip_hash_only_lines() {
        // Break caught: slicing a long emoji or Arabic first line at a byte index (a panic), or
        // labelling a tab "###".
        let long = "😀".repeat(60);
        let label = untitled_label(["###", "   #  ", &long]).text.unwrap();
        assert_eq!(label.chars().count(), LABEL_LIMIT);
        assert!(label.ends_with('…'));
        let arabic = "ملاحظات الاجتماع الأسبوعي حول خطة الإصدار القادم والمزيد";
        assert!(untitled_label([arabic]).text.unwrap().chars().count() <= LABEL_LIMIT);
        assert_eq!(untitled_label(["#idea"]).text.as_deref(), Some("idea"));
    }

    #[test]
    fn only_the_first_sixteen_lines_are_considered() {
        let mut lines = vec![""; 20];
        lines[18] = "late";
        assert_eq!(untitled_label(lines).text, None);
    }

    #[test]
    fn stems_drop_invalid_characters_trailing_dots_and_reserved_names() {
        // Break caught: a first line like `a/b: c?` or `CON` producing a name Windows refuses,
        // so the first save fails with a confusing error.
        assert_eq!(sanitize_stem(r#"a/b: "c"? <d>|*"#), "ab c d");
        assert_eq!(sanitize_stem("Notes...  "), "Notes");
        assert_eq!(sanitize_stem("Long title…"), "Long title");
        assert_eq!(sanitize_stem("con"), "con_");
        assert_eq!(sanitize_stem("LPT9.draft"), "LPT9.draft_");
        assert_eq!(sanitize_stem("  \t "), "Untitled");
        assert_eq!(sanitize_stem("tab\there"), "tabhere");
    }

    #[test]
    fn typed_names_keep_a_known_extension_and_otherwise_get_the_default() {
        assert_eq!(split_typed_name("plan.txt", "md"), ("plan".into(), "txt".into()));
        assert_eq!(split_typed_name("v1.2 plan", "md"), ("v1.2 plan".into(), "md".into()));
        assert_eq!(split_typed_name("data.JSON", "md"), ("data".into(), "JSON".into()));
        assert_eq!(split_typed_name(".md", "md"), ("Untitled".into(), "md".into()));
        assert_eq!(default_extension(Language::Json), "json");
        assert_eq!(default_extension(Language::PlainText), "md");
    }

    #[test]
    fn clashing_names_get_the_first_free_number() {
        let taken = ["Plan.md", "Plan 2.md"];
        let exists = |name: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(name));
        assert_eq!(free_name("Plan", "md", exists), "Plan 3.md");
        assert_eq!(free_name("plan", "md", |_| false), "plan.md");
        assert_eq!(free_name("PLAN", "MD", exists), "PLAN 3.MD");
    }

    #[test]
    fn a_saved_notes_title_is_its_file_stem() {
        assert_eq!(note_title(Path::new(r"D:\Notes\sub\Meeting notes.md")), "Meeting notes");
        assert!(is_note_extension("YML"));
        assert!(!is_note_extension("png"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::title`
Expected: a compile error.

- [ ] **Step 3: Write the implementation**

```rust
//! Untitled tab labels, turning a label or typed name into a safe filename, and the list of file
//! extensions that count as notes.

use crate::document::Language;
use std::path::Path;

pub const LABEL_LIMIT: usize = 40;
pub const LABEL_SCAN_LINES: usize = 16;

pub const NOTE_EXTENSIONS: [&str; 14] = [
    "md", "markdown", "txt", "text", "json", "log", "ini", "cfg", "conf", "yaml", "yml", "toml",
    "csv", "xml",
];

pub fn is_note_extension(extension: &str) -> bool {
    NOTE_EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(extension))
}

/// An untitled tab's label, and the last line whose edits can change it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Label {
    pub text: Option<String>,
    pub watch_through: usize,
}

pub fn untitled_label<'a>(lines: impl IntoIterator<Item = &'a str>) -> Label {
    for (index, line) in lines.into_iter().take(LABEL_SCAN_LINES).enumerate() {
        let text = line.trim().trim_start_matches('#').trim();
        if !text.is_empty() {
            return Label {
                text: Some(truncate(text)),
                watch_through: index,
            };
        }
    }
    Label {
        text: None,
        watch_through: LABEL_SCAN_LINES - 1,
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= LABEL_LIMIT {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(LABEL_LIMIT - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A filename stem Windows accepts: no `<>:"/\|?*` or control characters, no trailing dots,
/// spaces or label ellipsis, not a reserved device name, never empty.
pub fn sanitize_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .filter(|c| !c.is_control())
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ', '…']).trim().to_owned();
    if trimmed.is_empty() {
        return "Untitled".to_owned();
    }
    let device = trimmed.split('.').next().unwrap_or("");
    if RESERVED.iter().any(|reserved| reserved.eq_ignore_ascii_case(device)) {
        return format!("{trimmed}_");
    }
    trimmed
}

pub fn default_extension(language: Language) -> &'static str {
    match language {
        Language::Json => "json",
        Language::Markdown | Language::PlainText => "md",
    }
}

/// Splits what the user typed into a sanitized stem and an extension. A typed extension is kept
/// only when it is a note extension, so "v1.2 plan" stays one stem.
pub fn split_typed_name(input: &str, default_extension: &str) -> (String, String) {
    let input = input.trim();
    if let Some((stem, extension)) = input.rsplit_once('.')
        && is_note_extension(extension)
    {
        return (sanitize_stem(stem), extension.to_owned());
    }
    (sanitize_stem(input), default_extension.to_owned())
}

/// `stem.extension`, or `stem N.extension` with the first free N from 2.
pub fn free_name(stem: &str, extension: &str, exists: impl Fn(&str) -> bool) -> String {
    let first = format!("{stem}.{extension}");
    if !exists(&first) {
        return first;
    }
    (2..10_000)
        .map(|number| format!("{stem} {number}.{extension}"))
        .find(|candidate| !exists(candidate))
        .unwrap_or(first)
}

pub fn note_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_owned())
}
```

Add `pub mod title;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::title`
Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/title.rs
git commit -m "feat(library): untitled labels, safe filenames and note extensions"
```

---

### Task 7: Folder scan

**Files:**
- Create: `src/library/scan.rs`
- Modify: `src/library/mod.rs` (add `pub mod scan;`)

**Interfaces:**
- Consumes: `title::is_note_extension` (Task 6), `crate::platform::{OwnedHandle, wide_null, last_error}`.
- Produces:
  - `pub const NOTE_LIMIT: usize = 10_000`
  - `pub struct ScanEntry { pub path: PathBuf /* relative */, pub size: u64, pub mtime: u64 /* FILETIME ticks */, pub file_id: u64, pub online_only: bool }`
  - `pub struct Scan { pub volume: u32, pub entries: Vec<ScanEntry>, pub truncated: bool }` (`Default`)
  - `pub fn skip_directory(name: &str) -> bool`
  - `pub fn scan(folder: &Path, limit: usize) -> Result<Scan>`

`online_only` marks OneDrive placeholders (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` or `FILE_ATTRIBUTE_OFFLINE`). Reconciliation never hashes them, because reading one downloads it. Reparse-point **directories** (junctions, symbolic links) are skipped. Reparse-point **files** are kept, because OneDrive files are reparse points.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/scan.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!("fastpad-scan-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn paths(scan: &Scan) -> Vec<String> {
        let mut paths: Vec<_> = scan.entries.iter().map(|e| e.path.to_string_lossy().into_owned()).collect();
        paths.sort();
        paths
    }

    #[test]
    fn notes_are_found_in_subfolders_and_other_files_and_folders_are_skipped() {
        // Break caught: a repo folder listing node_modules, build output or images as notes.
        let scratch = Scratch::new("rules");
        scratch.file("a.md", "a");
        scratch.file(r"sub\deeper\b.TXT", "bb");
        scratch.file("picture.png", "x");
        scratch.file(r".git\c.md", "x");
        scratch.file(r"node_modules\d.md", "x");
        scratch.file(r"target\e.json", "x");
        scratch.file(r"Bin\f.md", "x");
        scratch.file(r"hidden\g.md", "x");
        let hidden = crate::platform::wide_null(&scratch.0.join("hidden").to_string_lossy());
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                hidden.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            );
        }
        let scan = scan(&scratch.0, NOTE_LIMIT).unwrap();
        assert_eq!(paths(&scan), [r"a.md", r"sub\deeper\b.TXT"]);
        let b = scan.entries.iter().find(|e| e.path.ends_with("b.TXT")).unwrap();
        assert_eq!(b.size, 2);
        assert_ne!(b.file_id, 0, "NTFS reports file IDs");
        assert_ne!(b.mtime, 0);
        assert!(!scan.truncated);
    }

    #[test]
    fn the_scan_stops_at_the_limit() {
        let scratch = Scratch::new("limit");
        for index in 0..5 {
            scratch.file(&format!("{index}.md"), "x");
        }
        let scan = scan(&scratch.0, 3).unwrap();
        assert_eq!(scan.entries.len(), 3);
        assert!(scan.truncated);
    }

    #[test]
    fn reparse_points_are_not_followed() {
        // Break caught: a junction back to the folder itself making the scan loop until the limit
        // with the same notes over and over.
        let scratch = Scratch::new("junction");
        scratch.file("a.md", "a");
        let link = scratch.0.join("loop");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&scratch.0)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink /J failed: {made:?}");
        let scan = scan(&scratch.0, NOTE_LIMIT).unwrap();
        assert_eq!(paths(&scan), ["a.md"]);
    }

    #[test]
    fn a_missing_folder_is_an_error_and_skip_rules_ignore_case() {
        assert!(scan(Path::new(r"Z:\fastpad-does-not-exist\x"), NOTE_LIMIT).is_err());
        assert!(skip_directory("Node_Modules"));
        assert!(skip_directory(".obsidian"));
        assert!(!skip_directory("notes"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::scan`
Expected: a compile error.

- [ ] **Step 3: Write the implementation**

```rust
//! Walks a folder for notes. Each directory is listed with `FileIdBothDirectoryInfo` queries,
//! which return names, sizes, write times and file IDs in bulk without opening any file.

use crate::Result;
use crate::platform::{OwnedHandle, last_error, wide_null};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, GetLastError};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_SYSTEM, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdBothDirectoryInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
};

pub const NOTE_LIMIT: usize = 10_000;
const SKIPPED: [&str; 4] = ["node_modules", "target", "bin", "obj"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanEntry {
    /// Relative to the scanned folder.
    pub path: PathBuf,
    pub size: u64,
    /// Last write time in FILETIME ticks.
    pub mtime: u64,
    /// 0 when the file system has no stable IDs (FAT, some network shares).
    pub file_id: u64,
    pub online_only: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scan {
    pub volume: u32,
    pub entries: Vec<ScanEntry>,
    pub truncated: bool,
}

pub fn skip_directory(name: &str) -> bool {
    name.starts_with('.') || SKIPPED.iter().any(|skipped| skipped.eq_ignore_ascii_case(name))
}

fn open_directory(path: &Path) -> Result<OwnedHandle> {
    let wide = wide_null(&path.to_string_lossy());
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    // CreateFileW returns INVALID_HANDLE_VALUE on failure, which from_raw_owned rejects.
    unsafe { OwnedHandle::from_raw_owned(handle) }.map_err(|_| last_error())
}

pub fn scan(folder: &Path, limit: usize) -> Result<Scan> {
    let root = open_directory(folder)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(root.as_raw(), &mut info) } == 0 {
        return Err(last_error());
    }
    let mut scan = Scan {
        volume: info.dwVolumeSerialNumber,
        ..Scan::default()
    };
    let mut pending = vec![(PathBuf::new(), Some(root))];
    // 64 KiB, 8-byte aligned as FILE_ID_BOTH_DIR_INFO requires.
    let mut buffer = vec![0_u64; 8 * 1024];
    while let Some((relative, handle)) = pending.pop() {
        let handle = match handle {
            Some(handle) => handle,
            // A subfolder that cannot be opened is skipped, not fatal.
            None => match open_directory(&folder.join(&relative)) {
                Ok(handle) => handle,
                Err(_) => continue,
            },
        };
        loop {
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    handle.as_raw(),
                    FileIdBothDirectoryInfo,
                    buffer.as_mut_ptr().cast(),
                    (buffer.len() * 8) as u32,
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_NO_MORE_FILES && relative.as_os_str().is_empty() {
                    return Err(crate::FastPadError::Win32(error));
                }
                break;
            }
            let mut offset = 0_usize;
            loop {
                // Entries are packed back to back; NextEntryOffset is 8-byte aligned.
                let entry = unsafe {
                    &*(buffer.as_ptr().cast::<u8>().add(offset).cast::<FILE_ID_BOTH_DIR_INFO>())
                };
                let name = unsafe {
                    std::slice::from_raw_parts(
                        std::ptr::addr_of!(entry.FileName).cast::<u16>(),
                        (entry.FileNameLength / 2) as usize,
                    )
                };
                let name = std::ffi::OsString::from_wide(name).to_string_lossy().into_owned();
                let attributes = entry.FileAttributes;
                let skipped_attributes = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM;
                if name != "." && name != ".." && attributes & skipped_attributes == 0 {
                    let path = relative.join(&name);
                    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 && !skip_directory(&name) {
                            pending.push((path, None));
                        }
                    } else if Path::new(&name)
                        .extension()
                        .is_some_and(|ext| super::title::is_note_extension(&ext.to_string_lossy()))
                    {
                        if scan.entries.len() >= limit {
                            scan.truncated = true;
                            return Ok(scan);
                        }
                        scan.entries.push(ScanEntry {
                            path,
                            size: entry.EndOfFile as u64,
                            mtime: entry.LastWriteTime as u64,
                            file_id: entry.FileId as u64,
                            online_only: attributes
                                & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_OFFLINE)
                                != 0,
                        });
                    }
                }
                if entry.NextEntryOffset == 0 {
                    break;
                }
                offset += entry.NextEntryOffset as usize;
            }
        }
    }
    Ok(scan)
}
```

If a `windows-sys` 0.61 constant or field name differs (for example `FileIdBothDirectoryInfo`, or `FileId` typed as `i64`), use the crate's spelling: look it up in `~/.cargo/registry/src/*/windows-sys-0.61.2/src/Windows/Win32/Storage/FileSystem/mod.rs`. Change only the name, not the logic.

Add `pub mod scan;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::scan`
Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/scan.rs
git commit -m "feat(library): folder scan with bulk file IDs and skip rules"
```

---
### Task 8: Reconciliation

**Files:**
- Create: `src/library/reconcile.rs`
- Modify: `src/library/mod.rs` (add `pub mod reconcile;`)

**Interfaces:**
- Consumes: `Library`, `NoteRef` (Task 2); `PendingOp`, `apply` (Task 3); `LocalState`, `CachedFile` (Task 5); `Scan`, `ScanEntry` (Task 7).
- Produces:
  - `pub const HASH_LIMIT: u64 = 64 * 1024 * 1024`
  - `pub const PURGE_AFTER_SECS: u64 = 30 * 24 * 60 * 60`
  - `pub struct Reconciled { pub ops: Vec<PendingOp>, pub relocated: Vec<(PathBuf, PathBuf)> }` (`Default`)
  - `pub fn reconcile(library: &mut Library, local: &mut LocalState, scan: &Scan, now: u64, hash: &mut dyn FnMut(&Path) -> Option<u64>) -> Reconciled`

`hash` receives a path **relative to the folder**. The returned ops have already been applied to `library`. The caller keeps them as pending, so they survive a replay if `library.ini` changed on disk.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/reconcile.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::NoteId;
    use crate::library::local::CachedFile;
    use crate::library::model::NoteRecord;
    use crate::library::scan::ScanEntry;
    use std::collections::HashMap;

    const VOLUME: u32 = 0xabcd;

    fn entry(path: &str, size: u64, file_id: u64) -> ScanEntry {
        ScanEntry { path: path.into(), size, mtime: 1, file_id, online_only: false }
    }

    fn record(id: u128, path: &str, size: u64, hash: u64) -> NoteRecord {
        let mut record = NoteRecord::new(NoteId(id), path.into());
        record.favorite = true;
        record.size = size;
        record.hash = hash;
        record
    }

    fn cached(path: &str, size: u64, file_id: u64) -> CachedFile {
        CachedFile { volume: VOLUME, file_id, mtime: 1, size, path: path.into() }
    }

    struct Fixture {
        library: Library,
        local: LocalState,
        scan: Scan,
        hashes: HashMap<String, u64>,
        hashed: Vec<String>,
    }

    impl Fixture {
        fn new(records: Vec<NoteRecord>, cache: Vec<CachedFile>, entries: Vec<ScanEntry>) -> Self {
            let mut local = LocalState::new(r"D:\Notes".into());
            local.files = cache;
            Self {
                library: Library { notes: records, ..Library::default() },
                local,
                scan: Scan { volume: VOLUME, entries, truncated: false },
                hashes: HashMap::new(),
                hashed: Vec::new(),
            }
        }

        fn run(&mut self, now: u64) -> Reconciled {
            let hashes = self.hashes.clone();
            let hashed = &mut self.hashed;
            reconcile(&mut self.library, &mut self.local, &self.scan, now, &mut |path| {
                let key = path.to_string_lossy().into_owned();
                hashed.push(key.clone());
                hashes.get(&key).copied()
            })
        }

        fn path_of(&self, id: u128) -> Option<PathBuf> {
            self.library.note(NoteId(id)).map(|note| note.path.clone())
        }
    }

    #[test]
    fn a_path_match_keeps_the_note_and_records_nothing_when_unchanged() {
        let mut fixture = Fixture::new(
            vec![record(1, "a.md", 3, 9)],
            vec![cached("a.md", 3, 5)],
            vec![entry("a.md", 3, 5)],
        );
        let result = fixture.run(100);
        assert!(result.ops.is_empty());
        assert!(fixture.hashed.is_empty(), "an unchanged file is not re-hashed");
    }

    #[test]
    fn a_rename_outside_fastpad_is_followed_by_file_id() {
        // Break caught: renaming a note in Explorer losing its notebook and tags.
        let mut fixture = Fixture::new(
            vec![record(1, "old.md", 3, 9)],
            vec![cached("old.md", 3, 55)],
            vec![entry(r"sub\new.md", 3, 55)],
        );
        let result = fixture.run(100);
        assert_eq!(fixture.path_of(1), Some(PathBuf::from(r"sub\new.md")));
        assert_eq!(result.relocated, vec![(PathBuf::from("old.md"), PathBuf::from(r"sub\new.md"))]);
        assert!(fixture.hashed.is_empty(), "the file ID was enough");
        assert_eq!(fixture.local.files[0].path, PathBuf::from(r"sub\new.md"));
    }

    #[test]
    fn a_synced_copy_is_matched_by_size_and_hash() {
        let mut fixture = Fixture::new(
            vec![record(1, "old.md", 3, 9)],
            vec![],
            vec![entry("other.md", 3, 70), entry("copy.md", 3, 71)],
        );
        fixture.hashes.insert("other.md".into(), 8);
        fixture.hashes.insert("copy.md".into(), 9);
        fixture.run(100);
        assert_eq!(fixture.path_of(1), Some(PathBuf::from("copy.md")));
    }

    #[test]
    fn the_same_size_with_a_different_hash_is_not_a_match() {
        let mut fixture = Fixture::new(vec![record(1, "old.md", 3, 9)], vec![], vec![entry("x.md", 3, 70)]);
        fixture.hashes.insert("x.md".into(), 8);
        fixture.run(100);
        assert_eq!(fixture.path_of(1), Some(PathBuf::from("old.md")));
        assert_eq!(fixture.local.missing_since(NoteId(1)), Some(100));
    }

    #[test]
    fn zero_file_ids_never_match() {
        // Break caught: on FAT or a network share every file reports ID 0, so a missing note
        // would "move" to whichever file came first.
        let mut fixture = Fixture::new(
            vec![record(1, "old.md", 3, 0)],
            vec![cached("old.md", 3, 0)],
            vec![entry("unrelated.md", 9, 0)],
        );
        fixture.run(100);
        assert_eq!(fixture.path_of(1), Some(PathBuf::from("old.md")));
    }

    #[test]
    fn a_case_only_rename_keeps_the_note_and_takes_the_new_case() {
        let mut fixture = Fixture::new(
            vec![record(1, "plan.md", 3, 9)],
            vec![cached("plan.md", 3, 5)],
            vec![entry("Plan.md", 3, 5)],
        );
        fixture.run(100);
        assert_eq!(fixture.path_of(1), Some(PathBuf::from("Plan.md")));
    }

    #[test]
    fn files_over_the_hash_limit_are_never_hashed() {
        let big = HASH_LIMIT + 1;
        let mut fixture = Fixture::new(vec![record(1, "old.log", big, 9)], vec![], vec![entry("huge.log", big, 70)]);
        fixture.run(100);
        assert!(fixture.hashed.is_empty());
        assert_eq!(fixture.path_of(1), Some(PathBuf::from("old.log")));
    }

    #[test]
    fn online_only_files_are_never_hashed() {
        // Break caught: reconciliation downloading every OneDrive placeholder of the same size.
        let mut placeholder = entry("cloud.md", 3, 70);
        placeholder.online_only = true;
        let mut fixture = Fixture::new(vec![record(1, "old.md", 3, 9)], vec![], vec![placeholder]);
        fixture.run(100);
        assert!(fixture.hashed.is_empty());
    }

    #[test]
    fn a_changed_file_gets_a_fresh_fingerprint() {
        let mut fixture = Fixture::new(
            vec![record(1, "a.md", 3, 9)],
            vec![cached("a.md", 3, 5)],
            vec![entry("a.md", 4, 5)],
        );
        fixture.hashes.insert("a.md".into(), 10);
        fixture.run(100);
        let note = fixture.library.note(NoteId(1)).unwrap();
        assert_eq!((note.size, note.hash), (4, 10));
    }

    #[test]
    fn a_deleted_note_restored_from_the_recycle_bin_comes_back() {
        let mut deleted = record(1, "a.md", 3, 9);
        deleted.deleted = true;
        let mut fixture = Fixture::new(vec![deleted], vec![], vec![entry("a.md", 3, 5)]);
        fixture.local.set_missing(NoteId(1), 50);
        fixture.hashes.insert("a.md".into(), 9);
        fixture.run(100);
        assert!(!fixture.library.note(NoteId(1)).unwrap().deleted);
        assert_eq!(fixture.local.missing_since(NoteId(1)), None);
    }

    #[test]
    fn a_record_missing_for_thirty_days_is_dropped_and_not_before() {
        let mut fixture = Fixture::new(vec![record(1, "gone.md", 3, 9)], vec![], vec![]);
        fixture.run(1_000);
        assert!(fixture.library.note(NoteId(1)).is_some());
        fixture.run(1_000 + PURGE_AFTER_SECS - 1);
        assert!(fixture.library.note(NoteId(1)).is_some());
        let result = fixture.run(1_000 + PURGE_AFTER_SECS);
        assert!(fixture.library.note(NoteId(1)).is_none());
        assert!(result.ops.contains(&PendingOp::Drop { id: NoteId(1) }));
        assert_eq!(fixture.local.missing_since(NoteId(1)), None);
    }

    #[test]
    fn records_outside_the_folder_are_left_alone() {
        let mut fixture = Fixture::new(vec![record(1, r"C:\elsewhere\a.md", 3, 9)], vec![], vec![]);
        let result = fixture.run(100);
        assert!(result.ops.is_empty());
        assert_eq!(fixture.local.missing_since(NoteId(1)), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::reconcile`
Expected: a compile error.

- [ ] **Step 3: Write the implementation**

```rust
//! Keeps records attached to their files. Scan results are matched against records in order:
//! path (ignoring case), then the cached file ID (a rename or move in Explorer), then size and
//! content hash (a copy or sync). Anything else is missing; after 30 days missing it is dropped.

use super::local::{CachedFile, LocalState};
use super::model::{Library, NoteRef};
use super::ops::{PendingOp, apply};
use super::scan::Scan;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const HASH_LIMIT: u64 = 64 * 1024 * 1024;
pub const PURGE_AFTER_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Reconciled {
    pub ops: Vec<PendingOp>,
    /// (old record path, new record path) for every note that moved.
    pub relocated: Vec<(PathBuf, PathBuf)>,
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

pub fn reconcile(
    library: &mut Library,
    local: &mut LocalState,
    scan: &Scan,
    now: u64,
    hash: &mut dyn FnMut(&Path) -> Option<u64>,
) -> Reconciled {
    let mut result = Reconciled::default();
    let by_path: HashMap<String, usize> = scan
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (key(&entry.path), index))
        .collect();
    let cache: HashMap<String, CachedFile> =
        local.files.iter().map(|file| (key(&file.path), file.clone())).collect();
    let mut claimed = vec![false; scan.entries.len()];
    let mut hashes: HashMap<usize, Option<u64>> = HashMap::new();
    let mut unmatched = Vec::new();

    // 1. Path.
    for record in library.notes.iter().filter(|record| !record.path.is_absolute()) {
        let note = NoteRef { id: record.id, path: record.path.clone() };
        let Some(&index) = by_path.get(&key(&record.path)) else {
            unmatched.push(note);
            continue;
        };
        claimed[index] = true;
        let entry = &scan.entries[index];
        local.clear_missing(record.id);
        if record.path != entry.path {
            result.ops.push(PendingOp::Relocate { note: note.clone(), path: entry.path.clone() });
            result.relocated.push((record.path.clone(), entry.path.clone()));
        }
        if record.deleted {
            result.ops.push(PendingOp::SetDeleted { note: note.clone(), value: false });
        }
        let changed = cache
            .get(&key(&entry.path))
            .is_none_or(|cached| cached.size != entry.size || cached.mtime != entry.mtime)
            || record.size != entry.size;
        if changed
            && !entry.online_only
            && entry.size <= HASH_LIMIT
            && let Some(value) = *hashes.entry(index).or_insert_with(|| hash(&entry.path))
            && (value != record.hash || entry.size != record.size)
        {
            result.ops.push(PendingOp::SetFingerprint { note, size: entry.size, hash: value });
        }
    }

    // 2. File ID, then 3. fingerprint, then 4. missing.
    for note in unmatched {
        let Some(record) = library.note(note.id).cloned() else {
            continue;
        };
        let by_id = cache
            .get(&key(&record.path))
            .filter(|cached| cached.file_id != 0 && cached.volume == scan.volume)
            .and_then(|cached| {
                (0..scan.entries.len())
                    .find(|&index| !claimed[index] && scan.entries[index].file_id == cached.file_id)
            });
        let found = by_id.or_else(|| {
            if record.size == 0 || record.size > HASH_LIMIT {
                return None;
            }
            (0..scan.entries.len()).find(|&index| {
                let entry = &scan.entries[index];
                !claimed[index]
                    && !entry.online_only
                    && entry.size == record.size
                    && *hashes.entry(index).or_insert_with(|| hash(&entry.path)) == Some(record.hash)
            })
        });
        match found {
            Some(index) => {
                claimed[index] = true;
                let path = scan.entries[index].path.clone();
                local.clear_missing(record.id);
                result.ops.push(PendingOp::Relocate { note: note.clone(), path: path.clone() });
                if record.deleted {
                    result.ops.push(PendingOp::SetDeleted { note, value: false });
                }
                result.relocated.push((record.path.clone(), path));
            }
            None => {
                local.set_missing(record.id, now);
                let since = local.missing_since(record.id).unwrap_or(now);
                if now.saturating_sub(since) >= PURGE_AFTER_SECS {
                    result.ops.push(PendingOp::Drop { id: record.id });
                    local.clear_missing(record.id);
                }
            }
        }
    }

    for op in &result.ops {
        let _ = apply(library, op);
    }
    local.files = scan
        .entries
        .iter()
        .map(|entry| CachedFile {
            volume: scan.volume,
            file_id: entry.file_id,
            mtime: entry.mtime,
            size: entry.size,
            path: entry.path.clone(),
        })
        .collect();
    for (old, new) in &result.relocated {
        local.rename_path(old, new);
    }
    local.missing.retain(|(_, id)| library.note(*id).is_some());
    result
}
```

Add `pub mod reconcile;` to `src/library/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::reconcile`
Expected: all 12 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs src/library/reconcile.rs
git commit -m "feat(library): reconcile records by path, file ID and fingerprint"
```

---

### Task 9: Library state: load, flush, rescan merge

**Files:**
- Modify: `src/library/mod.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–8.
- Produces (all in `crate::library`):
  - `pub enum Metadata { Ready, Unreadable }`
  - `pub struct NoteEntry { pub path: PathBuf /* relative */, pub size: u64, pub mtime: u64 }`
  - `pub struct LibraryState { pub folder, pub local_path, pub library, pub metadata, pub stamp: Option<FileStamp>, pub local, pub notes: Vec<NoteEntry>, pub truncated: bool, pub pending: Vec<PendingOp>, pub relocated: Vec<(PathBuf, PathBuf)> }`
  - `pub fn load(folder: &Path, local_path: &Path, now: u64) -> Result<LibraryState>`
  - `pub fn flush(state: &mut LibraryState) -> Result<bool>`
  - `pub fn write_local(state: &LibraryState)`
  - `pub fn merge_rescan(previous: LibraryState, fresh: LibraryState) -> LibraryState`
  - `LibraryState` methods:
    - `note_ref(&self, ids: &mut IdSource, path: &Path) -> NoteRef`
    - `apply(&mut self, op: PendingOp) -> Result<(), LibraryError>`
    - `add_note(&mut self, path: &Path)`, `remove_note(&mut self, path: &Path)`
    - `rename_note(&mut self, old: &Path, new: &Path)`
    - `record_for(&self, path: &Path) -> Option<&NoteRecord>`
  - Free functions: `pub fn record_path(folder: &Path, path: &Path) -> PathBuf`, `pub fn is_inside(folder: &Path, path: &Path) -> bool`, `pub fn now_unix() -> u64`
  - `pub struct DiskStamp { pub size: u64, pub modified: u64 }` (`Copy`, `Eq`) and `pub fn disk_stamp(path: &Path) -> Option<DiskStamp>`

Paths passed to `LibraryState` methods are **absolute**. The methods convert them with `record_path`.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/mod.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::{NotebookId, TagId};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!("fastpad-library-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("notes")).unwrap();
            Self(root)
        }
        fn folder(&self) -> PathBuf {
            self.0.join("notes")
        }
        fn local(&self) -> PathBuf {
            self.0.join("local.ini")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn record_paths_are_relative_inside_the_folder_ignoring_case_and_absolute_outside() {
        let folder = Path::new(r"D:\Notes");
        assert_eq!(record_path(folder, Path::new(r"d:\notes\sub\a.md")), PathBuf::from(r"sub\a.md"));
        assert_eq!(record_path(folder, Path::new(r"D:\Other\a.md")), PathBuf::from(r"D:\Other\a.md"));
        assert_eq!(record_path(folder, Path::new(r"D:\NotesArchive\a.md")), PathBuf::from(r"D:\NotesArchive\a.md"));
        assert!(is_inside(folder, Path::new(r"D:\NOTES\a.md")));
        assert!(!is_inside(folder, folder));
    }

    #[test]
    fn opening_a_folder_writes_nothing_into_it() {
        // Break caught: opening a repo as a folder creating .fastpad\ before anything is organized.
        let scratch = Scratch::new("readonly-open");
        std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.notes.len(), 1);
        assert!(!flush(&mut state).unwrap());
        assert!(!scratch.folder().join(".fastpad").exists());
    }

    #[test]
    fn organizing_creates_the_library_file_and_a_reload_sees_it() {
        let scratch = Scratch::new("organize");
        let note = scratch.folder().join("a.md");
        std::fs::write(&note, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &note);
        state.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();
        assert!(flush(&mut state).unwrap());
        assert!(state.pending.is_empty());
        let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(reloaded.record_for(&note).unwrap().favorite);
        assert_eq!(reloaded.record_for(&note).unwrap().path, PathBuf::from("a.md"));
    }

    #[test]
    fn a_file_changed_on_disk_is_merged_not_overwritten() {
        // Break caught: this PC's debounced write replacing the notebook another PC just synced.
        let scratch = Scratch::new("merge");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state.apply(PendingOp::SetFavorite { note: target.clone(), value: true }).unwrap();
        flush(&mut state).unwrap();

        // "Another PC" adds a notebook directly in the file.
        let path = store::library_file(&scratch.folder());
        let mut other = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        other.create_notebook(NotebookId(77), "Synced", 5).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &other).unwrap();

        state.apply(PendingOp::AddTag { note: target, tag: TagId(5), name: "idea".into() }).unwrap();
        flush(&mut state).unwrap();
        let final_state = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(final_state.library.notebook(NotebookId(77)).is_some());
        let record = final_state.record_for(&a).unwrap();
        assert!(record.favorite);
        assert_eq!(record.tags.len(), 1);
    }

    #[test]
    fn an_unreadable_library_file_is_never_overwritten() {
        let scratch = Scratch::new("unreadable");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let path = store::library_file(&scratch.folder());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "version=9\r\nnote=future\r\n").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.metadata, Metadata::Unreadable);
        assert_eq!(state.notes.len(), 1, "notes are still listed");
        let target = state.note_ref(&mut ids, &a);
        let _ = state.apply(PendingOp::SetFavorite { note: target, value: true });
        assert!(!flush(&mut state).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version=9\r\nnote=future\r\n");
    }

    #[test]
    fn a_rescan_keeps_changes_made_while_it_ran() {
        let scratch = Scratch::new("rescan");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        let target = previous.note_ref(&mut ids, &a);
        previous.apply(PendingOp::SetPinned { note: target, value: true }).unwrap();
        previous.local.note_opened(Path::new("a.md"), 150);
        let merged = merge_rescan(previous, fresh);
        assert!(merged.record_for(&a).unwrap().pinned);
        assert_eq!(merged.pending.len(), 1);
        assert_eq!(merged.local.recent[0].0, 150);
    }

    #[test]
    fn a_missing_folder_loads_as_an_empty_library() {
        let scratch = Scratch::new("missing-folder");
        let state = load(&scratch.0.join("not-yet"), &scratch.local(), 100).unwrap();
        assert!(state.notes.is_empty());
        assert_eq!(state.metadata, Metadata::Ready);
    }

    #[test]
    fn the_index_follows_saves_renames_and_deletes() {
        let scratch = Scratch::new("index");
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        state.add_note(&a);
        state.add_note(&a);
        state.add_note(Path::new(r"C:\elsewhere\x.md"));
        assert_eq!(state.notes.len(), 1);
        let mut ids = IdSource::new(1, 2);
        let target = state.note_ref(&mut ids, &a);
        state.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();
        let b = scratch.folder().join("b.md");
        state.rename_note(&a, &b);
        assert_eq!(state.notes[0].path, PathBuf::from("b.md"));
        assert_eq!(state.record_for(&b).unwrap().path, PathBuf::from("b.md"));
        state.remove_note(&b);
        assert!(state.notes.is_empty());
    }

    #[test]
    fn disk_stamps_change_when_a_file_is_rewritten() {
        let scratch = Scratch::new("stamp");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let first = disk_stamp(&a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "ab").unwrap();
        assert_ne!(disk_stamp(&a), Some(first));
        assert_eq!(disk_stamp(&scratch.folder().join("none.md")), None);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::tests`
Expected: a compile error.

- [ ] **Step 3: Write the implementation.** Replace everything in `src/library/mod.rs` above the tests with:

```rust
//! The note library: a folder of plain text files seen as notes, plus sparse organizational
//! metadata (notebooks, tags, favorites, pins) kept in `.fastpad\library.ini` and attached to
//! files by path, file ID and content fingerprint. Nothing here touches a window.

pub mod ids;
pub mod local;
pub mod model;
pub mod ops;
pub mod reconcile;
pub mod scan;
pub mod store;
pub mod title;

use crate::Result;
use ids::IdSource;
use local::LocalState;
use model::{Library, LibraryError, NoteRecord, NoteRef, same_path};
use ops::PendingOp;
use std::path::{Component, Path, PathBuf};
use store::{FileStamp, ReadOutcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metadata {
    Ready,
    /// `library.ini` is damaged or from a newer FastPad: organizing is off and it is never written.
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug)]
pub struct LibraryState {
    pub folder: PathBuf,
    pub local_path: PathBuf,
    pub library: Library,
    pub metadata: Metadata,
    pub stamp: Option<FileStamp>,
    pub local: LocalState,
    pub notes: Vec<NoteEntry>,
    pub truncated: bool,
    pub pending: Vec<PendingOp>,
    pub relocated: Vec<(PathBuf, PathBuf)>,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn component_key(component: Component<'_>) -> String {
    component.as_os_str().to_string_lossy().to_lowercase()
}

fn strip_folder(folder: &Path, path: &Path) -> Option<PathBuf> {
    let mut folder_parts = folder.components();
    let mut path_parts = path.components();
    loop {
        match (folder_parts.next(), path_parts.next()) {
            (None, Some(first)) => {
                let mut rest = PathBuf::from(first.as_os_str());
                rest.extend(path_parts);
                return Some(rest);
            }
            (Some(a), Some(b)) if component_key(a) == component_key(b) => {}
            _ => return None,
        }
    }
}

/// How a record stores `path`: relative inside `folder` (compared ignoring case), else absolute.
pub fn record_path(folder: &Path, path: &Path) -> PathBuf {
    strip_folder(folder, path).unwrap_or_else(|| path.to_path_buf())
}

pub fn is_inside(folder: &Path, path: &Path) -> bool {
    strip_folder(folder, path).is_some()
}

/// Size and write time of a file on disk, to notice edits made outside FastPad.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiskStamp {
    pub size: u64,
    pub modified: u64,
}

pub fn disk_stamp(path: &Path) -> Option<DiskStamp> {
    store::stamp(path).map(|stamp| DiskStamp {
        size: stamp.size,
        modified: stamp.modified,
    })
}

/// Reads both library files, scans the folder and reconciles. Runs on the worker thread.
pub fn load(folder: &Path, local_path: &Path, now: u64) -> Result<LibraryState> {
    let library_path = store::library_file(folder);
    let (mut library, metadata, stamp) = match store::read(&library_path) {
        ReadOutcome::Absent => (Library::default(), Metadata::Ready, None),
        ReadOutcome::Loaded(library, stamp) => (library, Metadata::Ready, Some(stamp)),
        ReadOutcome::Unreadable => (
            Library::default(),
            Metadata::Unreadable,
            store::stamp(&library_path),
        ),
    };
    let mut local = local::read(local_path, folder);
    let scan = if folder.is_dir() {
        scan::scan(folder, scan::NOTE_LIMIT)?
    } else {
        scan::Scan::default()
    };
    let reconciled = if metadata == Metadata::Ready {
        reconcile::reconcile(&mut library, &mut local, &scan, now, &mut |relative| {
            ids::hash_file(&folder.join(relative), reconcile::HASH_LIMIT)
        })
    } else {
        reconcile::Reconciled::default()
    };
    Ok(LibraryState {
        folder: folder.to_path_buf(),
        local_path: local_path.to_path_buf(),
        library,
        metadata,
        stamp,
        local,
        notes: scan
            .entries
            .iter()
            .map(|entry| NoteEntry {
                path: entry.path.clone(),
                size: entry.size,
                mtime: entry.mtime,
            })
            .collect(),
        truncated: scan.truncated,
        pending: reconciled.ops,
        relocated: reconciled.relocated,
    })
}

/// Writes pending operations to `library.ini`. If the file changed on disk since it was read,
/// it is re-read and the pending operations are replayed on top first. Returns whether it wrote.
pub fn flush(state: &mut LibraryState) -> Result<bool> {
    if state.metadata == Metadata::Unreadable || state.pending.is_empty() {
        return Ok(false);
    }
    let path = store::library_file(&state.folder);
    if store::stamp(&path) != state.stamp {
        let mut fresh = match store::read(&path) {
            ReadOutcome::Loaded(library, stamp) => {
                state.stamp = Some(stamp);
                library
            }
            ReadOutcome::Absent => {
                state.stamp = None;
                Library::default()
            }
            ReadOutcome::Unreadable => {
                state.metadata = Metadata::Unreadable;
                return Err(crate::FastPadError::Invariant(
                    "library.ini was replaced by a file this FastPad cannot read",
                ));
            }
        };
        ops::replay(&mut fresh, &state.pending);
        state.library = fresh;
    }
    state.library.prune();
    if state.stamp.is_none() && state.library == Library::default() {
        state.pending.clear();
        return Ok(false);
    }
    state.stamp = Some(store::write(&path, &state.library)?);
    state.pending.clear();
    Ok(true)
}

/// Saves the per-PC state. Failures are ignored: it only holds caches and conveniences.
pub fn write_local(state: &LibraryState) {
    let _ = local::write(&state.local_path, &state.local);
}

/// Installs a rescan's result without losing what changed while it ran.
pub fn merge_rescan(previous: LibraryState, mut fresh: LibraryState) -> LibraryState {
    if fresh.metadata == Metadata::Ready {
        ops::replay(&mut fresh.library, &previous.pending);
    }
    let mut pending = previous.pending;
    pending.append(&mut fresh.pending);
    fresh.pending = pending;
    fresh.local.merge_recent(&previous.local);
    fresh.local.autosave = previous.local.autosave;
    fresh
}

impl LibraryState {
    pub fn record_for(&self, path: &Path) -> Option<&NoteRecord> {
        self.library.note_by_path(&record_path(&self.folder, path))
    }

    /// The existing record's ID for `path`, or a new ID.
    pub fn note_ref(&self, ids: &mut IdSource, path: &Path) -> NoteRef {
        let stored = record_path(&self.folder, path);
        let id = self
            .library
            .note_by_path(&stored)
            .map_or_else(|| ids::NoteId(ids.next()), |record| record.id);
        NoteRef { id, path: stored }
    }

    /// Applies `op` to the live library and keeps it for the next write.
    pub fn apply(&mut self, op: PendingOp) -> std::result::Result<(), LibraryError> {
        ops::apply(&mut self.library, &op)?;
        self.pending.push(op);
        Ok(())
    }

    /// Adds a file FastPad just saved to the index, if it is a note inside the folder.
    pub fn add_note(&mut self, path: &Path) {
        let Some(relative) = strip_folder(&self.folder, path) else {
            return;
        };
        let is_note = relative
            .extension()
            .is_some_and(|ext| title::is_note_extension(&ext.to_string_lossy()));
        if !is_note || self.notes.iter().any(|note| same_path(&note.path, &relative)) {
            return;
        }
        let size = std::fs::metadata(path).map_or(0, |metadata| metadata.len());
        self.notes.push(NoteEntry { path: relative, size, mtime: 0 });
    }

    pub fn remove_note(&mut self, path: &Path) {
        let stored = record_path(&self.folder, path);
        self.notes.retain(|note| !same_path(&note.path, &stored));
    }

    /// Follows a rename FastPad made: the index, the recent list and any record.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
        self.remove_note(old);
        self.add_note(new);
        self.local.rename_path(&old_stored, &new_stored);
        if let Some(record) = self.library.note_by_path(&old_stored) {
            let note = NoteRef { id: record.id, path: old_stored };
            let _ = self.apply(PendingOp::Relocate { note, path: new_stored });
        }
    }
}
```

- [ ] **Step 4: Run all library tests to verify they pass**

Run: `cargo test --lib library::`
Expected: every library test passes (about 57).

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs
git commit -m "feat(library): load, merge-on-write flush and rescan merge"
```

---

## Batch B: Plumbing

### Command numbering for the whole plan

Each task adds its own commands. Add every variant to `CommandId` (`src/window/commands.rs`) with the value shown, append it to the `COMMANDS` array (bumping the array length), and add a line for it to the value test `native_command_values_are_stable_and_round_trip`. Each task also adds its palette entries to `command_palette::ENTRIES` (bumping the array length). The test `every_command_except_tab_positions_and_the_palette_is_listed_once` then keeps passing.

| Value | Variant | Palette label | `needs_document` | Task |
|---|---|---|---|---|
| 157 | `ToggleNotesMode` | Notes: Toggle notes mode | no | 10 |
| 158 | `OpenFolder` | File: Open folder... | no | 15 |
| 159 | `OpenRecentFolder` | File: Open recent folder... | no | 15 |
| 160 | `ToggleFolderAutosave` | Notes: Toggle autosave for this folder | no | 18 |
| 161 | `NoteReloadFromDisk` | Note: Reload from disk | yes | 18 |
| 162 | `NoteKeepMine` | Note: Keep my version | yes | 18 |
| 163 | `NoteToggleFavorite` | Note: Toggle favorite | yes | 19 |
| 164 | `NoteTogglePin` | Note: Toggle pin | yes | 19 |
| 165 | `NoteMoveToNotebook` | Note: Move to notebook... | yes | 19 |
| 166 | `NoteAddTag` | Note: Add tag... | yes | 19 |
| 167 | `NoteRemoveTag` | Note: Remove tag... | yes | 19 |
| 168 | `NotebookNew` | Notebook: New... | no | 19 |
| 169 | `NotebookRename` | Notebook: Rename... | no | 19 |
| 170 | `NotebookChangeColor` | Notebook: Change color... | no | 19 |
| 171 | `NotebookDelete` | Notebook: Delete... | no | 19 |
| 172 | `TagRename` | Tag: Rename... | no | 19 |
| 173 | `TagRemoveEverywhere` | Tag: Remove from all notes... | no | 19 |
| 174 | `NoteRename` | Note: Rename... | yes | 20 |
| 175 | `NoteDelete` | Note: Delete | yes | 20 |

`needs_document` is a `!matches!(self, ...)` list. Commands marked "no" are added to that list.

### Task 10: `notes_mode` setting and toggle

**Files:**
- Modify: `src/config/persisted.rs`, `src/config/defaults.rs`, `src/window/commands.rs`, `src/window/command_palette.rs`, `src/window/main_window.rs` (the `execute_command` arm and the `Settings` literal at about line 4867), `src/window/mod.rs`
- Create: `src/window/library_host.rs`

**Interfaces:**
- Produces:
  - `Settings::notes_mode: bool`, `SettingsDelta::notes_mode: Option<bool>`, and `config::defaults::DEFAULT_NOTES_MODE: bool = true`
  - `CommandId::ToggleNotesMode = 157`
  - `window::library_host::notes_mode_notice(enabled: bool) -> &'static str`

- [ ] **Step 1: Write the failing tests.** In the `persisted.rs` test module:

```rust
#[test]
fn notes_mode_accepts_the_boolean_spellings_and_defaults_on() {
    // Break caught: notes mode that cannot be turned off from fastpad.ini, or that starts off.
    assert!(crate::config::default_settings().notes_mode);
    for (value, expected) in [("off", false), ("0", false), ("yes", true), ("TRUE", true)] {
        assert_eq!(parse(&format!("notes_mode={value}")).notes_mode, Some(expected));
    }
    let delta = parse("notes_mode=maybe");
    assert_eq!(delta.notes_mode, None);
    assert_eq!(delta.warnings.len(), 1);
}
```

In the `main_window.rs` test module:

```rust
#[test]
fn notes_mode_toggle_saves_only_its_line_and_says_so() {
    // Break caught: a toggle lost on restart, or one that rewrites the rest of fastpad.ini.
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("notes-toggle");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::ToggleNotesMode);
    assert!(!app_mut(window.hwnd).settings.notes_mode);
    assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\nnotes_mode=false\r\n");
    assert!(notices(window.hwnd).contains(
        &crate::window::library_host::notes_mode_notice(false).to_owned()
    ));
    super::save_settings_to(None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib notes_mode -- --test-threads=1`
Expected: a compile error (`notes_mode` field missing).

- [ ] **Step 3: Implement it**
  - **`persisted.rs`:**
    - Add `pub notes_mode: bool` to `Settings`, with the doc comment `/// Whether an open folder is treated as a note library (sidebar data, autosave, first-save naming).`
    - Add `pub notes_mode: Option<bool>` to `SettingsDelta`.
    - Add to `apply_delta`: `if let Some(notes_mode) = delta.notes_mode { self.notes_mode = notes_mode; }`
    - Add to `apply_line`, after the `restore_session` arm:
      ```rust
      "notes_mode" => match parse_bool(value) {
          Some(notes_mode) => delta.notes_mode = Some(notes_mode),
          None => warn(delta, line_number, key, value),
      },
      ```
    - Add `notes_mode` to the list of recognized keys in `parse`'s doc comment.
  - **`defaults.rs`:** add `pub const DEFAULT_NOTES_MODE: bool = true;` and `notes_mode: DEFAULT_NOTES_MODE,` in `default_settings()`. End the doc sentence with "…session restore on, and notes mode on."
  - **The `Settings` literal in `main_window.rs` tests (about line 4867):** add `notes_mode: true,`.
  - **`commands.rs`:** `ToggleNotesMode = 157`, following the numbering table.
  - **`command_palette.rs`:** add `entry("Notes: Toggle notes mode", CommandId::ToggleNotesMode),` after "File: Toggle session restore".
  - **`src/window/mod.rs`:** add `pub(crate) mod library_host;`.
  - **`src/window/library_host.rs`:**
    ```rust
    //! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
    //! folder commands, first-save naming, autosave, and the organizing commands.

    pub(crate) fn notes_mode_notice(enabled: bool) -> &'static str {
        if enabled {
            "Notes mode is on. The open folder is your note library."
        } else {
            "Notes mode is off. FastPad works as a plain file editor."
        }
    }
    ```
  - **The `execute_command` arm**, next to `ToggleRestoreSession`:
    ```rust
    CommandId::ToggleNotesMode => {
        change_setting(hwnd, |settings| {
            settings.notes_mode = !settings.notes_mode;
            Some(("notes_mode", settings.notes_mode.to_string()))
        });
        let enabled = unsafe { app_ptr(hwnd) }
            .is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode);
        push_notice(hwnd, crate::window::library_host::notes_mode_notice(enabled).to_owned());
    }
    ```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib notes_mode -- --test-threads=1`, then `cargo test --lib -- window::command_palette window::commands`
Expected: all pass, including the "listed once" and "stable values" tests.

- [ ] **Step 5: Commit**

```bash
git add src/config src/window
git commit -m "feat(config): notes_mode setting and palette toggle"
```

---

### Task 11: Platform helpers and modal seams

**Files:**
- Create: `src/platform/files.rs`
- Modify: `src/platform/mod.rs` (`pub mod files;`), `src/platform/paths.rs`, `src/platform/dialogs.rs`, `src/window/commands.rs`, `src/window/modal.rs`, `src/window/mod.rs`, `src/window/panel.rs`, `src/window/main_window.rs` (callers of `choose_save_path`)

**Interfaces:**
- Produces:
  - `platform::files::rename_no_replace(old: &Path, new: &Path) -> Result<()>`
  - `platform::files::recycle(path: &Path) -> Result<()>`
  - `platform::paths::documents_dir() -> Result<PathBuf>` and `platform::paths::default_notes_folder() -> Result<PathBuf>` (`Documents\FastPad`)
  - `platform::dialogs::show_folder_dialog(owner: HWND) -> Result<Option<PathBuf>>`
  - `platform::dialogs::show_save_dialog(owner: HWND, suggested_name: &str, folder: Option<&Path>) -> Result<Option<PathBuf>>` (adds a parameter)
  - `window::commands::choose_folder_path(owner) -> Result<Option<PathBuf>>` and `choose_save_path(owner, suggested, folder: Option<&Path>)`
  - `window::modal::choose_save_path(hwnd, suggested, folder: Option<&Path>)`
  - `window::modal::choose_folder(hwnd) -> Result<Option<PathBuf>>`
  - `window::modal::confirm(hwnd, text: &str) -> bool`
  - Test seams: `answer_next_folder_dialog(impl FnOnce(HWND) -> Option<PathBuf> + 'static)` and `answer_next_confirm(impl FnOnce(HWND) -> bool + 'static)`, re-exported from `window/mod.rs` next to `answer_next_save_dialog`.
  - `window::panel::create_child_with_id(parent: HWND, class: &[u16], style: u32, id: u16) -> Result<HWND>`

- [ ] **Step 1: Write the failing tests** in `src/platform/files.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fastpad-files-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rename_never_replaces_an_existing_file_but_allows_a_case_change() {
        // Break caught: std::fs::rename's MOVEFILE_REPLACE_EXISTING silently destroying a note
        // that appeared under the new name after the clash check.
        let dir = scratch("rename");
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join("b.md"), "b").unwrap();
        assert!(rename_no_replace(&dir.join("a.md"), &dir.join("b.md")).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("b.md")).unwrap(), "b");
        rename_no_replace(&dir.join("a.md"), &dir.join("A.md")).unwrap();
        rename_no_replace(&dir.join("A.md"), &dir.join("c.md")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("c.md")).unwrap(), "a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycling_removes_the_file_from_its_folder() {
        let dir = scratch("recycle");
        let path = dir.join("gone.md");
        std::fs::write(&path, "x").unwrap();
        recycle(&path).unwrap();
        assert!(!path.exists());
        assert!(recycle(&dir.join("never-existed.md")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_notes_folder_is_under_documents() {
        let documents = crate::platform::paths::documents_dir().unwrap();
        assert!(documents.is_dir());
        assert_eq!(crate::platform::paths::default_notes_folder().unwrap(), documents.join("FastPad"));
    }
}
```

In the `main_window.rs` test module, a check that the new seams are wired:

```rust
#[test]
fn folder_and_confirm_seams_answer_inside_their_modal_scope() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    crate::window::answer_next_folder_dialog(|_| Some(std::path::PathBuf::from(r"D:\Notes")));
    assert_eq!(
        crate::window::modal::choose_folder(window.hwnd).unwrap(),
        Some(std::path::PathBuf::from(r"D:\Notes"))
    );
    crate::window::answer_next_confirm(|_| false);
    assert!(!crate::window::modal::confirm(window.hwnd, "Delete?"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- platform::files seams_answer --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

`src/platform/files.rs`:

```rust
//! File operations the note library needs that `std` does not offer safely: a rename that never
//! replaces its target, and deleting to the Recycle Bin.

use crate::Result;
use crate::platform::last_error;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    SHFileOperationW,
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`: fails if `new` already exists (a change of
/// letter case only is allowed, because it names the same file).
pub fn rename_no_replace(old: &Path, new: &Path) -> Result<()> {
    let (old_wide, new_wide) = (wide(old), wide(new));
    if unsafe { MoveFileExW(old_wide.as_ptr(), new_wide.as_ptr(), 0) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Sends one file to the Recycle Bin without any shell UI.
pub fn recycle(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(crate::FastPadError::Invariant("the file to delete does not exist"));
    }
    // SHFileOperationW takes a list ending in two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as _;
    let status = unsafe { SHFileOperationW(&mut operation) };
    if status != 0 || operation.fAnyOperationsAborted != 0 {
        return Err(crate::FastPadError::Win32(status as u32));
    }
    Ok(())
}
```

(Match `wFunc` and `fFlags` to the integer types `windows-sys` 0.61 declares. The `as _` casts cover that.)

In `src/platform/paths.rs`, add:

```rust
/// The user's Documents folder, from the known-folder database.
#[cfg(windows)]
pub fn documents_dir() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut raw = std::ptr::null_mut();
    let status = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut raw) };
    if status < 0 || raw.is_null() {
        if !raw.is_null() {
            unsafe { CoTaskMemFree(raw.cast()) };
        }
        return Err(FastPadError::Win32(status as u32));
    }
    let mut len = 0;
    // The shell owns this terminated UTF-16 allocation until CoTaskMemFree.
    unsafe {
        while *raw.add(len) != 0 {
            len += 1;
        }
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw, len)
    }));
    unsafe { CoTaskMemFree(raw.cast()) };
    Ok(path)
}

/// Where Ctrl+N notes go before any folder was opened: `Documents\FastPad`.
#[cfg(windows)]
pub fn default_notes_folder() -> Result<PathBuf> {
    Ok(documents_dir()?.join("FastPad"))
}
```

In `src/platform/dialogs.rs`:
1. Move the body of `show_open_dialog` into `fn run_open_dialog(owner: HWND, options_to_add: u32) -> Result<Option<PathBuf>>`, which ORs `options_to_add` into the options. `show_open_dialog(owner)` becomes `run_open_dialog(owner, FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)`.
2. Add `pub fn show_folder_dialog(owner: HWND) -> Result<Option<PathBuf>> { run_open_dialog(owner, FOS_FORCEFILESYSTEM | FOS_PICKFOLDERS) }`. `FOS_PICKFOLDERS` comes from `windows_sys::Win32::UI::Shell`.
3. In `FileDialogVtable`, replace the placeholder at the `SetFolder` slot with a typed field `set_folder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT`. IFileDialog's order after IUnknown is: Show, SetFileTypes, SetFileTypeIndex, GetFileTypeIndex, Advise, Unadvise, SetOptions, GetOptions, SetDefaultFolder, **SetFolder**, GetFolder, GetCurrentSelection, SetFileName, … Count the existing fields to find the slot, and do not reorder anything else.
4. `show_save_dialog` gains `folder: Option<&Path>`. After `set_file_name`, if the folder is `Some` and `is_dir()`, call `SHCreateItemFromParsingName(wide_path, null, &IID_ISHELL_ITEM, &mut item.0)` (with `const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe)`, and `item` an `Interface`). On success, call `(dialog.dialog().set_folder)(dialog.0, item.0)`. If either call fails, ignore it: the dialog opens where it normally would.

In `src/window/commands.rs`, add `pub(crate) fn choose_folder_path(owner: HWND) -> crate::Result<Option<PathBuf>> { crate::platform::dialogs::show_folder_dialog(owner) }`, and thread `folder: Option<&Path>` through `choose_save_path`.

In `src/window/modal.rs`, follow `SAVE_ANSWERS` and `answer_next_save_dialog` exactly:

```rust
pub(super) fn choose_save_path(
    hwnd: HWND,
    suggested_name: &str,
    folder: Option<&std::path::Path>,
) -> crate::Result<Option<PathBuf>> {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = SAVE_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return Ok(answer(hwnd));
    }
    crate::window::commands::choose_save_path(hwnd, suggested_name, folder)
}

pub(crate) fn choose_folder(hwnd: HWND) -> crate::Result<Option<PathBuf>> {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = FOLDER_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return Ok(answer(hwnd));
    }
    crate::window::commands::choose_folder_path(hwnd)
}

/// OK/Cancel warning. Returns whether the user chose OK.
pub(crate) fn confirm(hwnd: HWND, text: &str) -> bool {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = CONFIRM_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd);
    }
    let text = crate::platform::wide_null(text);
    let caption = crate::platform::wide_null("FastPad");
    unsafe { MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OKCANCEL | MB_ICONWARNING) == IDOK }
}
```

Add `FOLDER_ANSWERS: RefCell<VecDeque<Answer<Option<PathBuf>>>>` and `CONFIRM_ANSWERS: RefCell<VecDeque<Answer<bool>>>` to the `#[cfg(test)] thread_local!`. Add `answer_next_folder_dialog` and `answer_next_confirm`, following `answer_next_save_dialog`. Import `MessageBoxW`, `MB_OKCANCEL`, `MB_ICONWARNING` and `IDOK` the way `prompt_close_decision` imports its own. Change `mod modal;` to `pub(crate) mod modal;` in `window/mod.rs`, and re-export the two seams next to `answer_next_save_dialog`.

In `main_window.rs`, `save_active_document_as` passes `None` for now: `crate::window::modal::choose_save_path(hwnd, &suggested, None)`.

In `src/window/panel.rs`, add the following (its body is `create_child` with `id as HMENU` in place of the null menu):

```rust
/// Like `create_child`, but with a control ID so `WM_COMMAND` can tell the child apart.
pub(crate) fn create_child_with_id(parent: HWND, class: &[u16], style: u32, id: u16) -> crate::Result<HWND>
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- platform:: seams_answer save_as --test-threads=1`
Expected: all pass. The existing save-as tests still pass, because the dialog seam ignores `folder`.

- [ ] **Step 5: Commit**

```bash
git add src/platform src/window
git commit -m "feat(platform): folder picker, save in folder, safe rename, recycle bin, confirm prompt"
```

---

### Task 12: Picker mode for the command palette

**Files:**
- Modify: `src/window/command_palette.rs`, `src/window/main_window.rs`, `src/window/library_host.rs`

**Interfaces:**
- Produces, in `command_palette.rs`:
  - `pub(crate) enum PickerKind { RecentFolder, MoveToNotebook, AddTag, RemoveTag, RenameNotebook, RecolorNotebook, ChooseColor, DeleteNotebook, RenameTag, RemoveTagEverywhere }` (`Clone, Copy, Debug, Eq, PartialEq`)
  - `pub(crate) struct Picker { pub kind: PickerKind, pub items: Vec<String>, pub create: Option<&'static str> }`
  - `pub(crate) enum PickerRow { Item(usize), Create(String) }` and `pub(crate) enum PickerChoice { Item(usize), Create(String) }`
  - `pub(crate) fn picker_rows(picker: &Picker, query: &str) -> Vec<PickerRow>`
  - `pub(crate) fn picker_row_label(picker: &Picker, row: &PickerRow) -> String`
  - `CommandPalette::set_picker(&mut self, picker: Option<Picker>)`, `picker(&self) -> Option<&Picker>`, `set_picker_rows(&mut self, rows: Vec<PickerRow>)` and `selected_choice(&self) -> Option<PickerChoice>`
- Produces, in `main_window.rs`: `pub(crate) fn open_picker(hwnd: HWND, picker: Picker)`
- Produces, in `library_host.rs`: `pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice)`. It records the last pick under `cfg(test)`, and later tasks fill its `match`.

- [ ] **Step 1: Write the failing tests** in the `command_palette.rs` tests:

```rust
fn picker(create: Option<&'static str>) -> Picker {
    Picker {
        kind: PickerKind::AddTag,
        items: vec!["idea".into(), "reference".into(), "todo".into()],
        create,
    }
}

#[test]
fn picker_rows_filter_items_like_commands_and_offer_to_create_a_new_name() {
    // Break caught: typing a new tag name leaving nothing to press Enter on, or offering to
    // create a tag that already exists under another case.
    let with_create = picker(Some("Add tag"));
    assert_eq!(
        picker_rows(&with_create, ""),
        vec![PickerRow::Item(0), PickerRow::Item(1), PickerRow::Item(2)]
    );
    assert_eq!(
        picker_rows(&with_create, "ref"),
        vec![PickerRow::Item(1), PickerRow::Create("ref".into())]
    );
    assert_eq!(picker_rows(&with_create, "TODO"), vec![PickerRow::Item(2)]);
    assert_eq!(picker_rows(&picker(None), "zzz"), vec![]);
    assert_eq!(
        picker_row_label(&with_create, &PickerRow::Create("urgent".into())),
        "Add tag \u{201c}urgent\u{201d}"
    );
    assert_eq!(picker_row_label(&with_create, &PickerRow::Item(0)), "idea");
}
```

In the `main_window.rs` tests:

```rust
#[test]
fn a_picker_lists_its_items_and_enter_reports_the_choice() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    super::open_picker(window.hwnd, crate::window::command_palette::Picker {
        kind: crate::window::command_palette::PickerKind::RecentFolder,
        items: vec![r"D:\A".into(), r"D:\B".into()],
        create: None,
    });
    super::move_command_palette_selection(window.hwnd, 1);
    super::run_command_palette_selection(window.hwnd);
    assert_eq!(
        crate::window::library_host::take_last_pick(),
        Some((
            crate::window::command_palette::PickerKind::RecentFolder,
            crate::window::command_palette::PickerChoice::Item(1)
        ))
    );
    // The palette went back to command mode.
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert!(with_command_palette(window.hwnd, |p| p.picker().is_none()).unwrap());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib picker -- --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

In `command_palette.rs`:

```rust
/// What a picker is choosing; decides what `library_host::picked` does with the choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    RecentFolder,
    MoveToNotebook,
    AddTag,
    RemoveTag,
    RenameNotebook,
    RecolorNotebook,
    ChooseColor,
    DeleteNotebook,
    RenameTag,
    RemoveTagEverywhere,
}

/// A list of runtime items shown in the palette instead of commands.
#[derive(Clone, Debug)]
pub(crate) struct Picker {
    pub kind: PickerKind,
    pub items: Vec<String>,
    /// When set, a typed name that matches no item exactly is offered as "<create> "<name>"".
    pub create: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerRow {
    Item(usize),
    Create(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerChoice {
    Item(usize),
    Create(String),
}

pub(crate) fn picker_rows(picker: &Picker, query: &str) -> Vec<PickerRow> {
    let query = query.trim();
    let mut ranked: Vec<(u8, usize)> = picker
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if query.is_empty() {
                Some((0, index))
            } else {
                match_rank(query, item).map(|rank| (rank, index))
            }
        })
        .collect();
    ranked.sort_by_key(|&(rank, index)| (rank, index));
    let mut rows: Vec<PickerRow> = ranked.into_iter().map(|(_, index)| PickerRow::Item(index)).collect();
    let exact = picker.items.iter().any(|item| item.to_lowercase() == query.to_lowercase());
    if picker.create.is_some() && !query.is_empty() && !exact {
        rows.push(PickerRow::Create(query.to_owned()));
    }
    rows
}

pub(crate) fn picker_row_label(picker: &Picker, row: &PickerRow) -> String {
    match row {
        PickerRow::Item(index) => picker.items.get(*index).cloned().unwrap_or_default(),
        PickerRow::Create(name) => {
            format!("{} \u{201c}{name}\u{201d}", picker.create.unwrap_or("Create"))
        }
    }
}
```

`match_rank`'s ranks already order prefix, then word, then substring, then scattered. Check that `match_rank` sorts the same way `filter_entries` uses it, and keep the stable tiebreak on the item index.

`CommandPalette` gains two fields, `picker: Option<Picker>` and `picker_rows: Vec<PickerRow>`:
- `set_picker(picker)` stores the picker and clears the rows.
- `picker()` returns `self.picker.as_ref()`.
- `set_picker_rows(rows)` stores the rows.
- `selected_choice()` reads the list's current selection (as `selected_command` does), maps that index into `picker_rows`, and converts `PickerRow` into `PickerChoice`.
- `fill_list`: while `picker` is `Some`, add one list item per `picker_rows` entry (the owner-drawn list only needs the right count).
- `draw_item`: while `picker` is `Some`, draw `picker_row_label(picker, row)` as the label, with no shortcut text.
- `mark_hidden`: also clear `picker` and `picker_rows`.

In `main_window.rs`:
- `open_picker(hwnd, picker)` does what `open_command_palette` does, but calls `palette.set_picker(Some(picker))` right after `mark_shown`, and then `refilter_command_palette`.
- `refilter_command_palette`: when `palette.picker()` is `Some`, compute `command_palette::picker_rows(picker, &query)` and call `set_picker_rows`. Otherwise keep the command filtering as it is.
- `run_command_palette_selection`: when a picker is active, read `(picker.kind, palette.selected_choice())` before `close_command_palette(hwnd, true)`, then call `crate::window::library_host::picked(hwnd, kind, choice)` if a choice exists. The command path is unchanged.
- The existing `CommandId::CommandPalette` path must call `set_picker(None)`, so that opening the palette normally always shows commands.

In `library_host.rs`:

```rust
use crate::window::command_palette::{PickerChoice, PickerKind};
use windows_sys::Win32::Foundation::HWND;

#[cfg(test)]
thread_local! {
    static LAST_PICK: std::cell::RefCell<Option<(PickerKind, PickerChoice)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn take_last_pick() -> Option<(PickerKind, PickerChoice)> {
    LAST_PICK.with(|last| last.borrow_mut().take())
}

/// A picker row was chosen. Tasks 15 and 19 add one arm per kind.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    let _ = (hwnd, kind, choice);
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- picker command_palette --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window
git commit -m "feat(window): picker mode for the command palette"
```

---

### Task 13: Document, tabs and editor additions

**Files:**
- Modify: `src/document.rs`, `src/window/tabs.rs`, `src/editor/scintilla.rs`, `src/editor/scintilla_constants.rs`, `src/window/main_window.rs` (tests only)

**Interfaces:**
- Produces:
  - New `Document` fields, set by every constructor (`Document::untitled`, and every `Document { ... }` literal: search `Document {` in `src/`):
    - `pub untitled_label: Option<String>` (default `None`)
    - `pub label_watch: usize` (default `crate::library::title::LABEL_SCAN_LINES - 1`)
    - `pub disk_stamp: Option<crate::library::DiskStamp>` (default `None`)
    - `pub autosave_paused: bool` (default `false`)
  - The new `Document::title` rule for untitled tabs (below).
  - `Tabs::document_mut(&mut self, id: DocumentId) -> Option<&mut Document>`
  - `Tabs::rebind_path(&mut self, id: DocumentId, path: PathBuf) -> Result<(), DuplicateDocumentPath>`. It rejects a path another tab has, like `set_active_path`, and calls `self.view.update` the way `set_active_dirty` does, so the tab strip and accessibility see the new title.
  - `Editor::line_count(&self) -> Result<usize>` and `Editor::line_text(&self, line: usize) -> Result<String>` (without the line ending)

- [ ] **Step 1: Write the failing tests.** In `document.rs` tests:

```rust
#[test]
fn an_untitled_tab_shows_its_label_but_recovered_and_file_tabs_keep_their_names() {
    // Break caught: a crash-recovered tab losing its "Recovered:" prefix, or a saved file
    // showing its first line instead of its filename.
    let mut document = Document::test_fixture(DocumentId(1), false);
    assert_eq!(document.title(), "Untitled");
    document.untitled_label = Some("Meeting notes".into());
    assert_eq!(document.title(), "Meeting notes");
    document.dirty = true;
    assert_eq!(document.title(), "Meeting notes *");
    document.recovery_origin = Some(RecoveryOrigin {
        snapshot_path: "x.fps".into(),
        original_path: None,
        from_session: false,
    });
    assert_eq!(document.title(), "Recovered: Untitled *");
    document.recovery_origin.as_mut().unwrap().from_session = true;
    assert_eq!(document.title(), "Meeting notes *");
    document.path = Some(r"D:\Notes\plan.md".into());
    assert_eq!(document.title(), "plan.md *");
}
```

(If `test_fixture` does not set `path: None`, set it in the test.)

In the `tabs.rs` tests (follow the existing fixture style there):

```rust
#[test]
fn rebinding_a_path_rejects_one_another_tab_has() {
    let mut tabs = Tabs::from_documents(/* two fixtures, ids 1 and 2, paths a.md and b.md, as the existing tests build them */);
    assert!(tabs.rebind_path(DocumentId(1), "b.md".into()).is_err());
    tabs.rebind_path(DocumentId(1), "c.md".into()).unwrap();
    assert_eq!(tabs.document(DocumentId(1)).unwrap().path.as_deref(), Some(std::path::Path::new("c.md")));
    tabs.document_mut(DocumentId(2)).unwrap().autosave_paused = true;
    assert!(tabs.document(DocumentId(2)).unwrap().autosave_paused);
}
```

In the `main_window.rs` tests:

```rust
#[test]
fn the_editor_reads_single_lines_without_their_line_endings() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("first\r\nsecond\nthird").unwrap();
    assert_eq!(editor.line_count().unwrap(), 3);
    assert_eq!(editor.line_text(0).unwrap(), "first");
    assert_eq!(editor.line_text(1).unwrap(), "second");
    assert_eq!(editor.line_text(2).unwrap(), "third");
    assert_eq!(editor.line_text(9).unwrap(), "");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- untitled_tab rebinding_a_path single_lines --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

`Document::title`:

```rust
pub fn title(&self) -> String {
    let base = match (&self.path, &self.recovery_origin) {
        (None, Some(origin)) if !origin.from_session => {
            format!("Recovered: {}", origin.display_name())
        }
        // A session tab reopened unbound, because its file was already open elsewhere,
        // still names that file.
        (None, Some(origin)) if origin.original_path.is_some() => origin.display_name(),
        (None, _) => self.untitled_label.clone().unwrap_or_else(|| "Untitled".to_owned()),
        (Some(path), _) => file_name_or_untitled(Some(path)),
    };
    if self.dirty {
        format!("{base} *")
    } else {
        base
    }
}
```

With `untitled_label` set to `None`, this gives exactly the old titles.

In `scintilla_constants.rs`, add `SCI_GETLINECOUNT = 2154`, `SCI_GETLINE = 2153` and `SCI_LINELENGTH = 2350`, if they are missing. In `scintilla.rs`, add both methods next to `line_from_position`, with the same `cfg` split as the existing methods:

```rust
pub fn line_count(&self) -> Result<usize> {
    Ok(self.send(SCI_GETLINECOUNT, 0, 0)? as usize)
}

/// The text of `line` without its CR/LF, or empty past the last line.
pub fn line_text(&self, line: usize) -> Result<String> {
    if line >= self.line_count()? {
        return Ok(String::new());
    }
    let length = self.send(SCI_LINELENGTH, line, 0)? as usize;
    let mut buffer = vec![0_u8; length + 1];
    self.send(SCI_GETLINE, line, buffer.as_mut_ptr() as isize)?;
    buffer.truncate(length);
    let text = String::from_utf8_lossy(&buffer);
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}
```

Use whichever send helper `line_from_position` uses (its name may differ from `send`), and its error mapping.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- untitled_tab rebinding_a_path single_lines document:: tabs:: --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/document.rs src/window/tabs.rs src/editor src/window/main_window.rs
git commit -m "feat(document): untitled labels, disk stamps, path rebinding and line reads"
```

---

## Batch C: Window features

Every function in this batch that touches `App` follows the existing pattern:
- Call `unsafe { app_ptr(hwnd) }` and use the reference at once.
- Never hold it across a call that can re-enter the window procedure (dialogs, `complete_save`, `open_path`, `SetFocus`).
- Across such calls, take `window_identity(hwnd)` first and check `identity.is_live_for(hwnd)` afterwards.

`library_host.rs` reaches main-window helpers as `super::main_window::…`. Where a helper it needs is a private `fn`, make it `pub(super)`: this applies to `complete_save`, `activate_document_by_id`, `save_active_document`, `save_active_document_as`, `file_population_active`, `refresh_tabs`, `layout_editor_and_find_bar`, `close_active_document` and `create_new_document`. Change only the visibility. The helpers that are already `pub(crate)` stay as they are: `app_ptr`, `window_identity`, `push_notice`, `report_open_failure`, `open_path`, `invalidate_title_strip` and `close_find_bar`.

### Task 14: Library host: startup step, worker, ready, rescans and writes

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/messages.rs`, `src/window/mod.rs`, `src/app.rs`, `src/window/main_window.rs`

**Interfaces:**
- Consumes: `library::{load, flush, write_local, merge_rescan, LibraryState, Metadata}`, `library::local::{local_file, folders_file, read_folders}`, `platform::paths::default_notes_folder`.
- Produces:
  - Constants: `WM_FASTPAD_OPEN_LIBRARY = WM_APP + 9` (in the chain) and `WM_FASTPAD_LIBRARY_READY = WM_APP + 10` (not in the chain; it carries a `Box`).
  - `library_host::LibraryHost`, with its fields as below, and `LibraryHost::new(process_start: u64)`.
  - `App::library: LibraryHost`.
  - `library_host` functions:
    - `open_library_step(hwnd)`, `library_ready(hwnd, lparam)`, `start_load(hwnd)`, `request_rescan(hwnd)`
    - `activation_changed(hwnd, active: bool)`, `schedule_write(hwnd)`, `flush_now(hwnd)`, `notes_mode_changed(hwnd, enabled: bool)`
    - `with_state<R>(hwnd, f: impl FnOnce(&mut LibraryState) -> R) -> Option<R>`, `folder(hwnd) -> Option<PathBuf>`
  - Constants: `LIBRARY_WRITE_TIMER_ID: usize = 0x4650_4C57` and `RESCAN_AFTER: Duration = 5 s`.
  - Test seam: `#[cfg(test)] install_for_test(hwnd, state: LibraryState)`.

- [ ] **Step 1: Write the failing tests.** In the `messages.rs` tests, update the chain-order test:

```rust
assert_eq!(
    classify_deferred_message(WM_FASTPAD_RESTORE_SESSION, false),
    Some(DeferredAction::PostNext(WM_FASTPAD_OPEN_LIBRARY))
);
assert_eq!(
    classify_deferred_message(WM_FASTPAD_OPEN_LIBRARY, false),
    Some(DeferredAction::PostNext(WM_FASTPAD_OPEN_REQUEST))
);
assert_eq!(
    classify_deferred_message(WM_FASTPAD_OPEN_LIBRARY, true),
    Some(DeferredAction::RepostSelf(WM_FASTPAD_OPEN_LIBRARY))
);
assert_eq!(classify_deferred_message(WM_FASTPAD_LIBRARY_READY, false), None);
```

In the `main_window.rs` tests, add a helper and three tests:

```rust
struct LibraryScratch {
    root: std::path::PathBuf,
}

impl LibraryScratch {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("fastpad-libhost-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        Self { root }
    }
    fn folder(&self) -> std::path::PathBuf {
        self.root.join("notes")
    }
    fn data(&self) -> std::path::PathBuf {
        self.root.join("data")
    }
    fn note(&self, name: &str, text: &str) -> std::path::PathBuf {
        let path = self.folder().join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    /// Loads the folder synchronously and installs it, as LIBRARY_READY would.
    fn install(&self, hwnd: HWND) {
        let local = crate::library::local::local_file(&self.data(), &self.folder());
        let state = crate::library::load(&self.folder(), &local, crate::library::now_unix()).unwrap();
        crate::window::library_host::install_for_test(hwnd, state);
    }
}

impl Drop for LibraryScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Pumps posted messages until `done` or 5 s.
fn pump_until(hwnd: HWND, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        pump_posted_messages(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn the_library_step_loads_the_remembered_folder_on_a_worker_thread() {
    // Break caught: the scan running on the UI thread, or the remembered folder ignored.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("startup");
    scratch.note("a.md", "a");
    let mut folders = crate::library::local::RecentFolders::default();
    folders.push(scratch.folder());
    crate::library::local::write_folders(&crate::library::local::folders_file(&scratch.data()), &folders).unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(crate::window::library_host::folder(window.hwnd), Some(scratch.folder()));
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    assert_eq!(app_mut(window.hwnd).library.state.as_ref().unwrap().notes.len(), 1);
}

#[test]
fn with_notes_mode_off_the_library_step_does_nothing() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("off");
    let window = ProductionWindow::new(make_app());
    app_mut(window.hwnd).settings.notes_mode = false;
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::window::library_host::open_library_step(window.hwnd);
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    assert!(!app_mut(window.hwnd).library.scanning);
}

#[test]
fn a_stale_ready_message_is_dropped_and_writes_are_flushed_on_demand() {
    // Break caught: a slow scan of the previous folder replacing the folder just opened.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("stale");
    let a = scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let stale = crate::window::library_host::test_ready_payload(
        app_mut(window.hwnd).library.generation.wrapping_sub(1),
        Err("old".into()),
    );
    crate::window::library_host::library_ready(window.hwnd, stale);
    assert!(app_mut(window.hwnd).library.state.is_some());
    assert!(notices(window.hwnd).iter().all(|n| !n.contains("old")));

    crate::window::library_host::with_state(window.hwnd, |state| {
        let mut ids = crate::library::ids::IdSource::new(1, 1);
        let target = state.note_ref(&mut ids, &a);
        state.apply(crate::library::ops::PendingOp::SetFavorite { note: target, value: true }).unwrap();
    });
    crate::window::library_host::flush_now(window.hwnd);
    assert!(crate::library::store::library_file(&scratch.folder()).exists());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- messages library_step stale_ready --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

`messages.rs`:

```rust
// Deferred chain, between RESTORE_SESSION and OPEN_REQUEST.
pub const WM_FASTPAD_OPEN_LIBRARY: u32 = WM_APP + 9;
// Not part of the deferred chain: the library worker's result, as a `Box` the receiver frees.
pub const WM_FASTPAD_LIBRARY_READY: u32 = WM_APP + 10;
```

In `classify_deferred_message`:

```rust
WM_FASTPAD_RESTORE_SESSION => next_action(message, WM_FASTPAD_OPEN_LIBRARY, input_pending),
WM_FASTPAD_OPEN_LIBRARY => next_action(message, WM_FASTPAD_OPEN_REQUEST, input_pending),
```

Re-export both constants from `window/mod.rs` alongside the other `WM_FASTPAD_*`.

`main_window.rs`:
- **`handle_deferred`:** the arm that ran `restore_session_step` now matches `DeferredAction::PostNext(crate::window::WM_FASTPAD_OPEN_LIBRARY)`, and still reposts `RESTORE_SESSION` on `Continue`. Update the comment above it to "Only `WM_FASTPAD_RESTORE_SESSION` … produces this action". Add a new arm:
  ```rust
  // Only `WM_FASTPAD_OPEN_LIBRARY` processed with no input pending produces this action.
  if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_OPEN_REQUEST) {
      crate::window::library_host::open_library_step(hwnd);
  }
  ```
- **The window procedure's `_ =>` arm**, before `classify_deferred_message`:
  ```rust
  if message == crate::window::WM_FASTPAD_LIBRARY_READY {
      crate::window::library_host::library_ready(hwnd, lparam);
      return 0;
  }
  ```
- **A new arm:**
  ```rust
  WM_TIMER if wparam == crate::window::library_host::LIBRARY_WRITE_TIMER_ID => {
      crate::window::library_host::flush_now(hwnd);
      0
  }
  ```
- **A new arm**, which falls through to the default handling:
  ```rust
  WM_ACTIVATEAPP => {
      crate::window::library_host::activation_changed(hwnd, wparam != 0);
      unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
  }
  ```
- **`WM_DESTROY`:** `KillTimer(hwnd, crate::window::library_host::LIBRARY_WRITE_TIMER_ID)`.
- **`WM_CLOSE`:** call `crate::window::library_host::flush_now(hwnd)` right before `shutdown_ipc(hwnd)`.
- **The `ToggleNotesMode` arm:** after `change_setting`, call `crate::window::library_host::notes_mode_changed(hwnd, enabled)`.
- **Session-restore test helpers that expect the handover to `OPEN_REQUEST`:** `run_session_restore` (about line 6105) and every `discard_posted(…, WM_FASTPAD_OPEN_REQUEST)` after a restore (about lines 6379–6480) now see `WM_FASTPAD_OPEN_LIBRARY`. Discard or expect that message instead, and keep each test's meaning the same.

`app.rs`: add `pub(crate) library: crate::window::library_host::LibraryHost,` and initialize it in `App::new` with `crate::window::library_host::LibraryHost::new(process_start)`. It is the same `process_start` value the struct already stores, so compute it before the struct literal if it is not already a local.

`library_host.rs` (added to the Task 10 and 12 content):

```rust
use super::main_window::{app_ptr, push_notice, window_identity};
use crate::library::{self, LibraryState, Metadata, ids::IdSource};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::LPARAM;
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

pub(crate) const LIBRARY_WRITE_TIMER_ID: usize = 0x4650_4C57;
pub(crate) const RESCAN_AFTER: Duration = Duration::from_secs(5);
const WRITE_DELAY_MS: u32 = 500;

#[derive(Debug)]
pub(crate) struct LibraryHost {
    /// The open folder. Set as soon as the step runs, before its state has loaded.
    pub(crate) folder: Option<PathBuf>,
    pub(crate) state: Option<LibraryState>,
    /// `%LOCALAPPDATA%\FastPad`, resolved lazily. Under `cfg(test)` it is only ever pre-seeded.
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) generation: u64,
    pub(crate) scanning: bool,
    pub(crate) rescan_requested: bool,
    pub(crate) inactive_since: Option<Instant>,
    pub(crate) ids: IdSource,
    notified: Option<PathBuf>,
}

impl LibraryHost {
    pub(crate) fn new(process_start: u64) -> Self {
        Self {
            folder: None,
            state: None,
            data_dir: None,
            generation: 0,
            scanning: false,
            rescan_requested: false,
            inactive_since: None,
            ids: IdSource::new(process_start, std::process::id()),
            notified: None,
        }
    }
}

struct Loaded {
    generation: u64,
    folder: PathBuf,
    result: Result<LibraryState, String>,
}

fn host<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryHost) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.map(|mut app| f(&mut unsafe { app.as_mut() }.library))
}

fn notes_mode(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode)
}

pub(crate) fn folder(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| host.folder.clone()).flatten()
}

pub(crate) fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryState) -> R) -> Option<R> {
    host(hwnd, |host| host.state.as_mut().map(f)).flatten()
}

fn data_dir(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| {
        if host.data_dir.is_none() && !cfg!(test) {
            host.data_dir = crate::platform::fastpad_data_dir().ok();
        }
        host.data_dir.clone()
    })
    .flatten()
}

/// The folder to open at startup: a directory named on the command line, else the most recent
/// folder that still exists, else `Documents\FastPad`.
fn startup_folder(hwnd: HWND, data: &Path) -> Option<PathBuf> {
    let launch_dir = unsafe { app_ptr(hwnd) }.and_then(|app| match &unsafe { app.as_ref() }.launch.request {
        crate::launch::LaunchRequest::Open(path) => {
            let path = std::path::absolute(path).ok()?;
            path.is_dir().then_some(path)
        }
        crate::launch::LaunchRequest::New => None,
    });
    if let Some(path) = launch_dir {
        remember_folder(data, &path);
        return Some(path);
    }
    let recent = library::local::read_folders(&library::local::folders_file(data));
    if let Some(first) = recent.folders.first() {
        if first.is_dir() {
            return Some(first.clone());
        }
        push_notice(hwnd, format!(
            "FastPad could not find the folder {}. Using Documents\\FastPad instead.",
            first.display()
        ));
    }
    crate::platform::paths::default_notes_folder().ok()
}

pub(crate) fn remember_folder(data: &Path, folder: &Path) {
    let path = library::local::folders_file(data);
    let mut recent = library::local::read_folders(&path);
    recent.push(folder.to_path_buf());
    let _ = library::local::write_folders(&path, &recent);
}

/// `WM_FASTPAD_OPEN_LIBRARY`: picks the folder and starts the worker. Reads only `folders.ini`.
pub(crate) fn open_library_step(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some(folder) = startup_folder(hwnd, &data) else {
        return;
    };
    host(hwnd, |host| host.folder = Some(folder));
    start_load(hwnd);
}

pub(crate) fn start_load(hwnd: HWND) {
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some((folder, generation)) = host(hwnd, |host| {
        let folder = host.folder.clone()?;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = true;
        host.rescan_requested = false;
        Some((folder, host.generation))
    })
    .flatten() else {
        return;
    };
    let local_path = library::local::local_file(&data, &folder);
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let result = library::load(&folder, &local_path, library::now_unix())
            .map_err(|error| error.to_string());
        let payload = Box::into_raw(Box::new(Loaded { generation, folder, result }));
        if unsafe { PostMessageW(target as HWND, crate::window::WM_FASTPAD_LIBRARY_READY, 0, payload as isize) } == 0 {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

#[cfg(test)]
pub(crate) fn test_ready_payload(generation: u64, result: Result<LibraryState, String>) -> LPARAM {
    Box::into_raw(Box::new(Loaded { generation, folder: PathBuf::new(), result })) as LPARAM
}

/// `WM_FASTPAD_LIBRARY_READY`: installs the worker's result unless a newer load superseded it.
pub(crate) fn library_ready(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let loaded = unsafe { Box::from_raw(lparam as *mut Loaded) };
    let current = host(hwnd, |host| {
        if host.generation != loaded.generation {
            return false;
        }
        host.scanning = false;
        true
    })
    .unwrap_or(false);
    if !current {
        return;
    }
    let Loaded { folder, result, .. } = *loaded;
    match result {
        Ok(fresh) => install(hwnd, fresh),
        Err(error) => push_notice(hwnd, format!(
            "FastPad could not load the folder {}: {error}", folder.display()
        )),
    }
    if host(hwnd, |host| std::mem::take(&mut host.rescan_requested)).unwrap_or(false) {
        start_load(hwnd);
    }
}

fn install(hwnd: HWND, fresh: LibraryState) {
    let Some((relocated, first_time, truncated, unreadable, has_pending)) = host(hwnd, |host| {
        let state = match host.state.take() {
            Some(previous) if library::model::same_path(&previous.folder, &fresh.folder) => {
                library::merge_rescan(previous, fresh)
            }
            _ => fresh,
        };
        let first_time = !host.notified.as_ref().is_some_and(|f| library::model::same_path(f, &state.folder));
        host.notified = Some(state.folder.clone());
        let summary = (
            std::mem::take(&mut host.state.insert(state).relocated),
            first_time,
            host.state.as_ref().is_some_and(|s| s.truncated),
            host.state.as_ref().is_some_and(|s| s.metadata == Metadata::Unreadable),
            host.state.as_ref().is_some_and(|s| !s.pending.is_empty()),
        );
        summary
    }) else {
        return;
    };
    let folder = folder(hwnd).unwrap_or_default();
    for (old, new) in relocated {
        rebind_open_tab(hwnd, &folder.join(old), folder.join(new));
    }
    if first_time && truncated {
        push_notice(hwnd, "This folder has more than 10,000 notes. FastPad indexed the first 10,000.".to_owned());
    }
    if first_time && unreadable {
        push_notice(hwnd, "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so notebooks and tags are read-only.".to_owned());
    }
    if has_pending {
        schedule_write(hwnd);
    }
    with_state(hwnd, |state| library::write_local(state));
}

/// A note moved outside FastPad: an open tab for it follows the file.
fn rebind_open_tab(hwnd: HWND, old: &Path, new: PathBuf) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        match app.tabs.find_path(old) {
            Some(id) => app.tabs.rebind_path(id, new).is_ok(),
            None => false,
        }
    });
    if changed {
        super::main_window::invalidate_title_strip(hwnd);
    }
}

pub(crate) fn request_rescan(hwnd: HWND) {
    let scanning = host(hwnd, |host| {
        if host.scanning {
            host.rescan_requested = true;
        }
        host.scanning
    })
    .unwrap_or(true);
    if !scanning && folder(hwnd).is_some() {
        start_load(hwnd);
    }
}

/// `WM_ACTIVATEAPP`. Coming back after at least `RESCAN_AFTER` rescans, to catch Explorer edits.
pub(crate) fn activation_changed(hwnd: HWND, active: bool) {
    if !active {
        host(hwnd, |host| host.inactive_since = Some(Instant::now()));
        return;
    }
    let long_enough = host(hwnd, |host| {
        host.inactive_since.take().is_some_and(|since| since.elapsed() >= RESCAN_AFTER)
    })
    .unwrap_or(false);
    if long_enough && notes_mode(hwnd) {
        request_rescan(hwnd);
    }
}

pub(crate) fn schedule_write(hwnd: HWND) {
    unsafe {
        SetTimer(hwnd, LIBRARY_WRITE_TIMER_ID, WRITE_DELAY_MS, None);
    }
}

/// Writes pending metadata now (timer, folder switch, close).
pub(crate) fn flush_now(hwnd: HWND) {
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    let Some(result) = with_state(hwnd, |state| {
        let result = library::flush(state);
        library::write_local(state);
        result
    }) else {
        return;
    };
    if let Err(error) = result {
        push_notice(hwnd, format!("FastPad could not save this folder's notebooks and tags: {error}"));
    }
}

/// Notes mode was toggled: load the last folder, or flush and forget the library.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    if enabled {
        if folder(hwnd).is_none() {
            open_library_step(hwnd);
        }
        return;
    }
    flush_now(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
}

#[cfg(test)]
pub(crate) fn install_for_test(hwnd: HWND, state: LibraryState) {
    host(hwnd, |host| {
        host.folder = Some(state.folder.clone());
        host.generation = host.generation.wrapping_add(1);
    });
    install(hwnd, state);
}
```

Keep the existing `notes_mode_notice`, `picked` and `take_last_pick`, and import `HWND` once. `window_identity` is imported for later tasks; if clippy warns that it is unused in this task, drop the import and re-add it in Task 15.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- messages library_step stale_ready session --test-threads=1`
Expected: all pass, including the existing session-restore tests.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/window
git commit -m "feat(window): load the note library after first input and keep it in sync"
```

---

### Task 15: Opening folders: command, recents, launch argument, IPC, drop

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/commands.rs`, `src/window/command_palette.rs`, `src/window/menus.rs`, `src/window/main_window.rs`, `src/ipc/protocol.rs`, `src/ipc/client.rs`

**Interfaces:**
- Produces:
  - `CommandId::OpenFolder = 158` and `CommandId::OpenRecentFolder = 159`, with `Ctrl+Shift+O` bound to `OpenFolder`.
  - `IpcRequest::OpenFolder(PathBuf)`, with command byte 4.
  - `library_host::open_folder(hwnd: HWND, path: &Path)`, `library_host::choose_and_open_folder(hwnd)` and `library_host::open_recent_folder_picker(hwnd)`.
  - A `PickerKind::RecentFolder` arm in `picked`.
  - `WM_DROPFILES` handling. `DragAcceptFiles` is enabled in `build_chrome`.

- [ ] **Step 1: Write the failing tests.** In the `ipc/protocol.rs` tests:

```rust
#[test]
fn open_folder_frames_carry_the_path_like_open() {
    let request = IpcRequest::OpenFolder(PathBuf::from(r"D:\Notes"));
    let frame = encode_frame(&request).unwrap();
    assert_eq!(frame[FRAME_MAGIC.len()], 4);
    assert_eq!(decode_frame(&frame).unwrap(), request);
    assert!(encode_frame(&IpcRequest::OpenFolder(PathBuf::new())).is_err());
}
```

In the `ipc/client.rs` tests (create the module if there is none):

```rust
#[test]
fn a_directory_argument_is_forwarded_as_open_folder() {
    // Break caught: `fastpad D:\Notes` from a second launch trying to open the folder as a file.
    let dir = std::env::temp_dir();
    let request = ipc_request_for(&crate::LaunchRequest::Open(dir.clone().into_os_string())).unwrap();
    assert_eq!(request, crate::ipc::IpcRequest::OpenFolder(std::path::absolute(&dir).unwrap()));
}
```

In the `main_window.rs` tests:

```rust
#[test]
fn opening_another_folder_flushes_the_old_one_remembers_the_new_one_and_keeps_tabs() {
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("switch-a");
    let second = LibraryScratch::new("switch-b");
    let a = first.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    first.install(window.hwnd);
    super::open_path(window.hwnd, &a).unwrap();
    crate::window::library_host::with_state(window.hwnd, |state| {
        let mut ids = crate::library::ids::IdSource::new(1, 1);
        let target = state.note_ref(&mut ids, &a);
        state.apply(crate::library::ops::PendingOp::SetPinned { note: target, value: true }).unwrap();
    });

    crate::window::answer_next_folder_dialog({
        let folder = second.folder();
        move |_| Some(folder)
    });
    execute_command(window.hwnd, CommandId::OpenFolder);

    assert!(crate::library::store::library_file(&first.folder()).exists(), "old folder flushed");
    assert_eq!(crate::window::library_host::folder(window.hwnd), Some(second.folder()));
    let recent = crate::library::local::read_folders(&crate::library::local::folders_file(&first.data()));
    assert_eq!(recent.folders.first(), Some(&second.folder()));
    assert_eq!(super::tab_count(window.hwnd), 1, "open tabs stay open");
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
}

#[test]
fn opening_a_path_that_is_not_a_folder_explains_why() {
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    crate::window::library_host::open_folder(window.hwnd, std::path::Path::new(r"Z:\no\such\folder"));
    assert!(notices(window.hwnd).iter().any(|n| n.contains("is not a folder")));
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- open_folder directory_argument another_folder not_a_folder --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

`ipc/protocol.rs`:
- Add `const COMMAND_OPEN_FOLDER: u8 = 4;` and the variant `OpenFolder(PathBuf)`.
- Encode it as `IpcRequest::OpenFolder(path) => (COMMAND_OPEN_FOLDER, encode_path(path)?)`.
- Decode it as `COMMAND_OPEN_FOLDER => decode_path(payload).map(IpcRequest::OpenFolder)`.

`ipc/client.rs` `ipc_request_for`:

```rust
crate::LaunchRequest::Open(path) => {
    let path = std::path::absolute(path)?;
    // A second launch naming a folder opens it as the library of the running window.
    if path.is_dir() {
        Ok(IpcRequest::OpenFolder(path))
    } else {
        Ok(IpcRequest::Open(path))
    }
}
```

In `main_window.rs`:
- **`open_ipc_requests`:** add `crate::ipc::IpcRequest::OpenFolder(path) => crate::window::library_host::open_folder(hwnd, &path),`.
- **`handle_open_request`:** in the `LaunchRequest::Open(path)` arm, before `App::open_path`, add:
  ```rust
  if path.is_dir() {
      // OPEN_LIBRARY already opened it as the folder.
      if !notes_mode_enabled(hwnd) {
          push_notice(hwnd, format!("{} is a folder. Turn on notes mode to open folders.", path.display()));
      }
      unsafe {
          let _ = record_milestone(hwnd, Milestone::FileLoaded);
      }
  } else {
  ```
  Close that `else` block around the existing file-open code, and leave the `APPLY_LANGUAGE` post that follows unchanged. Add `fn notes_mode_enabled(hwnd: HWND) -> bool` next to it, reading `app.settings.notes_mode`.
- **`build_chrome`:** call `unsafe { windows_sys::Win32::UI::Shell::DragAcceptFiles(hwnd, 1) };`.
- **A new window procedure arm:**
  ```rust
  WM_DROPFILES => {
      crate::window::library_host::files_dropped(hwnd, wparam as windows_sys::Win32::UI::Shell::HDROP);
      0
  }
  ```

In `commands.rs`, `command_palette.rs` and `menus.rs`:
- Add `OpenFolder = 158` and `OpenRecentFolder = 159`, following the numbering table.
- Palette entries go after "File: Open...": `entry("File: Open folder...", CommandId::OpenFolder)` and `entry("File: Open recent folder...", CommandId::OpenRecentFolder)`.
- `accelerator_specs()` gains `accelerator(FCONTROL | FSHIFT, b'O', CommandId::OpenFolder)`. The array grows to 43; update the `specs.len() == 42` test to 43.
- In the File menu, after `&Open...`: `MenuEntry::command("Open &Folder...\tCtrl+Shift+O", CommandId::OpenFolder)`.
- `execute_command`:
  ```rust
  CommandId::OpenFolder => crate::window::library_host::choose_and_open_folder(hwnd),
  CommandId::OpenRecentFolder => crate::window::library_host::open_recent_folder_picker(hwnd),
  ```

In `library_host.rs`:

```rust
use crate::window::command_palette::Picker;

/// Opens `path` as the library, flushing the current one first. Open tabs stay open.
pub(crate) fn open_folder(hwnd: HWND, path: &Path) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, "Notes mode is off. Turn it on with Notes: Toggle notes mode to open folders.".to_owned());
        return;
    }
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    if !path.is_dir() {
        push_notice(hwnd, format!("{} is not a folder.", path.display()));
        return;
    }
    flush_now(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = Some(path.clone());
    });
    if let Some(data) = data_dir(hwnd) {
        remember_folder(&data, &path);
    }
    start_load(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
}

pub(crate) fn choose_and_open_folder(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let choice = crate::window::modal::choose_folder(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    match choice {
        Ok(Some(path)) => open_folder(hwnd, &path),
        Ok(None) => {}
        Err(error) => push_notice(hwnd, format!("FastPad could not open the folder picker: {error}")),
    }
}

fn recent_folders(hwnd: HWND) -> Vec<PathBuf> {
    data_dir(hwnd)
        .map(|data| library::local::read_folders(&library::local::folders_file(&data)).folders)
        .unwrap_or_default()
}

pub(crate) fn open_recent_folder_picker(hwnd: HWND) {
    let folders = recent_folders(hwnd);
    if folders.is_empty() {
        push_notice(hwnd, "No recent folders yet. Use File: Open folder.".to_owned());
        return;
    }
    super::main_window::open_picker(hwnd, Picker {
        kind: PickerKind::RecentFolder,
        items: folders.iter().map(|f| f.display().to_string()).collect(),
        create: None,
    });
}

/// Dropped folders open as the library (the last one wins); dropped files open as tabs.
pub(crate) fn files_dropped(hwnd: HWND, drop: windows_sys::Win32::UI::Shell::HDROP) {
    use windows_sys::Win32::UI::Shell::{DragFinish, DragQueryFileW};
    let count = unsafe { DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0) };
    let mut paths = Vec::new();
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, std::ptr::null_mut(), 0) } as usize;
        let mut buffer = vec![0_u16; length + 1];
        unsafe { DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32) };
        buffer.truncate(length);
        paths.push(PathBuf::from(<std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(&buffer)));
    }
    unsafe { DragFinish(drop) };
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let mut folder = None;
    for path in paths {
        if !identity.is_live_for(hwnd) {
            return;
        }
        if path.is_dir() {
            folder = Some(path);
        } else if let Err(error) = super::main_window::open_path(hwnd, &path) {
            super::main_window::report_open_failure(hwnd, &path, &error);
        }
    }
    if let Some(folder) = folder {
        open_folder(hwnd, &folder);
    }
}
```

In `picked`, replace the placeholder line `let _ = (hwnd, kind, choice);` with a `match` that later tasks extend:

```rust
match (kind, choice) {
    (PickerKind::RecentFolder, PickerChoice::Item(index)) => {
        if let Some(folder) = recent_folders(hwnd).get(index) {
            open_folder(hwnd, folder);
        }
    }
    _ => {}
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- ipc:: open_folder another_folder not_a_folder menus command_palette commands --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/ipc src/window
git commit -m "feat(window): open a folder from the menu, recents, command line, IPC or drop"
```

---

### Task 16: Untitled tab labels

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/main_window.rs`

**Interfaces:**
- Consumes: `library::title::{untitled_label, LABEL_SCAN_LINES}`, `Editor::line_text`, `Editor::line_count`, `Editor::line_from_position`, and `Document::untitled_label` and `label_watch` (Task 13).
- Produces: `library_host::text_changed(hwnd: HWND, position: usize)`, `library_host::refresh_label(hwnd: HWND)` and `library_host::clear_labels(hwnd: HWND)`.

- [ ] **Step 1: Write the failing test** in the `main_window.rs` tests:

```rust
#[test]
fn an_untitled_tab_is_labelled_by_its_first_line_as_you_type() {
    // Break caught: every untitled tab reading "Untitled", or the label recomputed on every
    // keystroke far below the first line.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    super::create_new_document(window.hwnd).unwrap();
    editor.set_text("\n## Meeting notes\nbody").unwrap();
    pump_posted_messages(window.hwnd);
    let title = || app_mut(window.hwnd).tabs.active().unwrap().title();
    assert_eq!(title(), "Meeting notes *");
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().label_watch, 1);

    app_mut(window.hwnd).settings.notes_mode = false;
    crate::window::library_host::clear_labels(window.hwnd);
    assert_eq!(title(), "Untitled *");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib labelled_by_its_first_line -- --test-threads=1`
Expected: a compile error (`text_changed`/`clear_labels` missing).

- [ ] **Step 3: Implement it**

In `library_host.rs`:

```rust
use crate::library::title;

/// Recomputes the active untitled tab's label from its first lines.
pub(crate) fn refresh_label(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let Some(editor) = app.editor.as_ref() else {
            return false;
        };
        let Some(active) = app.tabs.active() else {
            return false;
        };
        if active.path.is_some() {
            return false;
        }
        let id = active.id;
        let count = editor.line_count().unwrap_or(0).min(title::LABEL_SCAN_LINES);
        let lines: Vec<String> = (0..count).map(|line| editor.line_text(line).unwrap_or_default()).collect();
        let label = title::untitled_label(lines.iter().map(String::as_str));
        let Some(document) = app.tabs.document_mut(id) else {
            return false;
        };
        document.label_watch = label.watch_through;
        if document.untitled_label == label.text {
            return false;
        }
        document.untitled_label = label.text;
        true
    });
    if changed {
        super::main_window::refresh_tab_view(hwnd);
    }
}

/// `SCN_MODIFIED`: only edits at or above the label's line can change it.
pub(crate) fn text_changed(hwnd: HWND, position: usize) {
    let relevant = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        let (Some(editor), Some(active)) = (app.editor.as_ref(), app.tabs.active()) else {
            return false;
        };
        active.path.is_none()
            && editor.line_from_position(position).is_ok_and(|line| line <= active.label_watch)
    });
    if relevant {
        refresh_label(hwnd);
    }
}

/// Notes mode turned off: untitled tabs go back to "Untitled".
pub(crate) fn clear_labels(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let ids: Vec<_> = app.tabs.ids().collect();
        for id in ids {
            if let Some(document) = app.tabs.document_mut(id) {
                document.untitled_label = None;
            }
        }
    }
    super::main_window::refresh_tab_view(hwnd);
}
```

In `main_window.rs`:
- Add `pub(crate) fn refresh_tab_view(hwnd: HWND)`, which updates the tab view snapshot the same way `set_active_dirty` does (`app.tabs.view().update(...)`, or whichever call `Tabs` uses) and then calls `invalidate_title_strip(hwnd)`. If `Tabs` has no public way to refresh its view, add `pub(crate) fn refresh_view(&self)` to `Tabs`, doing what `set_active_dirty` does after it changes a document.
- **`handle_editor_notification`:** in the `text_change` branch, after the `app` borrow ends and before `record_edit`, call `crate::window::library_host::text_changed(hwnd, modification.position.max(0) as usize);`.
- **`refresh_tabs`:** at the end, call `crate::window::library_host::refresh_label(hwnd);`. This covers tabs restored from the session and switching to another untitled tab.
- **The `ToggleNotesMode` arm:** when the new state is off, call `crate::window::library_host::clear_labels(hwnd)`. When it is on, call `refresh_label(hwnd)`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib -- labelled_by_its_first_line untitled --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window
git commit -m "feat(window): label untitled tabs by their first line"
```

---

### Task 17: The inline name box and first save

**Files:**
- Create: `src/window/name_box.rs`
- Modify: `src/window/mod.rs` (`pub(crate) mod name_box;`), `src/app.rs` (`pub(crate) name_box: Option<crate::window::name_box::NameBox>`, initialized to `None`), `src/window/main_window.rs`, `src/window/library_host.rs`

**Interfaces:**
- Produces:
  - `pub(crate) enum NamePurpose { FirstSave(DocumentId), RenameNote(DocumentId), NewNotebook { then_move: Option<DocumentId> }, RenameNotebook(NotebookId), RenameTag(TagId) }` (`Clone, Debug, Eq, PartialEq`)
  - `NameBox::create(parent: HWND) -> Result<NameBox>`, plus `show(&mut self, purpose: NamePurpose, text: &str, suffix: String, browse: bool, colors: Palette)`, `hide`, `is_visible`, `owns(hwnd)`, `text() -> String`, `purpose() -> Option<&NamePurpose>`, `set_error(Option<String>)`, `layout(width, top, dpi, font)`, `focus`, `control_color(dc) -> HBRUSH` and `paint_panel(panel, font)`
  - `pub(crate) const fn name_box_height(dpi: u32) -> i32`
  - Control IDs `NAME_BOX_SAVE_ID: u16 = 1` and `NAME_BOX_BROWSE_ID: u16 = 2`
  - `library_host` functions:
    - `open_name_box(hwnd, purpose, text, suffix, browse)`, `close_name_box(hwnd)`
    - `name_box_submit(hwnd)`, `name_box_browse(hwnd)`
    - `save_command(hwnd)`, `save_as_command(hwnd)`
    - `suggested_file_name(hwnd) -> String`

- [ ] **Step 1: Write the failing tests** in the `main_window.rs` tests:

```rust
fn type_into_name_box(hwnd: HWND, text: &str) {
    let edit = app_mut(hwnd).name_box.as_ref().unwrap().edit_hwnd();
    let wide = crate::platform::wide_null(text);
    unsafe { SetWindowTextW(edit, wide.as_ptr()) };
}

#[test]
fn the_first_save_of_an_untitled_note_asks_for_a_name_in_the_folder_prefilled_from_its_label() {
    // Break caught: Ctrl+S on a new note opening the system dialog in some random folder, or
    // saving without letting the user confirm the name.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("first-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Meeting: notes?\nbody").unwrap();
    pump_posted_messages(window.hwnd);

    execute_command(window.hwnd, CommandId::Save);
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(name_box.text(), "Meeting notes.md");

    crate::window::library_host::name_box_submit(window.hwnd);
    let saved = scratch.folder().join("Meeting notes.md");
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "Meeting: notes?\nbody");
    assert!(!app_mut(window.hwnd).name_box.as_ref().unwrap().is_visible());
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(), Some(saved.as_path()));
    assert!(app_mut(window.hwnd).library.state.as_ref().unwrap().notes.iter().any(|n| n.path == std::path::Path::new("Meeting notes.md")));
    assert!(app_mut(window.hwnd).tabs.active().unwrap().disk_stamp.is_some());
}

#[test]
fn a_name_that_already_exists_is_refused_with_a_suggestion() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("clash");
    scratch.note("Plan.md", "old");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Plan").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    type_into_name_box(window.hwnd, "plan.MD");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert_eq!(std::fs::read_to_string(scratch.folder().join("Plan.md")).unwrap(), "old");
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(name_box.error(), Some("plan.MD already exists. Try plan 2.MD."));
}

#[test]
fn browse_and_the_close_prompt_use_the_save_dialog_starting_with_the_suggested_name() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("browse");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Ideas").unwrap();
    assert_eq!(crate::window::library_host::suggested_file_name(window.hwnd), "Ideas.md");
    let elsewhere = scratch.root.join("elsewhere.md");
    crate::window::answer_next_save_dialog({
        let elsewhere = elsewhere.clone();
        move |_| Some(elsewhere)
    });
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_browse(window.hwnd);
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "Ideas");
}

#[test]
fn with_notes_mode_off_save_uses_the_dialog_as_before() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("mode-off-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).settings.notes_mode = false;
    super::create_new_document(window.hwnd).unwrap();
    editor.set_text("x").unwrap();
    let target = scratch.root.join("x.txt");
    crate::window::answer_next_save_dialog({
        let target = target.clone();
        move |_| Some(target)
    });
    execute_command(window.hwnd, CommandId::Save);
    assert!(target.exists());
    assert!(app_mut(window.hwnd).name_box.is_none());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- first_save already_exists browse_and with_notes_mode_off_save --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

`src/window/name_box.rs`. Build it the way `find_bar.rs` builds `FindBar`: the same panel creation, edit creation, field subclass hook, colors, `layout` and painting. The differences are listed here.

```rust
//! The inline name box: one text field with Save and Browse… buttons, shown above the editor for
//! a first save and for naming notes, notebooks and tags. Enter saves, Esc cancels, Tab moves
//! between the field and the buttons.

use crate::document::DocumentId;
use crate::library::ids::{NotebookId, TagId};
use crate::window::palette::Palette;
use crate::window::panel::{create_child_with_id, create_panel, scale};
use windows_sys::Win32::Foundation::HWND;

pub(crate) const NAME_BOX_SAVE_ID: u16 = 1;
pub(crate) const NAME_BOX_BROWSE_ID: u16 = 2;
const NAME_BOX_HOOK_ID: usize = 0x4650_4E42;

pub(crate) const fn name_box_height(dpi: u32) -> i32 {
    scale(36, dpi)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NamePurpose {
    FirstSave(DocumentId),
    RenameNote(DocumentId),
    NewNotebook { then_move: Option<DocumentId> },
    RenameNotebook(NotebookId),
    RenameTag(TagId),
}

pub(crate) struct NameBox {
    panel: HWND,
    edit: HWND,
    save: HWND,
    browse: HWND,
    purpose: Option<NamePurpose>,
    /// Painted after the field, e.g. "in Notes". Replaced by `error` while one is set.
    suffix: String,
    error: Option<String>,
    show_browse: bool,
    visible: bool,
    colors: Palette,
    field_brush: windows_sys::Win32::Graphics::Gdi::HBRUSH,
}
```

Implementation notes:
- **`create(parent)`:** `create_panel(parent)`, then an `Edit` child from `create_child_with_id(panel, "Edit", WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL, 0)`. Add two `BUTTON` children with `BS_PUSHBUTTON | WS_TABSTOP`, labelled "Save" (`NAME_BOX_SAVE_ID`) and "Browse…" (`NAME_BOX_BROWSE_ID`). Then subclass all three with a hook like `find_field_proc`:
  - Swallow `WM_CHAR` 0x0d, 0x1b and 0x09.
  - `WM_KEYDOWN VK_RETURN` calls `crate::window::library_host::name_box_submit(parent)` from the edit or the Save button, and `name_box_browse(parent)` from Browse.
  - `VK_ESCAPE` calls `crate::window::library_host::close_name_box(parent)`.
  - `VK_TAB` moves focus: edit → Save → Browse (if shown) → edit, reversed with Shift.
  - Release the hook on `WM_NCDESTROY`.
- **`show`:** stores the purpose, suffix and `show_browse`, clears the error, sets the edit's text, sets `visible = true`, and shows or hides the Browse button. Like `FindBar::show`, it does not show the panel; `layout` does that.
- **`layout(width, top, dpi, font)`:** the edit fills from the left inset to `width - buttons - suffix width`. The two buttons are `scale(72, dpi)` wide each and sit at the right. The panel goes at `(0, top, width, name_box_height(dpi))` with `SWP_SHOWWINDOW`.
- **`paint_panel`:** paints the background like the find bar. It draws `error` (in the palette's error or accent color, falling back to the normal text color) or `suffix` between the edit and the buttons.
- **`focus`:** `SetFocus(edit)` and select everything *except the extension*: `EM_SETSEL 0, <index of the last '.'>`, or the whole text when there is no dot.
- **Accessors:** `text()` reads the edit. Also add `error() -> Option<&str>`, `purpose()`, `owns(hwnd)` (the panel or any of its three children) and `#[cfg(test)] edit_hwnd()`.
- **Accessibility:** set the edit's accessible name to "Name" with `SetWindowTextW` on a hidden static label placed before it, or through the same mechanism the find bar uses for its query field. Buttons already have text.

In `main_window.rs`, wire it like the find bar:
- **`layout_editor_and_find_bar`:** if the name box is visible, lay it out at `title_height` and reserve `name_box_height(dpi)` the same way the find bar's height is reserved. Opening the name box closes the find bar, so both are never visible at once.
- **`paint_panel`, `WM_CTLCOLOREDIT` and `WM_CTLCOLORBTN`:** branch on `name_box.owns(panel)`, as they do for `find_bar`.
- **`apply_theme`:** calls `set_colors`.
- **`refresh_tabs`:** closes the name box when the tab its purpose names is gone.
- **A new guarded arm before the general `WM_COMMAND` arm:**
  ```rust
  WM_COMMAND if name_box_owns(hwnd, lparam as HWND) => {
      match (wparam & 0xffff) as u16 {
          crate::window::name_box::NAME_BOX_SAVE_ID => crate::window::library_host::name_box_submit(hwnd),
          crate::window::name_box::NAME_BOX_BROWSE_ID => crate::window::library_host::name_box_browse(hwnd),
          _ => {}
      }
      0
  }
  ```
- **`CommandId::Save`** now calls `crate::window::library_host::save_command(hwnd)`, and **`CommandId::SaveAs`** calls `crate::window::library_host::save_as_command(hwnd)`. `save_active_document`/`save_active_document_as` stay as they are for the close-review path.
- **In `save_active_document_as`**, compute the suggested name with `crate::window::library_host::suggested_file_name(hwnd)` when the document is untitled, and pass `crate::window::library_host::folder(hwnd).as_deref()` as the dialog folder. That covers the close prompt's Save.
- **After a successful `complete_save`**, call `crate::window::library_host::document_saved(hwnd)`. It stamps the file and adds it to the index.

In `library_host.rs`:

```rust
use crate::window::name_box::{NameBox, NamePurpose};

fn active_untitled(hwnd: HWND) -> Option<crate::document::DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        active.path.is_none().then_some(active.id)
    })
}

/// "<sanitized label>.<extension for the tab's language>".
pub(crate) fn suggested_file_name(hwnd: HWND) -> String {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let active = unsafe { app.as_ref() }.tabs.active()?;
            let stem = title::sanitize_stem(active.untitled_label.as_deref().unwrap_or("Untitled"));
            Some(format!("{stem}.{}", title::default_extension(active.language)))
        })
        .unwrap_or_else(|| "Untitled.md".to_owned())
}

/// Ctrl+S: an untitled tab in notes mode is named in the name box; everything else as before.
pub(crate) fn save_command(hwnd: HWND) {
    if notes_mode(hwnd) && folder(hwnd).is_some() && let Some(id) = active_untitled(hwnd) {
        refresh_label(hwnd);
        let suffix = format!("in {}", folder_display_name(hwnd));
        open_name_box(hwnd, NamePurpose::FirstSave(id), &suggested_file_name(hwnd), suffix, true);
        return;
    }
    let _ = super::main_window::save_active_document(hwnd);
}

pub(crate) fn save_as_command(hwnd: HWND) {
    if notes_mode(hwnd) && folder(hwnd).is_some() && active_untitled(hwnd).is_some() {
        save_command(hwnd);
        return;
    }
    let _ = super::main_window::save_active_document_as(hwnd);
}

fn folder_display_name(hwnd: HWND) -> String {
    folder(hwnd)
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "the notes folder".to_owned())
}

pub(crate) fn open_name_box(hwnd: HWND, purpose: NamePurpose, text: &str, suffix: String, browse: bool) {
    super::main_window::close_find_bar(hwnd);
    let colors = super::main_window::current_palette(hwnd);
    let shown = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.name_box.is_none() {
            app.name_box = NameBox::create(hwnd).ok();
        }
        match app.name_box.as_mut() {
            Some(name_box) => {
                name_box.show(purpose, text, suffix, browse, colors);
                true
            }
            None => false,
        }
    });
    if !shown {
        push_notice(hwnd, "FastPad could not show the name box.".to_owned());
        return;
    }
    super::main_window::layout_editor_and_find_bar(hwnd);
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_ref() }.name_box.as_ref()
    {
        name_box.focus();
    }
}

pub(crate) fn close_name_box(hwnd: HWND) {
    let hidden = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }.name_box.as_mut().is_some_and(|name_box| {
            let was = name_box.is_visible();
            name_box.hide();
            was
        })
    });
    if hidden {
        super::main_window::layout_editor_and_find_bar(hwnd);
        super::main_window::focus_content(hwnd);
    }
}

fn name_box_state(hwnd: HWND) -> Option<(NamePurpose, String)> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        Some((name_box.purpose()?.clone(), name_box.text()))
    })
}

fn name_box_error(hwnd: HWND, error: String) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_mut() }.name_box.as_mut()
    {
        name_box.set_error(Some(error));
    }
}

/// Enter or Save in the name box.
pub(crate) fn name_box_submit(hwnd: HWND) {
    let Some((purpose, text)) = name_box_state(hwnd) else {
        return;
    };
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        // Tasks 19 and 20 add the other purposes.
        _ => close_name_box(hwnd),
    }
}

fn submit_first_save(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(folder) = folder(hwnd) else {
        return;
    };
    let language = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.document(id)?.language))
        .unwrap_or(crate::document::Language::Markdown);
    let (stem, extension) = title::split_typed_name(text, title::default_extension(language));
    let name = format!("{stem}.{extension}");
    let target = folder.join(&name);
    if target.exists() {
        let free = title::free_name(&stem, &extension, |candidate| folder.join(candidate).exists());
        name_box_error(hwnd, format!("{name} already exists. Try {free}."));
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&folder) {
        name_box_error(hwnd, format!("FastPad could not create {}: {error}", folder.display()));
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if !super::main_window::activate_document_by_id(hwnd, id) {
        return;
    }
    if super::main_window::complete_save(hwnd, &identity, Some(target)) && identity.is_live_for(hwnd) {
        close_name_box(hwnd);
    }
}

/// Browse… in the name box: the system Save As dialog, starting in the folder.
pub(crate) fn name_box_browse(hwnd: HWND) {
    let Some((NamePurpose::FirstSave(id), _)) = name_box_state(hwnd) else {
        return;
    };
    close_name_box(hwnd);
    if super::main_window::activate_document_by_id(hwnd, id) {
        let _ = super::main_window::save_active_document_as(hwnd);
    }
}

/// After any successful save: remember the file's disk stamp and index it if it is a note.
pub(crate) fn document_saved(hwnd: HWND) {
    let path = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.tabs.active()?.id;
        let document = app.tabs.document_mut(id)?;
        let path = document.path.clone()?;
        document.disk_stamp = library::disk_stamp(&path);
        document.autosave_paused = false;
        Some(path)
    });
    if let Some(path) = path {
        with_state(hwnd, |state| state.add_note(&path));
    }
}
```

If `main_window.rs` has no `current_palette(hwnd) -> Palette` or `focus_content(hwnd)` helper, add them. `current_palette` returns what `open_find_bar` passes to `bar.show` as colors. `focus_content` does what `close_find_bar` does to restore focus (`SetFocus(content_focus_target)`). Refactor `close_find_bar` to call `focus_content`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- first_save already_exists browse_and with_notes_mode_off_save save_ find_bar --test-threads=1`
Expected: all pass, including every existing save and find-bar test.

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/window
git commit -m "feat(window): name a new note inline on its first save"
```

---

### Task 18: Autosave and the disk-change guard

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/main_window.rs`, `src/window/commands.rs`, `src/window/command_palette.rs`

**Interfaces:**
- Produces:
  - `CommandId::ToggleFolderAutosave = 160`, `NoteReloadFromDisk = 161` and `NoteKeepMine = 162`
  - `library_host::AUTOSAVE_TIMER_ID: usize = 0x4650_4153` and `AUTOSAVE_DELAY_MS: u32 = 1000`
  - `pub(crate) enum Autosave { NotEligible, Saved, Paused, Failed }`
  - `library_host` functions: `autosave_active(hwnd) -> Autosave`, `autosave_all(hwnd)`, `schedule_autosave(hwnd)`, `toggle_folder_autosave(hwnd)`, `reload_from_disk(hwnd)`, `keep_mine(hwnd)`, `document_loaded(hwnd)`

- [ ] **Step 1: Write the failing tests** in the `main_window.rs` tests:

```rust
fn open_note(window: &ProductionWindow, scratch: &LibraryScratch, name: &str, text: &str) -> std::path::PathBuf {
    let path = scratch.note(name, text);
    scratch.install(window.hwnd);
    super::open_path(window.hwnd, &path).unwrap();
    pump_posted_messages(window.hwnd);
    path
}

#[test]
fn a_note_inside_the_folder_autosaves_and_closing_it_never_prompts() {
    // Break caught: a notes-folder file still asking "Save changes?" or losing edits on close.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("autosave");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.set_text("two").unwrap();
    assert_eq!(crate::window::library_host::autosave_active(window.hwnd), crate::window::library_host::Autosave::Saved);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);

    editor.set_text("three").unwrap();
    super::close_active_document(window.hwnd); // no answer_next_close_prompt: a prompt would fail the test
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "three");
}

#[test]
fn a_file_changed_on_disk_pauses_autosave_until_the_user_chooses() {
    // Break caught: autosave silently overwriting an edit OneDrive just synced from another PC.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("guard");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one");
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "from the other PC").unwrap();
    editor.set_text("mine").unwrap();
    assert_eq!(crate::window::library_host::autosave_active(window.hwnd), crate::window::library_host::Autosave::Paused);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "from the other PC");
    assert!(notices(window.hwnd).iter().any(|n| n.contains("changed on disk")));

    execute_command(window.hwnd, CommandId::NoteReloadFromDisk);
    assert_eq!(editor.text().unwrap(), "from the other PC");
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().autosave_paused);

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "again").unwrap();
    editor.set_text("mine for real").unwrap();
    crate::window::library_host::autosave_active(window.hwnd);
    execute_command(window.hwnd, CommandId::NoteKeepMine);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine for real");
}

#[test]
fn files_outside_the_folder_and_folders_with_autosave_off_are_not_autosaved() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("not-eligible");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let outside = scratch.root.join("outside.md");
    std::fs::write(&outside, "x").unwrap();
    super::open_path(window.hwnd, &outside).unwrap();
    editor.set_text("y").unwrap();
    assert_eq!(crate::window::library_host::autosave_active(window.hwnd), crate::window::library_host::Autosave::NotEligible);
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "x");

    let inside = open_note(&window, &scratch, "b.md", "b");
    execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
    editor.set_text("c").unwrap();
    assert_eq!(crate::window::library_host::autosave_active(window.hwnd), crate::window::library_host::Autosave::NotEligible);
    assert_eq!(std::fs::read_to_string(&inside).unwrap(), "b");
    assert!(!app_mut(window.hwnd).library.state.as_ref().unwrap().local.autosave);
}

#[test]
fn switching_tabs_autosaves_the_tab_being_left() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("switch-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let a = open_note(&window, &scratch, "a.md", "a");
    let b = scratch.note("b.md", "b");
    super::open_path(window.hwnd, &b).unwrap();
    editor.set_text("b2").unwrap();
    execute_command(window.hwnd, CommandId::SelectTab1);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b2");
    editor.set_text("a2").unwrap();
    super::create_new_document(window.hwnd).unwrap();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a2", "a new tab also leaves the old one");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- autosave changed_on_disk not_autosaved switching_tabs --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

In `library_host.rs`:

```rust
pub(crate) const AUTOSAVE_TIMER_ID: usize = 0x4650_4153;
pub(crate) const AUTOSAVE_DELAY_MS: u32 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Autosave {
    NotEligible,
    Saved,
    Paused,
    Failed,
}

fn folder_autosave(hwnd: HWND) -> bool {
    host(hwnd, |host| host.state.as_ref().is_none_or(|state| state.local.autosave)).unwrap_or(false)
}

/// The active tab's path, if autosave applies to it right now.
fn autosave_target(hwnd: HWND) -> Option<PathBuf> {
    if !notes_mode(hwnd) || !folder_autosave(hwnd) || super::main_window::file_population_active(hwnd) {
        return None;
    }
    let folder = folder(hwnd)?;
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        let path = active.path.clone()?;
        (active.dirty && !active.autosave_paused && library::is_inside(&folder, &path)).then_some(path)
    })
}

pub(crate) fn schedule_autosave(hwnd: HWND) {
    if autosave_target(hwnd).is_some() {
        unsafe {
            SetTimer(hwnd, AUTOSAVE_TIMER_ID, AUTOSAVE_DELAY_MS, None);
        }
    }
}

pub(crate) fn autosave_active(hwnd: HWND) -> Autosave {
    unsafe {
        KillTimer(hwnd, AUTOSAVE_TIMER_ID);
    }
    let Some(path) = autosave_target(hwnd) else {
        return Autosave::NotEligible;
    };
    let (known, now) = (
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.active()?.disk_stamp),
        library::disk_stamp(&path),
    );
    if known.is_some() && known != now {
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            if let Some(id) = app.tabs.active().map(|d| d.id)
                && let Some(document) = app.tabs.document_mut(id)
            {
                document.autosave_paused = true;
            }
        }
        push_notice(hwnd, format!(
            "{} changed on disk. Autosave is paused for it: use Note: Reload from disk or Note: Keep my version.",
            title::note_title(&path)
        ));
        return Autosave::Paused;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return Autosave::Failed;
    };
    if super::main_window::complete_save(hwnd, &identity, None) {
        Autosave::Saved
    } else {
        if identity.is_live_for(hwnd) {
            push_notice(hwnd, format!(
                "Autosave failed for {}. Your text is kept in recovery and FastPad will try again.",
                title::note_title(&path)
            ));
        }
        Autosave::Failed
    }
}

/// Before the window closes: save every eligible dirty tab. Failures fall back to the prompt.
pub(crate) fn autosave_all(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let ids: Vec<_> = unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.documents().filter(|d| d.dirty).map(|d| d.id).collect())
        .unwrap_or_default();
    for id in ids {
        if !identity.is_live_for(hwnd) || !super::main_window::activate_document_by_id(hwnd, id) {
            return;
        }
        autosave_active(hwnd);
    }
}

pub(crate) fn toggle_folder_autosave(hwnd: HWND) {
    let Some(enabled) = with_state(hwnd, |state| {
        state.local.autosave = !state.local.autosave;
        library::write_local(state);
        state.local.autosave
    }) else {
        push_notice(hwnd, "Loading folder…".to_owned());
        return;
    };
    push_notice(hwnd, if enabled {
        "Autosave is on for this folder.".to_owned()
    } else {
        "Autosave is off for this folder. Use Ctrl+S to save.".to_owned()
    });
}

/// Replaces the tab's text with the file on disk and resumes autosave.
pub(crate) fn reload_from_disk(hwnd: HWND) {
    let Some(path) = unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone()) else {
        return;
    };
    let loaded = match crate::file::loader::load(&path) {
        Ok(loaded) => loaded,
        Err(error) => {
            super::main_window::report_open_failure(hwnd, &path, &error);
            return;
        }
    };
    unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        if let Some(editor) = app.editor.as_ref() {
            let _ = editor.set_text(&loaded.text);
            editor.set_save_point();
        }
        app.tabs.set_active_dirty(false);
    });
    document_saved(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
}

/// Saves the tab over the changed file and resumes autosave.
pub(crate) fn keep_mine(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let _ = super::main_window::complete_save(hwnd, &identity, None);
}

/// After a file is opened into a tab: remember its disk stamp.
pub(crate) fn document_loaded(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|d| d.id)
            && let Some(document) = app.tabs.document_mut(id)
            && let Some(path) = document.path.clone()
        {
            document.disk_stamp = library::disk_stamp(&path);
        }
    }
}
```

`reload_from_disk` needs the loaded encoding stored on the tab if `LoadedFile` carries one. Copy what `open_path` does after `loader::load`: set `document.encoding` and check for NUL bytes (`CString::new`) before calling `set_text`.

In `main_window.rs`:
- **`handle_editor_notification`:** in the `text_change` branch, next to `text_changed`, call `crate::window::library_host::schedule_autosave(hwnd);`.
- **A new arm:**
  ```rust
  WM_TIMER if wparam == crate::window::library_host::AUTOSAVE_TIMER_ID => {
      crate::window::library_host::autosave_active(hwnd);
      0
  }
  ```
- **`WM_DESTROY`:** `KillTimer(hwnd, crate::window::library_host::AUTOSAVE_TIMER_ID)`.
- **`activation_changed(hwnd, false)`** (in `library_host`): also call `autosave_active(hwnd)`.
- **Every path that changes the active tab** calls `crate::window::library_host::autosave_active(hwnd);` before it changes it: `activate_document` (before `app.tabs.activate(id)`), `create_new_document`, and `open_path` (before it loads a new file into a new tab; its early return for an already-open path goes through `activate_document`). `grep -n "tabs.activate\|tabs.push\|replace_active_untitled" src/window/main_window.rs` lists them. After the call, re-check `identity.is_live_for(hwnd)` and any `revision` guard the function already has. During session restore the folder is not open yet (`OPEN_LIBRARY` runs after `RESTORE_SESSION`), so these calls are `NotEligible` there.
- **`close_active_document`:** at its top, `if crate::window::library_host::autosave_active(hwnd) == crate::window::library_host::Autosave::Saved { /* now clean: the existing flow closes without a prompt */ }`. No other change is needed, because a clean tab closes without prompting.
- **`WM_CLOSE`:** call `crate::window::library_host::autosave_all(hwnd);` after `drain_ipc_requests(hwnd)` and before `save_session_for_close`.
- **At the end of a successful `open_path`** (where `APPLY_LANGUAGE` is posted after a new load), call `crate::window::library_host::document_loaded(hwnd);`. Also call it in the session-restore `File` branch after its `open_path`.
- **Remember recent notes:** in `document_loaded`, when the path is inside the folder, also call `with_state(hwnd, |state| state.local.note_opened(&library::record_path(&state.folder, &path), library::now_unix()))`. This needs no write; the local file is written with the next flush.
- **Make `file_population_active` `pub(super)`.**
- **Commands:** `ToggleFolderAutosave` calls `toggle_folder_autosave`, `NoteReloadFromDisk` calls `reload_from_disk`, and `NoteKeepMine` calls `keep_mine`. Add them to the numbering table's palette entries, after "Notes: Toggle notes mode".

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- autosave changed_on_disk not_autosaved switching_tabs close_ session --test-threads=1`
Expected: all pass, including the existing close-prompt and session tests. Those use files outside any open folder, so autosave does not apply to them.

- [ ] **Step 5: Commit**

```bash
git add src/window
git commit -m "feat(window): autosave notes inside the folder with a disk-change guard"
```

---

### Task 19: Organizing commands

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/main_window.rs`, `src/window/commands.rs`, `src/window/command_palette.rs`

**Interfaces:**
- Produces:
  - `CommandId` values 163–173 (numbering table), each routed to `library_host::organize(hwnd, command)`.
  - `picked` arms for the kinds `MoveToNotebook`, `AddTag`, `RemoveTag`, `RenameNotebook`, `RecolorNotebook`, `ChooseColor`, `DeleteNotebook`, `RenameTag` and `RemoveTagEverywhere`.
  - `name_box_submit` arms for `NewNotebook`, `RenameNotebook` and `RenameTag`.
  - Helpers: `library_host::ready_library(hwnd) -> bool` and `library_host::active_file(hwnd) -> Option<PathBuf>`.

`library_host` keeps the notebook or tag that a multi-step picker flow is acting on in a field, `pending_target: Option<Target>`, where `enum Target { Notebook(NotebookId), Tag(TagId) }`. Add the field to `LibraryHost` with `None` in `new`.

- [ ] **Step 1: Write the failing tests** in the `main_window.rs` tests:

```rust
use crate::window::command_palette::{PickerChoice, PickerKind};

fn library(hwnd: HWND) -> &'static crate::library::model::Library {
    &app_mut(hwnd).library.state.as_ref().unwrap().library
}

#[test]
fn favorite_pin_notebook_and_tags_apply_to_the_active_file_and_persist() {
    // Break caught: organizing commands changing only memory, or recording the wrong file.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("organize");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a");
    let hwnd = window.hwnd;

    execute_command(hwnd, CommandId::NoteToggleFavorite);
    execute_command(hwnd, CommandId::NoteTogglePin);
    execute_command(hwnd, CommandId::NotebookNew);
    type_into_name_box(hwnd, "Work");
    crate::window::library_host::name_box_submit(hwnd);
    execute_command(hwnd, CommandId::NoteMoveToNotebook);
    // Row 0 is "Notes", row 1 is "Work".
    crate::window::library_host::picked(hwnd, PickerKind::MoveToNotebook, PickerChoice::Item(1));
    execute_command(hwnd, CommandId::NoteAddTag);
    crate::window::library_host::picked(hwnd, PickerKind::AddTag, PickerChoice::Create("#idea".into()));

    let record = app_mut(hwnd).library.state.as_ref().unwrap().record_for(&path).unwrap().clone();
    assert!(record.favorite && record.pinned);
    assert_eq!(library(hwnd).notebook(record.notebook.unwrap()).unwrap().name, "Work");
    assert_eq!(library(hwnd).tag(record.tags[0]).unwrap().name, "idea");

    crate::window::library_host::flush_now(hwnd);
    let reloaded = crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
    assert!(reloaded.record_for(&path).unwrap().favorite);
}

#[test]
fn duplicate_notebook_names_show_an_inline_error_and_keep_the_box_open() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("dup-notebook");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    open_note(&window, &scratch, "a.md", "a");
    for _ in 0..2 {
        execute_command(window.hwnd, CommandId::NotebookNew);
        type_into_name_box(window.hwnd, "work");
        crate::window::library_host::name_box_submit(window.hwnd);
    }
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(name_box.error(), Some("That name is already used."));
    assert_eq!(library(window.hwnd).notebooks.len(), 1);
}

#[test]
fn deleting_a_notebook_asks_first_and_moves_its_notes_to_notes() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("delete-notebook");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a");
    let hwnd = window.hwnd;
    execute_command(hwnd, CommandId::NotebookNew);
    type_into_name_box(hwnd, "Work");
    crate::window::library_host::name_box_submit(hwnd);
    crate::window::library_host::picked(hwnd, PickerKind::MoveToNotebook, PickerChoice::Item(1));

    crate::window::answer_next_confirm(|_| false);
    crate::window::library_host::picked(hwnd, PickerKind::DeleteNotebook, PickerChoice::Item(0));
    assert_eq!(library(hwnd).notebooks.len(), 1, "cancelled");

    crate::window::answer_next_confirm(|_| true);
    crate::window::library_host::picked(hwnd, PickerKind::DeleteNotebook, PickerChoice::Item(0));
    assert!(library(hwnd).notebooks.is_empty());
    let state = app_mut(hwnd).library.state.as_ref().unwrap();
    assert_eq!(state.record_for(&path).unwrap().notebook, None);
    assert!(path.exists(), "no note content is deleted");
}

#[test]
fn an_untitled_tab_must_be_saved_before_it_can_be_organized() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("untitled-organize");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::create_new_document(window.hwnd).unwrap();
    execute_command(window.hwnd, CommandId::NoteToggleFavorite);
    assert!(notices(window.hwnd).iter().any(|n| n == "Save this note first to organize it."));
}

#[test]
fn an_unreadable_library_disables_organizing_with_an_explanation() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("readonly");
    let ini = crate::library::store::library_file(&scratch.folder());
    std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
    std::fs::write(&ini, "version=99\r\n").unwrap();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    open_note(&window, &scratch, "a.md", "a");
    execute_command(window.hwnd, CommandId::NoteToggleFavorite);
    crate::window::library_host::flush_now(window.hwnd);
    assert_eq!(std::fs::read_to_string(&ini).unwrap(), "version=99\r\n");
    assert!(notices(window.hwnd).iter().any(|n| n.contains("read-only")));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- organized favorite_pin duplicate_notebook deleting_a_notebook unreadable_library --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

In `library_host.rs`:

```rust
use crate::library::ids::{NotebookId, TagId};
use crate::library::model::NotebookColor;
use crate::library::ops::PendingOp;
use crate::window::commands::CommandId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    Notebook(NotebookId),
    Tag(TagId),
}

const READ_ONLY: &str = "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so notebooks and tags are read-only.";

/// True when organizing can proceed; otherwise explains why not.
pub(crate) fn ready_library(hwnd: HWND) -> bool {
    match host(hwnd, |host| host.state.as_ref().map(|state| state.metadata)).flatten() {
        Some(Metadata::Ready) => true,
        Some(Metadata::Unreadable) => {
            push_notice(hwnd, READ_ONLY.to_owned());
            false
        }
        None => {
            push_notice(hwnd, if folder(hwnd).is_some() { "Loading folder…" } else { "Open a folder first." }.to_owned());
            false
        }
    }
}

pub(crate) fn active_file(hwnd: HWND) -> Option<PathBuf> {
    let path = unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    if path.is_none() {
        push_notice(hwnd, "Save this note first to organize it.".to_owned());
    }
    path
}

/// Applies one operation built from the host's ID source, then schedules the write.
fn apply_op(hwnd: HWND, build: impl FnOnce(&mut LibraryState, &mut IdSource) -> Option<PendingOp>)
    -> Result<(), crate::library::model::LibraryError>
{
    let result = host(hwnd, |host| {
        let LibraryHost { state, ids, .. } = host;
        let state = state.as_mut()?;
        let op = build(state, ids)?;
        Some(state.apply(op))
    })
    .flatten()
    .unwrap_or(Ok(()));
    if result.is_ok() {
        schedule_write(hwnd);
    }
    result
}

fn notebooks(hwnd: HWND) -> Vec<(NotebookId, String)> {
    with_state(hwnd, |state| {
        state.library.notebooks_in_order().iter().map(|n| (n.id, n.name.clone())).collect()
    })
    .unwrap_or_default()
}

fn tags(hwnd: HWND) -> Vec<(TagId, String)> {
    with_state(hwnd, |state| {
        let mut tags: Vec<_> = state.library.tags.iter().map(|t| (t.id, t.name.clone())).collect();
        tags.sort_by(|a, b| {
            state.library.tag_count(b.0).cmp(&state.library.tag_count(a.0)).then_with(|| a.1.cmp(&b.1))
        });
        tags
    })
    .unwrap_or_default()
}

fn note_tags(hwnd: HWND, path: &Path) -> Vec<(TagId, String)> {
    with_state(hwnd, |state| {
        state.record_for(path).map_or_else(Vec::new, |record| {
            record.tags.iter().filter_map(|id| Some((*id, state.library.tag(*id)?.name.clone()))).collect()
        })
    })
    .unwrap_or_default()
}

fn picker(hwnd: HWND, kind: PickerKind, items: Vec<String>, create: Option<&'static str>) {
    super::main_window::open_picker(hwnd, Picker { kind, items, create });
}

/// Every organizing command.
pub(crate) fn organize(hwnd: HWND, command: CommandId) {
    if !ready_library(hwnd) {
        return;
    }
    match command {
        CommandId::NoteToggleFavorite | CommandId::NoteTogglePin => {
            let Some(path) = active_file(hwnd) else { return };
            let favorite = command == CommandId::NoteToggleFavorite;
            let mut now_on = false;
            let _ = apply_op(hwnd, |state, ids| {
                let current = state.record_for(&path).is_some_and(|r| if favorite { r.favorite } else { r.pinned });
                now_on = !current;
                let note = state.note_ref(ids, &path);
                Some(if favorite {
                    PendingOp::SetFavorite { note, value: now_on }
                } else {
                    PendingOp::SetPinned { note, value: now_on }
                })
            });
            push_notice(hwnd, match (favorite, now_on) {
                (true, true) => "Added to Favorites.",
                (true, false) => "Removed from Favorites.",
                (false, true) => "Pinned.",
                (false, false) => "Unpinned.",
            }.to_owned());
        }
        CommandId::NoteMoveToNotebook => {
            if active_file(hwnd).is_none() {
                return;
            }
            let mut items = vec!["Notes".to_owned()];
            items.extend(notebooks(hwnd).into_iter().map(|(_, name)| name));
            picker(hwnd, PickerKind::MoveToNotebook, items, Some("New notebook"));
        }
        CommandId::NoteAddTag => {
            let Some(path) = active_file(hwnd) else { return };
            let on_note: Vec<TagId> = note_tags(hwnd, &path).into_iter().map(|(id, _)| id).collect();
            let items = tags(hwnd).into_iter().filter(|(id, _)| !on_note.contains(id)).map(|(_, n)| n).collect();
            picker(hwnd, PickerKind::AddTag, items, Some("Add tag"));
        }
        CommandId::NoteRemoveTag => {
            let Some(path) = active_file(hwnd) else { return };
            let items: Vec<String> = note_tags(hwnd, &path).into_iter().map(|(_, n)| n).collect();
            if items.is_empty() {
                push_notice(hwnd, "This note has no tags.".to_owned());
                return;
            }
            picker(hwnd, PickerKind::RemoveTag, items, None);
        }
        CommandId::NotebookNew => {
            open_name_box(hwnd, NamePurpose::NewNotebook { then_move: None }, "", "New notebook".to_owned(), false);
        }
        CommandId::NotebookRename | CommandId::NotebookChangeColor | CommandId::NotebookDelete => {
            let items: Vec<String> = notebooks(hwnd).into_iter().map(|(_, n)| n).collect();
            if items.is_empty() {
                push_notice(hwnd, "There are no notebooks yet. Use Notebook: New.".to_owned());
                return;
            }
            let kind = match command {
                CommandId::NotebookRename => PickerKind::RenameNotebook,
                CommandId::NotebookChangeColor => PickerKind::RecolorNotebook,
                _ => PickerKind::DeleteNotebook,
            };
            picker(hwnd, kind, items, None);
        }
        CommandId::TagRename | CommandId::TagRemoveEverywhere => {
            let items: Vec<String> = tags(hwnd).into_iter().map(|(_, n)| n).collect();
            if items.is_empty() {
                push_notice(hwnd, "There are no tags yet. Use Note: Add tag.".to_owned());
                return;
            }
            let kind = if command == CommandId::TagRename { PickerKind::RenameTag } else { PickerKind::RemoveTagEverywhere };
            picker(hwnd, kind, items, None);
        }
        _ => {}
    }
}

fn report(hwnd: HWND, result: Result<(), crate::library::model::LibraryError>) {
    if let Err(error) = result {
        push_notice(hwnd, error.to_string());
    }
}
```

The `picked` match gains these arms, in addition to the `RecentFolder` arm:

```rust
(PickerKind::MoveToNotebook, choice) => {
    let Some(path) = active_file(hwnd) else { return };
    match choice {
        PickerChoice::Item(0) => report(hwnd, apply_op(hwnd, |state, ids| {
            Some(PendingOp::SetNoteNotebook { note: state.note_ref(ids, &path), notebook: None })
        })),
        PickerChoice::Item(index) => {
            let Some((id, _)) = notebooks(hwnd).get(index - 1).cloned() else { return };
            report(hwnd, apply_op(hwnd, |state, ids| {
                Some(PendingOp::SetNoteNotebook { note: state.note_ref(ids, &path), notebook: Some(id) })
            }));
        }
        PickerChoice::Create(name) => {
            let then_move = unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id));
            open_name_box(hwnd, NamePurpose::NewNotebook { then_move }, &name, "New notebook".to_owned(), false);
        }
    }
}
(PickerKind::AddTag, choice) => {
    let Some(path) = active_file(hwnd) else { return };
    let name = match choice {
        PickerChoice::Create(name) => name,
        PickerChoice::Item(index) => {
            let on_note: Vec<TagId> = note_tags(hwnd, &path).into_iter().map(|(id, _)| id).collect();
            let Some((_, name)) = tags(hwnd).into_iter().filter(|(id, _)| !on_note.contains(id)).nth(index) else { return };
            name
        }
    };
    report(hwnd, apply_op(hwnd, |state, ids| {
        let tag = state.library.tag_by_name(name.trim().trim_start_matches('#')).map_or_else(|| TagId(ids.next()), |t| t.id);
        Some(PendingOp::AddTag { note: state.note_ref(ids, &path), tag, name })
    }));
}
(PickerKind::RemoveTag, PickerChoice::Item(index)) => {
    let Some(path) = active_file(hwnd) else { return };
    let Some((tag, _)) = note_tags(hwnd, &path).get(index).cloned() else { return };
    report(hwnd, apply_op(hwnd, |state, ids| Some(PendingOp::RemoveTag { note: state.note_ref(ids, &path), tag })));
}
(PickerKind::RenameNotebook, PickerChoice::Item(index)) => {
    let Some((id, name)) = notebooks(hwnd).get(index).cloned() else { return };
    open_name_box(hwnd, NamePurpose::RenameNotebook(id), &name, "Rename notebook".to_owned(), false);
}
(PickerKind::RecolorNotebook, PickerChoice::Item(index)) => {
    let Some((id, _)) = notebooks(hwnd).get(index).cloned() else { return };
    host(hwnd, |host| host.pending_target = Some(Target::Notebook(id)));
    let mut items = vec!["No color".to_owned()];
    items.extend(NotebookColor::ALL.iter().map(|c| c.name().to_owned()));
    picker(hwnd, PickerKind::ChooseColor, items, None);
}
(PickerKind::ChooseColor, PickerChoice::Item(index)) => {
    let Some(Some(Target::Notebook(id))) = host(hwnd, |host| host.pending_target.take()) else { return };
    let color = index.checked_sub(1).and_then(|i| NotebookColor::ALL.get(i).copied());
    report(hwnd, apply_op(hwnd, |_, _| Some(PendingOp::SetNotebookColor { id, color, now: library::now_unix() })));
}
(PickerKind::DeleteNotebook, PickerChoice::Item(index)) => {
    let Some((id, name)) = notebooks(hwnd).get(index).cloned() else { return };
    let count = with_state(hwnd, |state| state.library.notes.iter().filter(|n| n.notebook == Some(id)).count()).unwrap_or(0);
    let question = format!("Delete the notebook \u{201c}{name}\u{201d}? Its {count} notes move to Notes. No note is deleted.");
    if crate::window::modal::confirm(hwnd, &question) {
        report(hwnd, apply_op(hwnd, |_, _| Some(PendingOp::DeleteNotebook { id })));
    }
}
(PickerKind::RenameTag, PickerChoice::Item(index)) => {
    let Some((id, name)) = tags(hwnd).get(index).cloned() else { return };
    open_name_box(hwnd, NamePurpose::RenameTag(id), &name, "Rename tag".to_owned(), false);
}
(PickerKind::RemoveTagEverywhere, PickerChoice::Item(index)) => {
    let Some((id, name)) = tags(hwnd).get(index).cloned() else { return };
    let count = with_state(hwnd, |state| state.library.tag_count(id)).unwrap_or(0);
    if crate::window::modal::confirm(hwnd, &format!("Remove the tag \u{201c}{name}\u{201d} from {count} notes?")) {
        report(hwnd, apply_op(hwnd, |_, _| Some(PendingOp::RemoveTagEverywhere { id })));
    }
}
```

`name_box_submit` gains these arms:

```rust
NamePurpose::NewNotebook { then_move } => {
    let id = host(hwnd, |host| NotebookId(host.ids.next())).unwrap_or(NotebookId(0));
    match apply_op(hwnd, |_, _| Some(PendingOp::CreateNotebook { id, name: text.clone(), now: library::now_unix() })) {
        Err(error) => name_box_error(hwnd, error.to_string()),
        Ok(()) => {
            close_name_box(hwnd);
            if let Some(document) = then_move
                && super::main_window::activate_document_by_id(hwnd, document)
                && let Some(path) = active_file(hwnd)
            {
                report(hwnd, apply_op(hwnd, |state, ids| {
                    Some(PendingOp::SetNoteNotebook { note: state.note_ref(ids, &path), notebook: Some(id) })
                }));
            }
        }
    }
}
NamePurpose::RenameNotebook(id) => match apply_op(hwnd, |_, _| {
    Some(PendingOp::RenameNotebook { id, name: text.clone(), now: library::now_unix() })
}) {
    Err(error) => name_box_error(hwnd, error.to_string()),
    Ok(()) => close_name_box(hwnd),
},
NamePurpose::RenameTag(id) => match apply_op(hwnd, |_, _| Some(PendingOp::RenameTag { id, name: text.clone() })) {
    Err(error) => name_box_error(hwnd, error.to_string()),
    Ok(()) => close_name_box(hwnd),
},
```

`name_box_submit` must run `ready_library(hwnd)` before these three arms, and close the box when it returns false.

`apply_op` validates before recording, because `LibraryState::apply` returns the model's error without pushing the op. A name error therefore leaves nothing pending.

In `main_window.rs` `execute_command`, send all eleven commands to `crate::window::library_host::organize(hwnd, command)`. Add the eleven palette entries from the numbering table, grouped after the Notes entries.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- organized favorite_pin duplicate_notebook deleting_a_notebook unreadable_library command_palette commands --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window
git commit -m "feat(window): palette commands for favorites, pins, notebooks and tags"
```

---

### Task 20: Rename and delete a note

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/main_window.rs`, `src/window/commands.rs`, `src/window/command_palette.rs`

**Interfaces:**
- Produces:
  - `CommandId::NoteRename = 174` and `CommandId::NoteDelete = 175`
  - `library_host::rename_note(hwnd)`, which opens the name box, and its `NamePurpose::RenameNote` submit arm
  - `library_host::delete_note(hwnd)`
  - `main_window::close_document_without_prompt(hwnd: HWND, id: DocumentId)`

- [ ] **Step 1: Write the failing tests** in the `main_window.rs` tests:

```rust
#[test]
fn renaming_a_note_renames_its_file_and_keeps_its_metadata() {
    // Break caught: a rename losing the note's notebook, or the tab still pointing at the old path.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rename");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let old = open_note(&window, &scratch, "a.md", "text");
    execute_command(window.hwnd, CommandId::NoteToggleFavorite);
    execute_command(window.hwnd, CommandId::NoteRename);
    assert_eq!(app_mut(window.hwnd).name_box.as_ref().unwrap().text(), "a.md");
    type_into_name_box(window.hwnd, "Plan");
    crate::window::library_host::name_box_submit(window.hwnd);
    let new = scratch.folder().join("Plan.md");
    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "text");
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(), Some(new.as_path()));
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    assert!(state.record_for(&new).unwrap().favorite);
}

#[test]
fn renaming_onto_an_existing_file_is_refused() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rename-clash");
    scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let a = open_note(&window, &scratch, "a.md", "a");
    execute_command(window.hwnd, CommandId::NoteRename);
    type_into_name_box(window.hwnd, "B.md");
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(a.exists());
    assert_eq!(std::fs::read_to_string(scratch.folder().join("b.md")).unwrap(), "b");
    assert!(app_mut(window.hwnd).name_box.as_ref().unwrap().error().is_some());
}

#[test]
fn deleting_a_note_asks_then_recycles_it_and_keeps_its_record_hidden() {
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("delete-note");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "a");
    execute_command(window.hwnd, CommandId::NoteToggleFavorite);
    crate::window::answer_next_confirm(|_| false);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert!(path.exists());
    crate::window::answer_next_confirm(|_| true);
    execute_command(window.hwnd, CommandId::NoteDelete);
    assert!(!path.exists());
    assert_eq!(super::tab_count(window.hwnd), 0);
    let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
    let record = state.record_for(&path).unwrap();
    assert!(record.deleted);
    assert!(state.local.missing_since(record.id).is_some());
    assert!(state.notes.is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- renaming_a_note renaming_onto deleting_a_note --test-threads=1`
Expected: a compile error.

- [ ] **Step 3: Implement it**

In `library_host.rs`:

```rust
pub(crate) fn rename_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else { return };
    let Some(id) = unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id)) else { return };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    open_name_box(hwnd, NamePurpose::RenameNote(id), &name, "Rename".to_owned(), false);
}

fn submit_rename(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(old) = unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.document(id)?.path.clone()) else {
        close_name_box(hwnd);
        return;
    };
    let current_extension = old.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "md".into());
    let (stem, extension) = title::split_typed_name(text, &current_extension);
    let new = old.with_file_name(format!("{stem}.{extension}"));
    if new == old {
        close_name_box(hwnd);
        return;
    }
    let case_only = library::model::same_path(&new, &old);
    if new.exists() && !case_only {
        let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
        let free = title::free_name(&stem, &extension, |candidate| parent.join(candidate).exists());
        name_box_error(hwnd, format!("{} already exists. Try {free}.", new.file_name().unwrap_or_default().to_string_lossy()));
        return;
    }
    if let Err(error) = crate::platform::files::rename_no_replace(&old, &new) {
        name_box_error(hwnd, format!("FastPad could not rename the file: {error}"));
        return;
    }
    let rebound = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| unsafe { app.as_mut() }.tabs.rebind_path(id, new.clone()).is_ok());
    if !rebound {
        // Undo, so the tab and the disk agree.
        let _ = crate::platform::files::rename_no_replace(&new, &old);
        name_box_error(hwnd, "Another tab already has that file open.".to_owned());
        return;
    }
    with_state(hwnd, |state| state.rename_note(&old, &new));
    schedule_write(hwnd);
    close_name_box(hwnd);
    document_loaded(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
}

pub(crate) fn delete_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else { return };
    let Some(id) = unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id)) else { return };
    let question = format!(
        "Move \u{201c}{}\u{201d} to the Recycle Bin?",
        path.file_name().unwrap_or_default().to_string_lossy()
    );
    let Some(identity) = (unsafe { window_identity(hwnd) }) else { return };
    if !crate::window::modal::confirm(hwnd, &question) || !identity.is_live_for(hwnd) {
        return;
    }
    if let Err(error) = crate::platform::files::recycle(&path) {
        push_notice(hwnd, format!("FastPad could not delete {}: {error}", path.display()));
        return;
    }
    let now = library::now_unix();
    let has_record = with_state(hwnd, |state| state.record_for(&path).map(|r| r.id)).flatten();
    if let Some(note_id) = has_record {
        let _ = apply_op(hwnd, |state, ids| Some(PendingOp::SetDeleted { note: state.note_ref(ids, &path), value: true }));
        with_state(hwnd, |state| state.local.set_missing(note_id, now));
    }
    with_state(hwnd, |state| state.remove_note(&path));
    super::main_window::close_document_without_prompt(hwnd, id);
}
```

`name_box_submit` gains `NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),`.

In `main_window.rs`, add `pub(super) fn close_document_without_prompt(hwnd: HWND, id: DocumentId)`. It activates `id`, then runs the part of `close_active_document` that follows a `CloseDecision::Discard` answer: close the tab, remove its snapshot, and `refresh_tabs`. Build it by extracting that tail of `close_active_document` into this function and calling it from there, so the two cannot drift apart.

In `execute_command`: `NoteRename` calls `library_host::rename_note(hwnd)`, and `NoteDelete` calls `if library_host::ready_library(hwnd) { library_host::delete_note(hwnd) }`. Add the two palette entries.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- renaming_a_note renaming_onto deleting_a_note close_ command_palette commands --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window
git commit -m "feat(window): rename and delete notes, keeping their metadata"
```

---

## Batch D: Verification and docs

### Task 21: End-to-end tests and the library bench

**Files:**
- Create: `tests/windows/library.rs`
- Modify: `Cargo.toml` (a `[[test]]` entry), `src/bin/fastpad-bench.rs`

**Interfaces:**
- Consumes: the real `fastpad.exe` through `tests/windows/support`, and the `CommandId` values from Tasks 10–20.
- Produces:
  - The integration target `library`.
  - A `fastpad-bench library-scan DIR [--count N]` action.
  - A `--notes-folder DIR` option for the TTI run.

- [ ] **Step 1: Register the target** in `Cargo.toml`, after the `markdown_preview` entry:

```toml
[[test]]
name = "library"
path = "tests/windows/library.rs"
```

- [ ] **Step 2: Write the tests** in `tests/windows/library.rs`

```rust
#![cfg(windows)]
// Requires that no other FastPad runs in this session: the library belongs to the primary window.

mod support;

use fastpad::window::commands::CommandId;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use support::process::{FastPadProcess, wait_and_dismiss_dialog, wait_for_process_exit};
use support::win32::{Deadline, find_child_by_class, focused_window, scintilla_text, send_text};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_ACTIVATEAPP, WM_CLOSE, WM_COMMAND, WM_KEYDOWN,
};

static LIBRARY_TEST_LOCK: Mutex<()> = Mutex::new(());
const WAIT: Duration = Duration::from_secs(5);

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("fastpad-library-e2e-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("FastPad")).unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        Self { root }
    }
    fn folder(&self) -> PathBuf {
        self.root.join("notes")
    }
    fn note(&self, name: &str, text: &str) -> PathBuf {
        let path = self.folder().join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    fn library_ini(&self) -> PathBuf {
        self.folder().join(".fastpad").join("library.ini")
    }
    fn data(&self) -> PathBuf {
        self.root.join("FastPad")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn command(hwnd: HWND, command: CommandId) {
    unsafe {
        PostMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Deadline::after(WAIT);
    while !done() {
        assert!(!deadline.expired(), "timed out waiting for {what}");
        deadline.sleep_step();
    }
}

/// The worker has installed the folder once it writes the per-PC local file.
fn wait_for_library(data: &Scratch) {
    wait_until("the folder to load", || data.data().join("libraries").is_dir());
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn close(mut process: FastPadProcess, hwnd: HWND) {
    unsafe {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    wait_for_process_exit(process.id(), WAIT).unwrap();
    let _ = &mut process;
}

#[test]
fn a_new_note_is_named_inline_and_saved_into_the_opened_folder() {
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("first-save");
    let mut process = FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    send_text(editor, "Grocery list").unwrap();
    command(hwnd, CommandId::Save);
    wait_until("the name box to take focus", || focused_window(hwnd).is_ok_and(|f| f != editor));
    let field = focused_window(hwnd).unwrap();
    unsafe {
        PostMessageW(field, WM_KEYDOWN, VK_RETURN as usize, 0);
    }
    let saved = data.folder().join("Grocery list.md");
    wait_until("the note file", || read(&saved) == "Grocery list");
    close(process, hwnd);
}

#[test]
fn a_note_in_the_folder_autosaves_but_never_overwrites_an_outside_edit() {
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("autosave");
    let note = data.note("a.md", "one");
    let mut process = FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    // A second launch forwards the file to the running window.
    let forwarded = FastPadProcess::spawn_with_local_app_data([&note], &data.root).unwrap();
    wait_for_process_exit(forwarded.id(), WAIT).unwrap();
    wait_until("the note to open", || scintilla_text(editor).is_ok_and(|t| t == "one"));
    send_text(editor, "x").unwrap();
    wait_until("autosave", || read(&note) == scintilla_text(editor).unwrap());

    std::fs::write(&note, "synced").unwrap();
    send_text(editor, "y").unwrap();
    std::thread::sleep(Duration::from_millis(2_500));
    assert_eq!(read(&note), "synced", "autosave must not overwrite an outside edit");
    close(process, hwnd);
}

#[test]
fn a_note_keeps_its_favorite_after_being_renamed_in_explorer() {
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("explorer-rename");
    let note = data.note("a.md", "a");
    let mut process = FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    let forwarded = FastPadProcess::spawn_with_local_app_data([&note], &data.root).unwrap();
    wait_for_process_exit(forwarded.id(), WAIT).unwrap();
    wait_until("the note to open", || scintilla_text(editor).is_ok_and(|t| t == "a"));
    wait_for_library(&data);
    command(hwnd, CommandId::NoteToggleFavorite);
    wait_until("library.ini", || read(&data.library_ini()).contains("|f|-|1|"));
    assert!(read(&data.library_ini()).ends_with("|a.md\r\n"));

    std::fs::rename(&note, data.folder().join("b.md")).unwrap();
    unsafe {
        PostMessageW(hwnd, WM_ACTIVATEAPP, 0, 0);
    }
    std::thread::sleep(Duration::from_millis(5_200));
    unsafe {
        PostMessageW(hwnd, WM_ACTIVATEAPP, 1, 0);
    }
    wait_until("the record to follow the rename", || read(&data.library_ini()).ends_with("|b.md\r\n"));
    assert!(read(&data.library_ini()).contains("|f|"));
    close(process, hwnd);
}

#[test]
fn a_second_launch_with_a_folder_switches_the_running_window() {
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("ipc-folder");
    let other = data.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut process = FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let forwarded = FastPadProcess::spawn_with_local_app_data([&other], &data.root).unwrap();
    wait_for_process_exit(forwarded.id(), WAIT).unwrap();
    let folders = data.data().join("folders.ini");
    wait_until("folders.ini to list the new folder first", || {
        read(&folders).lines().find(|l| l.starts_with("folder=")) == Some(&format!("folder={}", other.display()))
    });
    close(process, hwnd);
}

#[test]
fn a_damaged_library_file_is_left_byte_for_byte_and_notes_still_open() {
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("damaged");
    let note = data.note("a.md", "still readable");
    std::fs::create_dir_all(data.library_ini().parent().unwrap()).unwrap();
    let damaged = b"version=99\r\n\xff\xfe garbage\r\n";
    std::fs::write(data.library_ini(), damaged).unwrap();
    let mut process = FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    let forwarded = FastPadProcess::spawn_with_local_app_data([&note], &data.root).unwrap();
    wait_for_process_exit(forwarded.id(), WAIT).unwrap();
    wait_until("the note to open", || scintilla_text(editor).is_ok_and(|t| t == "still readable"));
    wait_for_library(&data);
    command(hwnd, CommandId::NoteToggleFavorite);
    std::thread::sleep(Duration::from_millis(1_500));
    close(process, hwnd);
    assert_eq!(std::fs::read(data.library_ini()).unwrap(), damaged);
}

#[test]
fn with_notes_mode_off_nothing_is_written_and_save_uses_the_dialog() {
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("mode-off");
    std::fs::write(data.data().join("fastpad.ini"), "notes_mode=false\n").unwrap();
    std::fs::write(
        data.data().join("folders.ini"),
        format!("version=1\r\nfolder={}\r\n", data.folder().display()),
    )
    .unwrap();
    let mut process = FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    send_text(editor, "plain").unwrap();
    command(hwnd, CommandId::Save);
    wait_and_dismiss_dialog(process.id(), WAIT).unwrap();
    assert!(!data.data().join("libraries").exists());
    assert!(!data.folder().join(".fastpad").exists());
    unsafe {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    // restore_session is on by default, so closing does not prompt.
    wait_for_process_exit(process.id(), WAIT).unwrap();
}
```

If `wait_and_dismiss_dialog` presses Cancel on the Save As dialog, that is exactly what this test wants. If it presses OK, change the assertion to allow a saved `Untitled.txt` in the default save folder, and delete that file at the end.

- [ ] **Step 3: Build the executable and run the target**

Run: `cargo build`, then `cargo test --test library -- --test-threads=1`
Expected: all 6 pass. If a test times out, first check that no other `FastPad.exe` is running: the tests need to own the single-instance mutex.

- [ ] **Step 4: Add the bench action.** In `src/bin/fastpad-bench.rs`:
- **Extend `Action`** with `LibraryScan { folder: PathBuf, count: Option<usize> }`. `parse_args` accepts `library-scan DIR [--count N]`, next to the existing `compare` form. Update the usage text.
- **Add `fn run_library_scan(folder: &Path, count: Option<usize>) -> Result<i32, String>`:**
  1. If `count` is `Some(n)`, create `n` files `folder\batch{i / 500}\note{i}.md`, each with about 200 bytes of text, and one `.fastpad\library.ini` holding 200 favorite records spread across them. Build the file with `fastpad::library::store::write` and records made through `fastpad::library::ops::apply(SetFavorite { … })`.
  2. Use a scratch local file under `%TEMP%\fastpad-bench-library-<pid>.ini`, deleted at the end.
  3. Time one cold `fastpad::library::load(folder, local, now)`, the first one with no local file. Then time five warm loads.
  4. Report the note count, the cold time, and the warm median in milliseconds. Also report the approximate index size: the sum of `path.as_os_str().len() * 2 + size_of::<NoteEntry>()` over `state.notes`.
  5. Return exit code 2 if the warm median is at least 500 ms and `--enforce-reference` was given, and 0 otherwise.
- **Add `--notes-folder DIR` to the `Run` form.** When it is given, `run_once` writes `FastPad\folders.ini` (`version=1` and `folder=DIR`) into the scratch `LOCALAPPDATA` before spawning. The measured launch then opens that folder in its deferred step. TTI must stay within the existing thresholds.
- **Add unit tests** in the bench's test module for the new `parse_args` forms, following the existing ones.

- [ ] **Step 5: Run the bench once by hand and record the numbers**

Run:
```
cargo build --release --bin fastpad --bin fastpad-bench
target\release\fastpad-bench.exe library-scan %TEMP%\fastpad-10k --count 10000
target\release\fastpad-bench.exe --runs 30 --warmup 5 --notes-folder %TEMP%\fastpad-10k
target\release\fastpad-bench.exe --runs 30 --warmup 5
```
Expected: the library-scan warm median is under 500 ms on the reference i5-4590, and TTI p50 and p95 with `--notes-folder` are within noise of the run without it. Paste the three outputs into the PR description.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml tests/windows/library.rs src/bin/fastpad-bench.rs
git commit -m "test: note library end to end; bench: library scan and notes-folder TTI"
```

---

### Task 22: Docs

**Files:**
- Modify: `README.md`, `docs/superpowers/specs/2026-09-23-note-library-design.md`

- [ ] **Step 1: Update the spec.** Set `Status: Approved design`. Add a short section "Implementation notes", which records these decisions made while planning:
  - **Deleting to the Recycle Bin** uses `SHFileOperationW` with `FOF_ALLOWUNDO` instead of `IFileOperation`. It gives the same behavior with no COM vtable.
  - **Note extensions are an explicit list:** `md`, `markdown`, `txt`, `text`, `json`, `log`, `ini`, `cfg`, `conf`, `yaml`, `yml`, `toml`, `csv`, `xml`. `detect_language` knows only `json` and `md`.
  - **OneDrive online-only files** (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` or `OFFLINE`) are listed but never hashed, so reconciliation never downloads them.
  - **The disk-change notice** offers *Reload* and *Keep mine* as the palette commands "Note: Reload from disk" and "Note: Keep my version". Notices have no buttons yet.
  - **Dropping files on the window opens them as tabs.** FastPad handled no drops before, so "keep today's behavior" had nothing to keep.
  - **Two parts of the spec are deferred:** reordering notebooks is in the model (`move_notebook`) but has no command yet, and sub-project 2's sidebar will drive it. The inline name box shows its error as text next to the field.
- [ ] **Step 2: Update `README.md`:**
  - Add a section "Notes and folders" after the Markdown section. Keep it short and in the README's voice:
    - Open any folder with **Ctrl+Shift+O**, and it becomes your note library.
    - Ctrl+N gives a new note, labelled by its first line.
    - The first Ctrl+S asks for its name inline and saves it into the folder.
    - Files inside the folder save themselves.
    - Notebooks, tags, favorites and pins live in the command palette for now, and are stored in `.fastpad\library.ini` inside the folder, so they travel with it.
    - Nothing is written into a folder until you organize something.
    - `notes_mode=false` in `fastpad.ini` turns it all off.
  - Add `Open folder | Ctrl+Shift+O` to the shortcuts table.
- [ ] **Step 3: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-23-note-library-design.md
git commit -m "docs: note library in the README; record implementation decisions in the spec"
```

---

## Final review

- [ ] Run `cargo clippy --all-targets -- -D warnings`, then the full suite once: `cargo test -- --test-threads=1`. Before any run that launches the real app against the user's profile, back up and afterwards restore `%LOCALAPPDATA%\FastPad\fastpad.ini` and `folders.ini`. The integration tests use scratch `LOCALAPPDATA`, but the release-bench runs in Task 21 Step 5 do not touch the user's files either, so this matters only for manual checks.
- [ ] **Manual check with the release build:**
  1. Launch, press Ctrl+N and type. The tab label follows the first line.
  2. Press Ctrl+S, then Enter. The file appears in `Documents\FastPad`.
  3. Press Ctrl+Shift+O and choose a folder. Favorite a note from the palette, rename the note in Explorer, then switch back to FastPad after 5 s. `library.ini` follows the rename.
