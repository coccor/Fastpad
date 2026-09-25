//! `.fastpad\library.ini`: the pins that travel with the notebook.
//!
//! ```text
//! version=2
//! note=<id>|<flags>|<size>|<hash>|<path>
//! ```
//!
//! `<flags>` is `p` (pinned), `d` (deleted), both, or `-`. The path is relative to the notebook,
//! last and unescaped, so splitting a `note` line on its first four `|` characters is
//! unambiguous. Records whose path is not a plain relative path are dropped on read and never
//! written. Any other version, or none, makes the whole file unreadable, and an unreadable file
//! is never overwritten.

use super::ids::NoteId;
use super::model::{Library, NoteRecord};
use crate::Result;
use std::path::{Component, Path, PathBuf};

const VERSION: &str = "2";
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

/// Whether a record path is a plain path inside the notebook: only normal components. Shared
/// with `text_replace`, which rejects a target path that fails this before any read or write.
pub(super) fn is_notebook_path(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

pub fn encode(library: &Library) -> String {
    let mut output = format!("version={VERSION}\r\n");
    for note in library
        .notes
        .iter()
        .filter(|note| is_notebook_path(&note.path))
    {
        let flags = match (note.pinned, note.deleted) {
            (true, true) => "pd",
            (true, false) => "p",
            (false, true) => "d",
            (false, false) => "-",
        };
        output.push_str(&format!(
            "note={}|{flags}|{}|{:016x}|{}\r\n",
            note.id.to_hex(),
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
    let mut lines = Vec::new();
    for line in source.lines() {
        match line.split_once('=') {
            Some(("version", value)) => version = Some(value),
            Some(("note", value)) => lines.push(value),
            _ => {}
        }
    }
    if version? != VERSION {
        return None;
    }
    let mut library = Library {
        notes: lines
            .into_iter()
            .filter_map(parse_note)
            .filter(|note| is_notebook_path(&note.path))
            .collect(),
    };
    dedupe_by(&mut library.notes, |note| note.id.0);
    Some(library)
}

fn dedupe_by<T>(items: &mut Vec<T>, key: impl Fn(&T) -> u128) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(key(item)));
}

/// `<id>|<flags>|<size>|<hash>|<path>`.
fn parse_note(value: &str) -> Option<NoteRecord> {
    let mut fields = value.splitn(5, '|');
    let id = NoteId::parse_hex(fields.next()?)?;
    let flags = fields.next()?;
    finish_note(id, flags, fields)
}

/// The size, hash and path that end a `note` line. Only `p` and `d` flags count.
fn finish_note<'a>(
    id: NoteId,
    flags: &str,
    mut fields: impl Iterator<Item = &'a str>,
) -> Option<NoteRecord> {
    let size = fields.next()?.parse().ok()?;
    let hash = u64::from_str_radix(fields.next()?, 16).ok()?;
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    let mut note = NoteRecord::new(id, PathBuf::from(path));
    note.pinned = flags.contains('p');
    note.deleted = flags.contains('d');
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
    use crate::library::ids::NoteId;
    use crate::library::model::NoteRecord;

    fn sample() -> Library {
        let mut pinned = NoteRecord::new(NoteId(0xa), PathBuf::from(r"sub\a b|c.md"));
        pinned.pinned = true;
        pinned.size = 12;
        pinned.hash = 0xfeed;
        let mut deleted = NoteRecord::new(NoteId(0xb), PathBuf::from("b.md"));
        deleted.deleted = true;
        let mut both = NoteRecord::new(NoteId(0xc), PathBuf::from("c.md"));
        both.pinned = true;
        both.deleted = true;
        Library {
            notes: vec![pinned, deleted, both],
        }
    }

    #[test]
    fn a_library_round_trips_through_its_text_form() {
        // Break caught: a path with `|` or a flag combination not surviving a write and read.
        let text = encode(&sample());
        assert!(text.starts_with("version=2\r\n"));
        assert!(text.contains(
            "note=0000000000000000000000000000000a|p|12|000000000000feed|sub\\a b|c.md\r\n"
        ));
        assert!(
            text.contains("note=0000000000000000000000000000000b|d|0|0000000000000000|b.md\r\n")
        );
        assert!(
            text.contains("note=0000000000000000000000000000000c|pd|0|0000000000000000|c.md\r\n")
        );
        assert_eq!(parse(&text), Some(sample()));
    }

    #[test]
    fn only_version_two_files_are_readable() {
        // Break caught: a newer FastPad's file (or a damaged one) read as an empty library and
        // then overwritten, destroying every pin.
        assert_eq!(parse("note=x\r\n"), None);
        assert_eq!(parse("version=1\r\n"), None);
        assert_eq!(parse("version=3\r\n"), None);
        assert_eq!(parse("\u{feff}version=2\r\n"), Some(Library::default()));
    }

    #[test]
    fn malformed_lines_unknown_keys_and_unknown_flags_are_tolerated() {
        let text = "version=2\n\
            future=1\n\
            note=00000000000000000000000000000007|zq|5|0000000000000001|a.md\n\
            note=short\n\
            note=00000000000000000000000000000008|p|x|0000000000000001|b.md\n";
        let library = parse(text).unwrap();
        assert_eq!(library.notes.len(), 1);
        let note = &library.notes[0];
        assert!(!note.pinned && !note.deleted, "unknown flags are ignored");
        assert_eq!((note.size, note.hash), (5, 1));
    }

    #[test]
    fn absolute_path_records_are_dropped_on_read_and_never_written() {
        // Break caught: a record for a file outside the folder surviving into a write, which
        // has no way to say which drive it meant.
        let text = "version=2\r\n\
            note=0000000000000000000000000000000a|p|0|0000000000000000|C:\\elsewhere\\log.txt\r\n\
            note=0000000000000000000000000000000b|p|0|0000000000000000|\\rooted.md\r\n\
            note=0000000000000000000000000000000c|p|0|0000000000000000|..\\up.md\r\n\
            note=0000000000000000000000000000000d|p|0|0000000000000000|kept.md\r\n";
        let library = parse(text).unwrap();
        let kept: Vec<_> = library.notes.iter().map(|note| note.id).collect();
        assert_eq!(kept, [NoteId(0xd)]);
        let mut outside = NoteRecord::new(NoteId(0xe), PathBuf::from(r"D:\x.md"));
        outside.pinned = true;
        assert_eq!(
            encode(&Library {
                notes: vec![outside]
            }),
            "version=2\r\n"
        );
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
        // Break caught: a version 1 file read as loaded instead of unreadable, or reading it
        // rewriting it before anything changed.
        std::fs::write(
            &path,
            "version=1\r\nnote=0000000000000000000000000000000a|-|p|-|0|0000000000000000|a.md\r\n",
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "reading never rewrites"
        );
        std::fs::write(&path, "version=9\r\n").unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_held_open_by_another_process_reads_as_busy_not_unreadable() {
        // Break caught: a sharing violation while OneDrive syncs library.ini being taken for a
        // damaged file, which turns pinning off for the whole session.
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
