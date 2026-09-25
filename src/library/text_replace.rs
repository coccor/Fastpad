//! Replace across notes, run on worker threads: counts the matches in the notes a search listed,
//! and writes the replacements into notes that aren't open, one note at a time. A write happens
//! only when the note is still what the search read (its stamp), and keeps the note's encoding,
//! BOM and line endings. No Win32 and no window.

use super::path_key;
use super::store;
use super::text_search::{self, MAX_NOTE_BYTES, Stamp};
use crate::file::{encoding, saver};
use crate::search::Matcher;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// Why a note that couldn't be decoded wasn't written; the search never lists one.
const NOT_TEXT: &str = "The note isn't UTF-8 or UTF-16 text.";
/// Why a target whose path leaves the notebook (absolute, or with a `..`) is never read or
/// written. Targets come from search hits, which never produce such a path, but `apply` and
/// `count` are `pub` and must not trust that on their own.
const OUTSIDE_NOTEBOOK: &str = "The note's path leaves the notebook.";

/// A note to replace in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceTarget {
    /// Relative to the notebook, as the note list has it.
    pub path: PathBuf,
    /// What the search read (`TextHit.stamp`).
    pub stamp: Stamp,
}

/// How many matches a replace would change, and in how many notes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplaceCount {
    pub matches: usize,
    /// Notes with at least one match.
    pub notes: usize,
    /// Notes with at least one match whose text came from disk, not from an overlay (a dirty
    /// tab). These are the notes a replace could actually write to and save.
    pub closed_notes: usize,
}

/// What `apply` did. Every path is relative to the notebook.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplaceReport {
    /// Matches replaced, in every note saved.
    pub matches: usize,
    /// The notes that were replaced into and saved, each with the stamp of the saved file (read
    /// on the worker right after the save), so the library takes in the new size and time
    /// without touching the disk (`LibraryState::record_written`). Its length is how many notes
    /// were replaced into.
    pub written: Vec<(PathBuf, Stamp)>,
    /// The notes whose stamp no longer matched: changed since the search, and left as they are.
    pub changed: Vec<PathBuf>,
    /// The notes that couldn't be read, decoded or saved, with the error.
    pub failed: Vec<(PathBuf, String)>,
}

/// Counts the matches `matcher` finds in `targets`, as `Matcher::find_iter` finds them.
///
/// - `overlays` holds dirty tabs' text by relative path, compared with `path_key`; an overlay's
///   text is counted instead of the disk's, as in search.
/// - A note that can't be read or decoded, or is over `MAX_NOTE_BYTES`, counts 0.
/// - A note whose path leaves the notebook (absolute, or with a `..`) counts 0 and is never
///   opened.
/// - A note with at least one match whose text came from disk (not an overlay) is a
///   `closed_notes` note: a replace can actually write to it.
/// - `cancel` is read before each note. A cancelled count is partial; the caller drops it.
pub fn count(
    notebook: &Path,
    targets: &[ReplaceTarget],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
) -> ReplaceCount {
    let overlays: HashMap<String, &str> = overlays
        .iter()
        .map(|(path, text)| (path_key(path), text.as_str()))
        .collect();
    let mut total = ReplaceCount::default();
    let mut bytes = Vec::new();
    for target in targets {
        if cancel.load(Relaxed) {
            break;
        }
        if !store::is_notebook_path(&target.path) {
            continue;
        }
        let (matches, from_disk) = match overlays.get(&path_key(&target.path)) {
            Some(text) => (matcher.find_iter(text).len(), false),
            None => (
                disk_text(&notebook.join(&target.path), &mut bytes)
                    .map_or(0, |text| matcher.find_iter(&text).len()),
                true,
            ),
        };
        if matches > 0 {
            total.matches += matches;
            total.notes += 1;
            if from_disk {
                total.closed_notes += 1;
            }
        }
    }
    total
}

/// Replaces every match of `matcher` in `targets` with `template` (expanded as
/// `Matcher::replacements` expands it), one note at a time:
///
/// - The note is read, and its stamp (the opened file's size and last write time, as the search
///   takes them) compared with `target.stamp`. A mismatch, or bytes that don't add up to the
///   stamp's size, goes to `changed` and the note is left as it is.
/// - The bytes are decoded, and `Matcher::replace_text` gives the new text. With no match the
///   note is left as it is and is in no list.
/// - The text is encoded with the note's own `Encoding` (so a BOM stays) and written with
///   `saver::save_atomic`. Line endings stay as they were, since the text is never normalized.
///   The saved file's stamp is then read, here on the worker, and the note goes to `written`
///   with it. If that read fails (the note vanished in the moment after its save), its matches
///   still count but it is left out of `written`, so the library isn't told of it.
/// - A note whose path leaves the notebook (absolute, or with a `..`) is never opened: it goes
///   to `failed` before any read, the same as a note that can't be opened.
/// - A note that can't be opened (deleted since the search, say), read, decoded or saved goes to
///   `failed` with the error, and nothing is written to it.
/// - `cancel` is read before each note; a note already being written is finished, and the
///   report of the notes done so far is returned.
///
/// The stamp is checked just before the write, so a change in the moment between the two is
/// not seen.
pub fn apply(
    notebook: &Path,
    targets: &[ReplaceTarget],
    matcher: &Matcher,
    template: &str,
    cancel: &AtomicBool,
) -> ReplaceReport {
    apply_until(notebook, targets, matcher, template, &mut || {
        cancel.load(Relaxed)
    })
}

/// `apply`'s loop, asking `cancelled` before each note.
fn apply_until(
    notebook: &Path,
    targets: &[ReplaceTarget],
    matcher: &Matcher,
    template: &str,
    cancelled: &mut dyn FnMut() -> bool,
) -> ReplaceReport {
    let mut report = ReplaceReport::default();
    let mut bytes = Vec::new();
    for target in targets {
        if cancelled() {
            break;
        }
        match replace_note(notebook, target, matcher, template, &mut bytes) {
            Ok(Replaced::Written { matches, stamp }) => {
                report.matches += matches;
                if let Some(stamp) = stamp {
                    report.written.push((target.path.clone(), stamp));
                }
            }
            Ok(Replaced::NoMatch) => {}
            Ok(Replaced::Changed) => report.changed.push(target.path.clone()),
            Err(error) => report.failed.push((target.path.clone(), error)),
        }
    }
    report
}

enum Replaced {
    /// Saved, with the saved file's stamp (`None` when it couldn't be read after the save).
    Written {
        matches: usize,
        stamp: Option<Stamp>,
    },
    NoMatch,
    Changed,
}

fn replace_note(
    notebook: &Path,
    target: &ReplaceTarget,
    matcher: &Matcher,
    template: &str,
    bytes: &mut Vec<u8>,
) -> Result<Replaced, String> {
    if !store::is_notebook_path(&target.path) {
        return Err(OUTSIDE_NOTEBOOK.to_owned());
    }
    let path = notebook.join(&target.path);
    let stamp = text_search::read_bytes(&path, bytes).map_err(|error| error.to_string())?;
    // A file over the limit is read only to `MAX_NOTE_BYTES + 1`, so its length never matches
    // a stamp the search took (every hit's is within the limit).
    if stamp != target.stamp || bytes.len() as u64 != stamp.size {
        return Ok(Replaced::Changed);
    }
    let decoded = encoding::decode(bytes).map_err(|_| NOT_TEXT.to_owned())?;
    let (text, matches) = matcher.replace_text(&decoded.text, template);
    if matches == 0 {
        return Ok(Replaced::NoMatch);
    }
    saver::save_atomic(&path, &encoding::encode(&text, decoded.encoding))
        .map_err(|error| error.to_string())?;
    // The saved file's stamp, taken here on the worker so the UI thread never reads the disk
    // to update the library.
    let stamp = std::fs::metadata(&path)
        .ok()
        .map(|metadata| Stamp::of(&metadata));
    Ok(Replaced::Written { matches, stamp })
}

/// The note's text from disk, or `None` when it can't be read or decoded or is over the limit.
/// Shares `text_search::read_bytes`, the same open + metadata + read a search does, rather than
/// a second file-reading routine. The size check is against the stamp (`read_bytes` leaves
/// `bytes` empty for an over-the-limit file rather than reading it), not `bytes.len()`.
fn disk_text(path: &Path, bytes: &mut Vec<u8>) -> Option<String> {
    let stamp = text_search::read_bytes(path, bytes).ok()?;
    if stamp.size > MAX_NOTE_BYTES {
        return None;
    }
    encoding::decode(bytes).ok().map(|decoded| decoded.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::MatchOptions;
    use std::time::{Duration, SystemTime};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fastpad-text-replace-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        /// Writes `bytes` at `relative` and returns it as a target, stamped as a search would
        /// stamp what it read.
        fn note(&self, relative: &str, bytes: &[u8]) -> ReplaceTarget {
            let path = self.path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            ReplaceTarget {
                path: PathBuf::from(relative),
                stamp: stamp_of(&path),
            }
        }

        fn read(&self, relative: &str) -> Vec<u8> {
            std::fs::read(self.path(relative)).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stamp_of(path: &Path) -> Stamp {
        Stamp::of(&std::fs::File::open(path).unwrap().metadata().unwrap())
    }

    fn plain(query: &str) -> Matcher {
        Matcher::new(query, MatchOptions::default()).unwrap()
    }

    fn not_cancelled() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn a_note_changed_since_the_search_is_skipped_not_overwritten() {
        // Break caught (review focus 1): a sync client's or another editor's change overwritten
        // by a replace made from an older search, a deleted note recreated or crashing the
        // write, or a note without a match rewritten anyway.
        let scratch = Scratch::new("changed");
        let kept = scratch.note("kept.md", b"foo one");
        let grown = scratch.note("grown.md", b"foo two");
        let same_size = scratch.note("same-size.md", b"foo six");
        let deleted = scratch.note("deleted.md", b"foo ten");
        let no_match = scratch.note("no-match.md", b"nothing");
        std::fs::write(scratch.path("grown.md"), b"foo two, and more").unwrap();
        // The same size, and a last write time that is surely not the one the search took.
        std::fs::write(scratch.path("same-size.md"), b"foo SIX").unwrap();
        std::fs::File::options()
            .write(true)
            .open(scratch.path("same-size.md"))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
            .unwrap();
        std::fs::remove_file(scratch.path("deleted.md")).unwrap();
        let no_match_before = stamp_of(&scratch.path("no-match.md"));
        let targets = [kept, grown, same_size, deleted, no_match];

        let report = apply(&scratch.0, &targets, &plain("foo"), "bar", &not_cancelled());

        assert_eq!(scratch.read("kept.md"), b"bar one");
        assert_eq!(scratch.read("grown.md"), b"foo two, and more");
        assert_eq!(scratch.read("same-size.md"), b"foo SIX");
        assert!(!scratch.path("deleted.md").exists(), "never recreated");
        assert_eq!(scratch.read("no-match.md"), b"nothing");
        assert_eq!(stamp_of(&scratch.path("no-match.md")), no_match_before);
        assert_eq!(report.matches, 1);
        assert_eq!(
            report.written,
            [(PathBuf::from("kept.md"), stamp_of(&scratch.path("kept.md")))],
            "relative to the notebook, with the saved file's stamp"
        );
        assert_eq!(
            report.changed,
            [PathBuf::from("grown.md"), PathBuf::from("same-size.md")]
        );
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, PathBuf::from("deleted.md"));
        assert!(!report.failed[0].1.is_empty(), "the error is named");
    }

    #[test]
    fn a_write_keeps_the_encoding_bom_and_line_endings() {
        // Break caught (review focus 2): a BOM dropped or added, UTF-16 written back as UTF-8,
        // CRLF turned into LF (or the other way), a final newline added, or the replacement's
        // own non-ASCII text mis-encoded.
        fn utf16(text: &str, bom: [u8; 2], unit: fn(u16) -> [u8; 2]) -> Vec<u8> {
            let mut bytes = bom.to_vec();
            bytes.extend(text.encode_utf16().flat_map(unit));
            bytes
        }
        let scratch = Scratch::new("encodings");
        let mixed = "foo\r\nfoo\nlast foo";
        let bom_text = "foo ă\r\nbar\r\n";
        let cases: [(&str, Vec<u8>, Vec<u8>); 5] = [
            (
                "utf8.md",
                mixed.as_bytes().to_vec(),
                b"\xC8\x9B\xC4\x83\r\n\xC8\x9B\xC4\x83\nlast \xC8\x9B\xC4\x83".to_vec(),
            ),
            (
                "bom.md",
                [&[0xEF, 0xBB, 0xBF][..], bom_text.as_bytes()].concat(),
                [&[0xEF, 0xBB, 0xBF][..], "ță ă\r\nbar\r\n".as_bytes()].concat(),
            ),
            (
                "utf16le.md",
                utf16(mixed, [0xFF, 0xFE], u16::to_le_bytes),
                utf16("ță\r\nță\nlast ță", [0xFF, 0xFE], u16::to_le_bytes),
            ),
            (
                "utf16be.md",
                utf16(bom_text, [0xFE, 0xFF], u16::to_be_bytes),
                utf16("ță ă\r\nbar\r\n", [0xFE, 0xFF], u16::to_be_bytes),
            ),
            (
                "no-final-newline.md",
                b"a foo".to_vec(),
                "a ță".as_bytes().to_vec(),
            ),
        ];
        let targets: Vec<ReplaceTarget> = cases
            .iter()
            .map(|(name, before, _)| scratch.note(name, before))
            .collect();

        let report = apply(&scratch.0, &targets, &plain("foo"), "ță", &not_cancelled());

        for (name, _, after) in &cases {
            assert_eq!(&scratch.read(name), after, "{name}");
        }
        assert_eq!(report.matches, 3 + 1 + 3 + 1 + 1);
        assert_eq!(report.written.len(), 5);
        assert!(report.changed.is_empty() && report.failed.is_empty());
    }

    #[test]
    fn regex_mode_expands_groups_in_each_notes_matches() {
        let scratch = Scratch::new("groups");
        let targets = [
            scratch.note("a.md", b"ann@site\r\nbob@host"),
            scratch.note("b.md", b"x@y and $1@z"),
        ];
        let regex = Matcher::new(
            r"(\w+)@(\w+)",
            MatchOptions {
                regex: true,
                ..MatchOptions::default()
            },
        )
        .unwrap();

        let report = apply(&scratch.0, &targets, &regex, "$2 ($1) $$", &not_cancelled());

        assert_eq!(scratch.read("a.md"), b"site (ann) $\r\nhost (bob) $");
        assert_eq!(scratch.read("b.md"), b"y (x) $ and $z (1) $");
        assert_eq!(report.matches, 4);
        let written: Vec<&Path> = report
            .written
            .iter()
            .map(|(path, _)| path.as_path())
            .collect();
        assert_eq!(written, [Path::new("a.md"), Path::new("b.md")]);
    }

    #[test]
    fn a_count_takes_overlays_over_the_disk_and_an_unreadable_note_counts_nothing() {
        // Break caught: the prompt's N counted from the disk while a dirty tab holds other text,
        // an overlay missed because its path differs in case, or a missing note stopping the
        // count.
        let scratch = Scratch::new("count");
        let open = scratch.note("Open.md", b"foo foo foo");
        let closed = scratch.note("sub/closed.md", b"foo\r\nfoo");
        let none = scratch.note("none.md", b"nothing");
        let missing = ReplaceTarget {
            path: PathBuf::from("missing.md"),
            stamp: open.stamp,
        };
        let binary = scratch.note("binary.md", b"foo \xFF");
        let overlays = HashMap::from([(PathBuf::from("open.MD"), "foo".to_owned())]);
        let targets = [open, closed, none, missing, binary];

        let counted = count(
            &scratch.0,
            &targets,
            &overlays,
            &plain("foo"),
            &not_cancelled(),
        );

        assert_eq!(
            counted,
            ReplaceCount {
                matches: 3,
                notes: 2,
                closed_notes: 1,
            },
            "the overlay note (Open.md/open.MD) has matches but is not closed_notes"
        );
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            count(&scratch.0, &targets, &overlays, &plain("foo"), &cancelled),
            ReplaceCount::default()
        );
    }

    #[test]
    fn closed_notes_counts_only_notes_whose_matched_text_came_from_disk() {
        // R2: an overlay note with matches is not counted in `closed_notes`, a disk note with
        // matches is, and a disk note with 0 matches is not.
        let scratch = Scratch::new("closed-notes");
        let overlay_with_matches = scratch.note("dirty.md", b"nothing on disk");
        let disk_with_matches = scratch.note("clean.md", b"foo foo");
        let disk_no_match = scratch.note("empty.md", b"nothing");
        let overlays = HashMap::from([(PathBuf::from("dirty.md"), "foo foo foo".to_owned())]);
        let targets = [overlay_with_matches, disk_with_matches, disk_no_match];

        let counted = count(
            &scratch.0,
            &targets,
            &overlays,
            &plain("foo"),
            &not_cancelled(),
        );

        assert_eq!(counted.notes, 2, "the overlay and the disk note both match");
        assert_eq!(
            counted.closed_notes, 1,
            "only the disk note with matches is closed_notes"
        );
        assert_eq!(counted.matches, 3 + 2);
    }

    #[test]
    fn a_cancelled_replace_stops_before_the_next_note_and_reports_what_it_wrote() {
        // Break caught: a cancel that throws away the report of notes already saved (the library
        // never learns of FastPad's own writes), one that keeps writing, or a report whose paths
        // aren't relative to the notebook.
        let scratch = Scratch::new("cancel");
        let targets = [
            scratch.note("a.md", b"foo"),
            scratch.note("b.md", b"foo foo"),
        ];

        let cancelled = AtomicBool::new(true);
        let report = apply(&scratch.0, &targets, &plain("foo"), "bar", &cancelled);
        assert_eq!(report, ReplaceReport::default());
        assert_eq!(scratch.read("a.md"), b"foo");

        // Cancelled after the first note: its write is reported, the second note is untouched.
        let mut asked = 0;
        let report = apply_until(&scratch.0, &targets, &plain("foo"), "bar", &mut || {
            asked += 1;
            asked > 1
        });
        assert_eq!(scratch.read("a.md"), b"bar");
        assert_eq!(scratch.read("b.md"), b"foo foo");
        assert_eq!(report.matches, 1);
        assert_eq!(
            report.written,
            [(PathBuf::from("a.md"), stamp_of(&scratch.path("a.md")))]
        );
        assert!(report.changed.is_empty() && report.failed.is_empty());
    }

    #[test]
    fn a_target_path_outside_the_notebook_is_never_touched() {
        // Important 1: an absolute or `..`-escaping target path writing (or even reading) a file
        // outside the notebook folder. `notebook.join` alone doesn't stop either: `join` with an
        // absolute path discards `notebook`, and `..` walks back out of it.
        let scratch = Scratch::new("outside");
        let sibling = scratch.0.parent().unwrap().join(format!(
            "fastpad-text-replace-sibling-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&sibling);
        std::fs::create_dir_all(&sibling).unwrap();
        let outside_file = sibling.join("outside.md");
        std::fs::write(&outside_file, b"foo untouched").unwrap();
        let outside_before = stamp_of(&outside_file);
        let absolute = ReplaceTarget {
            path: outside_file.clone(),
            stamp: outside_before,
        };
        // Also escapes through the notebook itself: `..\<sibling folder name>\outside.md`
        // resolves right back to `outside_file` once joined onto `scratch.0`.
        let escaping = ReplaceTarget {
            path: Path::new("..")
                .join(sibling.file_name().unwrap())
                .join("outside.md"),
            stamp: outside_before,
        };
        let targets = [absolute, escaping];

        let count_result = count(
            &scratch.0,
            &targets,
            &HashMap::new(),
            &plain("foo"),
            &not_cancelled(),
        );
        let report = apply(&scratch.0, &targets, &plain("foo"), "bar", &not_cancelled());

        assert_eq!(
            count_result,
            ReplaceCount::default(),
            "counts 0, never opened"
        );
        assert_eq!(
            std::fs::read(&outside_file).unwrap(),
            b"foo untouched",
            "never written"
        );
        assert_eq!(stamp_of(&outside_file), outside_before, "never even opened");
        assert!(report.written.is_empty() && report.changed.is_empty());
        assert_eq!(report.failed.len(), 2, "both targets are reported failed");
        for (_, reason) in &report.failed {
            assert!(!reason.is_empty(), "the error is named");
        }
        let _ = std::fs::remove_dir_all(&sibling);
    }

    #[test]
    fn a_target_that_isnt_text_is_failed_and_left_untouched() {
        // Fix round 1, item 4: a binary target is reported in `failed`, not silently dropped or
        // written with mangled bytes.
        let scratch = Scratch::new("not-text");
        let bytes = vec![0x80, 0x81, b'f', b'o', b'o'];
        let target = scratch.note("binary.md", &bytes);

        let report = apply(
            &scratch.0,
            &[target],
            &plain("foo"),
            "bar",
            &not_cancelled(),
        );

        assert_eq!(scratch.read("binary.md"), bytes, "left untouched");
        assert!(report.written.is_empty() && report.changed.is_empty());
        assert_eq!(
            report.failed,
            [(PathBuf::from("binary.md"), NOT_TEXT.to_owned())]
        );
    }

    #[test]
    fn a_note_grown_past_the_limit_since_the_search_is_changed_not_written() {
        // Fix round 1, item 4: the stamp mismatch catches a note that grew past MAX_NOTE_BYTES
        // since the search read it, the reason for the size check `replace_note` runs before
        // decoding. It is reported `changed`, not written and not crashed on.
        let scratch = Scratch::new("grown-over-limit");
        let target = scratch.note("grows.md", b"foo");
        let mut grown = vec![b'x'; (MAX_NOTE_BYTES + 100) as usize];
        grown.extend_from_slice(b" foo");
        std::fs::write(scratch.path("grows.md"), &grown).unwrap();

        let report = apply(
            &scratch.0,
            &[target],
            &plain("foo"),
            "bar",
            &not_cancelled(),
        );

        assert_eq!(scratch.read("grows.md"), grown, "left untouched");
        assert!(report.written.is_empty() && report.failed.is_empty());
        assert_eq!(report.changed, [PathBuf::from("grows.md")]);
    }
}
