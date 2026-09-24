//! Text search over a notebook, run on a worker thread: each note's text (from disk, or from a
//! dirty tab's overlay) is matched, and each note's first match goes to a sink in batches with
//! the progress so far. A few reader threads open and match the notes in parallel; the worker
//! thread itself batches what they find. No Win32 and no window.

use super::name_search::folder_of;
use super::path_key;
use super::tree::natural_cmp;
use crate::file::encoding;
use crate::search::{Matcher, Snippet, first_snippet};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A search stops after this many matching notes.
pub const RESULT_CAP: usize = 500;
/// A larger note is skipped as `TooLarge`.
pub const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;
/// A batch goes out when it holds this many hits...
pub const BATCH_HITS: usize = 50;
/// ...or when this long has passed since the last one.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);
/// At most this many threads read notes at once. Opening and reading each file dominates a
/// search, and on the reference four-core machine four readers keep the disk cache and the
/// antivirus filter busy without starving the UI thread.
const MAX_READERS: usize = 4;

/// How many reader threads a search uses: `MAX_READERS`, fewer on a machine with fewer cores.
fn reader_count() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, MAX_READERS)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchNote {
    /// Relative to the notebook.
    pub path: PathBuf,
    /// The scan's size.
    pub size: u64,
    pub online_only: bool,
}

impl From<&super::NoteEntry> for SearchNote {
    fn from(note: &super::NoteEntry) -> Self {
        SearchNote {
            path: note.path.clone(),
            size: note.size,
            online_only: note.online_only,
        }
    }
}

/// What the search read: the opened file's size and last write time (FILETIME ticks).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub size: u64,
    pub mtime: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextHit {
    /// Relative to the notebook, as the note list has it.
    pub path: PathBuf,
    /// The file name without its extension.
    pub name: String,
    /// The parent folder joined with `\`; `""` at the root.
    pub folder: String,
    pub snippet: Snippet,
    /// `None` when the text came from an overlay.
    pub stamp: Option<Stamp>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipReason {
    OnlineOnly = 0,
    TooLarge = 1,
    Unreadable = 2,
    NotText = 3,
}

impl SkipReason {
    /// In the order the status line's tooltip lists them.
    pub const ALL: [SkipReason; 4] = [
        Self::OnlineOnly,
        Self::TooLarge,
        Self::Unreadable,
        Self::NotText,
    ];

    /// The index into `Progress::skipped`.
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Progress {
    /// Notes processed so far, skipped ones included.
    pub visited: usize,
    pub total: usize,
    /// Skipped notes by `SkipReason::index`.
    pub skipped: [usize; 4],
}

impl Progress {
    pub fn skipped_total(&self) -> usize {
        self.skipped.iter().sum()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunEnd {
    /// Every note was visited.
    Completed,
    /// `RESULT_CAP` notes matched before the last note.
    Capped,
    Cancelled,
}

/// Natural name order, then the folder (the root first), then the exact path.
pub fn hit_cmp(a: &TextHit, b: &TextHit) -> Ordering {
    natural_cmp(&a.name, &b.name)
        .then_with(|| natural_cmp(&a.folder, &b.folder))
        .then_with(|| a.path.cmp(&b.path))
}

/// Searches `notes` and streams each note's first match to `sink`.
///
/// - Notes are read by up to `MAX_READERS` threads at once, each taking the next note in the
///   list's order, so they finish (and are counted and batched) in about that order but not
///   exactly; which notes a capped search found is not fixed. The UI sorts the hits anyway.
/// - `overlays` holds dirty tabs' text by relative path (compared ignoring case). An overlay is
///   searched instead of the disk, even for a note that is online only or over the size limit.
/// - A batch goes to `sink` at `BATCH_HITS` hits, or when `BATCH_INTERVAL` has passed since the
///   last batch (checked after each note, so a batch can be empty and carry only progress), and
///   once more at the end even if empty.
/// - At `RESULT_CAP` hits the batch holding the last one is sent and the search ends `Capped`,
///   unless that note was the last, which ends `Completed`.
/// - `cancel` is read by each reader before each note, and by the batching loop before it counts
///   each note and before the final batch. Once it is set, nothing more is sent.
pub fn run(
    notebook: &Path,
    notes: &[SearchNote],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
) -> RunEnd {
    run_with_clock(
        notebook,
        notes,
        overlays,
        matcher,
        cancel,
        sink,
        &mut Instant::now,
        reader_count(),
    )
}

/// `run`, also pushing the path of every note it skipped onto `skipped`, so a narrowed search
/// can visit them again.
pub fn run_noting_skipped(
    notebook: &Path,
    notes: &[SearchNote],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
    skipped: &mut Vec<PathBuf>,
) -> RunEnd {
    search_all(
        notebook,
        notes,
        overlays,
        matcher,
        cancel,
        sink,
        &mut Instant::now,
        skipped,
        reader_count(),
    )
}

/// `run` with the clock and the reader count passed in, so tests can step time and, with one
/// reader, fix the order notes are counted in. The clock is read once at the start and once
/// after each note.
#[expect(
    clippy::too_many_arguments,
    reason = "`run`'s arguments plus the test clock and reader count"
)]
fn run_with_clock(
    notebook: &Path,
    notes: &[SearchNote],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
    clock: &mut dyn FnMut() -> Instant,
    readers: usize,
) -> RunEnd {
    search_all(
        notebook,
        notes,
        overlays,
        matcher,
        cancel,
        sink,
        clock,
        &mut Vec::new(),
        readers,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "`run_noting_skipped`'s arguments plus the test clock and reader count"
)]
fn search_all(
    notebook: &Path,
    notes: &[SearchNote],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
    clock: &mut dyn FnMut() -> Instant,
    skipped: &mut Vec<PathBuf>,
    readers: usize,
) -> RunEnd {
    let cancelled = || cancel.load(Relaxed);
    let overlays: Vec<(String, &str)> = overlays
        .iter()
        .map(|(path, text)| (path_key(path), text.as_str()))
        .collect();
    let overlays = overlays.as_slice();
    // The next note a reader takes, and whether the batching loop has stopped (capped or
    // cancelled), so readers don't start another note.
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (sender, outcomes) = mpsc::channel::<(usize, Searched)>();
        for _ in 0..readers.clamp(1, notes.len().max(1)) {
            let sender = sender.clone();
            let (next, stopped) = (&next, &stopped);
            scope.spawn(move || {
                // One buffer per reader for every file it reads; freed when the search ends.
                let mut bytes = Vec::new();
                while !stopped.load(Relaxed) && !cancelled() {
                    let index = next.fetch_add(1, Relaxed);
                    let Some(note) = notes.get(index) else {
                        break;
                    };
                    let outcome = search_note(notebook, note, overlays, matcher, &mut bytes);
                    if sender.send((index, outcome)).is_err() {
                        break;
                    }
                }
            });
        }
        // The loop below ends when every reader has finished and dropped its sender.
        drop(sender);
        let end = batch_outcomes(notes, outcomes, &cancelled, sink, clock, skipped);
        stopped.store(true, Relaxed);
        end
    })
}

/// The batching loop: counts each note's outcome as the readers send it, and sends batches,
/// honoring the cap and `cancelled`.
fn batch_outcomes(
    notes: &[SearchNote],
    outcomes: mpsc::Receiver<(usize, Searched)>,
    cancelled: &dyn Fn() -> bool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
    clock: &mut dyn FnMut() -> Instant,
    skipped: &mut Vec<PathBuf>,
) -> RunEnd {
    let mut progress = Progress {
        total: notes.len(),
        ..Progress::default()
    };
    let mut batch = Vec::new();
    let mut hits = 0;
    let mut last_sent = clock();
    for (index, outcome) in outcomes {
        if cancelled() {
            return RunEnd::Cancelled;
        }
        match outcome {
            Searched::Hit(hit) => {
                batch.push(hit);
                hits += 1;
            }
            Searched::NoMatch => {}
            Searched::Skipped(reason) => {
                progress.skipped[reason.index()] += 1;
                skipped.push(notes[index].path.clone());
            }
        }
        progress.visited += 1;
        if hits == RESULT_CAP && progress.visited < progress.total {
            sink(std::mem::take(&mut batch), progress);
            return RunEnd::Capped;
        }
        let now = clock();
        if batch.len() >= BATCH_HITS || now.saturating_duration_since(last_sent) >= BATCH_INTERVAL {
            sink(std::mem::take(&mut batch), progress);
            last_sent = now;
        }
    }
    if cancelled() {
        return RunEnd::Cancelled;
    }
    sink(batch, progress);
    RunEnd::Completed
}

enum Searched {
    Hit(TextHit),
    NoMatch,
    Skipped(SkipReason),
}

fn search_note(
    notebook: &Path,
    note: &SearchNote,
    overlays: &[(String, &str)],
    matcher: &Matcher,
    bytes: &mut Vec<u8>,
) -> Searched {
    let overlay = if overlays.is_empty() {
        None
    } else {
        let key = path_key(&note.path);
        overlays
            .iter()
            .find(|(overlay, _)| *overlay == key)
            .map(|(_, text)| *text)
    };
    let (snippet, stamp) = match overlay {
        Some(text) => (first_snippet(text, matcher), None),
        None => match read_note(notebook, note, bytes) {
            Ok((text, stamp)) => (first_snippet(&text, matcher), Some(stamp)),
            Err(reason) => return Searched::Skipped(reason),
        },
    };
    let Some(snippet) = snippet else {
        return Searched::NoMatch;
    };
    Searched::Hit(TextHit {
        path: note.path.clone(),
        name: note
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        folder: folder_of(&note.path),
        snippet,
        stamp,
    })
}

/// The note's text from disk and the stamp of what was read. An online-only note is never
/// opened, so a search never recalls it; a note over the limit by the scan's size is not opened
/// either, and one that grew past it since is not read past the limit.
fn read_note(
    notebook: &Path,
    note: &SearchNote,
    bytes: &mut Vec<u8>,
) -> Result<(String, Stamp), SkipReason> {
    if note.online_only {
        return Err(SkipReason::OnlineOnly);
    }
    if note.size > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    let file =
        std::fs::File::open(notebook.join(&note.path)).map_err(|_| SkipReason::Unreadable)?;
    let metadata = file.metadata().map_err(|_| SkipReason::Unreadable)?;
    if metadata.len() > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    bytes.clear();
    file.take(MAX_NOTE_BYTES + 1)
        .read_to_end(bytes)
        .map_err(|_| SkipReason::Unreadable)?;
    if bytes.len() as u64 > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    let decoded = encoding::decode(bytes).map_err(|_| SkipReason::NotText)?;
    Ok((
        decoded.text,
        Stamp {
            size: metadata.len(),
            mtime: metadata.last_write_time(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::MatchOptions;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fastpad-text-search-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        /// Writes `bytes` at `relative` and returns the note as a scan would list it.
        fn file(&self, relative: &str, bytes: &[u8]) -> SearchNote {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            note(relative, bytes.len() as u64)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn note(relative: &str, size: u64) -> SearchNote {
        SearchNote {
            path: PathBuf::from(relative),
            size,
            online_only: false,
        }
    }

    fn find(query: &str) -> Matcher {
        Matcher::new(query, MatchOptions::default()).unwrap()
    }

    type Batches = Vec<(Vec<TextHit>, Progress)>;

    /// Runs with `MAX_READERS` readers, or with one for a test that needs the notes counted in
    /// the list's order.
    fn collect_with(
        notebook: &Path,
        notes: &[SearchNote],
        overlays: &HashMap<PathBuf, String>,
        matcher: &Matcher,
        clock: &mut dyn FnMut() -> Instant,
        readers: usize,
    ) -> (RunEnd, Batches) {
        let mut batches = Vec::new();
        let cancel = AtomicBool::new(false);
        let end = run_with_clock(
            notebook,
            notes,
            overlays,
            matcher,
            &cancel,
            &mut |hits, progress| batches.push((hits, progress)),
            clock,
            readers,
        );
        (end, batches)
    }

    fn collect(
        notebook: &Path,
        notes: &[SearchNote],
        overlays: &HashMap<PathBuf, String>,
        matcher: &Matcher,
        clock: &mut dyn FnMut() -> Instant,
    ) -> (RunEnd, Batches) {
        collect_with(notebook, notes, overlays, matcher, clock, MAX_READERS)
    }

    fn search(
        notebook: &Path,
        notes: &[SearchNote],
        overlays: &HashMap<PathBuf, String>,
        query: &str,
    ) -> (RunEnd, Vec<TextHit>, Progress) {
        let (end, batches) = collect(notebook, notes, overlays, &find(query), &mut frozen_clock());
        let progress = batches.last().map(|(_, progress)| *progress).unwrap();
        let mut hits: Vec<TextHit> = batches.into_iter().flat_map(|(hits, _)| hits).collect();
        // Readers finish in no fixed order; the UI sorts by `hit_cmp` too.
        hits.sort_by(hit_cmp);
        (end, hits, progress)
    }

    fn names(hits: &[TextHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.name.as_str()).collect()
    }

    /// A clock that never moves, so only the hit count sends batches.
    fn frozen_clock() -> impl FnMut() -> Instant {
        let start = Instant::now();
        move || start
    }

    /// A clock that moves `step` further on each read, starting at no time at all.
    fn stepping_clock(step: Duration) -> impl FnMut() -> Instant {
        let start = Instant::now();
        let mut reads = 0;
        move || {
            let now = start + step * reads;
            reads += 1;
            now
        }
    }

    /// `count` overlay-only notes, all containing "needle", so nothing touches the disk.
    fn overlay_notes(count: usize) -> (Vec<SearchNote>, HashMap<PathBuf, String>) {
        let notes: Vec<SearchNote> = (0..count)
            .map(|index| note(&format!("n{index}.md"), 10))
            .collect();
        let overlays = notes
            .iter()
            .map(|note| (note.path.clone(), "a needle here".to_owned()))
            .collect();
        (notes, overlays)
    }

    #[test]
    fn a_hit_carries_its_name_folder_snippet_and_the_stamp_of_what_was_read() {
        // Break caught: a replace (3b) comparing against a stamp the search never took, or a
        // result row showing the extension or a folder with `/`.
        let scratch = Scratch::new("hit");
        let a = scratch.file(
            r"work\q1\Budget plan.md",
            b"Title\r\n\r\n    paid the invoice\r\n",
        );
        let b = scratch.file("todo.txt", b"nothing to see");
        let (end, hits, progress) = search(&scratch.0, &[a, b], &HashMap::new(), "INVOICE");
        assert_eq!(end, RunEnd::Completed);
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.path, PathBuf::from(r"work\q1\Budget plan.md"));
        assert_eq!(hit.name, "Budget plan");
        assert_eq!(hit.folder, r"work\q1");
        assert_eq!(hit.snippet.text, "paid the invoice");
        assert_eq!(&hit.snippet.text[hit.snippet.highlight.clone()], "invoice");
        let metadata = std::fs::metadata(scratch.0.join(&hit.path)).unwrap();
        assert_eq!(
            hit.stamp,
            Some(Stamp {
                size: metadata.len(),
                mtime: metadata.last_write_time(),
            })
        );
        assert_eq!(
            progress,
            Progress {
                visited: 2,
                total: 2,
                skipped: [0; 4],
            }
        );
    }

    #[test]
    fn an_overlay_wins_over_the_disk_both_ways() {
        // Break caught: searching the saved file of a note open with unsaved edits, so a phrase
        // only typed in the editor is missed, or a phrase deleted in the editor still shows.
        let scratch = Scratch::new("overlay");
        let edited = scratch.file("Edited.md", b"the old phrase");
        let untouched = scratch.file("Untouched.md", b"the old phrase and the new phrase");
        let notes = [edited, untouched];
        // The key differs in case from the note list's path; paths compare ignoring case.
        let overlays = HashMap::from([(PathBuf::from("EDITED.md"), "the new phrase".to_owned())]);
        let (_, hits, _) = search(&scratch.0, &notes, &overlays, "new phrase");
        assert_eq!(names(&hits), ["Edited", "Untouched"]);
        assert_eq!(
            hits[0].path,
            PathBuf::from("Edited.md"),
            "the note list's spelling"
        );
        assert_eq!(hits[0].stamp, None, "an overlay has no stamp");
        assert!(hits[1].stamp.is_some());
        let (_, hits, _) = search(&scratch.0, &notes, &overlays, "old phrase");
        assert_eq!(names(&hits), ["Untouched"]);
    }

    #[test]
    fn an_overlay_is_searched_even_for_an_online_only_or_oversized_note() {
        let scratch = Scratch::new("overlay-skips");
        let mut online = note("Online.md", 10);
        online.online_only = true;
        let large = note("Large.md", MAX_NOTE_BYTES + 1);
        let overlays = HashMap::from([
            (PathBuf::from("Online.md"), "typed needle".to_owned()),
            (PathBuf::from("Large.md"), "typed needle".to_owned()),
        ]);
        let (_, hits, progress) = search(&scratch.0, &[online, large], &overlays, "needle");
        assert_eq!(names(&hits), ["Large", "Online"]);
        assert_eq!(progress.skipped_total(), 0);
    }

    #[test]
    fn skipped_notes_are_counted_by_reason_and_never_matched() {
        // Break caught: an online-only note opened (recalling it from the cloud), a huge file read
        // whole, or a binary file's bytes "matching".
        let scratch = Scratch::new("skips");
        let mut online = scratch.file("online.md", b"needle");
        online.online_only = true;
        let mut large = scratch.file("large.md", b"needle");
        large.size = MAX_NOTE_BYTES + 1;
        let missing = note("deleted since the scan.md", 6);
        let binary = scratch.file(
            "binary.txt",
            &[0x80, 0x81, b'n', b'e', b'e', b'd', b'l', b'e'],
        );
        let fine = scratch.file("fine.md", b"needle");
        let notes = [online, large, missing, binary, fine];
        let (end, hits, progress) = search(&scratch.0, &notes, &HashMap::new(), "needle");
        assert_eq!(end, RunEnd::Completed);
        assert_eq!(names(&hits), ["fine"]);
        assert_eq!(progress.visited, 5);
        assert_eq!(progress.total, 5);
        for reason in SkipReason::ALL {
            assert_eq!(progress.skipped[reason.index()], 1, "{reason:?}");
        }
        assert_eq!(progress.skipped_total(), 4);
        assert_eq!(
            SkipReason::ALL.map(SkipReason::index),
            [0, 1, 2, 3],
            "the tooltip's order"
        );
    }

    #[test]
    fn a_note_that_grew_past_the_limit_since_the_scan_is_too_large() {
        let scratch = Scratch::new("grew");
        let mut text = vec![b'x'; MAX_NOTE_BYTES as usize];
        text.extend_from_slice(b" needle");
        let mut grown = scratch.file("grown.md", &text);
        grown.size = 10;
        let (_, hits, progress) = search(&scratch.0, &[grown], &HashMap::new(), "needle");
        assert!(hits.is_empty());
        assert_eq!(progress.skipped[SkipReason::TooLarge.index()], 1);
    }

    #[test]
    fn utf16_and_bom_notes_are_decoded_before_matching() {
        let scratch = Scratch::new("encodings");
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "Ünïcode needle".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let wide = scratch.file("wide.txt", &utf16);
        let bom = scratch.file("bom.md", b"\xEF\xBB\xBFneedle first");
        let (_, hits, _) = search(&scratch.0, &[wide, bom], &HashMap::new(), "needle");
        assert_eq!(names(&hits), ["bom", "wide"]);
        assert_eq!(hits[0].snippet.text, "needle first");
        assert_eq!(hits[1].snippet.text, "Ünïcode needle");
    }

    #[test]
    fn only_the_listed_notes_are_visited() {
        // Break caught: narrowing (the host passing the previous hits) still reading every note.
        let scratch = Scratch::new("listed");
        let a = scratch.file("a.md", b"needle");
        let _b = scratch.file("b.md", b"needle");
        let c = scratch.file("c.md", b"needle");
        let (_, hits, progress) = search(&scratch.0, &[a, c], &HashMap::new(), "needle");
        assert_eq!(names(&hits), ["a", "c"]);
        assert_eq!(progress.total, 2);
    }

    #[test]
    fn batches_go_out_every_fifty_hits_and_a_final_batch_follows() {
        let scratch = Scratch::new("count-batches");
        let (notes, overlays) = overlay_notes(120);
        let (end, batches) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Completed);
        let sizes: Vec<usize> = batches.iter().map(|(hits, _)| hits.len()).collect();
        assert_eq!(sizes, [50, 50, 20]);
        let visited: Vec<usize> = batches
            .iter()
            .map(|(_, progress)| progress.visited)
            .collect();
        assert_eq!(visited, [50, 100, 120]);
        // The final batch goes out even when it is empty.
        let (notes, overlays) = overlay_notes(50);
        let (_, batches) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        let sizes: Vec<usize> = batches.iter().map(|(hits, _)| hits.len()).collect();
        assert_eq!(sizes, [50, 0]);
    }

    #[test]
    fn a_batch_goes_out_when_fifty_milliseconds_have_passed() {
        // Break caught: a slow notebook with few matches showing nothing, and no progress, until
        // the whole search ends.
        let scratch = Scratch::new("interval-batches");
        let notes: Vec<SearchNote> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|name| note(&format!("{name}.md"), 10))
            .collect();
        let overlays = HashMap::from([
            (PathBuf::from("a.md"), "needle".to_owned()),
            (PathBuf::from("b.md"), "hay".to_owned()),
            (PathBuf::from("c.md"), "hay".to_owned()),
            (PathBuf::from("d.md"), "hay".to_owned()),
            (PathBuf::from("e.md"), "needle".to_owned()),
        ]);
        // The clock reads 0 at the start and 30, 60, 90, 120, 150 ms after each note: batches go
        // out after the second note (60 ms since the start) and the fourth (60 ms since then).
        let mut clock = stepping_clock(Duration::from_millis(30));
        // One reader, so the notes are counted in the list's order.
        let (end, batches) = collect_with(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut clock,
            1,
        );
        assert_eq!(end, RunEnd::Completed);
        let shape: Vec<(Vec<&str>, usize)> = batches
            .iter()
            .map(|(hits, progress)| (names(hits), progress.visited))
            .collect();
        assert_eq!(
            shape,
            [(vec!["a"], 2), (vec![], 4), (vec!["e"], 5)],
            "an interval batch can carry only progress"
        );
    }

    #[test]
    fn the_cap_ends_the_search_after_the_capping_batch() {
        let scratch = Scratch::new("cap");
        let (notes, overlays) = overlay_notes(RESULT_CAP + 20);
        let (end, batches) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Capped);
        let found: usize = batches.iter().map(|(hits, _)| hits.len()).sum();
        assert_eq!(found, RESULT_CAP);
        assert_eq!(
            batches.len(),
            RESULT_CAP / BATCH_HITS,
            "no batch after the cap"
        );
        let last = batches.last().unwrap().1;
        assert_eq!((last.visited, last.total), (RESULT_CAP, RESULT_CAP + 20));
        // Exactly the cap, with nothing left to visit, is a completed search.
        let (notes, overlays) = overlay_notes(RESULT_CAP);
        let (end, _) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Completed);
    }

    #[test]
    fn the_notes_a_search_skips_are_named() {
        // Break caught: a narrowed search visiting only the last hits, so a note the last search
        // skipped drops out of the status line and a note unreadable for a moment is never tried
        // again.
        let scratch = Scratch::new("skipped");
        let plain = scratch.file("plain.md", b"needle");
        let mut cloud = scratch.file("cloud.md", b"needle");
        cloud.online_only = true;
        let missing = note("gone.md", 5);
        let mut skipped = Vec::new();
        let end = run_noting_skipped(
            &scratch.0,
            &[plain, cloud, missing],
            &HashMap::new(),
            &find("needle"),
            &AtomicBool::new(false),
            &mut |_, _| {},
            &mut skipped,
        );
        assert_eq!(end, RunEnd::Completed);
        skipped.sort();
        assert_eq!(
            skipped,
            [PathBuf::from("cloud.md"), PathBuf::from("gone.md")]
        );
    }

    #[test]
    fn a_cancelled_search_stops_at_once_and_sends_nothing_more() {
        // Break caught: typing fast leaving the old search reading the whole notebook, or posting
        // batches after the query that started it was replaced.
        let scratch = Scratch::new("cancel");
        let (notes, overlays) = overlay_notes(200);
        let matcher = find("needle");
        let cancel = AtomicBool::new(true);
        let mut calls = 0;
        let end = run(
            &scratch.0,
            &notes,
            &overlays,
            &matcher,
            &cancel,
            &mut |_, _| {
                calls += 1;
            },
        );
        assert_eq!((end, calls), (RunEnd::Cancelled, 0));

        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let end = run_with_clock(
            &scratch.0,
            &notes,
            &overlays,
            &matcher,
            &cancel,
            &mut |hits, progress| {
                seen.push((hits.len(), progress.visited));
                cancel.store(true, Relaxed);
            },
            &mut frozen_clock(),
            MAX_READERS,
        );
        assert_eq!(end, RunEnd::Cancelled);
        assert_eq!(seen, [(50, 50)]);
    }

    #[test]
    fn parallel_readers_visit_every_note_once_with_skips_and_hits_counted() {
        // Break caught: two readers taking the same note (a hit twice, visited past the total),
        // a note no reader takes, or skips lost between the readers and the batching loop.
        let scratch = Scratch::new("parallel");
        let mut notes = Vec::new();
        for index in 0..300 {
            let text = if index % 3 == 0 { "a needle" } else { "hay" };
            notes.push(scratch.file(&format!("n{index}.md"), text.as_bytes()));
        }
        for index in 0..7 {
            notes.push(note(&format!("gone{index}.md"), 5));
        }
        let (end, hits, progress) = search(&scratch.0, &notes, &HashMap::new(), "needle");
        assert_eq!(end, RunEnd::Completed);
        let mut paths: Vec<&Path> = hits.iter().map(|hit| hit.path.as_path()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!((hits.len(), paths.len()), (100, 100));
        assert_eq!((progress.visited, progress.total), (307, 307));
        assert_eq!(progress.skipped[SkipReason::Unreadable.index()], 7);
        assert!(reader_count() >= 1 && reader_count() <= MAX_READERS);
    }

    #[test]
    fn hits_sort_by_natural_name_then_folder_with_the_root_first_then_path() {
        let hit = |path: &str| {
            let path = PathBuf::from(path);
            TextHit {
                name: path.file_stem().unwrap().to_string_lossy().into_owned(),
                folder: folder_of(&path),
                path,
                snippet: Snippet::default(),
                stamp: None,
            }
        };
        let mut hits = [
            hit(r"b\Note 10.md"),
            hit(r"a\Note 2.md"),
            hit("Note 2.txt"),
            hit(r"a\Note 2.md.bak"),
            hit("note 2.md"),
            hit("Alpha.md"),
        ];
        hits.sort_by(hit_cmp);
        let order: Vec<String> = hits
            .iter()
            .map(|hit| hit.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            order,
            [
                "Alpha.md",
                "Note 2.txt",
                "note 2.md",
                r"a\Note 2.md",
                r"a\Note 2.md.bak",
                r"b\Note 10.md",
            ]
        );
    }
}
