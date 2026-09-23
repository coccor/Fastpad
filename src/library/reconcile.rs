//! Keeps records attached to their files. Scan results are matched against records in order:
//! path (ignoring case); then the cached file ID (a rename or move in Explorer), checked for
//! every still-unmatched record before fingerprint matching is tried; then size and content
//! hash, restricted to files that are new since the last scan (a copy or sync). Anything still
//! unmatched is missing; after 30 days missing it is dropped. A truncated scan never marks
//! records missing or purges them, since a file past the scan limit is not really gone.
//! Only operations that actually applied are kept in the result.

use super::local::{CachedFile, LocalState};
use super::model::{Library, NoteRecord, NoteRef};
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

/// Applies `op`; only on success is it kept in `ops`.
fn apply_op(library: &mut Library, ops: &mut Vec<PendingOp>, op: PendingOp) -> bool {
    if apply(library, &op).is_ok() {
        ops.push(op);
        true
    } else {
        false
    }
}

/// Records a match found outside the path step (file ID or fingerprint): relocates the record,
/// and restores it if it was flagged deleted. Only ops that applied are reflected in `result`.
fn relocate(
    library: &mut Library,
    local: &mut LocalState,
    result: &mut Reconciled,
    record: &NoteRecord,
    path: PathBuf,
) {
    let note = NoteRef {
        id: record.id,
        path: record.path.clone(),
    };
    local.clear_missing(record.id);
    let op = PendingOp::Relocate {
        note: note.clone(),
        path: path.clone(),
    };
    if apply_op(library, &mut result.ops, op) {
        result.relocated.push((record.path.clone(), path));
    }
    if record.deleted {
        apply_op(
            library,
            &mut result.ops,
            PendingOp::SetDeleted { note, value: false },
        );
    }
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
    // The scan cache as of the previous scan. A path already in it is not "new".
    let cache: HashMap<String, CachedFile> = local
        .files
        .iter()
        .map(|file| (key(&file.path), file.clone()))
        .collect();
    let is_new = |index: usize| !cache.contains_key(&key(&scan.entries[index].path));
    let mut claimed = vec![false; scan.entries.len()];
    let mut hashes: HashMap<usize, Option<u64>> = HashMap::new();

    // 1. Path (ignoring case). A record whose path is already claimed by an earlier record
    // (two records collapsing onto the same case-insensitive path) is treated as unmatched.
    let mut unmatched = Vec::new();
    for record in library
        .notes
        .clone()
        .into_iter()
        .filter(|record| !record.path.is_absolute())
    {
        let index = by_path
            .get(&key(&record.path))
            .copied()
            .filter(|&index| !claimed[index]);
        let Some(index) = index else {
            unmatched.push(record);
            continue;
        };
        claimed[index] = true;
        let note = NoteRef {
            id: record.id,
            path: record.path.clone(),
        };
        let entry = &scan.entries[index];
        local.clear_missing(record.id);
        if record.path != entry.path {
            let op = PendingOp::Relocate {
                note: note.clone(),
                path: entry.path.clone(),
            };
            if apply_op(library, &mut result.ops, op) {
                result
                    .relocated
                    .push((record.path.clone(), entry.path.clone()));
            }
        }
        if record.deleted {
            apply_op(
                library,
                &mut result.ops,
                PendingOp::SetDeleted {
                    note: note.clone(),
                    value: false,
                },
            );
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
            apply_op(
                library,
                &mut result.ops,
                PendingOp::SetFingerprint {
                    note,
                    size: entry.size,
                    hash: value,
                },
            );
        }
    }

    // 2. File ID: a full pass over every unmatched record, against new files only, before any
    // record falls back to fingerprint matching.
    let mut still_unmatched = Vec::new();
    for record in unmatched {
        let by_id = cache
            .get(&key(&record.path))
            .filter(|cached| cached.file_id != 0 && cached.volume == scan.volume)
            .and_then(|cached| {
                (0..scan.entries.len()).find(|&index| {
                    !claimed[index]
                        && is_new(index)
                        && scan.entries[index].file_id == cached.file_id
                })
            });
        match by_id {
            Some(index) => {
                claimed[index] = true;
                relocate(
                    library,
                    local,
                    &mut result,
                    &record,
                    scan.entries[index].path.clone(),
                );
            }
            None => still_unmatched.push(record),
        }
    }

    // 3. Fingerprint: only records still unmatched after the file-ID pass, only against new files.
    let mut missing = Vec::new();
    for record in still_unmatched {
        let found = if record.size == 0 || record.size > HASH_LIMIT {
            None
        } else {
            (0..scan.entries.len()).find(|&index| {
                let entry = &scan.entries[index];
                !claimed[index]
                    && is_new(index)
                    && !entry.online_only
                    && entry.size == record.size
                    && *hashes.entry(index).or_insert_with(|| hash(&entry.path))
                        == Some(record.hash)
            })
        };
        match found {
            Some(index) => {
                claimed[index] = true;
                relocate(
                    library,
                    local,
                    &mut result,
                    &record,
                    scan.entries[index].path.clone(),
                );
            }
            None => missing.push(record),
        }
    }

    // 4. Missing. Skipped entirely for a truncated scan: a file past the scan limit is not
    // really gone, so it must not be marked missing or purged.
    if !scan.truncated {
        for record in missing {
            local.set_missing(record.id, now);
            let since = local.missing_since(record.id).unwrap_or(now);
            if now.saturating_sub(since) >= PURGE_AFTER_SECS
                && apply_op(library, &mut result.ops, PendingOp::Drop { id: record.id })
            {
                local.clear_missing(record.id);
            }
        }
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
        ScanEntry {
            path: path.into(),
            size,
            mtime: 1,
            file_id,
            online_only: false,
        }
    }

    fn record(id: u128, path: &str, size: u64, hash: u64) -> NoteRecord {
        let mut record = NoteRecord::new(NoteId(id), path.into());
        record.pinned = true;
        record.size = size;
        record.hash = hash;
        record
    }

    fn cached(path: &str, size: u64, file_id: u64) -> CachedFile {
        CachedFile {
            volume: VOLUME,
            file_id,
            mtime: 1,
            size,
            path: path.into(),
        }
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
                library: Library { notes: records },
                local,
                scan: Scan {
                    volume: VOLUME,
                    entries,
                    truncated: false,
                },
                hashes: HashMap::new(),
                hashed: Vec::new(),
            }
        }

        fn run(&mut self, now: u64) -> Reconciled {
            let hashes = self.hashes.clone();
            let hashed = &mut self.hashed;
            reconcile(
                &mut self.library,
                &mut self.local,
                &self.scan,
                now,
                &mut |path| {
                    let key = path.to_string_lossy().into_owned();
                    hashed.push(key.clone());
                    hashes.get(&key).copied()
                },
            )
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
        assert!(
            fixture.hashed.is_empty(),
            "an unchanged file is not re-hashed"
        );
    }

    #[test]
    fn a_rename_outside_fastpad_is_followed_by_file_id() {
        // Break caught: renaming a note in Explorer losing its pin.
        let mut fixture = Fixture::new(
            vec![record(1, "old.md", 3, 9)],
            vec![cached("old.md", 3, 55)],
            vec![entry(r"sub\new.md", 3, 55)],
        );
        let result = fixture.run(100);
        assert_eq!(fixture.path_of(1), Some(PathBuf::from(r"sub\new.md")));
        assert_eq!(
            result.relocated,
            vec![(PathBuf::from("old.md"), PathBuf::from(r"sub\new.md"))]
        );
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
        let mut fixture = Fixture::new(
            vec![record(1, "old.md", 3, 9)],
            vec![],
            vec![entry("x.md", 3, 70)],
        );
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
        let mut fixture = Fixture::new(
            vec![record(1, "old.log", big, 9)],
            vec![],
            vec![entry("huge.log", big, 70)],
        );
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

    #[test]
    fn file_id_matching_runs_to_completion_before_fingerprint_matching_is_tried() {
        // Break caught: an unmatched record with no usable cache entry falling back to
        // fingerprint matching and stealing a file that a later record would have claimed by
        // file ID.
        let mut fixture = Fixture::new(
            vec![record(1, "a.md", 3, 9), record(2, "b.md", 3, 9)],
            vec![cached("b.md", 3, 55)],
            vec![entry("c.md", 3, 55)],
        );
        fixture.hashes.insert("c.md".into(), 9);
        fixture.run(100);
        assert_eq!(
            fixture.path_of(2),
            Some(PathBuf::from("c.md")),
            "B keeps its file-ID match"
        );
        assert_eq!(
            fixture.path_of(1),
            Some(PathBuf::from("a.md")),
            "A is not relocated"
        );
        assert_eq!(
            fixture.local.missing_since(NoteId(1)),
            Some(100),
            "A goes missing instead"
        );
    }

    #[test]
    fn the_fingerprint_step_only_matches_files_new_since_the_last_scan() {
        // Break caught: a long-standing file matching a now-missing record's fingerprint by
        // coincidence and stealing that record, merely because it was still unclaimed.
        let mut fixture = Fixture::new(
            vec![record(1, "a.md", 3, 9)],
            vec![cached("b.md", 3, 70)],
            vec![entry("b.md", 3, 70)],
        );
        fixture.hashes.insert("b.md".into(), 9);
        fixture.run(100);
        assert_eq!(
            fixture.path_of(1),
            Some(PathBuf::from("a.md")),
            "b.md already existed, so it cannot match"
        );
        assert_eq!(fixture.local.missing_since(NoteId(1)), Some(100));
        assert!(
            fixture.hashed.is_empty(),
            "a file that is not new is never hashed"
        );
    }

    #[test]
    fn a_truncated_scan_never_marks_records_missing_or_purges_them() {
        let mut fixture = Fixture::new(vec![record(1, "gone.md", 3, 9)], vec![], vec![]);
        fixture.scan.truncated = true;
        fixture.run(1_000);
        assert_eq!(
            fixture.local.missing_since(NoteId(1)),
            None,
            "a truncated scan never marks missing"
        );
        fixture.local.set_missing(NoteId(1), 1_000);
        let result = fixture.run(1_000 + PURGE_AFTER_SECS);
        assert!(
            fixture.library.note(NoteId(1)).is_some(),
            "a truncated scan never purges"
        );
        assert!(result.ops.is_empty());
    }

    #[test]
    fn a_second_record_matching_an_already_claimed_path_goes_missing_instead() {
        // Break caught: two records collapsing onto the same case-insensitive path both being
        // treated as matched, leaving the library with two records pointing at one file.
        let mut fixture = Fixture::new(
            vec![record(1, "plan.md", 3, 9), record(2, "PLAN.MD", 3, 9)],
            vec![],
            vec![entry("Plan.md", 3, 5)],
        );
        fixture.run(100);
        let matched = [1_u128, 2]
            .into_iter()
            .filter(|&id| fixture.path_of(id) == Some(PathBuf::from("Plan.md")))
            .count();
        assert_eq!(matched, 1, "exactly one record claims the file");
        let missing = [NoteId(1), NoteId(2)]
            .into_iter()
            .filter(|&id| fixture.local.missing_since(id).is_some())
            .count();
        assert_eq!(missing, 1, "the other record goes missing");
    }
}
