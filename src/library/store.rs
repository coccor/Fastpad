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

#[cfg(test)]
thread_local! {
    static STATS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many stamps this thread has taken, so tests can prove a path does no disk access.
#[cfg(test)]
pub fn stats_taken() -> usize {
    STATS.with(std::cell::Cell::get)
}

pub fn stamp(path: &Path) -> Option<FileStamp> {
    #[cfg(test)]
    STATS.with(|stats| stats.set(stats.get() + 1));
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
        output.push_str(&format!(
            "tag={}|{}\r\n",
            tag.id.to_hex(),
            escape(&tag.name)
        ));
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
            note.tags
                .iter()
                .map(|tag| tag.to_hex())
                .collect::<Vec<_>>()
                .join(",")
        };
        output.push_str(&format!(
            "note={}|{}|{flags}|{tags}|{}|{:016x}|{}\r\n",
            note.id.to_hex(),
            note.notebook
                .map_or_else(|| "-".to_owned(), NotebookId::to_hex),
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
    Some(Notebook {
        id,
        name,
        color,
        sort,
        created,
        modified,
    })
}

fn parse_tag(value: &str) -> Option<Tag> {
    let (id, name) = value.split_once('|')?;
    let name = unescape(name);
    if name.trim().is_empty() {
        return None;
    }
    Some(Tag {
        id: TagId::parse_hex(id)?,
        name,
    })
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
    /// Read, but damaged or from a newer FastPad: never overwritten.
    Unreadable,
    /// Could not be read right now (a sharing violation while OneDrive syncs it, or it kept
    /// changing during the read). Nothing is known about its contents; try again later.
    Busy,
}

/// How many times a read that raced a write is retried before it is reported as busy.
const READ_ATTEMPTS: usize = 3;

pub fn read(path: &Path) -> ReadOutcome {
    read_via(path, |path| std::fs::read(path))
}

fn read_via(
    path: &Path,
    mut read_bytes: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> ReadOutcome {
    for _ in 0..READ_ATTEMPTS {
        // A stamp on both sides of the read proves the bytes belong to that stamp.
        let before = stamp(path);
        let bytes = match read_bytes(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ReadOutcome::Absent;
            }
            Err(_) => return ReadOutcome::Busy,
        };
        let after = stamp(path);
        let Some(stamp) = after.filter(|_| before == after) else {
            continue;
        };
        return match std::str::from_utf8(&bytes).ok().and_then(parse) {
            Some(library) => ReadOutcome::Loaded(library, stamp),
            None => ReadOutcome::Unreadable,
        };
    }
    ReadOutcome::Busy
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::{NoteId, NotebookId, TagId};
    use crate::library::model::{NoteRecord, Notebook, NotebookColor, Tag};

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
                Tag {
                    id: TagId(2),
                    name: "idea".into(),
                },
                Tag {
                    id: TagId(3),
                    name: "to|do".into(),
                },
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
        assert!(
            !note.favorite && !note.pinned && !note.deleted,
            "unknown flags are ignored"
        );
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

    #[test]
    fn a_file_held_open_by_another_process_reads_as_busy_not_unreadable() {
        // Break caught: a sharing violation while OneDrive syncs library.ini being taken for a
        // damaged file, which turns organizing off for the whole session.
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("fastpad-store-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = library_file(&dir);
        write(&path, &sample()).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(matches!(read(&path), ReadOutcome::Busy));
        drop(lock);
        assert!(matches!(read(&path), ReadOutcome::Loaded(..)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_that_races_a_write_is_retried_and_never_pairs_old_bytes_with_a_new_stamp() {
        // Break caught: the stamp taken after the bytes, so a sync landing mid-read left FastPad
        // holding the old library under the new file's stamp, and the next flush wrote over the
        // synced change without re-reading it.
        let dir = std::env::temp_dir().join(format!("fastpad-store-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = library_file(&dir);
        write(&path, &Library::default()).unwrap();
        let mut calls = 0;
        let outcome = read_via(&path, |path| {
            calls += 1;
            let bytes = std::fs::read(path);
            if calls == 1 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                write(path, &sample()).unwrap();
            }
            bytes
        });
        match outcome {
            ReadOutcome::Loaded(library, stamp) => {
                assert_eq!(calls, 2);
                assert_eq!(library, sample());
                assert_eq!(Some(stamp), super::stamp(&path));
            }
            _ => panic!("expected the retried read to load"),
        }
        let always_changing = read_via(&path, |path| {
            let bytes = std::fs::read(path);
            std::fs::write(path, format!("{}x", std::fs::read_to_string(path).unwrap())).unwrap();
            bytes
        });
        assert!(matches!(always_changing, ReadOutcome::Busy));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
