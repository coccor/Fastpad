# Split editors, PR 1 (one group): implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure FastPad so that the editor area is one `EditorGroup` child window with its own tab strip, find bar, editor, preview and image view, and so that documents live in a store that any editor can show. This changes no split behaviour yet. What users see: the tabs sit below the title bar, every tab remembers its caret and scroll position, and the title bar shows the window title.

**Architecture:**
- A hidden, message-only Scintilla (the *document host*) creates every Scintilla document. An editor can then show any document the host created.
- `Tabs` becomes a façade over a new `DocumentStore` plus a list of views (`EditorTab`) for the one group. This keeps its public API and its ~60 call sites almost unchanged in this PR.
- A new `EditorGroup` child window becomes the parent of the Scintilla editor, the find bar panel, the preview views and the image view. It paints and handles the tab strip, which moves out of `titlebar.rs`.
- The session format moves to version 2, with groups and per-view state. Version 1 is still read.

**Tech Stack:** Rust 2024 edition, `windows-sys`, Scintilla 5.6.6 plus Lexilla 5.5.3 (`native/`), and MSAA `IAccessible` for accessibility.

**Spec:** `docs/superpowers/specs/2026-09-28-split-editors-design.md`. This plan implements its §10 item 1. PRs 2 (splits) and 3 (drag and drop) get their own plans once this PR's code exists.

## Global Constraints

- **Latency:** nothing may block first paint or first input. One UI thread. The document host and the group window are the only additions before first paint: one message-only Scintilla and one plain child window.
- **Scintilla owns the text.** Never copy a document's text into Rust state to move it between views.
- **No new crates.** A new `windows-sys` feature must be added to `tools/audit-dependencies.ps1`'s allowlist in the same commit.
- **Compile gate:** `cargo clippy --all-targets --all-features -- -D warnings`.
- **Targeted tests:** `cargo test --lib -- <filter> --test-threads=1`. Window tests register window classes and must run serially.
- **The full suite runs once, at the end:** `cargo test --all-targets -- --test-threads=1`.
- **End-to-end tests** fail while the user's FastPad is running. Use a separate `CARGO_TARGET_DIR` if `target/debug/fastpad.exe` is locked.
- **Session keys:** `version=2`, `layout=`, `active_group=`, `group=<n>|active=<i>`, and `file=` / `snapshot=` in the v1 format (`<caret>|<anchor>|<firstline>|<target>`). Every file is written as version 2; version 1 is read as a single group.
- **Comments and test style:** match the surrounding code. Each test starts with a `// Break caught: …` comment naming the regression it guards against.
- Commit messages use the `feat(split-editors): …`, `refactor(split-editors): …` and `test(split-editors): …` prefixes. No attribution lines.

## Plan-time amendments to the spec

Record these in the spec's §10 as part of Part 9.

1. **Accessibility is MSAA, not UIA.** Spec §3.2 says "UIA provider". The strip's provider is the hand-built `IAccessible` in `accessibility.rs`. It moves to the group window and stays MSAA.
2. **The app menu "…" (`menus::show_overflow`) stays in the title bar.** It is the app menu (New, Open, Save, Find, Format JSON, palette, Exit), not a tab menu. The group's strip gets its own **"…"** button, which opens the tab-strip menu (New tab, Open…, Close all tabs). Close Group is added to that menu in PR 2.
3. **In PR 1 `App.tabs` stays**, as a façade over the `DocumentStore` and one group's views, instead of spec §3.3's separate `store` and `groups` fields. PR 2 splits the façade when it adds a second group.
4. **Double-clicking the title bar** now maximizes or restores the window, as a normal caption does. Double-click for New and right-click for the tab-strip menu move to the empty part of the group's strip.
5. **Split Right is not in the strip in PR 1.** Splits don't exist yet, so the strip's action cluster has only Preview Side, Preview Full and "…". Split Right is added in PR 2.

## Review Focus

These are the conditions most likely to hurt a user that no part's feature test covers directly. Each has a test in the part named in brackets.

1. **A search replace in a background tab** must not move the active tab's caret, selection or scroll, and must not change the active editor's document. [Part 3]
2. **Closing the tab that is showing** must not lose the next tab's saved caret. The tab that becomes active after a close gets its own view state back, not the closed tab's. [Part 2]
3. **Rendering and hit-testing must agree.** A click in the strip must hit what the strip painted, including when the strip is scrolled and when the preview buttons show. The layout used for painting and the one used for hit-testing must be the same function. [Part 6]
4. **Finding the main window.** The find bar's buttons and its Enter/Escape keys, the preview's links and Escape, and the image status must still reach the main window now that their parent is the group. Every `GetParent`-as-main assumption is replaced. [Part 5]
5. **Reading a version 1 session** from a 0.2.0 install. It must reopen every tab, with the saved caret on the active one, after the upgrade. [Part 4]

---

## Task 1: The document host

Afterwards, an `Editor` can show any document that a shared host created. Documents outlive the editors that showed them.

**Files:**
- Modify: `src/editor/scintilla.rs`: the `Editor` struct (:99), `Clone` (:103), `create` (:145), `create_document` (:241), `current_document` (:264), `use_document` (:286), `create_scintilla_child` (:1426), and the tests module (:1511).
- Modify: `tools/generate-scintilla-constants.ps1` (the name list near :45) and the regenerated `src/editor/scintilla_constants.rs`, to add `SCI_GETXOFFSET` and `SCI_SETXOFFSET`. Part 2 uses them.

**Interfaces:**
- Produces:
  - `Editor::create_document_host() -> Result<Editor>` (Windows only; a stub error elsewhere, as `create` has).
  - `Editor::create_with_host(parent: HWND, host: &Editor) -> Result<Editor>`.
  - `Editor::shares_documents_with(&self, other: &Editor) -> bool`.
  - `Editor::create(parent)` keeps its signature and makes an editor that is its own host.

- [ ] **Step 1: Add the X-offset constants.** Add `"SCI_GETXOFFSET", "SCI_SETXOFFSET"` to the generator's name list, then run the generator:

```powershell
./tools/generate-scintilla-constants.ps1
```

Expected: `scintilla_constants.rs` gains `pub const SCI_GETXOFFSET: u32 = 2398;` and `pub const SCI_SETXOFFSET: u32 = 2397;`, and nothing else changes (check with `git diff --stat`).

- [ ] **Step 2: Write the failing tests** at the end of the `scintilla.rs` tests module:

```rust
    #[test]
    fn a_host_document_shows_in_two_editors_and_outlives_both() {
        // Break caught: a document tied to the editor that created it, so a second group's editor
        // refuses it, or closing a group frees text another group still shows.
        let first = test_editor();
        let host = Editor::create_document_host().expect("document host");
        let left = Editor::create_with_host(first._host.0, &host).expect("left editor");
        let right = Editor::create_with_host(first._host.0, &host).expect("right editor");
        let document = left.create_document().unwrap();
        left.use_document(&document).unwrap();
        right.use_document(&document).unwrap();
        left.set_text("shared").unwrap();
        assert_eq!(right.text().unwrap(), "shared");
        drop(left);
        drop(right);
        host.use_document(&document).unwrap();
        assert_eq!(host.text().unwrap(), "shared");
    }

    #[test]
    fn an_editor_refuses_documents_from_another_host() {
        // Break caught: SCI_SETDOCPOINTER with a document whose owner can be destroyed under it.
        let fixture = test_editor();
        let host = Editor::create_document_host().expect("document host");
        let hosted = Editor::create_with_host(fixture._host.0, &host).expect("hosted editor");
        let foreign = fixture.create_document().unwrap();
        assert!(hosted.use_document(&foreign).is_err());
        assert!(hosted.shares_documents_with(&host));
        assert!(!hosted.shares_documents_with(&fixture));
    }
```

- [ ] **Step 3: Run them to confirm they fail.**

Run: `cargo test --lib -- scintilla::tests::a_host_document scintilla::tests::an_editor_refuses --test-threads=1`
Expected: a compile error, because `create_document_host`, `create_with_host` and `shares_documents_with` don't exist yet.

- [ ] **Step 4: Implement.**

In `Editor`, add the endpoint that creates and owns documents:

```rust
#[derive(Debug)]
pub struct Editor {
    endpoint: Rc<EditorEndpoint>,
    /// The Scintilla that creates this editor's documents: the shared document host (split
    /// editors spec §3.1), or the editor itself for one made by `create`.
    documents: Rc<EditorEndpoint>,
}

impl Clone for Editor {
    fn clone(&self) -> Self {
        Self {
            endpoint: Rc::clone(&self.endpoint),
            documents: Rc::clone(&self.documents),
        }
    }
}
```

Split `create` into a shared body. `create_scintilla_child` keeps its current behaviour, and a new sibling makes the message-only window:

```rust
    #[cfg(windows)]
    pub fn create(parent: HWND) -> Result<Self> {
        let endpoint = Self::open_endpoint(create_scintilla_child(parent)?)?;
        Self::finish(Rc::clone(&endpoint), endpoint)
    }

    /// A message-only Scintilla that creates every document and never shows one (split editors
    /// spec §3.1). Its notifications go nowhere, so edits made through it notify nobody.
    #[cfg(windows)]
    pub fn create_document_host() -> Result<Self> {
        let endpoint = Self::open_endpoint(create_scintilla_host()?)?;
        Ok(Self {
            documents: Rc::clone(&endpoint),
            endpoint,
        })
    }

    /// A visible editor under `parent` that shows documents `host` created.
    #[cfg(windows)]
    pub fn create_with_host(parent: HWND, host: &Editor) -> Result<Self> {
        let endpoint = Self::open_endpoint(create_scintilla_child(parent)?)?;
        Self::finish(endpoint, Rc::clone(&host.documents))
    }

    pub fn shares_documents_with(&self, other: &Editor) -> bool {
        Rc::ptr_eq(&self.documents, &other.documents)
    }

    #[cfg(windows)]
    fn open_endpoint(hwnd: HWND) -> Result<Rc<EditorEndpoint>> {
        require_hwnd(hwnd)?;
        // (the SCI_GETDIRECTFUNCTION / SCI_GETDIRECTPOINTER checks from today's `create`, as is)
        let endpoint = Rc::new(EditorEndpoint::new(/* as today */ hwnd, direct_fn, direct_ptr, true));
        endpoint.install_lifecycle_guard()?;
        Ok(endpoint)
    }

    #[cfg(windows)]
    fn finish(endpoint: Rc<EditorEndpoint>, documents: Rc<EditorEndpoint>) -> Result<Self> {
        let editor = Self { endpoint, documents };
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(editor.endpoint.hwnd) };
        editor.initialize_view(|editor| editor.apply_chrome_defaults(dpi))?;
        Ok(editor)
    }
```

Move today's direct-function code verbatim into `open_endpoint`; the comment in that sketch marks where. The host skips `initialize_view` because it never paints.

`create_scintilla_host`:

```rust
#[cfg(windows)]
fn create_scintilla_host() -> Result<HWND> {
    let class_name = wide_null("Scintilla");
    let hwnd = unsafe {
        // SAFETY: HWND_MESSAGE makes a message-only window: no parent to outlive, never shown.
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            1,
            1,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    Ok(hwnd)
}
```

Change the document methods to go through `documents`:
- In `create_document`, send `SCI_CREATEDOCUMENT` to `self.documents` and build `EditorDocument { raw, endpoint: Rc::clone(&self.documents) }`.
- In `current_document`, keep reading `SCI_GETDOCPOINTER` from `self.endpoint`, call `self.documents.retain_document(raw)`, and build the `EditorDocument` with `self.documents`.
- In `use_document`, replace the check with `if !Rc::ptr_eq(&self.documents, &document.endpoint)` and keep the same error text.
- In the `#[cfg(test)] Editor::test_fixture` (:1176), set `documents: Rc::clone(&endpoint)`.

Scintilla documents belong to the DLL, not to a window. That is why `SCI_ADDREFDOCUMENT` and `SCI_RELEASEDOCUMENT` sent to the host are valid for a document shown in any editor. Each `EditorDocument` holds an `Rc` to the host's endpoint, so the host window outlives every document.

- [ ] **Step 5: Run the new tests and today's editor tests.**

Run: `cargo test --lib -- scintilla:: --test-threads=1`
Expected: every test passes, including the two new ones.

**If the message-only host fails** (for example, `CreateWindowExW` returns null, or `SCI_CREATEDOCUMENT` returns 0), stop and report back. That would change the design. The fallback is a hidden `WS_POPUP` host window, which also has no parent to outlive.

- [ ] **Step 6: Clippy, then commit.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
git add src/editor tools/generate-scintilla-constants.ps1
git commit -m "feat(split-editors): a document host that any editor can show documents from"
```

---

## Task 2: The document store, views, and per-tab view state

Afterwards, `Tabs` keeps its public API but stores documents in a `DocumentStore` and tabs as views. Switching tabs saves the caret, selection and scroll of the tab being left and restores those of the tab shown.

**Files:**
- Create: `src/window/document_store.rs`.
- Create: `src/editor/view_state.rs`, re-exported from `src/editor/mod.rs` as `crate::editor::ViewState`.
- Modify: `src/editor/scintilla.rs` (add `view_state` / `apply_view_state`).
- Modify: `src/window/tabs.rs` (internals), `src/window/mod.rs` (`mod document_store;`).
- Modify: `src/document.rs` (remove the `preview` field from `Document`), and the readers of `document.preview` in `tabs.rs`, `main_window.rs` and `titlebar.rs`.
- Modify: `src/window/main_window.rs`: `activate_document` (:4008), `close_active_document` (:4068), `close_tab_at` (:4127), `apply_view_state` (:5227).

**Interfaces:**
- Produces:
  - `crate::editor::ViewState { caret: usize, anchor: usize, first_line: usize, x_offset: i32 }`, which derives `Clone, Copy, Debug, Default, Eq, PartialEq`.
  - `Editor::view_state(&self) -> Result<ViewState>`.
  - `Editor::apply_view_state(&self, state: ViewState) -> Result<()>`, which clamps the caret and anchor to the document length.
  - `ViewId(pub u64)`, `EditorTab { view: ViewId, document: DocumentId, view_state: ViewState, preview: bool }` in `tabs.rs`.
  - `Tabs::view_state(&self, id: DocumentId) -> ViewState` and `Tabs::set_view_state(&mut self, id: DocumentId, state: ViewState)`.
  - `Tabs::is_preview(&self, id: DocumentId) -> bool` replaces reads of `document.preview`.
  - `DocumentStore` (below).

- [ ] **Step 1: Write the `ViewState` test** in the `scintilla.rs` tests module:

```rust
    #[test]
    fn view_state_round_trips_and_clamps_to_a_shorter_document() {
        // Break caught: switching back to a tab lands at the top, or a saved caret past the end of
        // a file that shrank on disk panics or selects garbage.
        let editor = test_editor();
        editor.set_text(&"line\n".repeat(200)).unwrap();
        let saved = crate::editor::ViewState { caret: 500, anchor: 495, first_line: 90, x_offset: 0 };
        editor.apply_view_state(saved).unwrap();
        assert_eq!(editor.view_state().unwrap(), saved);
        editor.set_text("short").unwrap();
        editor.apply_view_state(saved).unwrap();
        let clamped = editor.view_state().unwrap();
        assert_eq!((clamped.caret, clamped.anchor), (5, 5));
    }
```

- [ ] **Step 2: Implement `ViewState`.** `src/editor/view_state.rs`:

```rust
/// Where one view of a document was: the selection (anchor to caret), the first visible display
/// line and the horizontal scroll (split editors spec §3.2).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViewState {
    pub caret: usize,
    pub anchor: usize,
    pub first_line: usize,
    pub x_offset: i32,
}
```

In `scintilla.rs` (Windows-only, with the usual non-Windows stubs):

```rust
    pub fn view_state(&self) -> Result<ViewState> {
        Ok(ViewState {
            caret: self.endpoint.send_direct_checked(SCI_GETCURRENTPOS, 0, 0)? as usize,
            anchor: self.endpoint.send_direct_checked(SCI_GETANCHOR, 0, 0)? as usize,
            first_line: self.first_visible_line()?,
            x_offset: self.endpoint.send_direct_checked(SCI_GETXOFFSET, 0, 0)? as i32,
        })
    }

    pub fn apply_view_state(&self, state: ViewState) -> Result<()> {
        let length = self.endpoint.send_direct_checked(SCI_GETLENGTH, 0, 0)?.max(0) as usize;
        self.set_selection(state.anchor.min(length)..state.caret.min(length))?;
        self.set_first_visible_line(state.first_line)?;
        self.endpoint
            .send_direct_checked(SCI_SETXOFFSET, state.x_offset.max(0) as usize, 0)?;
        Ok(())
    }
```

Add `SCI_GETANCHOR` and `SCI_GETLENGTH` to the constants import if they aren't there. `SCI_GETLENGTH` is already generated. `set_selection` takes `anchor..caret`, as `main_window::apply_view_state` uses it today.

- [ ] **Step 3: Write the `DocumentStore` tests.** `src/window/document_store.rs` ends with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentId};

    fn document(id: u64) -> Document {
        Document::test_fixture(DocumentId(id), false)
    }

    #[test]
    fn documents_are_kept_once_and_found_by_id() {
        // Break caught: a second view of a document duplicating it, so its two copies drift apart.
        let mut store = DocumentStore::default();
        store.insert(document(1)).unwrap();
        store.insert(document(2)).unwrap();
        assert!(store.insert(document(1)).is_err());
        assert_eq!(store.len(), 2);
        assert!(store.get(DocumentId(2)).is_some());
        assert_eq!(store.remove(DocumentId(1)).map(|d| d.id), Some(DocumentId(1)));
        assert!(store.get(DocumentId(1)).is_none());
    }

    #[test]
    fn a_path_already_open_is_refused() {
        // Break caught: the same file open in two documents, so saving one overwrites the other.
        let dir = std::env::temp_dir().join(format!("fastpad-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.txt");
        std::fs::write(&path, "a").unwrap();
        let mut store = DocumentStore::default();
        let mut first = document(1);
        first.path = Some(path.clone());
        store.insert(first).unwrap();
        let mut second = document(2);
        second.path = Some(dir.join(".").join("a.txt"));
        assert!(store.insert(second).is_err());
        assert_eq!(store.find_path(&path), Some(DocumentId(1)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_text_change_bumps_the_generation_once() {
        // Break caught: a document shown by two editors counting one edit twice.
        let mut store = DocumentStore::default();
        store.insert(document(1)).unwrap();
        let before = store.get(DocumentId(1)).unwrap().generation;
        assert!(store.note_text_change(DocumentId(1)));
        assert_eq!(store.get(DocumentId(1)).unwrap().generation, before + 1);
        assert!(!store.note_text_change(DocumentId(99)));
    }
}
```

- [ ] **Step 4: Implement `DocumentStore`.** Move the path-key helpers (`canonical_key`, `lexical_key`, `validate_unique_paths` and `DuplicateDocumentPath`) from `tabs.rs` into `document_store.rs` and re-export them from `tabs.rs` with `pub(crate) use`, so current importers still compile.

```rust
//! Every open document, once (split editors spec §3.1). Tabs are views onto these; a document
//! leaves the store when its last view closes.

use crate::document::{Document, DocumentId};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(crate) struct DocumentStore {
    documents: Vec<Document>,
}

impl DocumentStore {
    pub(crate) fn len(&self) -> usize { self.documents.len() }

    pub(crate) fn get(&self, id: DocumentId) -> Option<&Document> {
        self.documents.iter().find(|document| document.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: DocumentId) -> Option<&mut Document> {
        self.documents.iter_mut().find(|document| document.id == id)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Document> + '_ { self.documents.iter() }

    /// Refuses a document whose id is already here, or whose file another document already has.
    pub(crate) fn insert(&mut self, document: Document) -> Result<(), DuplicateDocumentPath> {
        if self.get(document.id).is_some() {
            return Err(DuplicateDocumentPath(document.path.clone().unwrap_or_default()));
        }
        if let Some(path) = document.path.as_deref() {
            let candidate = canonical_key(path)?;
            if self.documents.iter().any(|existing| {
                existing.path.as_deref().and_then(|p| canonical_key(p).ok()) == Some(candidate.clone())
            }) {
                return Err(DuplicateDocumentPath(candidate));
            }
        }
        self.documents.push(document);
        Ok(())
    }

    /// Puts `document` where `old` was, keeping its place; returns `old`.
    pub(crate) fn replace(&mut self, old: DocumentId, document: Document) -> Option<Document> {
        let slot = self.documents.iter_mut().find(|existing| existing.id == old)?;
        Some(std::mem::replace(slot, document))
    }

    pub(crate) fn remove(&mut self, id: DocumentId) -> Option<Document> {
        let index = self.documents.iter().position(|document| document.id == id)?;
        Some(self.documents.remove(index))
    }

    pub(crate) fn clear(&mut self) { self.documents.clear(); }

    /// `Tabs::find_path`'s lookup: the document whose file is `path`, compared on disk.
    pub(crate) fn find_path(&self, path: &Path) -> Option<DocumentId> {
        let key = canonical_key(path).ok()?;
        self.documents
            .iter()
            .find(|document| {
                document.path.as_deref().and_then(|p| canonical_key(p).ok()).is_some_and(|p| p == key)
            })
            .map(|document| document.id)
    }

    /// `Tabs::find_stored_path`'s lookup: compared lexically, without touching the disk.
    pub(crate) fn find_stored_path(&self, path: &Path) -> Option<DocumentId> {
        let key = lexical_key(path);
        self.documents
            .iter()
            .find(|document| document.path.as_deref().is_some_and(|p| lexical_key(p) == key))
            .map(|document| document.id)
    }

    /// Refuses `path` when a document other than `exclude` already has that file
    /// (today's `Tabs::reject_path_collision`, keyed by id instead of index).
    pub(crate) fn reject_path_collision(
        &self,
        exclude: DocumentId,
        path: PathBuf,
    ) -> Result<PathBuf, DuplicateDocumentPath> {
        if let Ok(candidate) = canonical_key(&path) {
            let collides = self.documents.iter().any(|existing| {
                existing.id != exclude
                    && existing.path.as_deref().and_then(|p| canonical_key(p).ok()).is_some_and(|p| p == candidate)
            });
            if collides {
                return Err(DuplicateDocumentPath(candidate));
            }
        }
        Ok(path)
    }

    /// A text change in `id`: a new generation, so recovery snapshots it. The reporting-editor
    /// rule (spec §3.4) makes sure each change arrives here once.
    pub(crate) fn note_text_change(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.get_mut(id) else { return false };
        if document.is_image() {
            return false;
        }
        document.generation = document.generation.saturating_add(1);
        true
    }
}
```

- [ ] **Step 5: Rebuild `Tabs` on top of the store.** The fields become:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub(crate) struct ViewId(pub u64);

/// One tab: a view onto a document in the store (split editors spec §3.2).
#[derive(Debug)]
pub(crate) struct EditorTab {
    pub(crate) view: ViewId,
    pub(crate) document: DocumentId,
    /// Where this view was when it last stopped being shown; the live editor is the truth while
    /// it is shown.
    pub(crate) view_state: crate::editor::ViewState,
    /// The preview tab (italic), replaced by the next single-click open. At most one per group.
    pub(crate) preview: bool,
}

#[derive(Debug)]
pub struct Tabs {
    store: DocumentStore,
    tabs: Vec<EditorTab>,
    recent: Vec<DocumentId>,
    selection: TabSelection,
    view: TabView,
    next_view: u64,
}
```

Rules for rewriting each existing method. The public signatures stay as they are.
- **Strip order and index lookups** (`active_index`, `activate`, `activate_index`, `select_after_removal`, `close_*`, `replace_*`) iterate `self.tabs`. They reach the document with `self.store.get(tab.document)`.
- **Document lookups by id** (`document`, `document_mut`, `find_path`, `find_stored_path`, `documents()`, `record_recovery_generation`, `next_dirty_review`, `dirty_review_is_current`) go through `self.store`. `documents()` must still yield documents **in strip order**, because Open Editors, session building and `titles()` rely on it. So map `self.tabs` to `self.store.get(tab.document)`.
- **`push`**: `self.store.insert(document)?`, then push an `EditorTab` with a new `ViewId`, `ViewState::default()` and `preview: false`. A pushed document always starts as a normal tab.
- **`replace_preview`** is the only way a preview tab is made. It creates its view with `preview: true`, including when it falls back to `push` because there is no clean preview to replace. It removes the replaced document from the store and inserts the new one. Today its callers pass a `Document` whose `preview` field is `true`. That field goes away (next bullet), so the flag lives only on the view.
- **`Document.preview` goes away.**
  - `grep -n "preview: true\|\.preview = \|\.preview\b" src/window/main_window.rs src/window/titlebar.rs src/window/tabs.rs` lists the sites.
  - Constructors that set `preview = true` before `replace_preview` stop doing so.
  - Readers use `tabs.is_preview(id)`.
  - `view_tabs` takes the `(tab, document)` pairs and sets `TabViewTab.preview` from `tab.preview`.
  - `promote`, `note_active_text_change` and `note_background_edit` clear `tab.preview`.
- **Removal** (`close_active`, `close_reviewed`, `close_clean_background`, `replace_preview`, `replace_active_untitled`) removes the `EditorTab`. It removes the store's document **only when no other tab still refers to it**, which is always the case in PR 1. The closed `Document` is returned exactly as today.
- **`note_active_text_change`** calls `self.store.note_text_change(active_document)`. It then clears the active view's `preview` flag, returning `true` when it did, as today.
- **`clear_for_shutdown`** clears `tabs`, `recent` and `store`.
- **New:** `view_state(id)` returns the first tab showing `id`'s `view_state`, or the default. `set_view_state(id, state)` sets it on that tab. `is_preview(id)` reports that tab's flag.

The existing `tabs.rs` tests are the regression net, so they must pass unchanged. Where a test builds a preview tab with `preview(id)` (tabs.rs:830), that helper now sets nothing on the document. Change it to push the document with `replace_preview` so the view carries the flag. That is the only test edit allowed.

- [ ] **Step 6: Add the view-state tests for `Tabs`** to the `tabs.rs` tests:

```rust
    #[test]
    fn each_tab_keeps_its_own_view_state() {
        // Break caught: switching tabs forgetting where you were, or one tab's caret landing in
        // another.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        let state = crate::editor::ViewState { caret: 7, anchor: 3, first_line: 2, x_offset: 0 };
        tabs.set_view_state(DocumentId(1), state);
        assert_eq!(tabs.view_state(DocumentId(1)), state);
        assert_eq!(tabs.view_state(DocumentId(2)), crate::editor::ViewState::default());
    }

    #[test]
    fn closing_the_shown_tab_keeps_the_next_tabs_view_state() {
        // Break caught (Review Focus 2): the tab activated by a close inheriting the closed tab's
        // caret.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        let second = crate::editor::ViewState { caret: 9, anchor: 9, first_line: 4, x_offset: 0 };
        tabs.set_view_state(DocumentId(2), second);
        tabs.activate(DocumentId(1)).unwrap();
        tabs.close_active(crate::document::CloseDecision::Discard).unwrap();
        assert_eq!(tabs.active().map(|d| d.id), Some(DocumentId(2)));
        assert_eq!(tabs.view_state(DocumentId(2)), second);
    }
```

Use whatever the non-cancelling `CloseDecision` variant is called in `document.rs`: the one `close_active` accepts for a clean tab.

- [ ] **Step 7: Save and restore the view state on switch.** In `main_window.rs`, change `activate_document` (:4008). After the autosave and identity re-check, and **before** `app.tabs.activate(id)`:

```rust
        if leaving {
            if let (Some(editor), Some(left)) = (app.editor.as_ref(), app.tabs.active().map(|d| d.id)) {
                if !app.tabs.active().is_some_and(|d| d.is_image()) {
                    if let Ok(state) = editor.view_state() {
                        app.tabs.set_view_state(left, state);
                    }
                }
            }
        }
```

After `editor.use_document(&handle)` succeeds, and only for a text tab:

```rust
        let _ = editor.apply_view_state(app.tabs.view_state(id));
```

- The close paths, `close_active_document` (:4068) and `close_tab_at` (:4127), `use_document` the newly active tab after `Tabs` removes the closed one. Apply that tab's `view_state` there in the same way.
- `main_window::apply_view_state` (:5227) turns into `editor.apply_view_state(ViewState { caret, anchor, first_line, x_offset: 0 })` for the session entry.

Keep the `app_ptr` borrow discipline: take what you need, drop the borrow, then make the Win32 calls. See how `activate_document` does it today.

- [ ] **Step 8: Add a window test for the switch** in the `main_window.rs` tests, next to the existing `SelectTab1` tests (around :6579), using the same fixtures they use:

```rust
    #[test]
    fn switching_tabs_restores_each_tabs_caret_and_scroll() {
        // Break caught: switching tabs resetting the caret to the start of the document.
        let window = /* the fixture the neighbouring tab tests use to make a window with an editor */;
        // Tab 1: 300 lines, caret on line 250, scrolled there.
        // New tab (CommandId::New), type text, then CommandId::SelectTab1.
        // Assert the editor's view_state() equals the one saved before switching away.
    }
```

Fill it in with the concrete fixture calls the neighbouring tests use: `install_test_editor(&window)`, `execute_command(window.hwnd, …)`, and the editor's `set_text` / `apply_view_state`. Assert both the caret and `first_line`.

- [ ] **Step 9: Clippy, the targeted tests, commit.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- tabs:: document_store:: scintilla::tests::view_state main_window::tests::switching_tabs --test-threads=1
git add -A src
git commit -m "refactor(split-editors): documents in a store, tabs as views with their own caret and scroll"
```

---

## Task 3: The host editor creates every document and reads and writes background tabs

Afterwards, every document comes from the host, and background tabs are read and changed through the host, never swapped into the visible editor.

**Files:**
- Modify: `src/app.rs`: add `pub(crate) document_host: Option<Editor>` (declared **after** `tabs`, so documents drop before the host).
- Modify: `src/window/main_window.rs`: `initialize_editor_with` (:1013), `with_inactive_document` (:4760) and its callers `read_inactive_text` (:4801), `document_text` (:4835), `replace_in_document` (:4885) and `reload_clean_document` (:4954), plus every `editor.create_document()` site (`grep -n "create_document()" src`).
- Modify: `src/bootstrap.rs` (:72-77), if it constructs the editor there.

**Interfaces:**
- Consumes: `Editor::create_document_host`, `Editor::create_with_host` (Part 1).
- Produces:
  - `App.document_host`.
  - `pub(crate) fn with_background_document<R>(hwnd: HWND, target: &EditorDocument, f: impl FnOnce(&Editor) -> Result<R>) -> Result<R>`, which replaces `with_inactive_document`.

- [ ] **Step 1: Write the failing test** in the `main_window.rs` tests, next to the existing Search-replace-in-background-tab tests (search for `note_background_edit` or `replace_in_document` in the tests):

```rust
    #[test]
    fn replacing_in_a_background_tab_leaves_the_active_view_alone() {
        // Break caught (Review Focus 1): a background replace swapping the document through the
        // visible editor, so the active tab's caret, selection or scroll jumps.
        // Arrange: tab A with long text, caret and first line set away from 0 (apply_view_state);
        // tab B with "foo foo"; activate A.
        // Act: replace_in_document for B, replacing "foo" with "bar" (as the neighbouring replace
        // tests call it).
        // Assert: B's text is "bar bar"; the editor's current document is still A's;
        // editor.view_state() equals the state set on A before the replace.
    }
```

Write it with the concrete calls the neighbouring replace tests use.

- [ ] **Step 2: Create the host first.** In `initialize_editor_with`:
  - create `Editor::create_document_host()?`;
  - pass the parent to `create_editor` and wrap it so that production calls `Editor::create_with_host(parent, &host)`. Change the closure type to `FnOnce(HWND, &Editor) -> Result<Editor>` and update the two call sites: `bootstrap.rs:72` and the tests' `install_test_editor` (:8503);
  - create the first untitled document with `editor.create_document()`, which now comes from the host, instead of `editor.current_document()`, and `editor.use_document(&it)`;
  - store the host in `app.document_host`.

- [ ] **Step 3: Replace `with_inactive_document`.**

```rust
/// Runs `f` on the document host showing `target`, for a document no visible editor shows
/// (split editors spec §3.1). The host is never painted and its notifications reach no window,
/// so neither the visible editor's view nor the notification handler sees any of this; callers
/// record edits themselves (`Tabs::note_background_edit`).
pub(crate) fn with_background_document<R>(
    hwnd: HWND,
    target: &EditorDocument,
    f: impl FnOnce(&Editor) -> Result<R>,
) -> Result<R> {
    let host = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.document_host.clone())
        .ok_or(FastPadError::Invariant("the document host is missing"))?;
    host.use_document(target)?;
    let result = f(&host);
    // Leave the host on a document of its own, so it never holds a closed tab's text alive.
    if let Ok(blank) = host.create_document() {
        let _ = host.use_document(&blank);
    }
    result
}
```

Rewrite the four callers to use it, and delete:
- the selection and first-line save and restore;
- the `set_file_population(true/false)` pair around the swap;
- the `use_document(active)` swap back.

Keep the early `populating_file` bail-outs that guard **other** reasons for population, and keep `note_background_edit` after a replace.

`reload_clean_document` has an **active-tab branch** (:4962-4976) that sets the population flag by hand. That branch still reloads through the visible editor, so leave it alone. Only its inactive branch moves to the host.

- [ ] **Step 4: Clippy and the targeted tests.** Run the new test plus the existing background tests:

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- main_window::tests::replacing_in_a_background main_window::tests::replace main_window::tests::snapshot main_window::tests::reload search_view:: --test-threads=1
```

Expected: PASS. If an existing test asserted on the old swap (for example, that the population flag was set during a background read), fix the test's expectation, not the host path, and say so in the commit body.

- [ ] **Step 5: Commit.**

```powershell
git add -A src
git commit -m "refactor(split-editors): create documents on the host and edit background tabs there"
```

---

## Task 4: Session version 2

Afterwards, `session.ini` is written as version 2, with a layout, groups and every view's state. Version 1 reads as one group. In PR 1 there is only one group; a multi-group file is flattened into one group in order.

**Files:**
- Modify: `src/session.rs` (format, types, tests).
- Modify: `src/window/main_window.rs`: `build_session` (:5717), `restore_session_entry` (:5104), `finish_session_restore` (:5169).

**Interfaces:**
- Produces:
  - `SessionLayout`:

    ```rust
    pub enum SessionLayout { Leaf(usize), Branch { axis: SessionAxis, children: Vec<(SessionLayout, f32)> } }
    pub enum SessionAxis { Row, Column }
    ```
  - `SessionGroup { number: usize, active: usize, entries: Vec<SessionEntry> }`.
  - `Session { layout: SessionLayout, active_group: usize, groups: Vec<SessionGroup> }`.
  - `SessionLayout::parse(&str) -> Option<SessionLayout>` and `SessionLayout::encode(&self) -> String`.
  - `Session::flattened(&self) -> (usize, Vec<SessionEntry>)`: every entry in group order, and the index of the active group's active entry.
  - `SessionEntry` is unchanged.
  - `SessionRestore` keeps its API, built from `flattened()`.

- [ ] **Step 1: Write the failing tests.** Replace `sample()` and the version test in `session.rs`, and keep the other existing tests, adjusted to the new struct shape:

```rust
    fn sample() -> Session {
        Session {
            layout: SessionLayout::Branch {
                axis: SessionAxis::Row,
                children: vec![
                    (SessionLayout::Leaf(1), 0.5),
                    (
                        SessionLayout::Branch {
                            axis: SessionAxis::Column,
                            children: vec![(SessionLayout::Leaf(2), 0.6), (SessionLayout::Leaf(3), 0.4)],
                        },
                        0.5,
                    ),
                ],
            },
            active_group: 1,
            groups: vec![
                SessionGroup {
                    number: 1,
                    active: 0,
                    entries: vec![SessionEntry::new(SessionSource::File(PathBuf::from(r"C:\notes\a b|c.txt")))],
                },
                SessionGroup {
                    number: 2,
                    active: 0,
                    entries: vec![SessionEntry {
                        source: SessionSource::Snapshot(RecoveryId::from_u128(0xabc)),
                        caret: 12,
                        anchor: 4,
                        first_line: 3,
                    }],
                },
                SessionGroup { number: 3, active: 0, entries: vec![SessionEntry::new(SessionSource::File(PathBuf::from(r"C:\x.md")))] },
            ],
        }
    }

    #[test]
    fn a_manifest_round_trips_through_its_text_form() {
        // Break caught: a layout, a group boundary, a path with `|` or a snapshot id that does not
        // survive a write and read.
        let text = sample().encode();
        assert!(text.starts_with("version=2\r\n"));
        assert!(text.contains("layout=row(1:0.5,column(2:0.6,3:0.4):0.5)\r\n"));
        assert!(text.contains("group=2|active=0\r\nsnapshot=12|4|3|00000000000000000000000000000abc\r\n"));
        assert_eq!(Session::parse(&text), Some(sample()));
    }

    #[test]
    fn a_version_one_manifest_reads_as_one_group() {
        // Break caught (Review Focus 5): upgrading from 0.2.0 opening to an empty window.
        let parsed = Session::parse("version=1\r\nactive=1\r\nfile=0|0|0|C:\\a.txt\r\nfile=5|2|1|C:\\b.txt\r\n").unwrap();
        assert_eq!(parsed.layout, SessionLayout::Leaf(1));
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].active, 1);
        assert_eq!(parsed.groups[0].entries[1].caret, 5);
        assert_eq!(parsed.flattened().0, 1);
    }

    #[test]
    fn only_versions_one_and_two_are_accepted() {
        // Break caught: a future or hand-damaged manifest restoring garbage instead of nothing.
        assert_eq!(Session::parse("active=0\r\nfile=0|0|0|C:\\a.txt\r\n"), None);
        assert_eq!(Session::parse("version=3\r\nfile=0|0|0|C:\\a.txt\r\n"), None);
    }

    #[test]
    fn a_bad_or_mismatched_layout_falls_back_to_one_group_in_file_order() {
        // Break caught: a damaged layout line losing every tab of the session.
        for layout in ["row(1:0.5", "row(1:0.5,9:0.5)", "column()", "row(1:x,2:1)"] {
            let text = format!(
                "version=2\r\nlayout={layout}\r\nactive_group=2\r\ngroup=1|active=0\r\nfile=0|0|0|C:\\a.txt\r\n\
                 group=2|active=0\r\nfile=0|0|0|C:\\b.txt\r\n"
            );
            let parsed = Session::parse(&text).unwrap();
            assert_eq!(parsed.layout, SessionLayout::Leaf(1), "{layout}");
            assert_eq!(parsed.groups.len(), 1, "{layout}");
            assert_eq!(parsed.groups[0].entries.len(), 2, "{layout}");
        }
    }

    #[test]
    fn ratios_are_normalized_and_out_of_range_indices_fall_back() {
        // Break caught: hand-edited ratios that no longer sum to 1 giving groups no width, or an
        // active index past the end activating nothing.
        let parsed = Session::parse(
            "version=2\r\nlayout=row(1:1,2:3)\r\nactive_group=7\r\ngroup=1|active=4\r\nfile=0|0|0|C:\\a.txt\r\n\
             group=2|active=0\r\nfile=0|0|0|C:\\b.txt\r\n",
        )
        .unwrap();
        let SessionLayout::Branch { children, .. } = &parsed.layout else { panic!("branch") };
        assert!((children[0].1 - 0.25).abs() < 1e-6 && (children[1].1 - 0.75).abs() < 1e-6);
        assert_eq!(parsed.active_group, 0);
        assert_eq!(parsed.groups[0].active, 0);
    }

    #[test]
    fn flattening_puts_groups_in_order_and_finds_the_active_entry() {
        // Break caught: PR 1 restoring a multi-group file with the wrong tab active.
        let (active, entries) = sample().flattened();
        assert_eq!(entries.len(), 3);
        assert_eq!(active, 1);
    }
```

`active_group` is an **index into `groups`** (0-based), unlike `number`, which is the name used in `layout`. Group numbers are written 1-based by `encode`: `number` is `index + 1`.

`SessionLayout` holds `f32` ratios, so `Session`, `SessionGroup` and `SessionLayout` derive `Clone, Debug, PartialEq`, without `Eq`. `Session::default()` goes away; nothing needs it once `build_session` builds the value directly.

A branch's ratios are normalized **only when their sum is off from 1 by more than 1e-4**. Otherwise a written 0.6 / 0.4 pair would come back as slightly different floats and the round-trip test would fail.

- [ ] **Step 2: Implement.**
  - `encode` writes:
    - `version=2`;
    - `layout=<layout.encode()>`;
    - `active_group=<index>`;
    - for each group, `group=<number>|active=<i>` followed by its entry lines, in today's entry format.
  - `SessionLayout::encode` writes a leaf as its number and a branch as `row(`/`column(`, with the children as `<child>:<ratio>` joined by `,`, then `)`. Ratios use `format!("{:.4}", r)` with trailing zeros and a trailing `.` trimmed, so 0.5 is written as `0.5` and 1.0 as `1`.
  - `SessionLayout::parse` is a small recursive-descent parser over the bytes:
    - `node := number | ("row"|"column") "(" child ("," child)* ")"`;
    - `child := node ":" float`.

    It rejects trailing input, empty branches, non-finite and non-positive ratios, and duplicate leaf numbers. It normalizes each branch's ratios to sum to 1.
  - `Session::parse`:
    - reads `version`;
    - for `1`, collects entries and `active` exactly as today into one group numbered 1, with `layout = Leaf(1)`;
    - for `2`, collects `layout`, `active_group` and `group=` lines; the entry lines that follow a `group=` line belong to it, and entry lines before any `group=` line are dropped;
    - then validates: the layout's leaf numbers must be exactly the set of group numbers. Otherwise the layout becomes `Leaf(1)`, every group's entries are concatenated in file order into one group numbered 1, and `active = 0`;
    - clamps `active_group` and each `active` to 0 when out of range;
    - removes empty groups; if a group is removed while the layout is still valid, its leaf is removed and the tree is collapsed. Write `SessionLayout::without(number) -> Option<SessionLayout>` for this, returning `None` when nothing is left;
    - returns `None` for any other version.
  - `flattened` concatenates the groups' entries in `groups` order and computes the active entry's index from `active_group` and that group's `active`.
  - Update the module doc comment (lines 1-8) to describe version 2 and the version 1 reading.

- [ ] **Step 3: Restore and build with per-view state.**
  - `build_session` produces **one** `SessionGroup { number: 1, active: <active index>, entries }` with `layout: SessionLayout::Leaf(1)` and `active_group: 0`.
  - Each entry's view state comes from:
    - the live editor (`editor.view_state()`) for the active text tab;
    - `app.tabs.view_state(id)` for every other tab.
  - Image tabs keep zeros.
  - `SessionRestore::new` takes `session.flattened()`. Store the flattened entries and active index in `SessionRestore` in place of `session`, and keep its methods working on them.
  - `restore_session_entry`: after a tab is restored, call `app.tabs.set_view_state(id, ViewState { caret, anchor, first_line, x_offset: 0 })` from the entry, so a tab the user switches to later lands where it was.
  - `finish_session_restore` keeps applying the active entry to the editor, as today.

- [ ] **Step 4: Clippy, the targeted tests, commit.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- session:: main_window::tests::session --test-threads=1
git add -A src
git commit -m "feat(split-editors): session format 2 with layout, groups and every tab's position"
```

Also run the session e2e target if one exists (`grep -ln "session" tests/*.rs`), with `cargo test --test <name> -- --test-threads=1`.

---

## Task 5: The group window hosts the editor, find bar, preview and image view

Afterwards, a `FastPadEditorGroup` child window sits in the content area and is the parent of the Scintilla editor, the find bar panel, the preview and SVG views and the image view. The tab strip is still in the title bar; Part 6 moves it.

**Files:**
- Create: `src/window/editor_group.rs` (window class, `GroupWindow`, window procedure, and layout of the group's children).
- Modify: `src/app.rs`:
  - `editor`, `find_bar`, `preview` and `image` move into `GroupWindow`, and `App` gains `pub(crate) groups: Vec<GroupWindow>` and `pub(crate) active_group: usize`;
  - accessors `active_group(&self) -> Option<&GroupWindow>`, `active_group_mut(&mut self) -> Option<&mut GroupWindow>` and `editor(&self) -> Option<&Editor>` (the active group's).
- Modify: `src/window/main_window.rs`:
  - `initialize_editor_with` (create the group, then the editor inside it);
  - `layout_editor_and_find_bar` (:1134);
  - the `WM_NOTIFY` branch (:559);
  - the divider branches in `WM_MOUSEMOVE` (:362), `WM_LBUTTONDOWN` (:403), `WM_CAPTURECHANGED` (:426), `WM_LBUTTONUP` (:487) and `WM_SETCURSOR` (:704), which move to the group;
  - `WM_PAINT`: the divider and empty hint are no longer painted by the main window;
  - `ensure_find_bar` (:1242);
  - every `app.editor` / `app.find_bar` / `app.preview` / `app.image` use.
- Modify: `src/window/panel.rs:102`, `src/window/find_bar.rs:1006`, `src/preview/view.rs` (the posts at :810, :1133, :1148, :1426 and :1523) and `src/window/image_view/mod.rs:746`. Every place that treats `GetParent(x)` as the main window now uses `main_window::root_of(x)`.
- Modify: `src/window/preview_host.rs`:
  - `layout`, `begin_divider_drag`, `drag_divider`, `cancel_divider_drag`, `end_divider_drag`, `cursor_over_divider` and `divider_rect` work in **group-client coordinates** and capture the group window;
  - `with_host`, `ensure_view` and `ensure_svg_view` use the active group's `PreviewHost` and the group as parent.
- Modify: `src/window/image_host.rs`: `ensure_view` uses the group as parent; `layout` takes a group-client rectangle.
- Modify: `src/window/titlebar.rs`: move the empty-hint drawing (:726-753) into `pub(crate) unsafe fn paint_empty_hint(dc: HDC, content: Rect, hint: &str, palette: Palette, font: HFONT, dpi: u32)`, which the group calls, and drop `empty_hint` and `divider` from `TitlePaint`.

**Interfaces:**
- Consumes: `App.document_host` and `Editor::create_with_host` (Parts 1 and 3).
- Produces:
  - `pub(crate) struct GroupWindow { pub(crate) hwnd: HWND, pub(crate) editor: Editor, pub(crate) find_bar: Option<FindBar>, pub(crate) preview: PreviewHost, pub(crate) image: ImageHost, pub(crate) strip: GroupStripState }`. `GroupStripState` is an empty `#[derive(Default)] struct` until Part 6 fills it.
  - `editor_group::create(main: HWND) -> Result<HWND>` registers the class `FastPadEditorGroup` once, with `WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS`.
  - `editor_group::layout(main: HWND, group: HWND)` places the find bar, editor, preview and image view inside the group's client rectangle.
  - `main_window::root_of(hwnd: HWND) -> HWND` is `GetAncestor(hwnd, GA_ROOT)`.
  - `main_window::group_of(main: HWND, child: HWND) -> Option<usize>` returns the index of the group whose window is `child` or an ancestor of it.

- [ ] **Step 1: Write the failing window tests** in the `main_window.rs` tests:

```rust
    #[test]
    fn the_editor_find_bar_and_preview_live_in_the_group_window() {
        // Break caught: a child left parented to the main window, painting over or under the
        // group and missing its layout.
        // Arrange: production window with the test editor installed; open the find bar
        // (CommandId::Find); open a Markdown tab and PreviewSide.
        // Assert: GetParent(editor hwnd) == the group hwnd; GetParent(find bar panel) == group;
        // GetParent(preview view hwnd) == group; the group's window rect is inside the main
        // client area below the title band and above the status bar.
    }

    #[test]
    fn find_bar_keys_and_preview_links_still_reach_the_main_window() {
        // Break caught (Review Focus 4): panel_proc and the find field hook, or the preview's
        // posted messages, sending to GetParent, now the group, so Enter, Escape and the toggle
        // buttons do nothing.
        // Arrange: text "abc abc", open find, type "abc" into the query field.
        // Act: send VK_RETURN to the query edit; then VK_ESCAPE.
        // Assert: the selection moved to the second match; the find bar is hidden.
        // Act: with a Markdown preview shown, post WM_FASTPAD_PREVIEW_ESCAPE from the preview view
        // the way the view does, and pump messages.
        // Assert: focus is back in the editor (preview_host::escape ran).
    }

    #[test]
    fn scintilla_notifications_still_mark_the_tab_dirty() {
        // Break caught: WM_NOTIFY now going to the group, which drops it, so typing never dirties
        // the tab and never autosaves.
        // Arrange: a clean tab. Act: insert text through the editor.
        // Assert: the active document is dirty and the strip title ends with " *".
    }
```

Write them with the fixtures and helpers the neighbouring find-bar and preview tests use (search the tests for `CommandId::Find` and `PreviewSide`).

- [ ] **Step 2: Add `root_of` and replace the parent-as-main assumptions.** Add to `main_window.rs`:

```rust
/// The top-level FastPad window that owns `hwnd`. Children now sit inside editor groups, so their
/// direct parent is no longer the main window (split editors spec §4.2).
pub(crate) fn root_of(hwnd: HWND) -> HWND {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(hwnd, GA_ROOT) }
}
```

Change each site:
- `panel.rs:102`: `let main = GetParent(panel)` becomes `root_of(panel)`.
- `find_bar.rs:1006` (`with_bar`): the same change.
- The `FindFieldHook.parent` stored at `find_bar.rs:1097`: pass `root_of(parent)` when installing.
- `preview/view.rs`: every `PostMessageW(GetParent(hwnd), …)` becomes `root_of(hwnd)`.
- `image_view/mod.rs:746`: the same change.

Check `grep -rn "GetParent(" src/window src/preview` for any other site that means the main window, and change those too. `GA_ROOT` may need a new `windows-sys` feature. If so, add it to `Cargo.toml` and to `tools/audit-dependencies.ps1`.

- [ ] **Step 3: Write `editor_group.rs`.** It has:
  - A window class registered once, the same way `panel.rs` registers its class. It uses `CS_DBLCLKS` (Part 6 needs double-clicks on the strip) and a null background brush; `WM_ERASEBKGND` returns 1.
  - A window procedure:
    - `WM_NOTIFY`, `WM_COMMAND`, `WM_CTLCOLOREDIT`, `WM_CTLCOLORSTATIC`, `WM_CTLCOLORBTN` and `WM_DRAWITEM` are forwarded with `SendMessageW(root_of(hwnd), message, wparam, lparam)` and its result returned. This keeps the main window's Scintilla notification and control-colour paths working unchanged.
    - `WM_SIZE` calls `layout(root_of(hwnd), hwnd)`.
    - `WM_SETFOCUS` does `SetFocus(main_window::content_focus_target(root_of(hwnd)))`. Make `content_focus_target` `pub(crate)`.
    - `WM_LBUTTONDOWN`, `WM_MOUSEMOVE`, `WM_LBUTTONUP`, `WM_CAPTURECHANGED` and `WM_SETCURSOR` call the `preview_host` divider functions with `(root_of(hwnd), hwnd, x, y)` in group-client coordinates. The logic is the same as the main window's branches today, which are deleted.
    - `WM_PAINT` fills the background with `palette.editor_background` and paints the divider (`preview_host::divider_rect`) with `palette.hover_background`. When no tab is open it calls `titlebar::paint_empty_hint` over the content rectangle.
  - `layout(main, group)` holds the child layout that `layout_editor_and_find_bar` does today, but relative to the group's client rectangle:
    1. `left = 0`, `top = 0` (Part 6 adds the strip's height), `width = client.right`.
    2. The find bar: `bar.layout(0, width, top, dpi, font)`, then `top += find_bar_height(dpi)` when visible.
    3. `area = RECT { 0, top, width, client.bottom }`.
    4. `let rects = preview_host::layout(main, area, dpi)`, then `image_host::layout(main, area)`.
    5. `MoveWindow(editor, rects.editor…)`.

- [ ] **Step 4: Rewire the main window.**
  - `initialize_editor_with` creates the group with `editor_group::create(hwnd)?`, then the editor with `create_editor(group, &host)`. It pushes a `GroupWindow` into `app.groups` with `active_group = 0`.
  - `layout_editor_and_find_bar` keeps the sidebar, palette and name box layout. It then computes the content rectangle it computes today, minus the find bar (the group owns that now). The name box stays a main-window child above the group, so `content_top` = title strip + menu band + name box. It ends with `MoveWindow(group_hwnd, area…)` followed by `editor_group::layout(hwnd, group_hwnd)`.
  - `with_editor`, `editor_hwnd`, `ensure_find_bar` (`FindBar::create(group_hwnd)`), `preview_host::with_host` and `image_host` reach their objects through `app.active_group()` / `app.active_group_mut()`. Do this as a mechanical sweep. List the sites with:

    ```powershell
    rg -n "\.editor\b|\.find_bar\b|\bapp\.preview\b|\bapp\.image\b|\.preview\.|\.image\." src/window src/bootstrap.rs src/app.rs
    ```

    Rewrite `app.editor.clone()` as `app.editor().cloned()`, `app.editor.as_ref()` as `app.editor()`, and `app.find_bar.as_mut()` as `app.active_group_mut().and_then(|g| g.find_bar.as_mut())`. Handle `app.preview` and `app.image` the same way.
  - `handle_editor_notification` keeps comparing `hwndFrom` to the active group's editor. The group forwards the message, so the main window's `WM_NOTIFY` branch is unchanged.
  - **The drop order in `App`**: `groups` is declared before `document_host`, so the group windows' editors go first. Keep the existing comment's rule that `image` is declared before `preview` inside `GroupWindow`, because the image view shares the preview's Direct2D factories.
  - **Destroying the main window** destroys the group window and its children as today's children are destroyed, because the group is a child of the main window. The `Editor` Rc guard (`destroyed`) already copes with a window destroyed before its `Editor` drops.
  - `refresh_tabs`'s editor show/hide and focus checks (:2412, :2419) use the group's editor. "Focus on the frame" (`GetFocus() == hwnd`) also counts focus on the group window (`GetFocus() == group`).
  - `preview_host.rs` and `image_host.rs`: their `GetFocus()` checks against the editor keep working through the accessors; checks against the main window also accept the group window.

- [ ] **Step 5: Clippy, the targeted tests, commit.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- main_window::tests::the_editor_find_bar main_window::tests::find_bar_keys main_window::tests::scintilla_notifications preview_host:: image_host:: find_bar:: --test-threads=1
cargo test --lib -- main_window::tests::find main_window::tests::preview main_window::tests::image --test-threads=1
git add -A src Cargo.toml tools/audit-dependencies.ps1
git commit -m "refactor(split-editors): an editor group window hosts the editor, find bar, preview and image view"
```

---

## Task 6: The tab strip moves into the group

Afterwards, the group paints its own strip at the top, with tabs on the left and Preview Side, Preview Full and "…" on the right, and handles all the strip's input. The title bar keeps only the logo space, the app menu "…", the window title text and the caption buttons.

**Files:**
- Create: `src/window/group_strip.rs` (pure layout and hit-testing, plus painting).
- Modify: `src/window/editor_group.rs` (strip input, painting, and the strip in the layout).
- Modify: `src/window/titlebar.rs`: remove the tab, preview-button and scroll geometry from `TitleBarLayout` and `draw_strip`, and draw the window title.
- Modify: `src/window/main_window.rs`: remove the strip branches from `main_window_proc` (the WM_LBUTTONUP `Tab`/`CloseTab`/Preview, middle click, wheel, thumb drag, the WM_NCLBUTTONDBLCLK → New, WM_NCRBUTTONUP → strip menu); `nonclient_hit_test` callers; `tab_count`/`tab_scroll`/`title_layout`/`preview_buttons_visible`; `refresh_tabs`'s `scroll_to_reveal`; `invalidate_title_strip` (:5859), which must also invalidate the group window.
- Modify: `src/app.rs`: `tab_thumb_grab`, `middle_press`, `last_tab_click` and the strip half of `title_pointer` move into `GroupStripState`.
- Modify: `src/window/tabs.rs`: `TabSelection` drops `strip_left`, because the strip starts at the group's x = 0.

**Interfaces:**
- Produces, in `group_strip.rs`:

```rust
pub(crate) enum StripTarget { Tab(usize), CloseTab(usize), ScrollBar, PreviewSide, PreviewFull, More, Empty }

pub(crate) struct StripLayout {
    pub(crate) height: i32,
    pub(crate) tabs: Rect,            // the tab viewport
    pub(crate) preview_side: Option<Rect>,
    pub(crate) preview_full: Option<Rect>,
    pub(crate) more: Rect,
    pub(crate) scroll: i32,
    pub(crate) max_scroll: i32,
    pub(crate) scroll_bar: Option<Rect>,
    // private: tab_width, tab_rects, close_tab_rects, min_thumb
}

impl StripLayout {
    pub(crate) fn calculate(width: i32, dpi: u32, tab_count: usize, scroll: i32, preview_buttons: bool) -> Self;
    pub(crate) fn hit_test(&self, point: Point) -> StripTarget;
    pub(crate) fn tab(&self, index: usize) -> Option<Rect>;
    pub(crate) fn close_tab(&self, index: usize) -> Option<Rect>;
    pub(crate) fn scroll_to_reveal(&self, index: usize) -> i32;
    pub(crate) fn scroll_by_wheel(&self, delta: i32, wheel_delta: i32) -> i32;
    pub(crate) fn scroll_thumb(&self) -> Option<Rect>;
    pub(crate) fn scroll_for_thumb(&self, thumb_left: i32) -> i32;
}

pub(crate) fn strip_height(dpi: u32) -> i32; // scale(40, dpi): as tall as today's tabs (spec §4.2)

pub(crate) struct StripPaint<'a> {
    pub(crate) titles: &'a [&'a str],
    pub(crate) active: usize,
    pub(crate) preview_tab: Option<usize>,
    pub(crate) scroll: i32,
    pub(crate) palette: Palette,
    pub(crate) fonts: TitleFontHandles,
    pub(crate) hovered: Option<StripTarget>,
    pub(crate) pressed: Option<StripTarget>,
    pub(crate) preview: Option<crate::preview::PreviewMode>,
}

pub(crate) unsafe fn paint(dc: HDC, layout: &StripLayout, dpi: u32, input: &StripPaint<'_>);
```

`GroupStripState`, in `editor_group.rs`, holds `hovered`, `pressed`, `thumb_grab: Option<i32>`, `middle_press: Option<(usize, DocumentId)>` and `last_tab_click: Option<(DocumentId, u32)>`.

- [ ] **Step 1: Port the geometry and write its tests.** Move today's tab geometry from `TitleBarLayout::calculate_with_offset` (titlebar.rs:267-340) into `StripLayout::calculate`, with these changes:
  - There are no caption buttons and no 48 px drag strip.
  - From the right: `more` is `scale(40)` wide at the right edge, and when `preview_buttons` is set, `preview_full` and then `preview_side` (each `scale(40)`) sit to its left.
  - The tab viewport is `0 .. buttons_left`.
  - The tab width, the close box, the scroll range and the scroll bar use today's formulas unchanged.

Move `scroll_to_reveal`, `scroll_by_wheel`, `scroll_thumb`, `thumb_width` and `scroll_for_thumb` as they are. Move the titlebar tests that cover tab geometry, scrolling and the thumb into `group_strip.rs` and adapt them to `StripLayout::calculate`. Then add:

```rust
    #[test]
    fn every_painted_rect_hits_its_own_target() {
        // Break caught (Review Focus 3): paint and hit-test disagreeing, so a click lands on the
        // neighbouring tab or on a button that is not drawn there.
        for (width, count, scroll, preview) in [(900, 3, 0, false), (900, 12, 250, true), (300, 2, 0, true)] {
            let layout = StripLayout::calculate(width, 144, count, scroll, preview);
            let centre = |r: Rect| Point { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
            for index in 0..count {
                if let Some(tab) = layout.tab(index) {
                    let close = layout.close_tab(index).unwrap();
                    if tab.right <= layout.tabs.left || tab.left >= layout.tabs.right { continue; }
                    let label = Point { x: (tab.left + close.left) / 2, y: (tab.top + tab.bottom) / 2 };
                    if label.x > layout.tabs.left && label.x < layout.tabs.right {
                        assert_eq!(layout.hit_test(label), StripTarget::Tab(index));
                    }
                    if close.left >= layout.tabs.left && close.right <= layout.tabs.right {
                        assert_eq!(layout.hit_test(centre(close)), StripTarget::CloseTab(index));
                    }
                }
            }
            assert_eq!(layout.hit_test(centre(layout.more)), StripTarget::More);
            assert_eq!(layout.preview_side.is_some(), preview);
            if let Some(side) = layout.preview_side {
                assert_eq!(layout.hit_test(centre(side)), StripTarget::PreviewSide);
            }
        }
    }

    #[test]
    fn the_space_after_the_last_tab_is_empty_strip() {
        // Break caught: a double-click after the last tab not opening a new tab because it hit
        // a stale tab rectangle.
        let layout = StripLayout::calculate(1200, 96, 2, 0, false);
        let after = layout.tab(1).unwrap().right + 10;
        assert_eq!(layout.hit_test(Point { x: after, y: 10 }), StripTarget::Empty);
    }
```

Derive `PartialEq, Eq, Debug, Clone, Copy` on `StripTarget`. The scroll bar's hit area is the bottom `scale(8)` of `tabs`, and it is tested **before** tabs, as today.

- [ ] **Step 2: Port the painting.** Move `draw_strip`'s tab section (titlebar.rs:874-969), the preview buttons (:994-1031) and the double-buffering (`paint_strip_buffered`) into `group_strip::paint`. Draw `more` with the same `GLYPH_MORE` and hover/press treatment the title overflow uses. Share glyph and fill helpers by making them `pub(crate)` in `titlebar.rs`; don't copy them.

- [ ] **Step 3: Wire the group window.**
  - `editor_group::layout` puts the strip first: `top = group_strip::strip_height(dpi)`. It invalidates the strip rectangle.
  - `WM_PAINT` builds `StripPaint` from `main_window::tab_snapshot(root)` (:2285), the palette and fonts (`title_chrome`), the group's `GroupStripState` and `preview_host`'s button mode, then calls `group_strip::paint` for the top band.
  - Input moves from `main_window_proc` into the group procedure. Each branch calls the same `main_window` helpers it calls today, made `pub(crate)` where needed:
    - `WM_MOUSEMOVE`: thumb drag, `TrackMouseEvent(TME_LEAVE)`, hover update and invalidation, and `preview_host::button_hover`.
    - `WM_MOUSELEAVE`: clear hover and `middle_press`, and `button_hover(None)`.
    - `WM_LBUTTONDOWN`: press; `ScrollBar` starts the thumb drag (SetCapture on the group).
    - `WM_LBUTTONUP`:
      - `Tab(i)`: `activate_tab`, plus today's double-click promotion via `last_tab_click`;
      - `CloseTab(i)`: activate the tab, then `CommandId::CloseTab`;
      - `PreviewSide` / `PreviewFull`: `preview_host::click_button`;
      - `More`: `menus::show_tab_strip_menu(group, x, y, has_tabs)`, then `execute_command(root, command)`.
    - `WM_LBUTTONDBLCLK` on `Empty`: `execute_command(root, CommandId::New)`.
    - `WM_RBUTTONUP` on `Empty`: `menus::show_tab_strip_menu(group, …)`.
    - `WM_MBUTTONDOWN` / `WM_MBUTTONUP`: today's middle-click close through `close_tab_at`.
    - `WM_MOUSEWHEEL` / `WM_MOUSEHWHEEL` over the strip: `scroll_tabs`, with the tab-scroll offset in `app.tabs` as today.
    - `WM_CAPTURECHANGED`: end the thumb drag and the divider drag.
  - `track_popup` takes the window whose client coordinates it is given, so pass the group.
  - `refresh_tabs` computes `scroll_to_reveal` with `StripLayout::calculate(group width, …)`.
  - `invalidate_title_strip` invalidates the group's strip rectangle as well as the main window.

- [ ] **Step 4: Slim the title bar.**
  - `TitleBarLayout::calculate_with_offset(client, dpi, left)` drops `tab_count`, `scroll` and `preview_buttons`. It keeps `sidebar`, the caption buttons, `overflow` (the app menu) and `drag_region`, which now spans from `left` to `overflow.left`.
  - Delete the tab and preview fields and methods it no longer has. The compiler lists every caller; `accessibility.rs` is rewritten in Part 7.
  - `hit_test` returns `Caption` over `drag_region`, so `nonclient_hit_test` makes the whole band a real caption. Dragging moves the window, and a double-click maximizes (plan-time amendment 4).
  - `draw_strip` paints the band background, the window title, then overflow and the caption buttons. The title text is `main_window::window_title(active title)`, the same string `sync_window_title` sets. It is drawn with `fonts.text` and `palette.muted_foreground`, left-aligned in `drag_region` with a `scale(12)` margin, single line with an end ellipsis.
  - `TitlePaint` drops `titles`, `active`, `preview_tab`, `scroll` and `preview`, and gains `title: &'a str`.
  - Delete from `main_window_proc` the branches listed under Files. `WM_NCLBUTTONDBLCLK` and `WM_NCRBUTTONUP` on `HTCAPTION` go to `DefWindowProcW`.

- [ ] **Step 5: Write the window tests** in the `main_window.rs` tests. Use the helpers the existing tab tests use to click; they post `WM_LBUTTONUP` with client coordinates. Now the target is the group window at the strip layout's rectangles:

```rust
    #[test]
    fn clicking_a_tab_in_the_group_strip_activates_it() {
        // Break caught: strip input still handled by the main window, so clicks on the moved
        // strip do nothing.
    }

    #[test]
    fn middle_clicking_a_tab_in_the_group_strip_closes_it() {
        // Break caught: middle-click close lost in the move.
    }

    #[test]
    fn double_clicking_the_empty_strip_opens_a_new_tab_and_the_title_bar_does_not() {
        // Break caught: New still bound to the caption double-click, which must now maximize.
        // Assert tab count +1 after WM_LBUTTONDBLCLK on StripTarget::Empty in the group, and
        // unchanged after WM_NCLBUTTONDBLCLK with HTCAPTION on the main window. The tests don't
        // check maximization; only the tab count.
    }

    #[test]
    fn the_title_bar_shows_the_window_title() {
        // Break caught: the title bar blank after the tabs left it.
        // Assert the TitlePaint built for WM_PAINT carries window_title(active title); build it
        // through the same helper WM_PAINT uses.
    }
```

Write each with the concrete message posts and the `StripLayout` rectangles.

- [ ] **Step 6: Clippy, the targeted tests, commit.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- group_strip:: titlebar:: main_window::tests::clicking_a_tab main_window::tests::middle_clicking main_window::tests::double_clicking_the_empty main_window::tests::the_title_bar main_window::tests::tab --test-threads=1
git add -A src
git commit -m "feat(split-editors): the tab strip moves from the title bar into the editor group"
```

---

## Task 7: Accessibility for the moved strip

Afterwards, the group window answers `WM_GETOBJECT` with the tab-list provider: tabs, the preview buttons and "…". The main window's provider exposes only the title bar: app menu, minimize, maximize and close.

**Files:**
- Modify: `src/window/accessibility.rs`:
  - `AccessibleProvider` gains `kind: ProviderKind`;
  - `children_from_view` (:933), `native_layout` (:914), `child_screen_rect` (:874), `accessible_hit_test` (:763), `accessible_default_action` (:642) and `child_name` (:849) branch on it.
- Modify: `src/window/editor_group.rs` (`WM_GETOBJECT`), `src/app.rs` (`GroupWindow.accessibility`; `App::ensure_accessibility` becomes the title-bar kind).

**Interfaces:**
- Produces:
  - `pub(crate) enum ProviderKind { TitleBar, GroupStrip }`.
  - `AccessibilityState::ensure(&mut self, hwnd: HWND, kind: ProviderKind, view: TabView, selection: TabSelection) -> *mut c_void`.

The children and names by kind:
- `GroupStrip`: the root is named "Tabs", role `ROLE_SYSTEM_PAGETABLIST`. Its children are one `Tab(title)` per tab, then `PreviewSide` and `PreviewFull` when shown, then `More`. Locations come from `StripLayout::calculate(group client width, dpi, …)` offset by `ClientToScreen(group)`. Select works as today, via `WM_FASTPAD_ACCESSIBLE_SELECT` to `root_of(group)`. Default actions: tab → close, as today; buttons post `WM_LBUTTONUP` at the button's centre to the **group**.
- `TitleBar`: the root is named "FastPad title bar", role `ROLE_SYSTEM_TITLEBAR`. Its children are `Overflow` (named "Application menu"), `Minimize`, `Maximize` and `Close`, with today's actions.

- [ ] **Step 1: Write the failing tests**, next to the existing accessibility tests that use `ensure_for_test` (:101):

```rust
    #[test]
    fn the_group_strip_provider_lists_tabs_and_strip_buttons_only() {
        // Break caught: Narrator on the strip announcing caption buttons, or missing the tabs
        // after the move.
        // Build a TabView with two tabs and preview_buttons = true; ensure_for_test with
        // ProviderKind::GroupStrip; assert the child count is 2 + 2 + 1, child 1's name is the
        // first title and role PAGETAB, and the last child is named "More actions".
    }

    #[test]
    fn the_title_bar_provider_lists_the_app_menu_and_caption_buttons() {
        // Break caught: caption buttons unreachable by Narrator once tabs left the title bar.
        // ensure_for_test with ProviderKind::TitleBar; assert four children named
        // "Application menu", "Minimize", "Maximize", "Close".
    }
```

Extend `ensure_for_test` to take a `ProviderKind` and update its existing callers to pass `GroupStrip`, which keeps their tab expectations meaningful.

- [ ] **Step 2: Implement** the `kind` branches listed above.
  - `WM_GETOBJECT` with `OBJID_CLIENT` in the group procedure calls `ensure(group, ProviderKind::GroupStrip, tabs.view(), tabs.selection())` on the group's `AccessibilityState`, then `object_result`.
  - The main window keeps its branch, with `ProviderKind::TitleBar`.
  - Names: "More actions" for `More`, and "Preview side by side" / "Preview full" for the preview buttons. Reuse today's preview names if `child_name` already has them.

- [ ] **Step 3: Clippy, the targeted tests, commit.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --lib -- accessibility:: --test-threads=1
git add -A src
git commit -m "feat(split-editors): accessible tab list on the group strip, title bar provider for the caption"
```

---

## Task 8: Documentation and the startup check

**Files:**
- Modify: `README.md`, if it describes the tab strip's position, Ctrl+1..9, or double-clicking the title bar. Search it with `rg -n "title bar|Ctrl\+1|double-click" README.md`.
- Modify: `docs/superpowers/specs/2026-09-28-split-editors-design.md`: add a §10 note listing the five plan-time amendments above.

- [ ] **Step 1: Update the README** lines that the search finds. Ctrl+1..9 is unchanged in PR 1. Only the strip's position and the title bar double-click change.

- [ ] **Step 2: Record the amendments** in the spec under §10's item 1, as a sub-list "Plan-time amendments", copied from this plan.

- [ ] **Step 3: Startup benchmark.** Compare against `main`. The document host and the group window are the only first-paint additions.

```powershell
git stash list  # must be empty; commit everything first
./tools/benchmark.ps1 -Runs 50 -Warmup 5 -Output $env:TEMP\split-pr1.jsonl
git worktree add $env:TEMP\fastpad-main main
# copy native\out\x64 into the worktree (see the worktree memory), then from the worktree:
./tools/benchmark.ps1 -Runs 50 -Warmup 5 -Output $env:TEMP\split-main.jsonl
cargo run --release --bin fastpad-bench -- compare $env:TEMP\split-main.jsonl $env:TEMP\split-pr1.jsonl
```

Expected: no milestone reported as a regression. If one is, report the numbers. Don't tune without asking.

- [ ] **Step 4: Commit.**

```powershell
git add README.md docs/superpowers/specs/2026-09-28-split-editors-design.md
git commit -m "docs(split-editors): tabs below the title bar; plan-time amendments"
```

---

## Task 9: Full suite and review

- [ ] **Step 1: Run the full suite once.**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets -- --test-threads=1
```

Expected: 0 failed. If FastPad is running, the end-to-end targets fail; close it or set `CARGO_TARGET_DIR`.

- [ ] **Step 2: Whole-branch review** with superpowers:requesting-code-review against `main`, then fix what it finds, with one commit per fix wave.

- [ ] **Step 3: Manual checks** for the user, listed in the PR:
  - Narrator on the strip and on the title bar caption buttons.
  - High contrast.
  - 150–200% DPI, covering the strip, the title text and the preview divider.
  - Dragging the window by the title bar, and double-clicking it to maximize.
  - Double-click on empty strip space for New.
  - A background Search replace while scrolled deep in another tab.
