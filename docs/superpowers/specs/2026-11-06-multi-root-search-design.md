# Multi-root notebooks, phase 2: search across notebooks

**Branch:** `feat/multi-root-search`, stacked on `feat/multi-root-notebooks` (phase 1, PR #96). One PR, retargeted to `main` once phase 1 merges.
**Builds on:** `docs/superpowers/specs/2026-11-05-multi-root-notebooks-design.md`.

## 1. Goal

Search, Replace in files, Quick Open and Tags cover every open notebook instead of the target notebook. Each result says which notebook it came from. Wikilinks, backlinks and link completion stay inside their own notebook, as in phase 1.

Phase 2 also removes the phase 1 migration shims (phase 1 spec §3.5) from production code.

**Unchanged with one notebook open:** with exactly one notebook open, every string, label and layout in the three views is what it is today.

## 2. Decisions

| Question | Decision |
|---|---|
| How a result names its notebook | The row's dim folder text starts with the notebook name, as `Work › projects`. The list stays flat and ranked as today. The notebook part appears only when two or more notebooks are open. |
| Replace in files | Covers every notebook the search covered. The confirmation names them. |
| Narrowing to one notebook | A scope dropdown in the Search view only. Quick Open and Tags always cover every open notebook. |
| Search engine shape | One search job over a list of notebooks, sharing the existing reader pool and the result cap (§4.3). |
| Result identity | A `NotebookId` beside the existing notebook-relative path. Absolute paths are not stored in results. |
| Indexes | Stay per notebook, unchanged. Views merge their output. |
| Switching tabs or notebooks | No longer clears the Search query. |
| Tag completion in the editor | Unchanged: it offers the current note's notebook's tags only. |

## 3. Shared plumbing

### 3.1 Result identity

Add `NoteRef { notebook: NotebookId, path: PathBuf }`, where `path` is relative to that notebook's folder. `TextHit`, `QuickMatch`, `PickerChoice::Note` / `PickerChoice::View` and the Tags view's note rows carry a `NotebookId` beside their relative path (directly, or as a `NoteRef`).

Opening any result finds the folder through `with_notebook(hwnd, id, …)`. If that notebook is no longer open, nothing opens and a notice says "That note's notebook was closed."

Links code (`Resolver`, `facts`, `backlinks`, `path_key`) keeps keying on relative paths per notebook. `NoteRef` lives in the UI layer only; `NoteIndex` and `IndexedNote` do not change.

### 3.2 Which notebooks are searched

`searchable_notebooks(hwnd) -> Vec<Searchable>` returns, in sidebar order, every open notebook whose state is loaded: `Searchable { id: NotebookId, folder: PathBuf, label: String }`.

- Unavailable notebooks and notebooks still loading are skipped. The three views' headers add "(1 notebook still loading)" / "(2 notebooks still loading)" or "(1 notebook unavailable)" while any are skipped. When a skipped notebook finishes loading, the views refresh (§4.2, §5, §6).
- `label` is the notebook's name. When two open notebooks have the same name, each label adds its parent folder's name in parentheses, as `Work (D:)` / `Work (Archive)`. If that is still ambiguous, the full parent path is used.

### 3.3 Dim label and ordering

`notebook_prefixed(label, folder_text, several: bool) -> String` builds the dim text: `Work › projects`, `Work` for a note at the notebook root, and the unchanged `folder_text` when `several` is false. Search, Quick Open and Tags all use it.

Where two results tie under a view's existing order, the sidebar's notebook order breaks the tie.

## 4. Search view

### 4.1 Scope

`SearchView.notebook` / `notebook_name` are replaced by `scope: Option<NotebookId>` (`None` = all notebooks).

- With two or more notebooks open, the header row shows a scope button reading "All notebooks ▾" or "*Work* ▾". Clicking it (or Enter / Space when focused) opens a menu: "All notebooks", a separator, then one item per open notebook by label, in sidebar order, with a check on the current choice. Picking an item sets the scope and reruns the current query.
- The scope lives for the session only. It starts as All at startup.
- If the scoped notebook closes, the scope returns to All and the query reruns.
- With one notebook open there is no scope button.
- Header text: "Searching all notebooks", or "Searching *Work*" when scoped or when only one notebook is open. Placeholder: "Search text in all notebooks", or "Search text in *Work*" in the same cases.
- The scope button is a UIA button named "Search scope: All notebooks" / "Search scope: Work".

### 4.2 Query lifetime

- Switching tabs, or changing which notebook is the target, does not touch the query or results. The phase 1 clear-on-switch in `library_changed` / `target_changed` is removed.
- Opening or closing a notebook, or a skipped notebook finishing its load, reruns the current non-empty query. Results from a closed notebook are removed immediately, before the rerun returns.
- File changes and index updates behave as today.

### 4.3 Running a search

`spawn` builds one job from `searchable_notebooks`, filtered by the scope: a list of `(NotebookId, folder, notes snapshot)`. The worker walks the notebooks in that order with the existing `MAX_READERS` readers. `RESULT_CAP` (500) is shared across all notebooks. Batches carry ids. Stale generations are dropped as today. When a batch arrives, hits whose notebook is no longer open are dropped.

`run_now` (the in-memory pass) loops over the same list.

Sorting stays `hit_cmp` (name, then folder), with sidebar order as the final tie-break (§3.3).

### 4.4 Narrowing

`list_mark` and `note_mark` become per notebook, keyed by `NotebookId`. A narrowed search reuses previous hits only if the set of searched notebooks is identical and every notebook's marks match. Otherwise it runs in full.

### 4.5 Replace in files

`ReplacePlan` becomes `parts: Vec<ReplacePart { id: NotebookId, folder: PathBuf, targets: … }>`, one per notebook with matches.

- `text_replace::count` and `text_replace::apply` run per part, against that part's folder.
- `dirty_overlays`, `open_notes` and `clean_tabs_on` check each target against its own part's folder. `record_written_all` runs on each part's slot.
- `replace_notebook` becomes the set of ids in the plan. While a replace runs, those notebooks are pinned as today's one notebook is.
- Confirmation: "Replace 14 matches in 6 notes across Work and Personal?" With three or more: "across Work, Personal and Archive". With one notebook involved, the text is today's, with no "across …".
- If a notebook closes during a replace, its remaining part is skipped. The completion notice adds "Skipped Personal: it was closed."
- A part that fails is reported per notebook in the existing per-file failure style. Other parts still apply.

### 4.6 Opening a hit

Opening a hit joins its relative path to the folder of the hit's `NotebookId` (§3.1), never to a view-wide folder.

## 5. Quick Open

- `quick_open_rows` runs the existing matcher over each `searchable_notebooks` entry's `state.notes`, merges the ranked lists by score (sidebar order breaks ties) and keeps the top `QUICK_OPEN_ROWS` (50).
- Empty query: open tabs from every open notebook, in today's order. Tabs that belong to no notebook stay out.
- Rows use `notebook_prefixed` for their dim text.
- Placeholder: "Search notes in all notebooks", or today's "Search notes in *Work*" with one notebook.
- `PickerChoice::Note` and `PickerChoice::View` carry a `NotebookId`. Picking resolves the folder by id (§3.1).
- The Move-to-notebook picker and link pickers are unchanged.

## 6. Tags view

- `refresh` builds one tag tree from every `searchable_notebooks` entry's `note_index_of(…).tags()`.
  - A tag present in several notebooks is one node. Its count is the sum.
  - Nested tags (`a/b`) merge level by level the same way.
  - `expanded` stays keyed by tag name.
- Note rows under a tag are sorted as today, use `notebook_prefixed` for their dim text, and carry a `NoteRef`. Opening one uses §3.1.
- Header: "Tags in all notebooks", or "Tags in *Work*" with one notebook.
- `TagsView.notebook` / `notebook_name` are removed. Tags no longer follow `target_changed`. They refresh when any slot's index finishes, when a notebook opens or closes, and when a skipped notebook finishes loading.
- Any filter inside the Tags view filters the merged tree.

## 7. Removing the phase 1 shims

`library_host::folder(hwnd)`, `library_host::with_state(hwnd, f)` and `library_host::note_index(hwnd)` mean "the default notebook". Every production caller moves to an explicit notebook: `notebook_of`, `target_notebook`, `link_notebook`, `default_notebook` or `with_notebook`, whichever matches what the caller means. The three functions then become `#[cfg(test)]` helpers named `default_folder`, `with_default_state` and `default_note_index`, used only by tests that open one notebook.

## 8. Edge cases

| Case | Behaviour |
|---|---|
| A notebook closes while a search runs | Its hits are removed at once; arriving batches drop them (§4.2, §4.3). |
| A notebook opens while a search runs | The query reruns (§4.2). |
| Opening a result whose notebook closed | Nothing opens; notice "That note's notebook was closed." |
| A notebook is unavailable or loading | Skipped, named in the header count; views refresh when it loads (§3.2). |
| Two notebooks share a name | Labels add the parent folder (§3.2), in rows and in the scope menu. |
| A replace part fails | Reported per notebook; other parts apply (§4.5). |
| More than 500 hits in total | The existing "500+ results" behaviour, with the cap shared (§4.3). |
| The scoped notebook closes | Scope returns to All and the query reruns (§4.1). |

## 9. Testing

**Unit tests**

- Quick Open merge: ranking by score across two notebooks, sidebar order as tie-break, the 50-row cap.
- Tag tree merge: shared tags sum their counts, nested tags merge, notes keep their `NotebookId`.
- `notebook_prefixed`: one vs several notebooks, root-level notes.
- Labels: duplicate names add the parent folder; still-ambiguous names use the full parent path.
- `ReplacePlan` split per notebook, and the confirmation text for one, two and three notebooks.
- Narrowing: reuse only when the notebook set and every per-notebook mark match.

**Integration tests** (with `notebooks_a_and_b`)

- Search finds hits in A and B, labelled `A ›` / `B ›`. The header reads "Searching all notebooks".
- The scope menu limits results to B. Closing B returns the scope to All and reruns.
- Switching between an A tab and a B tab keeps the query and results.
- Opening a hit from B opens B's file.
- Replace across A and B writes both files; the confirmation names both. Closing B mid-replace skips it and the notice says so.
- Closing B while a search runs drops B's hits.
- Quick Open lists notes from A and B and opens the picked one from the right notebook. The empty query lists tabs from both.
- Tags merges `#x` from A and B into one node with count 2. Both note rows open the right file.
- Wikilink resolution and tag completion still stay inside the note's notebook.
- With one notebook open, the existing Search, Quick Open and Tags tests pass unchanged, apart from tests that pinned phase 1's target-notebook behaviour, which are rewritten to this spec.
