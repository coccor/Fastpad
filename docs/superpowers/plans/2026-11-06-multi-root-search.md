# Multi-root notebooks, phase 2: search across notebooks — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Search, Replace in files, Quick Open and Tags cover every open notebook, with each result labelled by its notebook. The phase 1 default-notebook shims leave production code.

**Architecture:** Indexes, note lists and link resolution stay per notebook. Pure merge functions in `src/library` (`quick_open::search_many`, `index::merge_tags`, `text_search::run_roots`, `roots::notebook_labels`) take several notebooks' data and return results tagged with a caller-chosen key. The window layer passes `NotebookId` as that key, and resolves a result's folder by id only when it is opened. `searchable_notebooks(hwnd)` is the one list every view iterates over.

**Tech Stack:** Rust, Win32 via windows-sys, Scintilla. Unit tests in `src/library`. Window tests in `src/window/**/tests*`, run in the `in_process` target.

**Spec:** `docs/superpowers/specs/2026-11-06-multi-root-search-design.md`. The phase 1 spec, `docs/superpowers/specs/2026-11-05-multi-root-notebooks-design.md`, defines `NotebookId`, slots and `target_notebook`.

## Global Constraints

- **Unchanged with one notebook open:** every string, label and layout in Search, Quick Open and Tags stays exactly as it is today. Existing single-notebook tests pass unmodified, except the phase 1 target-notebook tests named in each task.
- **Notebook part in the dim text:** it appears only when two or more notebooks are open. The separator is ` › ` (space, U+203A, space).
- **Copy, verbatim:**
  - Header: "Searching all notebooks" / "Searching {name}". Placeholder: "Search text in all notebooks" / "Search text in {name}".
  - Quick Open placeholder: "Search notes in all notebooks".
  - Tags header: "Tags in all notebooks" / "Tags in {name}".
  - Scope button: "All notebooks ▾" / "{name} ▾". Scope menu: "All notebooks", then a separator, then each notebook. Accessible names: "Search scope: All notebooks" / "Search scope: {name}".
  - Closed notebook notice: "That note's notebook was closed."
  - Skipped notebooks: " (1 notebook still loading)", " (N notebooks still loading)", " (1 notebook unavailable)", " (N notebooks unavailable)", appended to the header. With both kinds, loading comes first, as " (1 notebook still loading, 1 notebook unavailable)".
  - Replace skip: "Skipped {name}: it was closed."
- **Labels:** a notebook's name; on a duplicate name, "{name} ({parent folder name})"; if that is still a duplicate, "{name} ({full parent path})".
- **Caps:** `RESULT_CAP` (500) and `QUICK_OPEN_ROWS` (50) are totals across notebooks.
- **No changes** to `NoteIndex`, `IndexedNote`, `Resolver`, backlinks, the Links view, wikilink completion or tag completion.
- **CI gate:** `cargo fmt --all -- --check`; `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings`; `cargo test --locked --target x86_64-pc-windows-msvc --no-fail-fast -- --test-threads=1`.
- **Commits:** no `Co-Authored-By` trailer. Never push to `main`.

## Review Focus

1. **A notebook closes while results are on screen** (Search, Quick Open still open, Tags). Rows from it vanish, and activating a stale row shows the closed notice instead of opening a path joined to another notebook's folder. Tests: Task 1 (`open_note_ref_of_a_closed_notebook_notifies`), Task 5 (`closing_b_drops_its_hits`).
2. **Same relative path in two notebooks** (both have `todo.md`). Results stay distinct, and each opens its own file; nothing dedupes by relative path. Tests: Task 2 (`same_path_in_two_notebooks_gives_two_rows`), Task 3 (`merge_keeps_same_path_from_two_notebooks`), Task 5 (`same_named_note_in_both_opens_each`).
3. **Dirty tab in notebook B while replacing across A and B.** The overlay for B's note is found by B's folder, not A's, and the tab is edited, not the file. Test: Task 6 (`replace_across_uses_each_notebooks_dirty_tab`).
4. **A tag with different spellings across notebooks** (`#Project` in A, `#project` in B). One node, with the spelling from the first notebook in sidebar order and the counts summed. Test: Task 3 (`merge_folds_case_and_keeps_first_spelling`).
5. **Narrowing after a notebook opens mid-session.** A longer query must not reuse hits that never searched the new notebook. Test: Task 4 (`narrowing_reruns_when_the_notebook_set_changes`).

---

### Task 1: Searchable notebooks, labels and opening by `NoteRef`

**Files:**
- Modify: `src/library/roots.rs` (labels and prefix, pure)
- Create: `src/window/library_host/searchable.rs`; declare it in `src/window/library_host.rs`
- Test: unit tests in `src/library/roots.rs`; `src/window/main_window/tests/searchable_notebooks.rs` (register it in the tests module)

**Interfaces:**
- Produces:
  - `crate::library::roots::notebook_labels(folders: &[PathBuf]) -> Vec<String>` (same order and length as the input).
  - `crate::library::roots::notebook_prefixed(label: &str, folder_text: &str, several: bool) -> String`.
  - `library_host::Searchable { pub id: NotebookId, pub folder: PathBuf, pub label: String }`.
  - `library_host::searchable_notebooks(hwnd) -> Vec<Searchable>`.
  - `library_host::skipped_notebooks(hwnd) -> Skipped`, where `Skipped { pub loading: usize, pub unavailable: usize }`.
  - `library_host::skipped_suffix(skipped: Skipped) -> String` (`""` when both are 0).
  - `library_host::several_notebooks(hwnd) -> bool` (true when two or more notebooks are open, counting loading and unavailable ones).
  - `library_host::NoteRef { pub notebook: NotebookId, pub path: PathBuf }`.
  - `library_host::open_note_ref(hwnd, note: &NoteRef, keep: bool, line: Option<u32>) -> bool`.
- Consumes: `notebooks(hwnd)`, `with_notebook`, `notebook_folder`, `notebook_display_name`, the slot's `state.is_some()` and the unavailable list (all phase 1), and `main_window::opening::open_note`.

- [ ] **Step 1: Write the failing unit tests in `src/library/roots.rs`**

```rust
#[test]
fn labels_are_names_unless_two_collide() {
    let f = |s: &str| PathBuf::from(s);
    assert_eq!(notebook_labels(&[f(r"C:\n\Work"), f(r"C:\n\Home")]), ["Work", "Home"]);
    assert_eq!(
        notebook_labels(&[f(r"D:\Work"), f(r"C:\Archive\Work"), f(r"C:\n\Home")]),
        ["Work (D:)", "Work (Archive)", "Home"]
    );
    assert_eq!(
        notebook_labels(&[f(r"C:\a\x\Work"), f(r"C:\b\x\Work")]),
        [r"Work (C:\a\x)", r"Work (C:\b\x)"]
    );
}

#[test]
fn prefixed_only_with_several() {
    assert_eq!(notebook_prefixed("Work", r"team\q3", false), r"team\q3");
    assert_eq!(notebook_prefixed("Work", r"team\q3", true), r"Work › team\q3");
    assert_eq!(notebook_prefixed("Work", "", true), "Work");
    assert_eq!(notebook_prefixed("Work", "", false), "");
}
```

Names are compared ignoring case. The parent folder name of a drive root is the drive (`D:`).

- [ ] **Step 2: Run them and see them fail**

Run: `cargo test --locked --target x86_64-pc-windows-msvc --lib roots::`
Expected: compile errors (`notebook_labels` not found).

- [ ] **Step 3: Implement both functions in `src/library/roots.rs`.** Then run Step 2's command; expected PASS.

- [ ] **Step 4: Write failing window tests in `tests/searchable_notebooks.rs`.** Use `notebooks_a_and_b`.

```rust
#[test]
fn searchable_lists_loaded_notebooks_in_sidebar_order() {
    let ab = notebooks_a_and_b("searchable_order", "a", "b");
    let list = library_host::searchable_notebooks(ab.window.hwnd);
    assert_eq!(list.iter().map(|s| (s.id, s.label.as_str())).collect::<Vec<_>>(),
               [(ab.a_id, "A"), (ab.b_id, "B")]);
    assert!(library_host::several_notebooks(ab.window.hwnd));
    assert_eq!(library_host::skipped_suffix(library_host::skipped_notebooks(ab.window.hwnd)), "");
}

#[test]
fn skipped_suffix_wording() {
    use library_host::{skipped_suffix, Skipped};
    assert_eq!(skipped_suffix(Skipped { loading: 1, unavailable: 0 }), " (1 notebook still loading)");
    assert_eq!(skipped_suffix(Skipped { loading: 2, unavailable: 0 }), " (2 notebooks still loading)");
    assert_eq!(skipped_suffix(Skipped { loading: 0, unavailable: 3 }), " (3 notebooks unavailable)");
    assert_eq!(skipped_suffix(Skipped { loading: 1, unavailable: 1 }),
               " (1 notebook still loading, 1 notebook unavailable)");
}

#[test]
fn open_note_ref_opens_from_its_own_notebook() {
    let ab = notebooks_a_and_b("noteref_open", "a", "b");
    assert!(library_host::open_note_ref(ab.window.hwnd,
        &library_host::NoteRef { notebook: ab.b_id, path: "b.md".into() }, true, None));
    assert_eq!(active_path(&ab.window), Some(ab.b.join("b.md")));
}

#[test]
fn open_note_ref_of_a_closed_notebook_notifies() {
    let ab = notebooks_a_and_b("noteref_closed", "a", "b");
    assert!(library_host::close_notebook_of(ab.window.hwnd, ab.b_id));
    assert!(!library_host::open_note_ref(ab.window.hwnd,
        &library_host::NoteRef { notebook: ab.b_id, path: "b.md".into() }, true, None));
    assert_eq!(last_notice(&ab.window).as_deref(), Some("That note's notebook was closed."));
    assert_ne!(active_path(&ab.window), Some(ab.a.join("b.md")));
}
```

Use the test module's existing helpers for the active tab's path and the latest notice. If their names differ from `active_path` and `last_notice`, use the existing ones; don't add duplicates.

- [ ] **Step 5: Run them and see them fail**

Run: `cargo test --locked --target x86_64-pc-windows-msvc --test in_process searchable_notebooks -- --test-threads=1`
Expected: compile errors.

- [ ] **Step 6: Implement `searchable.rs`.**
  - `searchable_notebooks` returns slots with `state.is_some()`, in `notebooks` order, with labels from `notebook_labels` over every open folder.
  - `skipped_notebooks` counts open slots without state as loading, and the phase 1 unavailable list as unavailable.
  - `open_note_ref` resolves the folder with `notebook_folder`. When that is `None`, it shows the notice through the existing notice helper and returns false. Otherwise it calls `open_note(hwnd, &folder.join(&note.path), keep, line)`, matching that function's real parameters.

- [ ] **Step 7: Run Step 5's command.** Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/library/roots.rs src/window/library_host.rs src/window/library_host/searchable.rs src/window/main_window/tests/searchable_notebooks.rs src/window/main_window/tests.rs
git commit -m "feat(notes): list searchable notebooks, label them, open a note by notebook id"
```

---

### Task 2: Quick Open across notebooks

**Files:**
- Modify: `src/library/quick_open.rs`
- Modify: `src/window/command_palette.rs` (`PickerRow::Note`, `PickerChoice::Note` / `View`, the accessible name, the placeholder)
- Modify: `src/window/main_window/command_palette_ui.rs` (`quick_open_rows`, `open_quick_open_choice`; remove `quick_open_notebook`)
- Modify: `src/window/library_host/pickers.rs` (the `PickerChoice::View` arm)
- Test: unit tests in `quick_open.rs`; `src/window/main_window/tests/command_palette.rs`

**Interfaces:**
- Consumes: Task 1's `searchable_notebooks`, `several_notebooks`, `notebook_prefixed`, `NoteRef` and `open_note_ref`.
- Produces:
  - `quick_open::search_many<'a, K: Copy, P>(sources: Vec<(K, Vec<&'a P>)>, query: &str, limit: usize) -> Vec<(K, QuickMatch)>`, where `P: AsRef<Path> + ?Sized + 'a`.
  - `PickerRow::Note { notebook: NotebookId, label: String, found: QuickMatch, line: Option<u32> }`, where `label` is the dim prefix, or `""` when only one notebook is open.
  - `PickerChoice::Note { notebook: NotebookId, path, line }` and `PickerChoice::View { notebook: NotebookId, path, group }`.

- [ ] **Step 1: Failing unit tests in `quick_open.rs`**

```rust
#[test]
fn search_many_ranks_across_sources_and_breaks_ties_by_source_order() {
    let a = [PathBuf::from("ideas.md"), PathBuf::from(r"x\meeting notes.md")];
    let b = [PathBuf::from("meeting.md"), PathBuf::from("ideas.md")];
    let found = search_many(vec![(0u32, a.iter().collect()), (1u32, b.iter().collect())], "meeting", 50);
    assert_eq!(found[0].0, 1);                       // exact name in B beats A's longer name
    assert_eq!(found[0].1.path, PathBuf::from("meeting.md"));
    let ideas = search_many(vec![(0u32, a.iter().collect()), (1u32, b.iter().collect())], "ideas", 50);
    assert_eq!(ideas.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [0, 1]); // equal score: source order
}

#[test]
fn same_path_in_two_notebooks_gives_two_rows() {
    let a = [PathBuf::from("todo.md")];
    let b = [PathBuf::from("todo.md")];
    assert_eq!(search_many(vec![(0u32, a.iter().collect()), (1u32, b.iter().collect())], "todo", 50).len(), 2);
}

#[test]
fn search_many_caps_the_total() {
    let a: Vec<PathBuf> = (0..40).map(|i| PathBuf::from(format!("n{i}.md"))).collect();
    let b: Vec<PathBuf> = (0..40).map(|i| PathBuf::from(format!("n{i}x.md"))).collect();
    assert_eq!(search_many(vec![(0u32, a.iter().collect()), (1u32, b.iter().collect())], "n", 50).len(), 50);
}
```

- [ ] **Step 2: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --lib quick_open::`. Expected: compile errors.

- [ ] **Step 3: Implement `search_many`.** Refactor `search` so each source is scored with today's per-note scoring and `rank` comparison. Merge all sources under that same comparison, with source order as the last tie-break before the path, and truncate to `limit`. `search` becomes `search_many` with one source, so its results are unchanged. Run Step 2's command; expected PASS, with every existing `quick_open` test still passing.

- [ ] **Step 4: Failing window tests in `tests/command_palette.rs`.** Rewrite `quick_open_lists_the_target_notebook_and_names_it` (phase 1 behaviour) as `quick_open_lists_every_notebook_and_labels_rows`:
  - With A holding `a.md` and B holding `b.md`, typing `md` lists both rows.
  - The B row's accessible name is "b.md, in B". A note at the root uses the label as its folder; a note in a subfolder reads "x.md, in B › sub".
  - The placeholder is "Search notes in all notebooks".
  - Picking the B row opens `B\b.md`.
  
  Add `quick_open_empty_query_lists_tabs_from_both`: open `a.md` and `b.md` as tabs, and with an empty query both appear as View rows. Add `quick_open_with_one_notebook_is_unchanged`: one notebook, so the placeholder is "Search notes in {name}" and the accessible name has no notebook part.

- [ ] **Step 5: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --test in_process command_palette -- --test-threads=1`.

- [ ] **Step 6: Implement.**
  - `quick_open_rows` builds sources from `searchable_notebooks` and each slot's `state.notes` paths.
  - Each row's `label` is `notebook_prefixed(label, "", true)` when `several_notebooks` is true, and empty otherwise. The painted dim text and the accessible name use `notebook_prefixed(label, &found.folder, several)`.
  - With an empty query, list tabs whose path is inside any open notebook (`notebook_of`), each with that notebook's id.
  - Picking calls `open_note_ref`. The `View` arm resolves the folder by the choice's `notebook`.
  - With no notebook open at all, keep `NO_NOTEBOOK`.
  - The skipped-notebooks suffix isn't added to the Quick Open placeholder. Quick Open has no header, and spec §3.2's count applies to the Search and Tags headers.

- [ ] **Step 7: Run Step 5's command.** Expected: PASS.

- [ ] **Step 8: Commit** with `feat(quick-open): list notes from every open notebook`.

---

### Task 3: Tags across notebooks

**Files:**
- Modify: `src/library/index.rs` (add the merge, generic over a key)
- Modify: `src/window/tags_view.rs`
- Modify: `src/window/side_panel.rs` (refresh triggers)
- Modify: `src/window/library_host/index.rs` and `notebooks.rs` (call `tags_view` refresh on index ready, open and close)
- Test: unit tests in `index.rs`; `tags_view.rs` tests; `src/window/main_window/tests/index_and_tags.rs` and `tests/notebook_tree_roots.rs`

**Interfaces:**
- Consumes: Task 1's `searchable_notebooks`, `several_notebooks`, `skipped_notebooks`, `skipped_suffix`, `notebook_prefixed`, `NoteRef` and `open_note_ref`.
- Produces:
  - `index::MergedTag<K> { pub name: String, pub full: String, pub notes: Vec<(K, PathBuf)>, pub count: usize, pub children: Vec<MergedTag<K>> }`.
  - `index::merge_tags<K: Copy>(sources: Vec<(K, Vec<TagNode>)>) -> Vec<MergedTag<K>>`.
  - `TagRowKind::Note(NoteRef)`, with the row's dim text built from `notebook_prefixed`.
  - `TagAction::Open { note: NoteRef, keep: bool }`.

- [ ] **Step 1: Failing unit tests in `index.rs`**

```rust
#[test]
fn merge_sums_counts_and_merges_children() {
    let a = vec![node("project", "project", &["a.md"], vec![node("q3", "project/q3", &["q.md"], vec![])])];
    let b = vec![node("project", "project", &["b.md"], vec![])];
    let merged = merge_tags(vec![(0u32, a), (1u32, b)]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].count, 3);
    assert_eq!(merged[0].notes, [(0, PathBuf::from("a.md")), (1, PathBuf::from("b.md"))]);
    assert_eq!(merged[0].children[0].full, "project/q3");
}

#[test]
fn merge_folds_case_and_keeps_first_spelling() {
    let a = vec![node("Project", "project", &["a.md"], vec![])];
    let b = vec![node("project", "project", &["b.md"], vec![])];
    let merged = merge_tags(vec![(0u32, a), (1u32, b)]);
    assert_eq!((merged[0].name.as_str(), merged[0].count), ("Project", 2));
}

#[test]
fn merge_keeps_same_path_from_two_notebooks() {
    let a = vec![node("x", "x", &["todo.md"], vec![])];
    let b = vec![node("x", "x", &["todo.md"], vec![])];
    assert_eq!(merge_tags(vec![(0u32, a), (1u32, b)])[0].notes.len(), 2);
}
```

`node(name, full, notes, children)` builds a `TagNode` whose `count` is `notes.len()` plus its children's counts. Add it to the test module if it isn't there. The merged order is by `full`, as `tags()` orders.

- [ ] **Step 2: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --lib index::`.

- [ ] **Step 3: Implement `merge_tags`.** Then run Step 2's command; expected PASS.

- [ ] **Step 4: Failing window tests.**
  - Rewrite the phase 1 "tags catch up to the target notebook" test in `tests/notebook_tree_roots.rs` as `tags_merge_every_notebook`. A holds `a.md` with `#x` and B holds `b.md` with `#x`. Expect one `x` row with count 2 and the header "Tags in all notebooks". Expanded, it shows two note rows whose dim text is "A" and "B". Activating the B row opens `B\b.md`.
  - Add `tags_refresh_when_a_notebook_closes`: close B, and the count becomes 1.
  - Add `tags_with_one_notebook_is_unchanged`: the header is "Tags in {name}" and rows have no notebook part.
  - Update `the_header_names_the_notebook_whose_tags_it_lists` in `tags_view.rs` to the new `header_text(Option<&str>)` contract: `None` gives "Tags in all notebooks". Keep the `Some(name)` case.

- [ ] **Step 5: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --test in_process tags -- --test-threads=1`.

- [ ] **Step 6: Implement.**
  - `refresh` merges `note_index_of(hwnd, s.id).tags()` for each `searchable_notebooks` entry.
  - The header uses `header_text(None)` when `several_notebooks`, or the single notebook's name, plus `skipped_suffix`.
  - Remove `TagsView.notebook` / `notebook_name` and `target_changed`, and their callers in `side_panel.rs`.
  - Call `tags_view::refresh` from the index-ready path, from `notebook_opened`, from `close_notebook_of`, and when a slot finishes loading.
  - `tag_rows` takes `&[MergedTag<NotebookId>]`. Its labels come from the searchable list, so map ids to labels once per refresh.

- [ ] **Step 7: Run Step 5's command, then the lib tags tests.** Expected: PASS.

- [ ] **Step 8: Commit** with `feat(tags): merge tags from every open notebook`.

---

### Task 4: Search engine over several notebooks (library)

**Files:**
- Modify: `src/library/text_search.rs`
- Test: unit tests in the same file

**Interfaces:**
- Produces:
  - `text_search::SearchRoot<'a> { pub folder: &'a Path, pub notes: &'a [SearchNote], pub overlays: &'a HashMap<PathBuf, String> }`.
  - `text_search::run_roots(roots: &[SearchRoot], matcher: &Matcher, cancel: &AtomicBool, sink: &mut dyn FnMut(Vec<(usize, TextHit)>, Progress), skipped: &mut Vec<(usize, PathBuf)>) -> RunEnd`.
  - The `usize` is the index into `roots`. `Progress.total` is the sum of every root's notes, and `RESULT_CAP` counts across roots.
  - `run` and `run_noting_skipped` stay as one-root wrappers, so existing tests pass unchanged.
- Also produces the per-notebook narrowing check used by Task 5, as a pure function: `text_search::marks_match(previous: &[(u32, u64)], current: &[(u32, u64)]) -> bool`. It is true only for identical key sets with equal marks, in any order. Keys are `NotebookId.0`.

- [ ] **Step 1: Failing unit tests**

```rust
#[test]
fn run_roots_tags_hits_with_their_root_and_shares_the_cap() {
    // two temp folders, each with RESULT_CAP notes containing "needle"
    // assert: RunEnd::Capped, total hits == RESULT_CAP, both root indices may appear,
    //         every hit's path exists under roots[index].folder
}

#[test]
fn run_roots_progress_totals_every_root() {
    // root 0 with 2 notes, root 1 with 3 notes, query matching none
    // assert: final Progress { visited: 5, total: 5, .. } and RunEnd::Completed
}

#[test]
fn run_roots_reads_each_roots_overlay_by_its_own_folder() {
    // root 0 and root 1 both have "n.md" on disk without the needle;
    // only root 1's overlays map "n.md" to text with the needle
    // assert: exactly one hit, with index 1
}

#[test]
fn narrowing_reruns_when_the_notebook_set_changes() {
    assert!(marks_match(&[(1, 10), (2, 20)], &[(2, 20), (1, 10)]));
    assert!(!marks_match(&[(1, 10)], &[(1, 10), (2, 20)]));
    assert!(!marks_match(&[(1, 10), (2, 20)], &[(1, 10), (2, 21)]));
}
```

Write the first three with the file's existing temp-folder helpers, keeping to the assertions in the comments.

- [ ] **Step 2: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --lib text_search::`.

- [ ] **Step 3: Implement.** Generalise `search_all` over one work list of `(root index, &SearchNote)` in root order, so the `MAX_READERS` readers, batching and the cap are shared. Each note's file is `roots[i].folder.join(path)`, and its overlay lookup uses `roots[i].overlays`. `run_with_clock` and the one-root wrappers call it with a single root and strip the index.

- [ ] **Step 4: Run Step 2's command.** Expected: PASS, with every existing `text_search` test passing.

- [ ] **Step 5: Commit** with `feat(search): run one text search over several notebooks`.

---

### Task 5: Search view across notebooks (results, query lifetime, scope)

**Files:**
- Modify: `src/window/text_search_host.rs` (search half: `SearchBatch`, `Narrowing`, `Running`, `list`, `searched`, `run_now`, `Job`, `spawn`, `batch_arrived`, `library_changed`, `notes_reloaded`)
- Modify: `src/window/search_view.rs`, `search_view/query.rs`, `search_view/text.rs`, `search_view/paint.rs` (the scope button and dim text), `search_view/tests.rs`
- Modify: `src/window/main_window/opening.rs` (search hit opening)
- Modify: `src/window/side_panel.rs` (remove the clear-on-target-change call)
- Test: `src/window/main_window/tests/search_view.rs`, `search_view/tests.rs`

**Interfaces:**
- Consumes: Task 1 (`searchable_notebooks`, `several_notebooks`, `skipped_*`, `notebook_prefixed`, `NoteRef`, `open_note_ref`) and Task 4 (`SearchRoot`, `run_roots`, `marks_match`).
- Produces:
  - `search_view::SearchHit { pub notebook: NotebookId, pub hit: TextHit }`, the view's result type.
  - `SearchBatch.hits: Vec<SearchHit>` and `SearchBatch.skipped: Vec<NoteRef>`.
  - `SearchView.scope: Option<NotebookId>` (`None` = All).
  - `search_view::set_scope(hwnd, scope: Option<NotebookId>)`, which reruns the query.
  - `search_view::scope(hwnd) -> Option<NotebookId>`.
  - `text_search_host::searched_roots(hwnd) -> Vec<Searchable>`: `searchable_notebooks` filtered by the scope.
  - `test_batch(generation, hits: Vec<SearchHit>, end)`, updated for the new hit type.
- Removed: `SearchView.notebook` / `notebook_name`, `search_view::notebook(hwnd)`, and the clear in `target_changed`.

- [ ] **Step 1: Failing window tests in `tests/search_view.rs`.** Rewrite the phase 1 "Searching A/B" target test as `search_covers_every_notebook`:
  - Both notes contain `needle`. Search, then:
    - both rows appear;
    - their dim text is "A" / "B";
    - the header is "Searching all notebooks";
    - the placeholder is "Search text in all notebooks".
  - Add these tests:
    - `switching_tabs_keeps_the_query`: open `a.md`, then `b.md`. The query box still holds `needle`, and both rows remain.
    - `opening_a_hit_from_b_opens_bs_file`.
    - `same_named_note_in_both_opens_each`: both notebooks get `todo.md` containing `needle`. There are two rows, and each opens its own folder's file.
    - `closing_b_drops_its_hits`: close B after the results arrive. Only A's row remains, and the query reruns.
    - `scope_limits_to_one_notebook`:
      1. `set_scope(Some(b_id))` leaves only B's row.
      2. The header reads "Searching B".
      3. The scope button's accessible name is "Search scope: B".
      4. Closing B sets `scope(hwnd) == None` and reruns.
    - `scope_button_hidden_with_one_notebook`: one notebook, so there is no scope button in the UIA tree, the header reads "Searching {name}", and the existing single-notebook tests are unchanged.
    - `opening_a_notebook_reruns_the_query`: search in A only, open B, and B's hit appears.
- Update any `search_view/tests.rs` unit tests that build `TextHit` lists to use `SearchHit`. Their expected strings don't change.

- [ ] **Step 2: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --test in_process search_view -- --test-threads=1`.

- [ ] **Step 3: Implement the search half.**
  - `Job` carries one entry per searched notebook: `(NotebookId, folder, notes, overlays from dirty_overlays(hwnd, &folder))`. The worker builds `SearchRoot`s and calls `run_roots`, then maps each index back to its id.
  - `Narrowing.paths` becomes `Vec<NoteRef>`.
  - `Narrowing.notes`, `Running.notes` and `TextSearchHost.list` become `Vec<(u32, u64)>`, with one mark per notebook, compared with `marks_match`.
  - `batch_arrived` drops hits whose notebook is no longer in `notebooks(hwnd)`.
  - Sort with `hit_cmp`, then by the notebook's position in `searchable_notebooks`.
  - `run_now` loops over the searched roots the same way.

- [ ] **Step 4: Implement the query lifetime.**
  - `library_changed` and `target_changed` no longer clear the query on a notebook change.
  - `notebook_opened`, `close_notebook_of` and load completion call a new `text_search_host::notebooks_changed(hwnd)`. It first removes rows from closed notebooks and resets the scope to `None` if the scoped notebook is gone. Then it reruns the current query if it is non-empty.

- [ ] **Step 5: Implement the scope button.**
  - It's drawn in the header row, right-aligned, only when `several_notebooks`, with the text "All notebooks ▾" / "{label} ▾".
  - A click, or Enter / Space when it has focus, opens a `TrackPopupMenu` with "All notebooks", a separator, then the labels, the current one checked. Picking calls `set_scope`.
  - It's exposed as a UIA button named "Search scope: …".
  - Header text: `header_text` gets the scope's label, or `None` for all, and appends `skipped_suffix`. With one notebook, it passes that notebook's name, exactly as today.

- [ ] **Step 6: Opening a hit** calls `open_note_ref`. Remove the join to a view-wide folder in `opening.rs`.

- [ ] **Step 7: Run Step 2's command, then all `search_view` lib tests.** Expected: PASS.

- [ ] **Step 8: Commit** with `feat(search): search every open notebook, with a scope menu`.

---

### Task 6: Replace in files across notebooks

**Files:**
- Modify: `src/window/text_search_host.rs` (replace half: `Candidate`, `ReplacePlan`, `replace_notebook`, `prepare`, `spawn_count`, `confirm_and_apply`, `apply_plan`, `spawn_write`, `report_written`, `clean_tabs_on`, `open_notes`, `target_overlays`, `confirm_text`, `report_text`)
- Test: `src/window/main_window/tests/search_replace.rs` and `search_replace_writes.rs`; the unit tests next to `confirm_text`

**Interfaces:**
- Consumes: Task 5's `SearchHit` and `NoteRef`.
- Produces:
  - `Candidate { pub notebook: NotebookId, pub path, pub name, pub stamp }`.
  - `ReplacePart { id: NotebookId, folder: PathBuf, candidates: Vec<Candidate> }`, and `ReplacePlan.parts: Vec<ReplacePart>` replacing `notebook_id` / `notebook` / `candidates`.
  - `TextSearchHost.replace_notebooks: Vec<NotebookId>` replacing `replace_notebook`.
  - `confirm_text(count, listed, capped, template, notebooks: &[String]) -> String`.
  - `report_text(…, closed: &[String])`, which appends "Skipped {name}: it was closed." once per closed notebook.

- [ ] **Step 1: Failing unit tests for the text**

```rust
#[test]
fn confirm_text_names_notebooks_only_when_several() {
    let count = ReplaceCount { matches: 14, notes: 6, closed_notes: 0 };
    assert_eq!(confirm_text(count, 6, false, "x", &["Work".into()]),
               "Replace 14 matches in 6 notes with \"x\"?");
    assert_eq!(confirm_text(count, 6, false, "x", &["Work".into(), "Personal".into()]),
               "Replace 14 matches in 6 notes across Work and Personal with \"x\"?");
    assert_eq!(confirm_text(count, 6, false, "x", &["Work".into(), "Personal".into(), "Archive".into()]),
               "Replace 14 matches in 6 notes across Work, Personal and Archive with \"x\"?");
    assert_eq!(confirm_text(count, 6, true, "x", &["Work".into(), "Personal".into()]),
               "Replace 14 matches in the 6 listed notes across Work and Personal with \"x\"? More notes match; search again to replace in the rest.");
}
```

Add a `report_text` case with `closed = ["Personal"]` whose output ends with "Skipped Personal: it was closed.". Put it after today's text, separated by a space, on the same line.

- [ ] **Step 2: Failing window tests in `search_replace_writes.rs`**
  - `replace_across_writes_both_notebooks`: A and B notes both contain `needle`. Replace all with `pin`, answer Yes, and both files on disk contain `pin`. The question contains "across A and B".
  - `replace_across_uses_each_notebooks_dirty_tab`: open `b.md` and edit it to contain `needle` without saving. Replace all, and B's tab text changes while B's file on disk is untouched. A's file is written.
  - `replace_skips_a_notebook_closed_mid_replace`: hold the write (use the existing `held_written` / `test_apply` seams), close B, and let the write go on. A is written, B is not, and the report notice contains "Skipped B: it was closed."
  - `replace_with_one_notebook_is_unchanged`: the existing single-notebook tests pass untouched.

- [ ] **Step 3: Run them and see them fail.** Run `cargo test --locked --target x86_64-pc-windows-msvc --test in_process search_replace -- --test-threads=1` and `--lib text_search_host`.

- [ ] **Step 4: Implement.**
  - `prepare` groups candidates by notebook, in sidebar order, into parts.
  - Count and write jobs loop over the parts, calling `text_replace::count` / `apply` with each part's folder and overlays. Sum the counts, and concatenate the reports, making their paths `NoteRef`s.
  - `dirty_overlays`, `open_notes` and `clean_tabs_on` are called per part folder.
  - Before applying each part, skip it if its id is no longer open, and record its label for the report.
  - `record_written_all` runs on each part's slot via `with_notebook`.
  - `plan_current` and `replacing` treat any id in `replace_notebooks` as pinned.

- [ ] **Step 5: Run Step 3's commands.** Expected: PASS.

- [ ] **Step 6: Commit** with `feat(search): replace in files across every searched notebook`.

---

### Task 7: Remove the default-notebook shims; README

**Files:**
- Modify: `src/window/library_host.rs` (`folder`, `with_state`), `src/window/library_host/index.rs` (`note_index`)
- Modify: every production caller. Find them with `grep -rn "library_host::\(folder\|with_state\|note_index\)(\|\bfolder(hwnd)\|with_state(hwnd" src --include=*.rs`, ignoring `tests/` and `#[cfg(test)]` blocks. Known so far: `src/window/inline_name.rs`, `src/window/markdown_completion.rs`, and callers inside `library_host/`.
- Modify: tests that call the shims (≈57 uses in 18 files under `src/window/main_window/tests/`), which switch to the renamed helpers.
- Modify: `README.md` (the Search, Quick Open and Tags sections)

**Interfaces:**
- Produces, `#[cfg(test)]` only: `library_host::default_folder(hwnd) -> Option<PathBuf>`, `library_host::with_default_state(hwnd, f) -> Option<R>` and `library_host::default_note_index(hwnd) -> Arc<NoteIndex>`. These have the same behaviour as the removed shims.

- [ ] **Step 1: Replace each production caller** with the explicit notebook it means:
  - a file-specific answer uses `notebook_of(hwnd, path)`;
  - a command's notebook uses `target_notebook`;
  - a link or completion lookup uses `link_notebook`;
  - quick capture or startup uses `default_notebook` or `capture_notebook`.

  Leave a one-line comment only where the choice isn't obvious from the call.
- [ ] **Step 2: Rename the shims** to the `#[cfg(test)]` helpers above and update the test call sites mechanically.
- [ ] **Step 3: Verify** that `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings` is clean. Then `grep -rn "fn folder(hwnd\|fn with_state(\|fn note_index(" src/window/library_host*` should find only nothing or `#[cfg(test)]` items.
- [ ] **Step 4: README.** Say that Search, Replace in files, Quick Open and Tags cover every open notebook and label results by notebook. Describe the Search scope menu. Keep the existing style and length.
- [ ] **Step 5: Run the full CI gate** (Global Constraints). Expected: all green.
- [ ] **Step 6: Commit** with `refactor(notes): drop the default-notebook shims; README for search across notebooks`.
