use super::*;
use crate::library::ids::NoteId;

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("fastpad-library-{label}-{}", std::process::id()));
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
    assert_eq!(
        record_path(folder, Path::new(r"d:\notes\sub\a.md")),
        PathBuf::from(r"sub\a.md")
    );
    assert_eq!(
        record_path(folder, Path::new(r"D:\Other\a.md")),
        PathBuf::from(r"D:\Other\a.md")
    );
    assert_eq!(
        record_path(folder, Path::new(r"D:\NotesArchive\a.md")),
        PathBuf::from(r"D:\NotesArchive\a.md")
    );
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
    assert_eq!(flush(&mut state).unwrap(), Flushed::Nothing);
    assert!(!scratch.folder().join(".fastpad").exists());
}

#[test]
fn pinning_creates_the_library_file_and_a_reload_sees_it() {
    let scratch = Scratch::new("organize");
    let note = scratch.folder().join("a.md");
    std::fs::write(&note, "a").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &note);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    assert!(state.is_pinned(&note));
    assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
    assert!(state.pending.is_empty());
    let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    assert!(reloaded.is_pinned(&note));
    assert_eq!(
        reloaded.record_for(&note).unwrap().path,
        PathBuf::from("a.md")
    );
}

#[test]
fn a_file_changed_on_disk_is_merged_not_overwritten() {
    // Break caught: this PC's debounced write replacing the pin another PC just synced.
    let scratch = Scratch::new("merge");
    let [a, b, c] = ["a.md", "b.md", "c.md"].map(|name| {
        let path = scratch.folder().join(name);
        std::fs::write(&path, name).unwrap();
        path
    });
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &a);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    flush(&mut state).unwrap();

    // "Another PC" pins b.md directly in the file.
    let path = store::library_file(&scratch.folder());
    let mut other = match store::read(&path) {
        store::ReadOutcome::Loaded(library, _) => library,
        _ => panic!("expected a library"),
    };
    other
        .resolve_note(&NoteRef {
            id: NoteId(77),
            path: "b.md".into(),
        })
        .pinned = true;
    std::thread::sleep(std::time::Duration::from_millis(20));
    store::write(&path, &other).unwrap();

    let target = state.note_ref(&mut ids, &c);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    flush(&mut state).unwrap();
    let final_state = load(&scratch.folder(), &scratch.local(), 102).unwrap();
    assert!(final_state.is_pinned(&a));
    assert!(final_state.is_pinned(&b), "the synced pin survives");
    assert!(final_state.is_pinned(&c));
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
    let _ = state.apply(PendingOp::SetPinned {
        note: target,
        value: true,
    });
    assert_eq!(flush(&mut state).unwrap(), Flushed::Nothing);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "version=9\r\nnote=future\r\n"
    );
}

#[test]
fn flush_refuses_to_overwrite_a_library_file_replaced_by_an_unreadable_one() {
    // Break caught: a flush re-reading the file mid-write, finding it replaced by something
    // this FastPad cannot parse, and writing over it anyway.
    let scratch = Scratch::new("flush-unreadable");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &a);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();

    // Another process replaces library.ini with a file from a newer, unreadable version.
    let path = store::library_file(&scratch.folder());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "version=9\r\nnote=future\r\n").unwrap();

    assert!(flush(&mut state).is_err());
    assert_eq!(state.metadata, Metadata::Unreadable);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "version=9\r\nnote=future\r\n"
    );
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
    previous
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    previous.local.set_expanded(Path::new("sub"), true);
    let merged = merge_rescan(previous, fresh);
    assert!(merged.record_for(&a).unwrap().pinned);
    assert_eq!(merged.pending.len(), 1);
    assert!(merged.local.is_expanded(Path::new("SUB")));
}

#[test]
fn a_rescan_keeps_the_root_collapsed_state_the_ui_set_while_it_ran() {
    // Break caught: a rescan reopening the root row the user just collapsed.
    let scratch = Scratch::new("rescan-root");
    std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
    let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    previous.local.root_collapsed = true;
    let merged = merge_rescan(previous, fresh);
    assert!(merged.local.root_collapsed);
}

#[test]
fn a_rescan_does_not_revert_a_pin_already_flushed_while_it_ran() {
    // Break caught: merge_rescan replaying an empty pending list onto the rescan's own
    // (stale) snapshot of the library and installing the rescan's stamp, silently reverting
    // a change the live library had already flushed to disk before the merge happened.
    let scratch = Scratch::new("rescan-flush");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    let target = previous.note_ref(&mut ids, &a);
    previous
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    assert_eq!(flush(&mut previous).unwrap(), Flushed::Wrote);

    let mut merged = merge_rescan(previous, fresh);
    assert!(merged.is_pinned(&a), "the flushed pin is still visible");
    assert!(
        flush(&mut merged).unwrap() == Flushed::Nothing,
        "nothing pending: the flush is a no-op"
    );
    let reloaded = load(&scratch.folder(), &scratch.local(), 102).unwrap();
    assert!(reloaded.is_pinned(&a), "the flush did not revert it");
}

#[test]
fn merging_notes_keeps_index_edits_made_during_the_rescan() {
    // Break caught: merge_rescan taking fresh.notes verbatim, losing a note added or a
    // rename made through the live index while a background rescan was still running.
    let scratch = Scratch::new("rescan-notes");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let old = scratch.folder().join("old.md");
    std::fs::write(&old, "x").unwrap();

    let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();

    // A note saved through FastPad while the rescan was running: only in `previous`'s index.
    let b = scratch.folder().join("b.md");
    std::fs::write(&b, "b").unwrap();
    previous.add_note(&b);

    // A rename made through FastPad: `fresh` still has the old name (the file no longer
    // exists there), `previous` has the new one.
    let renamed = scratch.folder().join("renamed.md");
    std::fs::rename(&old, &renamed).unwrap();
    previous.rename_note(&old, &renamed);

    let merged = merge_rescan(previous, fresh);
    let paths: std::collections::HashSet<_> = merged
        .notes
        .iter()
        .map(|note| note.path.to_string_lossy().to_lowercase())
        .collect();
    assert!(paths.contains("a.md"));
    assert!(
        paths.contains("b.md"),
        "a note added during the rescan survives"
    );
    assert!(
        paths.contains("renamed.md"),
        "a rename made during the rescan survives"
    );
    assert!(
        !paths.contains("old.md"),
        "the old name is gone from disk and is not kept from fresh"
    );
    assert_eq!(merged.notes.len(), 3);
}

#[test]
fn a_rescan_absorbs_a_change_made_outside_fastpad_while_it_was_inactive() {
    // Break caught: treating any stamp mismatch as "the live library flushed during the
    // scan" and keeping a stale library forever when the file actually changed from
    // outside (another PC's sync) while this FastPad made no local changes of its own.
    let scratch = Scratch::new("rescan-outside-sync");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let b = scratch.folder().join("b.md");
    std::fs::write(&b, "b").unwrap();
    let mut ids = IdSource::new(1, 2);

    // Pin once, so `library.ini` exists and `previous` loads with a real stamp.
    let mut setup = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = setup.note_ref(&mut ids, &a);
    setup
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    assert_eq!(flush(&mut setup).unwrap(), Flushed::Wrote);

    let previous = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    assert!(previous.stamp.is_some());

    // Another PC syncs a pin directly into the file while this FastPad is inactive.
    let path = store::library_file(&scratch.folder());
    let mut outside = match store::read(&path) {
        store::ReadOutcome::Loaded(library, _) => library,
        _ => panic!("expected a library"),
    };
    outside
        .resolve_note(&NoteRef {
            id: NoteId(77),
            path: "b.md".into(),
        })
        .pinned = true;
    std::thread::sleep(std::time::Duration::from_millis(20));
    store::write(&path, &outside).unwrap();

    let fresh = load(&scratch.folder(), &scratch.local(), 102).unwrap();
    assert_ne!(fresh.stamp, previous.stamp);
    let current = store::stamp(&path);

    let merged = merge_rescan(previous, fresh);
    assert!(merged.is_pinned(&b), "the outside change is visible");
    assert!(merged.is_pinned(&a), "the earlier pin is still there");
    assert_eq!(merged.stamp, current);
}

#[test]
fn note_entry_mtimes_are_filetime_ticks_like_a_scan_reports() {
    // Break caught: add_note (and merge_notes) storing mtime as Unix nanoseconds while
    // scan.rs reports Windows FILETIME ticks (100 ns since 1601), so entries from different
    // sources in the same list could not be compared or sorted.
    let scratch = Scratch::new("mtime-scale");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let raw_nanos = disk_stamp(&a).unwrap().modified;
    let expected = raw_nanos / 100 + FILETIME_UNIX_EPOCH_TICKS;

    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let from_scan = state.notes[0].mtime;
    state.notes.clear();
    state.add_note(&a);
    let from_add_note = state.notes[0].mtime;

    assert_eq!(from_add_note, expected);
    // Both are FILETIME ticks for the same write; allow a little slack for rounding between
    // the OS-reported FILETIME and the nanosecond-precision `SystemTime` conversion.
    let ticks_per_second = 10_000_000;
    assert!(
        from_scan.abs_diff(from_add_note) < ticks_per_second,
        "scan: {from_scan}, add_note: {from_add_note}"
    );
}

#[test]
fn a_truncated_rescan_caps_the_merged_notes_list_at_the_scan_limit() {
    // Break caught: notes saved during the rescan surviving the notes merge and pushing a
    // truncated scan's list past the limit it was supposed to be capped at.
    let scratch = Scratch::new("rescan-cap");
    let mut previous = bare_state(&scratch, entries(0), false);
    for index in 0..5 {
        let saved = scratch.folder().join(format!("saved{index}.md"));
        std::fs::write(&saved, "x").unwrap();
        previous.add_note(&saved);
    }
    let fresh = bare_state(&scratch, entries(scan::NOTE_LIMIT), true);
    let merged = merge_rescan(previous, fresh);
    assert_eq!(merged.notes.len(), scan::NOTE_LIMIT);
}

#[test]
fn a_replace_write_is_recorded_as_a_save_is_without_reading_the_disk() {
    // Break caught: FastPad's own replace read as an outside change by the next rescan (the
    // old size or time kept, or the note missing from `touched`), an online-only flag left
    // set so search keeps skipping the note, a stamp taken from the disk on the UI thread
    // for each written note, or a note that isn't listed added.
    let scratch = Scratch::new("record-written");
    let mut state = bare_state(&scratch, entries(2), false);
    state.notes[1].online_only = true;
    let stamp = text_search::Stamp {
        size: 42,
        mtime: 133_700_000_000_000_000,
    };
    let taken = store::stats_taken();

    assert!(state.record_written(Path::new("F1.md"), stamp));
    assert!(!state.record_written(Path::new("missing.md"), stamp));

    assert_eq!(store::stats_taken(), taken, "no stamp taken from the disk");
    let note = &state.notes[1];
    assert_eq!(
        (note.size, note.mtime, note.online_only),
        (42, stamp.mtime, false)
    );
    assert_eq!(state.notes[0].size, 0, "only that note changes");
    assert_eq!(state.notes.len(), 2, "nothing is added");
    assert_eq!(state.touched, [PathBuf::from("F1.md")]);
}

#[test]
fn record_written_all_updates_every_listed_note_in_one_pass() {
    // Break caught (R1): a batch of several written notes handled by re-scanning `notes` once
    // per note instead of once for the whole batch, or a path in the batch that isn't listed
    // changing something or being counted.
    let scratch = Scratch::new("record-written-all");
    let mut state = bare_state(&scratch, entries(3), false);
    state.notes[0].online_only = true;
    state.notes[2].online_only = true;
    let stamp0 = text_search::Stamp {
        size: 10,
        mtime: 100,
    };
    let stamp2 = text_search::Stamp {
        size: 30,
        mtime: 300,
    };
    let written = [
        (PathBuf::from("F0.md"), stamp0),
        (PathBuf::from("missing.md"), stamp0),
        (PathBuf::from("F2.md"), stamp2),
    ];

    let recorded = state.record_written_all(&written);

    assert_eq!(recorded, 2, "the unlisted path is not counted");
    assert_eq!(
        (
            state.notes[0].size,
            state.notes[0].mtime,
            state.notes[0].online_only
        ),
        (10, 100, false)
    );
    assert_eq!(
        state.notes[1].size, 0,
        "the note not in the batch is untouched"
    );
    assert_eq!(
        (
            state.notes[2].size,
            state.notes[2].mtime,
            state.notes[2].online_only
        ),
        (30, 300, false)
    );
    assert_eq!(
        state.touched,
        [PathBuf::from("F0.md"), PathBuf::from("F2.md")]
    );
    assert_eq!(state.notes.len(), 3, "nothing is added");
}

/// `count` index entries that exist only in memory.
fn entries(count: usize) -> Vec<NoteEntry> {
    (0..count)
        .map(|index| NoteEntry {
            path: PathBuf::from(format!("f{index}.md")),
            size: 0,
            mtime: 0,
            online_only: false,
        })
        .collect()
}

fn bare_state(scratch: &Scratch, notes: Vec<NoteEntry>, truncated: bool) -> LibraryState {
    let paths: Vec<PathBuf> = notes.iter().map(|note| note.path.clone()).collect();
    LibraryState {
        folder: scratch.folder(),
        local_path: scratch.local(),
        library: Library::default(),
        metadata: Metadata::Ready,
        stamp: None,
        local: LocalState::new(scratch.folder()),
        tree: tree::NoteTree::build(&paths, &[], &[]),
        notes,
        folders: Vec::new(),
        folder_changes: Vec::new(),
        truncated,
        pending: Vec::new(),
        relocated: Vec::new(),
        touched: Vec::new(),
        written_local: local::Conveniences::default(),
    }
}

fn mark_online_only(path: &Path) {
    let wide = crate::platform::wide_null(&path.to_string_lossy());
    let marked = unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
            wide.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE,
        )
    };
    assert_ne!(marked, 0, "SetFileAttributesW failed");
}

fn online_only(state: &LibraryState, relative: &str) -> bool {
    state
        .notes
        .iter()
        .find(|note| note.path == Path::new(relative))
        .unwrap()
        .online_only
}

#[test]
fn an_online_only_file_is_marked_in_the_note_list() {
    // Break caught: text search opening a OneDrive online-only note, which downloads it.
    let scratch = Scratch::new("online-only");
    let cloud = scratch.folder().join("cloud.md");
    std::fs::write(&cloud, "a").unwrap();
    std::fs::write(scratch.folder().join("here.md"), "b").unwrap();
    mark_online_only(&cloud);
    let state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    assert!(online_only(&state, "cloud.md"));
    assert!(!online_only(&state, "here.md"));
}

#[test]
fn a_rename_keeps_a_notes_online_only_flag_and_a_save_clears_it() {
    // Break caught: renaming an online-only note in FastPad making the next search download
    // it, or a note FastPad just saved still skipped as online only.
    let scratch = Scratch::new("online-rename");
    let old = scratch.folder().join("old.md");
    std::fs::write(&old, "a").unwrap();
    mark_online_only(&old);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let new = scratch.folder().join("new.md");
    std::fs::rename(&old, &new).unwrap();
    state.rename_note(&old, &new);
    assert!(online_only(&state, "new.md"));
    std::fs::write(&new, "saved").unwrap();
    state.add_note(&new);
    assert!(!online_only(&state, "new.md"));
    let added = scratch.folder().join("added.md");
    std::fs::write(&added, "c").unwrap();
    assert!(state.add_note(&added));
    assert!(!online_only(&state, "added.md"));
}

#[test]
fn a_merged_rescan_keeps_the_scans_online_only_flag_for_a_touched_note() {
    let scratch = Scratch::new("online-merge");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let mut previous = bare_state(&scratch, Vec::new(), false);
    previous.add_note(&a);
    let seen_online_only = NoteEntry {
        path: PathBuf::from("a.md"),
        size: 1,
        mtime: 0,
        online_only: true,
    };
    let fresh = bare_state(&scratch, vec![seen_online_only], false);
    let merged = merge_rescan(previous, fresh);
    assert!(online_only(&merged, "a.md"));
}

#[test]
fn a_bulk_change_made_outside_fastpad_costs_no_stat_when_a_rescan_is_merged() {
    // Break caught: the merge stat-ing every path that differs between the old index and the
    // rescan, on the UI thread, so deleting a few thousand notes in Explorer froze FastPad.
    let scratch = Scratch::new("rescan-bulk");
    let previous = bare_state(&scratch, entries(2_000), false);
    let fresh = bare_state(&scratch, entries(0), false);
    let before = store::stats_taken();
    let merged = merge_rescan(previous, fresh);
    assert_eq!(store::stats_taken() - before, 0);
    assert!(merged.notes.is_empty(), "the rescan proved them gone");
}

#[test]
fn a_rescan_that_cannot_read_the_library_file_keeps_the_live_library_and_is_not_unreadable() {
    // Break caught: a sharing violation while OneDrive synced library.ini turning organizing
    // off, or a rescan that met one replacing the live pins with an empty library.
    use std::os::windows::fs::OpenOptionsExt;
    let scratch = Scratch::new("busy-load");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = previous.note_ref(&mut ids, &a);
    previous
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    assert_eq!(flush(&mut previous).unwrap(), Flushed::Wrote);

    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(store::library_file(&scratch.folder()))
        .unwrap();
    let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    drop(lock);
    assert_eq!(fresh.metadata, Metadata::Busy);
    let merged = merge_rescan(previous, fresh);
    assert_eq!(merged.metadata, Metadata::Ready);
    assert!(merged.is_pinned(&a));
}

#[test]
fn a_flush_that_meets_a_busy_library_file_keeps_its_operations_for_a_retry() {
    // Break caught: a flush that could not re-read a synced library.ini dropping the pending
    // operations or marking the library unreadable.
    use std::os::windows::fs::OpenOptionsExt;
    let scratch = Scratch::new("busy-flush");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let b = scratch.folder().join("b.md");
    std::fs::write(&b, "b").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &a);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    // Another PC's sync creates the file, so the flush must re-read it, while it is held open.
    let path = store::library_file(&scratch.folder());
    let mut other = Library::default();
    other
        .resolve_note(&NoteRef {
            id: NoteId(77),
            path: "b.md".into(),
        })
        .pinned = true;
    store::write(&path, &other).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let busy = flush(&mut state).unwrap();
    drop(lock);
    assert_eq!(busy, Flushed::Busy);
    assert_eq!(state.pending.len(), 1);
    assert_eq!(state.metadata, Metadata::Ready);

    assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
    let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    assert!(reloaded.is_pinned(&b));
    assert!(reloaded.is_pinned(&a));
}

#[test]
fn the_load_writes_the_local_file_and_only_convenience_changes_need_another_write() {
    // Break caught: the UI thread encoding and writing the whole scan cache on every rescan
    // and every metadata flush, or the worker rewriting an unchanged local file.
    let scratch = Scratch::new("local-writes");
    std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let written = std::fs::read_to_string(scratch.local()).unwrap();
    assert!(written.contains("|a.md\r\n"), "{written:?}");
    let modified = std::fs::metadata(scratch.local())
        .unwrap()
        .modified()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let _unchanged = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    assert_eq!(
        std::fs::metadata(scratch.local())
            .unwrap()
            .modified()
            .unwrap(),
        modified,
        "an unchanged local file is not rewritten"
    );

    assert!(take_local_changes(&mut state).is_none());
    state.local.files.clear();
    assert!(
        take_local_changes(&mut state).is_none(),
        "the scan cache alone"
    );
    state.local.set_expanded(Path::new("sub"), true);
    let changed = take_local_changes(&mut state).expect("an expansion change is written");
    assert_eq!(changed.expanded, [PathBuf::from("sub")]);
    assert!(take_local_changes(&mut state).is_none());
}

#[test]
fn folder_spellings_normalize_to_one() {
    // Break caught: `D:\Notes\` and `D:\Notes` keying two local states and two recent rows.
    assert_eq!(
        normalize_folder(Path::new(r"D:\Notes\")),
        PathBuf::from(r"D:\Notes")
    );
    assert_eq!(
        normalize_folder(Path::new(r"D:\Notes\.\")),
        PathBuf::from(r"D:\Notes")
    );
    assert_eq!(normalize_folder(Path::new(r"D:\")), PathBuf::from(r"D:\"));
    assert_eq!(
        local::folder_key(Path::new(r"D:\Notes\")),
        local::folder_key(Path::new(r"d:\notes"))
    );
    let mut recent = local::RecentFolders::default();
    recent.push(PathBuf::from(r"D:\Notes\"));
    recent.push(PathBuf::from(r"D:\Notes"));
    assert_eq!(recent.folders, [PathBuf::from(r"D:\Notes")]);
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
    assert!(state.add_note(&a), "a new note is inserted");
    assert!(
        !state.add_note(&a),
        "saving it again only updates its entry"
    );
    assert!(!state.add_note(Path::new(r"C:\elsewhere\x.md")));
    assert_eq!(state.notes.len(), 1);
    let mut ids = IdSource::new(1, 2);
    let target = state.note_ref(&mut ids, &a);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    let b = scratch.folder().join("b.md");
    state.rename_note(&a, &b);
    assert_eq!(state.notes[0].path, PathBuf::from("b.md"));
    assert_eq!(state.record_for(&b).unwrap().path, PathBuf::from("b.md"));
    state.remove_note(&b);
    assert!(state.notes.is_empty());
}

#[test]
fn the_state_lists_the_scans_folders_and_a_rescan_replaces_them() {
    // Break caught: an empty folder known only until the first rescan, or a folder deleted in
    // Explorer kept alive by the merge.
    let scratch = Scratch::new("folder-list");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join("empty")).unwrap();
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(folder.join(r"sub\a.md"), "a").unwrap();
    let sorted = |state: &LibraryState| {
        let mut folders = state.folders.clone();
        folders.sort();
        folders
    };
    let previous = load(&folder, &scratch.local(), 100).unwrap();
    assert_eq!(sorted(&previous), ["empty", "sub"].map(PathBuf::from));
    std::fs::remove_dir(folder.join("empty")).unwrap();
    std::fs::create_dir(folder.join("later")).unwrap();
    let fresh = load(&folder, &scratch.local(), 101).unwrap();
    let merged = merge_rescan(previous, fresh);
    assert_eq!(sorted(&merged), ["later", "sub"].map(PathBuf::from));
}

fn sorted_folders(state: &LibraryState) -> Vec<PathBuf> {
    let mut folders = state.folders.clone();
    folders.sort();
    folders
}

fn sorted_notes(state: &LibraryState) -> Vec<PathBuf> {
    let mut notes: Vec<PathBuf> = state.notes.iter().map(|note| note.path.clone()).collect();
    notes.sort();
    notes
}

#[test]
fn renaming_a_folder_rewrites_its_notes_records_expanded_entries_and_tree() {
    // Break caught: a folder rename leaving notes, pins or expanded folders at the old path,
    // so rows open files that are gone, pins vanish, or the renamed folder and its nested
    // empty folders collapse.
    let scratch = Scratch::new("folder-rename");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join(r"work\empty\deeper")).unwrap();
    std::fs::write(folder.join(r"work\plan.md"), "p").unwrap();
    std::fs::write(folder.join("top.md"), "t").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&folder, &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &folder.join(r"work\plan.md"));
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    state.local.set_expanded(Path::new("work"), true);
    state.local.set_expanded(Path::new(r"work\empty"), true);
    state.local.set_expanded(Path::new("other"), true);
    std::fs::rename(folder.join("work"), folder.join("Archive")).unwrap();

    state.rename_folder(Path::new("work"), Path::new("Archive"));

    assert_eq!(
        sorted_notes(&state),
        [PathBuf::from(r"Archive\plan.md"), PathBuf::from("top.md")]
    );
    assert!(
        state.is_pinned(Path::new(r"Archive\plan.md")),
        "the record moved"
    );
    assert!(!state.is_pinned(Path::new(r"work\plan.md")));
    assert_eq!(
        state.local.expanded,
        [
            PathBuf::from("Archive"),
            PathBuf::from(r"Archive\empty"),
            PathBuf::from("other")
        ]
    );
    assert_eq!(
        sorted_folders(&state),
        ["Archive", r"Archive\empty", r"Archive\empty\deeper"].map(PathBuf::from)
    );
    assert!(state.is_folder(Path::new(r"archive\EMPTY\deeper")));
    assert!(!state.is_folder(Path::new("work")));
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
    assert!(state.pending.iter().any(|op| matches!(
        op,
        PendingOp::Relocate { path, .. } if path == Path::new(r"Archive\plan.md")
    )));
    assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
    let reloaded = load(&folder, &scratch.local(), 101).unwrap();
    assert!(reloaded.is_pinned(Path::new(r"Archive\plan.md")));
}

#[test]
fn removing_a_folder_drops_its_notes_marks_their_records_deleted_and_forgets_its_expansion() {
    // Break caught: a deleted folder's notes still listed (rows that open nothing), its
    // pinned note's record looking alive, or its expanded entries left in the local file.
    let scratch = Scratch::new("folder-remove");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join(r"old\inner")).unwrap();
    std::fs::write(folder.join(r"old\a.md"), "a").unwrap();
    std::fs::write(folder.join(r"old\inner\b.md"), "b").unwrap();
    std::fs::write(folder.join("keep.md"), "k").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&folder, &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &folder.join(r"old\a.md"));
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    state.local.set_expanded(Path::new("old"), true);
    state.local.set_expanded(Path::new(r"old\inner"), true);
    assert_eq!(state.notes_under(Path::new("OLD")), 2);
    assert!(state.is_listed(Path::new("keep.md")));
    assert!(state.is_listed(Path::new(r"Old\Inner")));
    assert!(!state.is_listed(Path::new("new")));

    state.remove_folder(Path::new("old"), 500);

    assert_eq!(sorted_notes(&state), [PathBuf::from("keep.md")]);
    let record = state.library.note_by_path(Path::new(r"old\a.md")).unwrap();
    assert!(record.deleted);
    assert_eq!(state.local.missing_since(record.id), Some(500));
    assert!(!state.is_pinned(Path::new(r"old\a.md")));
    assert!(state.local.expanded.is_empty());
    assert!(state.folders.is_empty());
    assert!(!state.is_folder(Path::new(r"old\inner")));
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
    assert_eq!(tree_rows(&state.tree).len(), 1);
}

#[test]
fn a_folder_changed_while_a_rescan_ran_is_not_undone_by_its_result() {
    // Break caught: a rescan that listed the folders before FastPad renamed or made one
    // bringing the old row back (with its note at a path that is gone) and hiding the new
    // one until the next rescan; or a replay that breaks a rescan that already saw it.
    let scratch = Scratch::new("folder-merge");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join(r"work\empty")).unwrap();
    std::fs::write(folder.join(r"work\plan.md"), "p").unwrap();
    std::fs::write(folder.join("top.md"), "t").unwrap();
    let mut previous = load(&folder, &scratch.local(), 100).unwrap();
    let stale = load(&folder, &scratch.local(), 101).unwrap();
    std::fs::rename(folder.join("work"), folder.join("Archive")).unwrap();
    previous.rename_folder(Path::new("work"), Path::new("Archive"));
    std::fs::create_dir(folder.join("Made")).unwrap();
    previous.add_folder(Path::new("Made"));

    let merged = merge_rescan(previous, stale);

    assert_eq!(
        sorted_folders(&merged),
        ["Archive", r"Archive\empty", "Made"].map(PathBuf::from)
    );
    assert_eq!(
        sorted_notes(&merged),
        [PathBuf::from(r"Archive\plan.md"), PathBuf::from("top.md")]
    );
    assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));

    let mut previous = load(&folder, &scratch.local(), 102).unwrap();
    std::fs::rename(folder.join("Archive"), folder.join("Final")).unwrap();
    previous.rename_folder(Path::new("Archive"), Path::new("Final"));
    let seen = load(&folder, &scratch.local(), 103).unwrap();

    let merged = merge_rescan(previous, seen);

    assert_eq!(
        sorted_folders(&merged),
        ["Final", r"Final\empty", "Made"].map(PathBuf::from)
    );
    assert_eq!(
        sorted_notes(&merged),
        [PathBuf::from(r"Final\plan.md"), PathBuf::from("top.md")]
    );
    assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
}

#[test]
fn a_rescan_that_found_a_renamed_folder_gone_keeps_its_notes_under_the_new_name() {
    // Break caught: a rescan that listed `sub` just before FastPad renamed it, then found it
    // gone when it read inside, dropping the folder's notes from the list and the tree until
    // the next rescan.
    let scratch = Scratch::new("rename-mid-scan");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(folder.join(r"sub\a.md"), "a").unwrap();
    std::fs::write(folder.join("top.md"), "t").unwrap();
    let mut previous = load(&folder, &scratch.local(), 100).unwrap();
    // What that rescan saw: the folder, and nothing in it.
    std::fs::rename(folder.join(r"sub\a.md"), folder.join("a.parked")).unwrap();
    let emptied = load(&folder, &scratch.local(), 101).unwrap();
    std::fs::rename(folder.join("a.parked"), folder.join(r"sub\a.md")).unwrap();
    std::fs::rename(folder.join("sub"), folder.join("Moved")).unwrap();
    previous.rename_folder(Path::new("sub"), Path::new("Moved"));

    let merged = merge_rescan(previous, emptied);

    assert_eq!(sorted_folders(&merged), [PathBuf::from("Moved")]);
    assert_eq!(
        sorted_notes(&merged),
        [PathBuf::from(r"Moved\a.md"), PathBuf::from("top.md")]
    );
    assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
}

/// What a folder command could change, to prove a refused one changed nothing.
fn folder_snapshot(state: &LibraryState) -> impl PartialEq + std::fmt::Debug + use<> {
    let records: Vec<(PathBuf, bool)> = state
        .library
        .notes
        .iter()
        .map(|record| (record.path.clone(), record.deleted))
        .collect();
    (
        sorted_notes(state),
        sorted_folders(state),
        records,
        state.local.expanded.clone(),
        state.pending.len(),
        state.folder_changes.len(),
        tree_rows(&state.tree),
    )
}

#[test]
fn a_folder_change_on_an_empty_or_escaping_path_changes_nothing() {
    // Break caught: `remove_folder("")` taking every note (an empty folder is "at or under"
    // everything), or a rename from or onto the root, `..` or an absolute path.
    let scratch = Scratch::new("folder-bad-path");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::write(folder.join(r"sub\a.md"), "a").unwrap();
    std::fs::write(folder.join("top.md"), "t").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&folder, &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &folder.join("top.md"));
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    state.local.set_expanded(Path::new("sub"), true);
    let before = folder_snapshot(&state);

    for bad in ["", ".", "..", r"\sub", r"C:\sub"] {
        state.remove_folder(Path::new(bad), 500);
        state.rename_folder(Path::new(bad), Path::new("moved"));
        state.rename_folder(Path::new("sub"), Path::new(bad));
        assert_eq!(folder_snapshot(&state), before, "{bad:?}");
    }
    state.rename_folder(Path::new("sub"), Path::new(r"..\out"));
    assert_eq!(folder_snapshot(&state), before);
}

#[test]
fn a_folder_change_leaves_a_sibling_that_shares_its_name_prefix_alone() {
    // Break caught: a textual prefix test treating `subway` as inside `sub`, so deleting or
    // renaming `sub` drops or moves `subway`'s notes, records and expanded entry.
    let scratch = Scratch::new("folder-prefix-sibling");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join("sub")).unwrap();
    std::fs::create_dir_all(folder.join("subway")).unwrap();
    std::fs::write(folder.join(r"sub\a.md"), "a").unwrap();
    std::fs::write(folder.join(r"subway\b.md"), "b").unwrap();
    let prepared = |at: u64| {
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&folder, &scratch.local(), at).unwrap();
        for note in [r"sub\a.md", r"subway\b.md"] {
            let target = state.note_ref(&mut ids, &folder.join(note));
            state
                .apply(PendingOp::SetPinned {
                    note: target,
                    value: true,
                })
                .unwrap();
        }
        state.local.set_expanded(Path::new("sub"), true);
        state.local.set_expanded(Path::new("subway"), true);
        state
    };
    let sibling_intact = |state: &LibraryState| {
        assert!(state.is_listed(Path::new(r"subway\b.md")));
        assert!(state.is_pinned(Path::new(r"subway\b.md")));
        let record = state
            .library
            .note_by_path(Path::new(r"subway\b.md"))
            .unwrap();
        assert!(!record.deleted);
        assert!(state.is_folder(Path::new("subway")));
        assert!(
            state
                .local
                .expanded
                .iter()
                .any(|entry| entry == Path::new("subway"))
        );
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(state));
    };

    let mut state = prepared(100);
    state.remove_folder(Path::new("sub"), 500);
    assert!(!state.is_listed(Path::new(r"sub\a.md")));
    sibling_intact(&state);

    let mut state = prepared(101);
    state.rename_folder(Path::new("sub"), Path::new("renamed"));
    assert!(state.is_listed(Path::new(r"renamed\a.md")));
    sibling_intact(&state);
    assert_eq!(
        sorted_folders(&state),
        ["renamed", "subway"].map(PathBuf::from)
    );
}

#[test]
fn a_folder_removed_while_a_rescan_ran_does_not_come_back_with_its_result() {
    // Break caught: a rescan that listed the folders before FastPad recycled one bringing
    // its row and its notes back, rows that open files that are gone.
    let scratch = Scratch::new("folder-merge-removed");
    let folder = scratch.folder();
    std::fs::create_dir_all(folder.join(r"old\inner")).unwrap();
    std::fs::write(folder.join(r"old\a.md"), "a").unwrap();
    std::fs::write(folder.join(r"old\inner\b.md"), "b").unwrap();
    std::fs::write(folder.join("keep.md"), "k").unwrap();
    let mut previous = load(&folder, &scratch.local(), 100).unwrap();
    let stale = load(&folder, &scratch.local(), 101).unwrap();
    std::fs::remove_dir_all(folder.join("old")).unwrap();
    previous.remove_folder(Path::new("old"), 500);

    let merged = merge_rescan(previous, stale);

    assert!(sorted_folders(&merged).is_empty());
    assert_eq!(sorted_notes(&merged), [PathBuf::from("keep.md")]);
    assert!(!merged.is_folder(Path::new("old")));
    assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
}

#[test]
fn a_note_saved_into_an_unlisted_folder_lists_its_folders_so_the_tree_matches_a_rebuild() {
    // Break caught: a Save As into a folder the scan never listed (made in the save dialog)
    // giving the tree folder rows that a rebuild from the state's folders would not have,
    // so the rows drift once the note is removed and the emptied folders stay.
    let scratch = Scratch::new("folder-unlisted");
    let folder = scratch.folder();
    let mut state = load(&folder, &scratch.local(), 100).unwrap();
    std::fs::create_dir_all(folder.join(r"new\deeper")).unwrap();
    let note = folder.join(r"new\deeper\n.md");
    std::fs::write(&note, "n").unwrap();
    assert!(state.add_note(&note));
    assert!(!state.add_note(&folder.join(r"NEW\Deeper\n.md")));
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

    state.remove_note(&note);

    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
    assert_eq!(
        sorted_folders(&state),
        ["new", r"new\deeper"].map(PathBuf::from)
    );
    let top = folder.join("top.md");
    std::fs::write(&top, "t").unwrap();
    assert!(state.add_note(&top));
    std::fs::create_dir(folder.join(r"new\other")).unwrap();
    let moved = folder.join(r"New\other\m.md");
    std::fs::rename(&top, &moved).unwrap();
    state.rename_note(&top, &moved);
    assert_eq!(
        sorted_folders(&state),
        [r"New\other", "new", r"new\deeper"].map(PathBuf::from),
        "a rename lists only the missing folder, once, spelled as in its path"
    );
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
}

#[test]
fn a_note_saved_into_a_new_folder_while_a_rescan_ran_lists_that_folder_after_the_merge() {
    // Break caught: a rescan that listed the folders before the save dialog made a new one
    // merging the saved note back in with folder rows its folder list lacks.
    let scratch = Scratch::new("folder-unlisted-merge");
    let folder = scratch.folder();
    let mut previous = load(&folder, &scratch.local(), 100).unwrap();
    let stale = load(&folder, &scratch.local(), 101).unwrap();
    std::fs::create_dir(folder.join("made")).unwrap();
    let note = folder.join(r"made\n.md");
    std::fs::write(&note, "n").unwrap();
    assert!(previous.add_note(&note));

    let merged = merge_rescan(previous, stale);

    assert_eq!(sorted_folders(&merged), [PathBuf::from("made")]);
    assert_eq!(sorted_notes(&merged), [PathBuf::from(r"made\n.md")]);
    assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
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

fn tree_rows(tree: &tree::NoteTree) -> Vec<tree::TreeRow> {
    tree.rows(&|_| true)
}

/// What a fresh build of the state's notes, folders and pins shows.
fn rebuilt_rows(state: &LibraryState) -> Vec<tree::TreeRow> {
    let paths: Vec<PathBuf> = state.notes.iter().map(|note| note.path.clone()).collect();
    tree_rows(&tree::NoteTree::build(
        &paths,
        &state.folders,
        &pinned_paths(&state.library),
    ))
}

#[test]
fn the_tree_follows_the_index_and_the_pins() {
    // Break caught: the sidebar tree drifting from the notes index after a save, a pin, a
    // rename or a delete, so a row opens a file that is gone or a pin shows on the wrong note.
    let scratch = Scratch::new("tree-follows");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
    std::fs::write(scratch.folder().join(r"sub\b.md"), "b").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    assert_eq!(state.tree.note_count(), 2);
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

    let c = scratch.folder().join("c.md");
    std::fs::write(&c, "c").unwrap();
    state.add_note(&c);
    state.add_note(&c);
    assert_eq!(state.tree.note_count(), 3);
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

    let target = state.note_ref(&mut ids, &c);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    assert!(
        tree_rows(&state.tree)[0].pinned,
        "a pinned note sorts first"
    );
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

    let renamed = scratch.folder().join(r"sub\z.md");
    std::fs::rename(&c, &renamed).unwrap();
    state.rename_note(&c, &renamed);
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
    assert!(
        tree_rows(&state.tree)
            .iter()
            .any(|row| row.pinned && row.kind == tree::RowKind::Note(PathBuf::from(r"sub\z.md"))),
        "the pin follows the rename"
    );

    state.remove_note(&renamed);
    state.remove_note(&scratch.folder().join(r"sub\b.md"));
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
    assert!(
        tree_rows(&state.tree)
            .iter()
            .any(|row| row.kind == tree::RowKind::Folder(PathBuf::from("sub"))),
        "an emptied folder stays while it is on disk"
    );
}

#[test]
fn merging_a_rescan_leaves_the_tree_equal_to_a_rebuild() {
    // Break caught: the merged state keeping the rescan's tree, which misses a note saved or
    // a pin set while the rescan ran.
    let scratch = Scratch::new("tree-merge");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
    let b = scratch.folder().join("b.md");
    std::fs::write(&b, "b").unwrap();
    previous.add_note(&b);
    let target = previous.note_ref(&mut ids, &a);
    previous
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    let merged = merge_rescan(previous, fresh);
    assert_eq!(merged.tree.note_count(), 2);
    assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
    assert!(tree_rows(&merged.tree)[0].pinned);
}

#[test]
fn a_flush_that_rereads_another_pcs_pins_updates_the_tree() {
    // Break caught: a pin synced from another PC reaching library.ini and the live library
    // but never the tree, so the row stays unpinned until the next rescan.
    let scratch = Scratch::new("tree-flush");
    let a = scratch.folder().join("a.md");
    std::fs::write(&a, "a").unwrap();
    std::fs::write(scratch.folder().join("b.md"), "b").unwrap();
    let mut ids = IdSource::new(1, 2);
    let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
    let target = state.note_ref(&mut ids, &a);
    state
        .apply(PendingOp::SetPinned {
            note: target,
            value: true,
        })
        .unwrap();
    let path = store::library_file(&scratch.folder());
    let mut other = Library::default();
    other
        .resolve_note(&NoteRef {
            id: NoteId(77),
            path: "b.md".into(),
        })
        .pinned = true;
    store::write(&path, &other).unwrap();
    assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
    assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
    assert_eq!(
        tree_rows(&state.tree)
            .iter()
            .filter(|row| row.pinned)
            .count(),
        2
    );
}
