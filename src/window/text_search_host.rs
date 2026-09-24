//! Text search scheduling (note-search spec §7): the 150 ms debounce, the generation that makes a
//! late batch stale, the cancel flag, the worker thread and the narrowing record. The worker runs
//! `library::text_search::run` and posts its batches to the main window, which hands the current
//! generation's to the Search view (`search_view::apply_batch`).
//!
//! The timer lives on the main window, as `LIBRARY_WRITE_TIMER_ID` does, not on the panel as spec
//! §7 said: every other timer is the main window's, and the panel goes away with notes mode.

use crate::document::DocumentId;
use crate::library::text_search::{self, Progress, RunEnd, SearchNote, TextHit};
use crate::library::{self, NoteEntry, path_key};
use crate::search::{MatchOptions, Matcher};
use crate::window::{library_host, search_view};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

pub(crate) const TEXT_SEARCH_TIMER_ID: usize = 0x4650_5453;
pub(crate) const DEBOUNCE_MS: u32 = 150;
/// The shortest query that runs, in characters (spec §4).
pub(crate) const MIN_QUERY_CHARS: usize = 2;

/// One `WM_FASTPAD_TEXT_SEARCH_BATCH` payload, posted as `Box::into_raw` in `lparam`.
/// `batch_arrived` frees it; a post that fails is freed on the worker.
#[derive(Debug)]
pub(crate) struct SearchBatch {
    pub generation: u64,
    pub hits: Vec<TextHit>,
    pub progress: Progress,
    /// `Some` only on the last batch of a run that was not cancelled.
    pub end: Option<RunEnd>,
    /// The notes the run skipped (online only, too large, unreadable or not text), on the `end`
    /// batch only. A narrowed search visits them again.
    pub skipped: Vec<PathBuf>,
}

/// What the last completed search found, so a longer plain query visits only those notes.
#[derive(Clone, Debug)]
struct Narrowing {
    query: String,
    options: MatchOptions,
    /// Its hits and the notes it skipped.
    paths: Vec<PathBuf>,
    /// The `note_mark` of the notes it searched and the `overlay_mark` of the tab texts it read.
    /// A note added, removed or saved, or an edit in a dirty tab, makes the old hits unsafe.
    notes: u64,
    overlays: u64,
}

/// The search that is running, recorded as `Narrowing` if it completes.
#[derive(Clone, Debug)]
struct Running {
    query: String,
    options: MatchOptions,
    notes: u64,
    overlays: u64,
}

#[derive(Debug, Default)]
pub(crate) struct TextSearchHost {
    /// Bumped by every start and every cancel: a batch of any other generation is stale.
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    previous: Option<Narrowing>,
    running: Option<Running>,
    /// The `list_mark` of the note list the last search started over, so a library change runs
    /// the query again only when a note was added, removed or renamed (spec §7). `None` after a
    /// reload, which always runs it again.
    list: Option<u64>,
}

fn with_host<R>(hwnd: HWND, f: impl FnOnce(&mut TextSearchHost) -> R) -> Option<R> {
    unsafe { super::main_window::app_ptr(hwnd) }
        .map(|mut app| f(&mut unsafe { app.as_mut() }.text_search))
}

/// Whether `query` runs: at least `MIN_QUERY_CHARS` characters, not all white space.
pub(crate) fn searchable(query: &str) -> bool {
    query.chars().count() >= MIN_QUERY_CHARS && !query.trim().is_empty()
}

/// A mark of the note list: the paths in order and whether each is online-only. Sizes and times
/// are left out, so FastPad's own save of a listed note is no change.
fn list_mark(notes: &[NoteEntry]) -> u64 {
    let mut hasher = DefaultHasher::new();
    notes.len().hash(&mut hasher);
    for note in notes {
        note.path.hash(&mut hasher);
        note.online_only.hash(&mut hasher);
    }
    hasher.finish()
}

/// A strict mark of the notes, for narrowing: `list_mark` plus each note's size and time, so a
/// save by FastPad (which may add the phrase a longer query looks for) ends narrowing.
fn note_mark(notes: &[NoteEntry]) -> u64 {
    let mut hasher = DefaultHasher::new();
    notes.len().hash(&mut hasher);
    for note in notes {
        note.path.hash(&mut hasher);
        note.online_only.hash(&mut hasher);
        note.size.hash(&mut hasher);
        note.mtime.hash(&mut hasher);
    }
    hasher.finish()
}

/// A mark of the dirty tabs' texts, the same whatever order the map iterates in.
fn overlay_mark(overlays: &HashMap<PathBuf, String>) -> u64 {
    overlays
        .iter()
        .fold(overlays.len() as u64, |mark, (path, text)| {
            let mut hasher = DefaultHasher::new();
            path_key(path).hash(&mut hasher);
            text.hash(&mut hasher);
            mark.wrapping_add(hasher.finish())
        })
}

/// Whether `query` may visit only `previous`'s hits: a plain query without whole word (a longer
/// whole word can match where the shorter one was no whole word), the same options, containing
/// the previous query, over the same tab texts. The note list is checked by the caller.
fn narrows(previous: &Narrowing, query: &str, options: MatchOptions, overlays: u64) -> bool {
    !options.regex
        && !options.whole_word
        && previous.options == options
        && query.contains(previous.query.as_str())
        && previous.overlays == overlays
}

/// A keystroke in the box: cancels any running search and restarts the debounce.
pub(crate) fn schedule(hwnd: HWND) {
    cancel(hwnd);
    unsafe {
        SetTimer(hwnd, TEXT_SEARCH_TIMER_ID, DEBOUNCE_MS, None);
    }
}

/// `WM_TIMER` for `TEXT_SEARCH_TIMER_ID`: the debounce ended. Inside a nested modal loop or while
/// a file is populated it does nothing and leaves the timer armed, so it tries again at the next
/// tick: reading the dirty tabs swaps editor documents, which neither may see.
pub(crate) fn timer(hwnd: HWND) {
    if busy(hwnd) {
        return;
    }
    run_now(hwnd);
}

/// A modal loop runs or a file is being populated (the guard `snapshot_next_document` uses).
fn busy(hwnd: HWND) -> bool {
    super::main_window::file_population_active(hwnd) || super::modal::modal_active(hwnd)
}

/// Cancels, kills the timer, and starts the search for the view's query and options now. It does
/// nothing for a query that is too short or all white space, with no notebook, or while the
/// library loads (`LIBRARY_READY` runs it through `library_changed`). A bad pattern shows its
/// error and runs nothing. Called inside a modal loop or while a file is populated, it waits for
/// the debounce timer instead (`timer`).
pub(crate) fn run_now(hwnd: HWND) {
    cancel(hwnd);
    if busy(hwnd) {
        unsafe {
            SetTimer(hwnd, TEXT_SEARCH_TIMER_ID, DEBOUNCE_MS, None);
        }
        return;
    }
    let Some((query, options)) = search_view::current_query(hwnd) else {
        return;
    };
    if !searchable(&query) {
        return;
    }
    // Compiled first, so a bad pattern shows even while the notebook loads or none is open.
    let matcher = match Matcher::new(&query, options) {
        Ok(matcher) => matcher,
        Err(error) => {
            search_view::set_pattern_error(hwnd, Some(error.message));
            return;
        }
    };
    let Some(notebook) = library_host::folder(hwnd) else {
        return;
    };
    if library_host::with_state(hwnd, |_| ()).is_none() {
        return;
    }
    // Read with nothing of the App borrowed: a background dirty tab is swapped into the editor.
    let overlays = dirty_overlays(hwnd, &notebook);
    let overlays_mark = overlay_mark(&overlays);
    let candidate = with_host(hwnd, |host| host.previous.clone())
        .flatten()
        .filter(|previous| narrows(previous, &query, options, overlays_mark));
    let Some((notes, list, marked)) = library_host::with_state(hwnd, |state| {
        let list = list_mark(&state.notes);
        let marked = note_mark(&state.notes);
        let keep: Option<HashSet<String>> = candidate
            .filter(|previous| previous.notes == marked)
            .map(|previous| previous.paths.iter().map(|path| path_key(path)).collect());
        let notes = state
            .notes
            .iter()
            .filter(|note| {
                keep.as_ref()
                    .is_none_or(|keep| keep.contains(&path_key(&note.path)))
            })
            .map(SearchNote::from)
            .collect::<Vec<_>>();
        (notes, list, marked)
    }) else {
        return;
    };
    let total = notes.len();
    let Some((generation, cancel)) = with_host(hwnd, |host| {
        host.generation = host.generation.wrapping_add(1);
        let flag = Arc::new(AtomicBool::new(false));
        host.cancel = Some(Arc::clone(&flag));
        host.list = Some(list);
        host.running = Some(Running {
            query: query.clone(),
            options,
            notes: marked,
            overlays: overlays_mark,
        });
        (host.generation, flag)
    }) else {
        return;
    };
    search_view::begin_search(hwnd, &query, total);
    spawn(
        hwnd,
        Job {
            notebook,
            notes,
            overlays,
            matcher,
            generation,
            cancel,
        },
    );
}

/// Everything the worker owns. It is dropped, note copy and overlays included, when it ends.
struct Job {
    notebook: PathBuf,
    notes: Vec<SearchNote>,
    overlays: HashMap<PathBuf, String>,
    matcher: Matcher,
    generation: u64,
    cancel: Arc<AtomicBool>,
}

fn spawn(hwnd: HWND, job: Job) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let Job {
            notebook,
            notes,
            overlays,
            matcher,
            generation,
            cancel,
        } = job;
        let post = |batch: SearchBatch| -> bool {
            let payload = Box::into_raw(Box::new(batch));
            let posted = unsafe {
                PostMessageW(
                    target as HWND,
                    crate::window::WM_FASTPAD_TEXT_SEARCH_BATCH,
                    0,
                    payload as isize,
                )
            } != 0;
            if !posted {
                // The window is gone: nothing else will free it.
                drop(unsafe { Box::from_raw(payload) });
            }
            posted
        };
        let mut last = Progress::default();
        let mut sink = |hits: Vec<TextHit>, progress: Progress| {
            last = progress;
            let batch = SearchBatch {
                generation,
                hits,
                progress,
                end: None,
                skipped: Vec::new(),
            };
            if !post(batch) {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let mut skipped = Vec::new();
        let end = text_search::run_noting_skipped(
            &notebook,
            &notes,
            &overlays,
            &matcher,
            &cancel,
            &mut sink,
            &mut skipped,
        );
        if end != RunEnd::Cancelled && !cancel.load(Ordering::Relaxed) {
            post(SearchBatch {
                generation,
                hits: Vec::new(),
                progress: last,
                end: Some(end),
                skipped,
            });
        }
    });
}

/// `WM_FASTPAD_TEXT_SEARCH_BATCH`: takes the box back. A batch of any generation but the current
/// one is dropped. A completed search is recorded for narrowing.
pub(crate) fn batch_arrived(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let mut batch = *unsafe { Box::from_raw(lparam as *mut SearchBatch) };
    if with_host(hwnd, |host| host.generation) != Some(batch.generation) {
        return;
    }
    let end = batch.end;
    let skipped = std::mem::take(&mut batch.skipped);
    search_view::apply_batch(hwnd, batch);
    let Some(end) = end else {
        return;
    };
    let paths = if end == RunEnd::Completed {
        let mut paths = search_view::result_paths(hwnd);
        paths.extend(skipped);
        paths
    } else {
        Vec::new()
    };
    with_host(hwnd, |host| {
        host.cancel = None;
        host.previous = match (end, host.running.take()) {
            (RunEnd::Completed, Some(run)) => Some(Narrowing {
                query: run.query,
                options: run.options,
                paths,
                notes: run.notes,
                overlays: run.overlays,
            }),
            _ => None,
        };
    });
}

/// Stops the running search and the debounce. Batches it already posted are stale from here on.
pub(crate) fn cancel(hwnd: HWND) {
    with_host(hwnd, |host| {
        if let Some(flag) = host.cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
        host.generation = host.generation.wrapping_add(1);
        host.running = None;
    });
    unsafe {
        KillTimer(hwnd, TEXT_SEARCH_TIMER_ID);
    }
}

/// A notebook change or close, or notes mode off: cancels and forgets what the old notebook's
/// searches found.
pub(crate) fn forget(hwnd: HWND) {
    cancel(hwnd);
    with_host(hwnd, |host| {
        host.previous = None;
        host.list = None;
    });
}

/// The library was loaded or rescanned (`library_host::install`): the next library change runs
/// the query again even if no note was added or removed, since the files may have changed.
/// A search still running read the files before the rescan, so its end records no narrowing.
pub(crate) fn notes_reloaded(hwnd: HWND) {
    with_host(hwnd, |host| {
        host.previous = None;
        host.running = None;
        host.list = None;
    });
}

/// The shown Search view's library changed (`search_view::library_changed`, `shown`): runs the
/// query again when the note list moved on since the last search. The run waits for the
/// debounce timer, so the overlays (which may swap a tab into the editor) are never read inside
/// an install, a save or a notebook change. A search already running goes on until then.
pub(crate) fn library_changed(hwnd: HWND) {
    let Some((query, _)) = search_view::current_query(hwnd) else {
        return;
    };
    if !searchable(&query) {
        return;
    }
    let Some(list) = library_host::with_state(hwnd, |state| list_mark(&state.notes)) else {
        return;
    };
    if with_host(hwnd, |host| host.list) == Some(Some(list)) {
        return;
    }
    unsafe {
        SetTimer(hwnd, TEXT_SEARCH_TIMER_ID, DEBOUNCE_MS, None);
    }
}

/// The dirty tabs whose file is inside `notebook`, keyed by the path relative to it, with the
/// editor's text. Clean and untitled tabs are left out: the disk has their text, or they are no
/// note. Call it with nothing of the App borrowed.
pub(crate) fn dirty_overlays(hwnd: HWND, notebook: &Path) -> HashMap<PathBuf, String> {
    let dirty: Vec<(DocumentId, PathBuf)> = unsafe { super::main_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| document.dirty)
                .filter_map(|document| {
                    let path = document.path.as_deref()?;
                    library::is_inside(notebook, path)
                        .then(|| (document.id, library::record_path(notebook, path)))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut overlays = HashMap::with_capacity(dirty.len());
    for (id, relative) in dirty {
        if let Some(text) = super::main_window::document_text(hwnd, id) {
            overlays.insert(relative, text);
        }
    }
    overlays
}

#[cfg(test)]
pub(crate) fn generation(hwnd: HWND) -> u64 {
    with_host(hwnd, |host| host.generation).unwrap_or(0)
}

/// The running search's cancel flag, `None` while nothing runs.
#[cfg(test)]
pub(crate) fn cancel_flag(hwnd: HWND) -> Option<Arc<AtomicBool>> {
    with_host(hwnd, |host| host.cancel.clone()).flatten()
}

/// A boxed batch, as the worker posts it.
#[cfg(test)]
pub(crate) fn test_batch(generation: u64, hits: Vec<TextHit>, end: Option<RunEnd>) -> LPARAM {
    Box::into_raw(Box::new(SearchBatch {
        generation,
        hits,
        progress: Progress::default(),
        end,
        skipped: Vec::new(),
    })) as LPARAM
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_QUERY_CHARS, Narrowing, list_mark, narrows, note_mark, overlay_mark, searchable,
    };
    use crate::library::NoteEntry;
    use crate::search::MatchOptions;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn a_query_runs_from_two_characters_and_never_when_it_is_all_white_space() {
        // Break caught: a one-letter query reading every note in the notebook, a two-byte letter
        // counted as two characters, or a query of spaces running at all.
        assert_eq!(MIN_QUERY_CHARS, 2);
        assert!(!searchable(""));
        assert!(!searchable("a"));
        assert!(!searchable("é"), "one character in two bytes");
        assert!(searchable("ab"));
        assert!(searchable("éa"));
        assert!(searchable(" a"), "a leading space is part of the phrase");
        assert!(!searchable("   "));
        assert!(!searchable("\t\t"));
    }

    fn note(path: &str, online_only: bool) -> NoteEntry {
        NoteEntry {
            path: PathBuf::from(path),
            size: 10,
            mtime: 20,
            online_only,
        }
    }

    #[test]
    fn the_list_mark_follows_the_note_paths_and_not_their_sizes() {
        // Break caught: a save of a listed note re-running the search on the next refresh, or a
        // note added, renamed or made online-only leaving the old results in place.
        let base = list_mark(&[note("a.md", false), note("b.md", false)]);
        assert_eq!(base, list_mark(&[note("a.md", false), note("b.md", false)]));
        assert_ne!(base, list_mark(&[note("a.md", false)]), "a note removed");
        assert_ne!(
            base,
            list_mark(&[note("a.md", false), note("c.md", false)]),
            "a note renamed"
        );
        assert_ne!(
            base,
            list_mark(&[note("a.md", false), note("b.md", true)]),
            "a note went online-only"
        );
        let mut saved = note("b.md", false);
        saved.size = 99;
        saved.mtime = 7;
        assert_eq!(
            base,
            list_mark(&[note("a.md", false), saved]),
            "a save of a listed note is no list change"
        );
    }

    #[test]
    fn the_overlay_mark_changes_with_any_tab_text_and_not_with_order() {
        // Break caught: narrowing kept after an edit in a dirty tab, so a phrase typed there after
        // the last search is never found.
        let mut first = HashMap::new();
        first.insert(PathBuf::from("a.md"), "one".to_owned());
        first.insert(PathBuf::from("b.md"), "two".to_owned());
        let mut second = HashMap::new();
        second.insert(PathBuf::from("b.md"), "two".to_owned());
        second.insert(PathBuf::from("a.md"), "one".to_owned());
        assert_eq!(overlay_mark(&first), overlay_mark(&second));
        second.insert(PathBuf::from("a.md"), "one!".to_owned());
        assert_ne!(overlay_mark(&first), overlay_mark(&second));
        assert_ne!(overlay_mark(&HashMap::new()), overlay_mark(&first));
    }

    #[test]
    fn the_note_mark_also_follows_sizes_and_times() {
        // Break caught: narrowing kept after FastPad saved a new phrase into a listed note, which
        // changes its size and time but not the list of paths.
        let base = note_mark(&[note("a.md", false)]);
        assert_eq!(base, note_mark(&[note("a.md", false)]));
        let mut saved = note("a.md", false);
        saved.size = 99;
        assert_ne!(base, note_mark(std::slice::from_ref(&saved)), "a new size");
        saved.size = 10;
        saved.mtime = 7;
        assert_ne!(base, note_mark(&[saved]), "a new time");
        assert_ne!(base, note_mark(&[note("a.md", true)]), "online-only");
        assert_ne!(base, note_mark(&[note("b.md", false)]), "another path");
    }

    #[test]
    fn narrowing_needs_a_longer_plain_query_with_the_same_options_and_tab_texts() {
        // Break caught: a whole-word or regex query narrowed to an earlier query's hits ("xfoo"
        // contains "foo", but "foo" is not a whole word in "xfoo"), a shorter query narrowed, or
        // narrowing across a change of options.
        let plain = MatchOptions::default();
        let previous = Narrowing {
            query: "foo".to_owned(),
            options: plain,
            paths: Vec::new(),
            notes: 1,
            overlays: 2,
        };
        assert!(narrows(&previous, "foo", plain, 2), "the same query again");
        assert!(narrows(&previous, "food", plain, 2));
        assert!(narrows(&previous, "a foo", plain, 2));
        assert!(
            !narrows(&previous, "fo", plain, 2),
            "a shorter query can match more"
        );
        assert!(
            !narrows(&previous, "Foo", plain, 2),
            "contained only after folding"
        );
        let case = MatchOptions {
            case: true,
            ..plain
        };
        assert!(!narrows(&previous, "food", case, 2), "the options changed");
        let whole = MatchOptions {
            whole_word: true,
            ..plain
        };
        let previous_whole = Narrowing {
            options: whole,
            ..previous.clone()
        };
        assert!(!narrows(&previous_whole, "xfoo", whole, 2));
        let regex = MatchOptions {
            regex: true,
            ..plain
        };
        let previous_regex = Narrowing {
            options: regex,
            ..previous.clone()
        };
        assert!(!narrows(&previous_regex, "foo|bar", regex, 2));
        assert!(
            !narrows(&previous, "food", plain, 3),
            "a dirty tab's text changed"
        );
    }
}
