//! Text search scheduling (note-search spec §7): the 150 ms debounce, the generation that makes a
//! late batch stale, the cancel flag, the worker thread and the narrowing record. The worker runs
//! `library::text_search::run` and posts its batches to the main window, which hands the current
//! generation's to the Search view (`search_view::apply_batch`).
//!
//! The timer lives on the main window, as `LIBRARY_WRITE_TIMER_ID` does, not on the panel as spec
//! §7 said: every other timer is the main window's, and the panel goes away with notes mode.

use crate::document::DocumentId;
use crate::library::model::same_path;
use crate::library::text_replace::{self, ReplaceCount, ReplaceReport, ReplaceTarget};
use crate::library::text_search::{self, Progress, RunEnd, SearchNote, Stamp, TextHit};
use crate::library::{self, NoteEntry, path_key};
use crate::search::{MatchOptions, Matcher};
use crate::window::search_view::thousands;
use crate::window::{library_host, search_view};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

pub(crate) const TEXT_SEARCH_TIMER_ID: usize = 0x4650_5453;
pub(crate) const DEBOUNCE_MS: u32 = 150;
/// The shortest query that runs, in characters (spec §4).
pub(crate) const MIN_QUERY_CHARS: usize = 2;
/// Retries a replace count that arrived inside a modal loop or a file population (`replace_timer`).
pub(crate) const REPLACE_TIMER_ID: usize = 0x4650_5250;
const REPLACE_RETRY_MS: u32 = 50;
/// The most note names a replace report lists before "and K more".
const REPORT_NAMES: usize = 3;
/// A count target's stamp for a hit found in a tab's text. The count reads that tab's text (an
/// overlay) and never compares stamps; no write target ever carries it.
const OVERLAY_STAMP: Stamp = Stamp { size: 0, mtime: 0 };

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
    /// Bumped by every replace start and by `cancel_replace`: a count or write report of any
    /// other generation is stale. Separate from the search's `generation`.
    replace_generation: u64,
    replace_cancel: Option<Arc<AtomicBool>>,
    /// A replace is between its start and its report, or its question's No: another press waits.
    replacing: bool,
    /// A count that arrived inside a modal loop or a file population, asked about once both end.
    held: Option<Box<ReplaceCounted>>,
    /// The write workers not known to have ended. `WM_DESTROY` joins them (`join_writers`), so
    /// the note being written when the window closes is finished, never left half written.
    writers: Vec<JoinHandle<()>>,
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
    // New rows raise the panel's reorder, as a keystroke in the panel does.
    crate::window::side_panel::with_accessible_events(hwnd, || {
        search_view::apply_batch(hwnd, batch)
    });
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

/// A notebook change or close, or notes mode off: cancels the search and any replace, and
/// forgets what the old notebook's searches found. A write already running finishes its file;
/// its report is stale and dropped.
pub(crate) fn forget(hwnd: HWND) {
    cancel(hwnd);
    cancel_replace(hwnd);
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

/// A listed note a replace may change: its path relative to the notebook, its name for the
/// question and the report, and the stamp the search read (`None` when the hit came from a tab's
/// text).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub name: String,
    pub stamp: Option<Stamp>,
}

/// What a replace is for, carried from the start through the count to the write.
#[derive(Debug)]
pub(crate) struct ReplacePlan {
    notebook: PathBuf,
    matcher: Matcher,
    template: String,
    candidates: Vec<Candidate>,
    /// The `path_key`s of the candidates the count read from a tab's text. One whose tab closed
    /// before the split was never counted from its file and the question never warned about it,
    /// so it is never written.
    counted_open: HashSet<String>,
    /// The results were capped: only the listed notes are replaced (spec §12a).
    capped: bool,
    /// A row's replace: one note, asked about only if it isn't open.
    single: bool,
}

/// One `WM_FASTPAD_REPLACE_COUNTED` payload, posted as `Box::into_raw` in `lparam`.
#[derive(Debug)]
pub(crate) struct ReplaceCounted {
    pub generation: u64,
    pub count: ReplaceCount,
    plan: ReplacePlan,
}

/// One `WM_FASTPAD_REPLACE_WRITTEN` payload: the closed notes' report and what the open tabs
/// took on the UI thread before the write began.
#[derive(Debug)]
pub(crate) struct ReplaceWritten {
    pub generation: u64,
    pub report: ReplaceReport,
    pub tab_matches: usize,
    pub tab_notes: usize,
}

/// One `WM_FASTPAD_REPLACE_RELOADED` payload: the files of the clean tabs open on a note the
/// write saved (opened while it ran), read on a worker.
#[derive(Debug)]
pub(crate) struct ReplaceReloaded {
    reloads: Vec<Reload>,
}

#[derive(Debug)]
struct Reload {
    id: DocumentId,
    /// The tab's path, as the tab has it.
    path: PathBuf,
    /// Read before the file, as a file open reads it.
    stamp: Option<library::DiskStamp>,
    /// `None` when the file couldn't be read, decoded or shown: the tab is left as it is.
    loaded: Option<crate::file::loader::LoadedFile>,
}

/// Replace all (spec §11): every listed note, after the question.
pub(crate) fn replace_all(hwnd: HWND) {
    let (candidates, capped) = search_view::replace_candidates(hwnd);
    start_replace(hwnd, candidates, capped, false);
}

/// A row's replace button: that note only, asked about only if it isn't open.
pub(crate) fn replace_in(hwnd: HWND, relative: &Path) {
    let (candidates, _) = search_view::replace_candidates(hwnd);
    let one = candidates
        .into_iter()
        .filter(|candidate| same_path(&candidate.path, relative))
        .collect::<Vec<_>>();
    if !one.is_empty() {
        start_replace(hwnd, one, false, true);
    }
}

/// Counts the matches in `candidates` on a worker. The question waits for the count.
fn start_replace(hwnd: HWND, candidates: Vec<Candidate>, capped: bool, single: bool) {
    if !search_view::replace_open(hwnd)
        || !search_view::replace_all_enabled(hwnd)
        || busy(hwnd)
        || with_host(hwnd, |host| host.replacing).unwrap_or(true)
    {
        return;
    }
    let Some(prepared) = prepare(hwnd, candidates, capped, single) else {
        return;
    };
    let Some((generation, cancel)) = with_host(hwnd, |host| {
        host.replace_generation = host.replace_generation.wrapping_add(1);
        let flag = Arc::new(AtomicBool::new(false));
        host.replace_cancel = Some(Arc::clone(&flag));
        host.replacing = true;
        (host.replace_generation, flag)
    }) else {
        return;
    };
    search_view::set_replacing(hwnd, true);
    let (overlays, targets, plan) = prepared;
    spawn_count(
        hwnd,
        CountJob {
            generation,
            cancel,
            overlays,
            targets,
            plan,
        },
    );
}

/// What the count needs: the tab texts, the targets and the plan. `None` without a view, a
/// notebook, or a pattern that compiles.
fn prepare(
    hwnd: HWND,
    candidates: Vec<Candidate>,
    capped: bool,
    single: bool,
) -> Option<(HashMap<PathBuf, String>, Vec<ReplaceTarget>, ReplacePlan)> {
    // The results' query and options, not the box's text, which may be newer.
    let (query, options) = search_view::run_query(hwnd)?;
    let matcher = Matcher::new(&query, options).ok()?;
    let notebook = library_host::folder(hwnd)?;
    let template = search_view::replace_text(hwnd);
    // Read with nothing of the App borrowed: a background tab is swapped into the editor.
    let overlays = target_overlays(hwnd, &notebook, &candidates);
    let counted_open = overlays.keys().map(|path| path_key(path)).collect();
    let targets = candidates
        .iter()
        .map(|candidate| ReplaceTarget {
            path: candidate.path.clone(),
            stamp: candidate.stamp.unwrap_or(OVERLAY_STAMP),
        })
        .collect();
    Some((
        overlays,
        targets,
        ReplacePlan {
            notebook,
            matcher,
            template,
            candidates,
            counted_open,
            capped,
            single,
        },
    ))
}

/// Everything the count worker owns.
struct CountJob {
    generation: u64,
    cancel: Arc<AtomicBool>,
    overlays: HashMap<PathBuf, String>,
    targets: Vec<ReplaceTarget>,
    plan: ReplacePlan,
}

/// Posts `payload` to `target` as `Box::into_raw`, freeing it here when the post fails (the window
/// is gone: nothing else will).
fn post_boxed<T>(target: isize, message: u32, payload: T) -> bool {
    let payload = Box::into_raw(Box::new(payload));
    let posted = unsafe { PostMessageW(target as HWND, message, 0, payload as isize) } != 0;
    if !posted {
        drop(unsafe { Box::from_raw(payload) });
    }
    posted
}

fn spawn_count(hwnd: HWND, job: CountJob) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let CountJob {
            generation,
            cancel,
            overlays,
            targets,
            plan,
        } = job;
        let count =
            text_replace::count(&plan.notebook, &targets, &overlays, &plan.matcher, &cancel);
        drop(overlays);
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        post_boxed(
            target,
            crate::window::WM_FASTPAD_REPLACE_COUNTED,
            ReplaceCounted {
                generation,
                count,
                plan,
            },
        );
    });
}

/// `WM_FASTPAD_REPLACE_COUNTED`: takes the box back. A stale generation is dropped. Inside a
/// modal loop or a file population the count is held, since the question is a modal loop of its
/// own and a background tab is swapped into the editor, and `replace_timer` asks once both end.
pub(crate) fn replace_counted(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let counted = unsafe { Box::from_raw(lparam as *mut ReplaceCounted) };
    if with_host(hwnd, |host| host.replace_generation) != Some(counted.generation) {
        return;
    }
    if busy(hwnd) {
        with_host(hwnd, |host| host.held = Some(counted));
        unsafe {
            SetTimer(hwnd, REPLACE_TIMER_ID, REPLACE_RETRY_MS, None);
        }
        return;
    }
    confirm_and_apply(hwnd, *counted);
}

/// `WM_TIMER` for `REPLACE_TIMER_ID`: asks about a held count once no modal loop or file
/// population runs. Until then the timer stays armed.
pub(crate) fn replace_timer(hwnd: HWND) {
    if busy(hwnd) {
        return;
    }
    unsafe {
        KillTimer(hwnd, REPLACE_TIMER_ID);
    }
    let Some(counted) = with_host(hwnd, |host| host.held.take()).flatten() else {
        return;
    };
    if with_host(hwnd, |host| host.replace_generation) != Some(counted.generation) {
        return;
    }
    confirm_and_apply(hwnd, *counted);
}

/// Asks (spec §11), then applies: the open tabs in the editor now, the closed notes on a worker.
/// The warning that notes are saved, and a row's question at all, come only when the count read
/// a match from a note's file (`ReplaceCount::closed_notes`).
fn confirm_and_apply(hwnd: HWND, counted: ReplaceCounted) {
    let ReplaceCounted {
        generation,
        count,
        plan,
    } = counted;
    if count.matches == 0 {
        // The notes changed since the search: the results should show that.
        finish_replace(hwnd);
        run_now(hwnd);
        return;
    }
    let question = if plan.single {
        let name = plan
            .candidates
            .first()
            .map_or("", |candidate| candidate.name.as_str());
        (count.closed_notes > 0).then(|| row_confirm_text(count.matches, name, &plan.template))
    } else {
        Some(confirm_text(
            count,
            plan.candidates.len(),
            plan.capped,
            &plan.template,
        ))
    };
    // Asked with nothing of the App borrowed: the question is a nested modal loop.
    if let Some(question) = question
        && !super::modal::confirm(hwnd, &question)
    {
        if with_host(hwnd, |host| host.replace_generation) == Some(generation) {
            finish_replace(hwnd);
        }
        return;
    }
    // A notebook change while the question was up makes the plan stale.
    if with_host(hwnd, |host| host.replace_generation) != Some(generation) {
        return;
    }
    apply_plan(hwnd, generation, plan, count.closed_notes > 0);
}

/// Splits the candidates as the tabs are now and applies the plan. `warned` is whether the
/// question said that notes that aren't open are saved; without it no file is written.
fn apply_plan(hwnd: HWND, generation: u64, plan: ReplacePlan, warned: bool) {
    let ReplacePlan {
        notebook,
        matcher,
        template,
        candidates,
        counted_open,
        ..
    } = plan;
    let open = open_notes(hwnd, &notebook);
    let mut tab_matches = 0;
    let mut tab_notes = 0;
    let mut closed = Vec::new();
    let mut stale = Vec::new();
    for candidate in candidates {
        let key = path_key(&candidate.path);
        match open.get(&key) {
            // An open tab, active or background, clean or dirty: changed in the editor from its
            // live text as one undo action, and not saved (spec §12).
            Some(&id) => {
                match super::main_window::replace_in_document(hwnd, id, &matcher, &template) {
                    Some(0) => {}
                    Some(replaced) => {
                        tab_matches += replaced;
                        tab_notes += 1;
                    }
                    None => stale.push(candidate.path),
                }
            }
            // Its tab closed since the count, which read the tab's text, and the question never
            // said a note would be saved.
            None if !warned && counted_open.contains(&key) => stale.push(candidate.path),
            None => match candidate.stamp {
                Some(stamp) => closed.push(ReplaceTarget {
                    path: candidate.path,
                    stamp,
                }),
                // Found in a tab's text, and that tab has closed since: its file was never read.
                None => stale.push(candidate.path),
            },
        }
    }
    if closed.is_empty() {
        let report = ReplaceReport {
            changed: stale,
            ..ReplaceReport::default()
        };
        report_written(hwnd, report, tab_matches, tab_notes);
        return;
    }
    let Some(cancel) = with_host(hwnd, |host| host.replace_cancel.clone()).flatten() else {
        return;
    };
    let writer = spawn_write(
        hwnd,
        WriteJob {
            generation,
            cancel,
            notebook,
            matcher,
            template,
            targets: closed,
            stale,
            tab_matches,
            tab_notes,
        },
    );
    with_host(hwnd, |host| {
        host.writers.retain(|writer| !writer.is_finished());
        host.writers.push(writer);
    });
}

/// Everything the write worker owns.
struct WriteJob {
    generation: u64,
    cancel: Arc<AtomicBool>,
    notebook: PathBuf,
    matcher: Matcher,
    template: String,
    targets: Vec<ReplaceTarget>,
    /// Notes skipped before the write began, reported as changed since the search.
    stale: Vec<PathBuf>,
    tab_matches: usize,
    tab_notes: usize,
}

/// Writes the closed notes on a worker. A cancelled write finishes the note in hand and posts
/// nothing: its report is stale (a notebook change) or has no window left to go to.
fn spawn_write(hwnd: HWND, job: WriteJob) -> JoinHandle<()> {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let WriteJob {
            generation,
            cancel,
            notebook,
            matcher,
            template,
            targets,
            stale,
            tab_matches,
            tab_notes,
        } = job;
        #[cfg(test)]
        let _ended = writer_hooks::Ended;
        let mut report = text_replace::apply(&notebook, &targets, &matcher, &template, &cancel);
        #[cfg(test)]
        writer_hooks::pause();
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        report.changed.extend(stale);
        post_boxed(
            target,
            crate::window::WM_FASTPAD_REPLACE_WRITTEN,
            ReplaceWritten {
                generation,
                report,
                tab_matches,
                tab_notes,
            },
        );
    })
}

/// `WM_FASTPAD_REPLACE_WRITTEN`: takes the box back. A stale generation is dropped: the notebook
/// it wrote into is no longer open.
pub(crate) fn replace_written(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let written = *unsafe { Box::from_raw(lparam as *mut ReplaceWritten) };
    if with_host(hwnd, |host| host.replace_generation) != Some(written.generation) {
        return;
    }
    report_written(hwnd, written.report, written.tab_matches, written.tab_notes);
}

/// The end of a replace: the library takes every written note's new size and time from the
/// stamps the write worker took (`record_written_all`, one pass, no disk access), as it does for
/// a FastPad save, so the next rescan reads no outside change. A clean tab opened on a written
/// note while the write ran is read again from its file on a worker (`replace_reloaded`); a
/// dirty one keeps its text, and its old disk stamp pauses its autosave as for any change made
/// outside FastPad. Then the report is pushed and the query runs again, so the results show
/// what still matches.
fn report_written(hwnd: HWND, report: ReplaceReport, tab_matches: usize, tab_notes: usize) {
    if !report.written.is_empty() {
        library_host::with_state(hwnd, |state| state.record_written_all(&report.written));
        if let Some(notebook) = library_host::folder(hwnd) {
            let reloads = clean_tabs_on(hwnd, &notebook, &report.written);
            if !reloads.is_empty() {
                spawn_reload(hwnd, reloads);
            }
        }
    }
    let changed = report
        .changed
        .iter()
        .map(|path| note_name(path))
        .collect::<Vec<_>>();
    let failed = report
        .failed
        .iter()
        .map(|(path, _)| note_name(path))
        .collect::<Vec<_>>();
    let text = report_text(
        tab_matches + report.matches,
        tab_notes + report.written.len(),
        &changed,
        &failed,
    );
    finish_replace(hwnd);
    super::main_window::push_notice(hwnd, text);
    run_now(hwnd);
}

/// The clean tabs whose file is one of `written` (relative to `notebook`), with the tab's path.
/// Read from the tabs only: no disk.
fn clean_tabs_on(
    hwnd: HWND,
    notebook: &Path,
    written: &[(PathBuf, Stamp)],
) -> Vec<(DocumentId, PathBuf)> {
    let keys = written
        .iter()
        .map(|(path, _)| path_key(path))
        .collect::<HashSet<_>>();
    unsafe { super::main_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| !document.dirty)
                .filter_map(|document| {
                    let path = document.path.as_deref()?;
                    (library::is_inside(notebook, path)
                        && keys.contains(&path_key(&library::record_path(notebook, path))))
                    .then(|| (document.id, path.to_path_buf()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Reads `tabs`' files on a worker, each as a file open reads it, and posts them back.
fn spawn_reload(hwnd: HWND, tabs: Vec<(DocumentId, PathBuf)>) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let reloads = tabs
            .into_iter()
            .map(|(id, path)| {
                let stamp = library::disk_stamp(&path);
                // A NUL byte cannot round-trip through Scintilla's UTF-8 buffer.
                let loaded = crate::file::loader::load(&path)
                    .ok()
                    .filter(|loaded| std::ffi::CString::new(loaded.text.as_str()).is_ok());
                Reload {
                    id,
                    path,
                    stamp,
                    loaded,
                }
            })
            .collect();
        post_boxed(
            target,
            crate::window::WM_FASTPAD_REPLACE_RELOADED,
            ReplaceReloaded { reloads },
        );
    });
}

/// `WM_FASTPAD_REPLACE_RELOADED`: takes the box back and shows each file's text in its tab, if
/// the tab is still open on it and still clean (`main_window::reload_clean_document`).
pub(crate) fn replace_reloaded(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let reloaded = *unsafe { Box::from_raw(lparam as *mut ReplaceReloaded) };
    for reload in reloaded.reloads {
        if let Some(loaded) = reload.loaded {
            super::main_window::reload_clean_document(
                hwnd,
                reload.id,
                &reload.path,
                &loaded,
                reload.stamp,
            );
        }
    }
}

/// Ends the replace in hand: another may start, and the Search view's results open again.
fn finish_replace(hwnd: HWND) {
    with_host(hwnd, |host| {
        host.replacing = false;
        host.replace_cancel = None;
    });
    search_view::set_replacing(hwnd, false);
}

/// Stops a replace: a count or write still running stops before its next note (a note being
/// written finishes), and whatever it posts is stale. A held count is dropped.
pub(crate) fn cancel_replace(hwnd: HWND) {
    with_host(hwnd, |host| {
        if let Some(flag) = host.replace_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
        host.replace_generation = host.replace_generation.wrapping_add(1);
        host.replacing = false;
        host.held = None;
    });
    unsafe {
        KillTimer(hwnd, REPLACE_TIMER_ID);
    }
    search_view::set_replacing(hwnd, false);
}

/// `WM_DESTROY`, after `cancel_replace`: waits for the write workers, each of which finishes the
/// note it is writing and then stops, so closing never leaves a note half written.
pub(crate) fn join_writers(hwnd: HWND) {
    let writers = with_host(hwnd, |host| std::mem::take(&mut host.writers)).unwrap_or_default();
    for writer in writers {
        let _ = writer.join();
    }
}

/// The tabs whose file is inside `notebook`, active or background, clean or dirty, keyed by the
/// `path_key` of the path relative to it. Read from the tabs only: no disk.
fn open_notes(hwnd: HWND, notebook: &Path) -> HashMap<String, DocumentId> {
    unsafe { super::main_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter_map(|document| {
                    let path = document.path.as_deref()?;
                    library::is_inside(notebook, path)
                        .then(|| (path_key(&library::record_path(notebook, path)), document.id))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The editor text of every open tab among `candidates`, active or background, clean or dirty,
/// keyed by the candidate's relative path, for the count: the replace changes an open tab's
/// editor text, so that is the text to count in. A background tab is swapped into the editor and
/// back (`document_text`); no file is read. Call it with nothing of the App borrowed.
fn target_overlays(
    hwnd: HWND,
    notebook: &Path,
    candidates: &[Candidate],
) -> HashMap<PathBuf, String> {
    let open = open_notes(hwnd, notebook);
    let mut overlays = HashMap::new();
    for candidate in candidates {
        if let Some(&id) = open.get(&path_key(&candidate.path))
            && let Some(text) = super::main_window::document_text(hwnd, id)
        {
            overlays.insert(candidate.path.clone(), text);
        }
    }
    overlays
}

/// A note's name as the Search view shows it: the file name without its extension.
fn note_name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// "1 match" or "1,234 matches".
fn count_of(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", thousands(count))
    }
}

/// Replace all's question (spec wording). `listed` is how many notes the results list; a capped
/// search names them instead of the notes that matched. The second line, that notes are saved,
/// comes only when the count read a match from a note's file (`closed_notes`).
pub(crate) fn confirm_text(
    count: ReplaceCount,
    listed: usize,
    capped: bool,
    template: &str,
) -> String {
    let matches = count_of(count.matches, "match", "matches");
    let mut text = if capped {
        format!(
            "Replace {matches} in the {} listed notes with \"{template}\"? More notes match; search again to replace in the rest.",
            thousands(listed)
        )
    } else {
        format!(
            "Replace {matches} in {} with \"{template}\"?",
            count_of(count.notes, "note", "notes")
        )
    };
    if count.closed_notes > 0 {
        text.push_str("\nNotes that aren't open are saved and can't be undone.");
    }
    text
}

/// A row's question, asked only for a note that isn't open.
pub(crate) fn row_confirm_text(matches: usize, name: &str, template: &str) -> String {
    format!(
        "Replace {} in \"{name}\" with \"{template}\"? The note is saved and this can't be undone.",
        count_of(matches, "match", "matches")
    )
}

/// The report notification, on one line: what was replaced, what was skipped and what couldn't
/// be written, then the skipped and failed notes' names in brackets, at most `REPORT_NAMES`,
/// then "and K more".
pub(crate) fn report_text(
    matches: usize,
    notes: usize,
    changed: &[String],
    failed: &[String],
) -> String {
    let mut text = format!(
        "Replaced {} in {}.",
        count_of(matches, "match", "matches"),
        count_of(notes, "note", "notes")
    );
    match changed.len() {
        0 => {}
        1 => text.push_str(" 1 note was skipped because it changed since the search."),
        skipped => text.push_str(&format!(
            " {} notes were skipped because they changed since the search.",
            thousands(skipped)
        )),
    }
    match failed.len() {
        0 => {}
        1 => text.push_str(" 1 note couldn't be written."),
        unwritten => text.push_str(&format!(
            " {} notes couldn't be written.",
            thousands(unwritten)
        )),
    }
    let total = changed.len() + failed.len();
    if total > 0 {
        let mut names = changed
            .iter()
            .chain(failed)
            .take(REPORT_NAMES)
            .cloned()
            .collect::<Vec<_>>();
        if total > REPORT_NAMES {
            names.push(format!("and {} more", thousands(total - REPORT_NAMES)));
        }
        text.push_str(&format!(" ({})", names.join(", ")));
    }
    text
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

/// Test hooks on the write worker: a pause after its notes, and a count of the workers that
/// ended, so a test can see `WM_DESTROY` wait for one (`join_writers`).
#[cfg(test)]
pub(crate) mod writer_hooks {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    static PAUSE_MS: AtomicU64 = AtomicU64::new(0);
    static ENDED: AtomicUsize = AtomicUsize::new(0);

    /// Every write worker from here on waits `ms` after its last note.
    pub(crate) fn set_pause(ms: u64) {
        PAUSE_MS.store(ms, Ordering::SeqCst);
    }

    pub(super) fn pause() {
        let ms = PAUSE_MS.load(Ordering::SeqCst);
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    /// How many write workers have ended.
    pub(crate) fn ended() -> usize {
        ENDED.load(Ordering::SeqCst)
    }

    /// Counts the worker as ended when it drops, on every path out of it.
    pub(super) struct Ended;

    impl Drop for Ended {
        fn drop(&mut self) {
            ENDED.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// A replace is between its start and its report.
#[cfg(test)]
pub(crate) fn replacing(hwnd: HWND) -> bool {
    with_host(hwnd, |host| host.replacing).unwrap_or(false)
}

/// A count waits for a modal loop or a file population to end.
#[cfg(test)]
pub(crate) fn replace_held(hwnd: HWND) -> bool {
    with_host(hwnd, |host| host.held.is_some()).unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn replace_generation(hwnd: HWND) -> u64 {
    with_host(hwnd, |host| host.replace_generation).unwrap_or(0)
}

/// A boxed count of Replace all over the view's listed notes, as the count worker posts it, for
/// a replace started with `generation`.
#[cfg(test)]
pub(crate) fn test_counted(hwnd: HWND, generation: u64, count: ReplaceCount) -> LPARAM {
    let (candidates, capped) = search_view::replace_candidates(hwnd);
    let (_, _, plan) = prepare(hwnd, candidates, capped, false).expect("a finished search");
    Box::into_raw(Box::new(ReplaceCounted {
        generation,
        count,
        plan,
    })) as LPARAM
}

/// A boxed write report, as the write worker posts it.
#[cfg(test)]
pub(crate) fn test_written(generation: u64, report: ReplaceReport) -> LPARAM {
    Box::into_raw(Box::new(ReplaceWritten {
        generation,
        report,
        tab_matches: 0,
        tab_notes: 0,
    })) as LPARAM
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_QUERY_CHARS, Narrowing, confirm_text, list_mark, narrows, note_mark, overlay_mark,
        report_text, row_confirm_text, searchable,
    };
    use crate::library::NoteEntry;
    use crate::library::text_replace::ReplaceCount;
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
    #[test]
    fn the_question_counts_matches_and_notes_and_warns_only_when_a_note_is_saved() {
        // Break caught: "1 matches", a count without its thousands separator, the capped
        // question claiming only the notes that matched, the warning missing when a note's file
        // will be written or shown when every match is in an open tab, or a per-row question
        // for a note it can't undo (spec §11, §12a).
        let count = |matches, notes, closed_notes| ReplaceCount {
            matches,
            notes,
            closed_notes,
        };
        assert_eq!(
            confirm_text(count(1, 1, 0), 1, false, "x"),
            "Replace 1 match in 1 note with \"x\"?"
        );
        assert_eq!(
            confirm_text(count(1_234, 12, 1), 12, false, "y"),
            "Replace 1,234 matches in 12 notes with \"y\"?\nNotes that aren't open are saved and can't be undone."
        );
        assert_eq!(
            confirm_text(count(2_000, 480, 0), 500, true, ""),
            "Replace 2,000 matches in the 500 listed notes with \"\"? More notes match; search again to replace in the rest."
        );
        assert_eq!(
            confirm_text(count(2_000, 480, 479), 500, true, "z"),
            "Replace 2,000 matches in the 500 listed notes with \"z\"? More notes match; search again to replace in the rest.\nNotes that aren't open are saved and can't be undone."
        );
        assert_eq!(
            row_confirm_text(3, "Q1 budget", "z"),
            "Replace 3 matches in \"Q1 budget\" with \"z\"? The note is saved and this can't be undone."
        );
        assert_eq!(
            row_confirm_text(1, "a", "b"),
            "Replace 1 match in \"a\" with \"b\"? The note is saved and this can't be undone."
        );
    }

    #[test]
    fn the_report_is_one_line_that_counts_what_was_replaced_and_names_what_was_not() {
        // Break caught: a skipped note reported as replaced, "1 notes were skipped", a note that
        // couldn't be written left unnamed, a line break in the one-line status bar, or a list
        // of 400 names in one notification.
        let names = |names: &[&str]| {
            names
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(report_text(1, 1, &[], &[]), "Replaced 1 match in 1 note.");
        assert_eq!(
            report_text(5, 2, &names(&["b"]), &[]),
            "Replaced 5 matches in 2 notes. 1 note was skipped because it changed since the search. (b)"
        );
        assert_eq!(
            report_text(0, 0, &[], &names(&["c", "d"])),
            "Replaced 0 matches in 0 notes. 2 notes couldn't be written. (c, d)"
        );
        assert_eq!(
            report_text(3_000, 1, &names(&["x"]), &names(&["y", "z"])),
            "Replaced 3,000 matches in 1 note. 1 note was skipped because it changed since the search. 2 notes couldn't be written. (x, y, z)"
        );
        let changed = (0..1_200)
            .map(|index| format!("n{index}"))
            .collect::<Vec<_>>();
        let report = report_text(9, 9, &changed, &names(&["f"]));
        assert_eq!(
            report,
            "Replaced 9 matches in 9 notes. 1,200 notes were skipped because they changed since the search. 1 note couldn't be written. (n0, n1, n2, and 1,198 more)"
        );
        assert!(!report.contains('\n'));
    }
}
