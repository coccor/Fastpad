//! The `library-scan` benchmark: generated library and text-search fixtures, and the timings
//! of library loads, tree builds, quick open and text search against their reference budgets.

use super::*;

/// Pinned records written into a generated library, spread evenly over its notes.
pub(super) const LIBRARY_SCAN_PINS: usize = 200;
pub(super) const LIBRARY_SCAN_WARM_LOADS: usize = 5;
/// The warm-load median must stay below this on the reference machine.
pub(super) const LIBRARY_SCAN_REFERENCE_MS: f64 = 500.0;
/// Spec §12 targets on the reference machine.
pub(super) const TREE_BUILD_REFERENCE_MS: f64 = 20.0;
pub(super) const TREE_ROWS_REFERENCE_MS: f64 = 16.0;
/// The quick-open spec's §3.6 budget for one Ctrl+P keystroke.
pub(super) const QUICK_OPEN_REFERENCE_MS: f64 = 5.0;
/// The note-search spec's §14 text search targets on the reference machine, over
/// `TEXT_SEARCH_NOTES` notes of about 4 KB each with a warm OS cache.
pub(super) const TEXT_SEARCH_NOTES: usize = 10_000;
pub(super) const TEXT_SEARCH_NOTE_BYTES: usize = 4_096;
/// Every this-many-th note mentions the invoice: 200 hits, so the full search visits every note
/// without reaching the 500-note cap.
pub(super) const TEXT_SEARCH_RARE_EVERY: usize = 50;
pub(super) const TEXT_SEARCH_FIRST_BATCH_REFERENCE_MS: f64 = 50.0;
pub(super) const TEXT_SEARCH_FULL_REFERENCE_MS: f64 = 400.0;
pub(super) const TEXT_SEARCH_BATCH_UI_REFERENCE_MS: f64 = 2.0;

/// Times one cold and several warm `library::load` calls of `folder`, first generating `count`
/// notes and a `library.ini` into it when asked.
pub(super) fn run_library_scan(
    folder: &Path,
    count: Option<usize>,
    enforce_reference: bool,
) -> Result<i32, String> {
    if let Some(count) = count {
        // Generating into a folder that holds anything could overwrite someone's notes.
        let occupied = std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some());
        if occupied {
            return Err(format!(
                "{} is not empty; library-scan --count only generates into a new or empty folder",
                folder.display()
            ));
        }
        create_library_fixture(folder, count)?;
    }
    let local = ScratchFile(
        std::env::temp_dir().join(format!("fastpad-bench-library-{}.ini", std::process::id())),
    );
    let _ = std::fs::remove_file(&local.0);

    let load = || {
        let started = std::time::Instant::now();
        let state = fastpad::library::load(folder, &local.0, fastpad::library::now_unix())
            .map_err(|error| format!("could not load {}: {error}", folder.display()))?;
        Ok::<_, String>((state, started.elapsed().as_secs_f64() * 1_000.0))
    };
    // The load writes the per-PC cache itself, so the warm loads find it.
    let (state, cold_ms) = load()?;
    let mut warm_ms = Vec::with_capacity(LIBRARY_SCAN_WARM_LOADS);
    for _ in 0..LIBRARY_SCAN_WARM_LOADS {
        warm_ms.push(load()?.1);
    }
    warm_ms.sort_by(f64::total_cmp);
    let warm_median_ms = warm_ms[warm_ms.len() / 2];
    let index_bytes = state
        .notes
        .iter()
        .map(|note| {
            note.path.as_os_str().len() * 2 + std::mem::size_of::<fastpad::library::NoteEntry>()
        })
        .sum::<usize>();

    let paths = state
        .notes
        .iter()
        .map(|note| note.path.clone())
        .collect::<Vec<_>>();
    let pinned = state
        .library
        .notes
        .iter()
        .filter(|record| record.pinned)
        .map(|record| record.path.clone())
        .collect::<Vec<_>>();
    let tree_build_ms = median_ms(|| {
        std::hint::black_box(fastpad::library::tree::NoteTree::build(
            &paths,
            &state.folders,
            &pinned,
        ));
    });
    let tree = fastpad::library::tree::NoteTree::build(&paths, &state.folders, &pinned);
    // Every folder expanded: the fixture's folders hold 500 notes each.
    let tree_rows_expanded_ms = median_ms(|| {
        std::hint::black_box(tree.rows(&|_| true));
    });
    let quick_open_ms = median_ms(|| {
        std::hint::black_box(fastpad::library::quick_open::search(&paths, "nt 12", 50));
    });
    let (text_search_first_batch_ms, text_search_full_ms, text_search_batch_ui_ms) =
        text_search_timings()?;

    println!("notes={}", state.notes.len());
    println!("cold_ms={cold_ms:.1}");
    println!("warm_median_ms={warm_median_ms:.1}");
    println!("index_bytes~{index_bytes}");
    println!("tree_build_ms={tree_build_ms:.2}");
    println!("tree_rows_expanded_ms={tree_rows_expanded_ms:.2}");
    println!("quick_open_ms={quick_open_ms:.2}");
    println!("text_search_first_batch_ms={text_search_first_batch_ms:.2}");
    println!("text_search_full_ms={text_search_full_ms:.2}");
    println!("text_search_batch_ui_ms={text_search_batch_ui_ms:.3}");
    if enforce_reference {
        let failures = [
            (
                "library-scan warm median",
                warm_median_ms,
                LIBRARY_SCAN_REFERENCE_MS,
            ),
            ("tree build", tree_build_ms, TREE_BUILD_REFERENCE_MS),
            (
                "tree rows, all expanded",
                tree_rows_expanded_ms,
                TREE_ROWS_REFERENCE_MS,
            ),
            ("quick open", quick_open_ms, QUICK_OPEN_REFERENCE_MS),
            (
                "text search, first batch",
                text_search_first_batch_ms,
                TEXT_SEARCH_FIRST_BATCH_REFERENCE_MS,
            ),
            (
                "text search, whole notebook",
                text_search_full_ms,
                TEXT_SEARCH_FULL_REFERENCE_MS,
            ),
            (
                "text search, one batch on the UI thread",
                text_search_batch_ui_ms,
                TEXT_SEARCH_BATCH_UI_REFERENCE_MS,
            ),
        ]
        .into_iter()
        .filter(|(_, measured, limit)| measured >= limit)
        .collect::<Vec<_>>();
        for (what, measured, limit) in &failures {
            eprintln!("reference threshold failed: {what}={measured:.2}ms (limit {limit}ms)");
        }
        if !failures.is_empty() {
            return Ok(2);
        }
    }
    Ok(0)
}

/// The median of five timings of `work`, in milliseconds.
pub(super) fn median_ms(mut work: impl FnMut()) -> f64 {
    let mut times = (0..LIBRARY_SCAN_WARM_LOADS)
        .map(|_| {
            let started = std::time::Instant::now();
            work();
            started.elapsed().as_secs_f64() * 1_000.0
        })
        .collect::<Vec<_>>();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// Writes `count` notes as `batch{i / 500}\note{i}.md`, about 200 bytes each, and a
/// `.fastpad\library.ini` with pinned records spread across them.
pub(super) fn create_library_fixture(folder: &Path, count: usize) -> Result<(), String> {
    use fastpad::library::ids::{IdSource, NoteId, fnv1a};
    use fastpad::library::model::{Library, NoteRef};
    use fastpad::library::ops::{PendingOp, apply};

    let relative = |index: usize| PathBuf::from(format!(r"batch{}\note{index}.md", index / 500));
    let text = |index: usize| {
        let mut text = format!("# Note {index}\r\n\r\n");
        while text.len() < 200 {
            text.push_str("The quick brown fox jumps over the lazy dog. ");
        }
        text.truncate(200);
        text
    };
    for index in 0..count {
        let path = folder.join(relative(index));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, text(index))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }

    let mut library = Library::default();
    let mut ids = IdSource::new(fastpad::library::now_unix(), std::process::id());
    let pins = LIBRARY_SCAN_PINS.min(count);
    for pin in 0..pins {
        let index = pin * count / pins;
        let note = NoteRef {
            id: NoteId(ids.next()),
            path: relative(index),
        };
        let content = text(index);
        for op in [
            PendingOp::SetPinned {
                note: note.clone(),
                value: true,
            },
            PendingOp::SetFingerprint {
                note,
                size: content.len() as u64,
                hash: fnv1a(content.as_bytes()),
            },
        ] {
            apply(&mut library, &op)
                .map_err(|error| format!("could not build library.ini: {error}"))?;
        }
    }
    let path = fastpad::library::store::library_file(folder);
    fastpad::library::store::write(&path, &library)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(())
}

/// Note `index` of the text-search fixture: about `TEXT_SEARCH_NOTE_BYTES` of prose, every line
/// holding "lazy dog". One note in `TEXT_SEARCH_RARE_EVERY` also mentions an invoice, on its
/// last line, so a search for it reads the whole note first.
pub(super) fn text_search_note(index: usize) -> String {
    let mut text = format!("# Note {index}\r\n\r\n");
    while text.len() < TEXT_SEARCH_NOTE_BYTES - 64 {
        text.push_str("The quick brown fox jumps over the lazy dog.\r\n");
    }
    if index.is_multiple_of(TEXT_SEARCH_RARE_EVERY) {
        text.push_str(&format!("Paid the invoice march {index}.\r\n"));
    }
    text
}

/// Writes `TEXT_SEARCH_NOTES` fixture notes into `folder`, 500 per subfolder.
pub(super) fn create_text_search_fixture(folder: &Path) -> Result<(), String> {
    for index in 0..TEXT_SEARCH_NOTES {
        let path = folder.join(format!(r"batch{}\note{index}.md", index / 500));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, text_search_note(index))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }
    Ok(())
}

/// The scratch folder for the text-search notebook: `bench-notes\text-search-<pid>` in the build's
/// `target` directory, which git ignores. Not `%TEMP%`: the antivirus scans files freshly written
/// there, and reading them would time the scanner, not the search. `fastpad-bench.exe` runs from
/// `target\<profile>`, so `target` is the nearest ancestor of that name, or else the exe folder's
/// parent.
pub(super) fn text_search_scratch_root() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not find the bench executable: {error}"))?;
    let target = exe
        .ancestors()
        .skip(1)
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("target"))
        })
        .or_else(|| exe.parent().and_then(Path::parent))
        .ok_or_else(|| format!("{} has no parent folder", exe.display()))?;
    Ok(target
        .join("bench-notes")
        .join(format!("text-search-{}", std::process::id())))
}

/// Times the note-search spec's §14 text search over a notebook generated in a scratch folder.
/// Returns, in milliseconds:
/// - the first batch for a phrase in every note;
/// - the whole search for a phrase in one note in fifty;
/// - the UI thread's sorted insert of one 50-hit batch into 450 shown results.
///
/// The `InvalidateRect` that follows a batch is not part of it: it only queues a paint.
pub(super) fn text_search_timings() -> Result<(f64, f64, f64), String> {
    use fastpad::library::text_search::{self, BATCH_HITS, Progress, TextHit};
    use fastpad::search::{MatchOptions, Matcher, Snippet};
    use std::sync::atomic::{AtomicBool, Ordering};

    // Removed on drop, with the notebook and its library.ini, on every return.
    let root = ScratchDir(text_search_scratch_root()?);
    let _ = std::fs::remove_dir_all(&root.0);
    let notebook = root.0.join("notes");
    std::fs::create_dir_all(&notebook)
        .map_err(|error| format!("could not create {}: {error}", notebook.display()))?;
    create_text_search_fixture(&notebook)?;
    let local = root.0.join("library.ini");
    let state = fastpad::library::load(&notebook, &local, fastpad::library::now_unix())
        .map_err(|error| format!("could not load {}: {error}", notebook.display()))?;
    let notes = text_search::search_notes(&state.notes);
    let overlays = std::collections::HashMap::new();
    let common =
        Matcher::new("lazy dog", MatchOptions::default()).map_err(|error| error.to_string())?;
    let rare = Matcher::new("invoice march", MatchOptions::default())
        .map_err(|error| error.to_string())?;
    // One untimed pass warms the OS cache, as §14 measures.
    text_search::run(
        &notebook,
        &notes,
        &overlays,
        &rare,
        &AtomicBool::new(false),
        &mut |_: Vec<TextHit>, _: Progress| {},
    );

    // The first batch fills at 50 hits, after 50 notes; the sink then cancels the rest.
    let first_batch_ms = median_ms(|| {
        let cancel = AtomicBool::new(false);
        text_search::run(
            &notebook,
            &notes,
            &overlays,
            &common,
            &cancel,
            &mut |hits: Vec<TextHit>, _: Progress| {
                std::hint::black_box(hits);
                cancel.store(true, Ordering::Relaxed);
            },
        );
    });
    let full_ms = median_ms(|| {
        std::hint::black_box(text_search::run(
            &notebook,
            &notes,
            &overlays,
            &rare,
            &AtomicBool::new(false),
            &mut |hits: Vec<TextHit>, _: Progress| {
                std::hint::black_box(hits);
            },
        ));
    });

    let hit = |index: usize| TextHit {
        path: PathBuf::from(format!(r"batch{}\note{index}.md", index / 500)),
        name: format!("note{index}"),
        folder: format!("batch{}", index / 500),
        snippet: Snippet {
            text: format!("Paid the invoice march {index}."),
            highlight: 9..22,
        },
        stamp: None,
    };
    let mut shown = (0..450).map(|i| hit(i * 20)).collect::<Vec<_>>();
    shown.sort_by(text_search::hit_cmp);
    let batch = (0..BATCH_HITS)
        .map(|i| hit(i * 20 + 10))
        .collect::<Vec<_>>();
    let mut times = Vec::with_capacity(LIBRARY_SCAN_WARM_LOADS);
    for _ in 0..LIBRARY_SCAN_WARM_LOADS {
        let mut results = shown.clone();
        let incoming = batch.clone();
        let started = std::time::Instant::now();
        for hit in incoming {
            let at = results
                .binary_search_by(|probe| text_search::hit_cmp(probe, &hit))
                .unwrap_or_else(|at| at);
            results.insert(at, hit);
        }
        times.push(started.elapsed().as_secs_f64() * 1_000.0);
        std::hint::black_box(results);
    }
    times.sort_by(f64::total_cmp);
    Ok((first_batch_ms, full_ms, times[times.len() / 2]))
}

/// A scratch folder removed on drop, including on an early `?` return. Its parent goes too
/// when nothing else is left in it (`remove_dir` refuses a folder that isn't empty).
pub(super) struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// A scratch file removed on drop, including on an early `?` return.
pub(super) struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
