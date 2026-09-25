//! Copying into the notebook tree (open editors spec §4.4-§4.7): the plan and the clash prompts
//! on the UI thread, each answered replace sent to the Recycle Bin there as Delete does, the
//! copies on one queued worker, and the result back on the UI thread, which indexes the new
//! notes, selects a single copied row, reloads replaced clean tabs and says what happened.

use super::library_host;
use super::main_window::{TabMark, app_ptr, push_notice};
use super::tree_copy::{self, Outcome, Planned, Refusal};
use crate::document::DocumentId;
use crate::library::tree::{self, RowKind};
use crate::library::{self, model::same_path};
use crate::platform::files::{self, Copied};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// One drop's copies, in order, as the worker gets them.
struct CopyJob {
    target: isize,
    items: Vec<(PathBuf, PathBuf)>,
    /// Clean tabs open on a replaced file, to read again once it is copied.
    reloads: Vec<(DocumentId, PathBuf, TabMark)>,
    /// Notices known before the copy: refusals, failed recycles.
    notices: Vec<String>,
    /// The dirty tab whose saved version is copied, named once its copy worked.
    dirty_tab: Option<String>,
    /// The notebook copied into, and the folder in it, relative to it (empty is the root).
    root: PathBuf,
    folder: PathBuf,
}

/// One item's result.
#[derive(Debug)]
struct ItemDone {
    destination: PathBuf,
    is_folder: bool,
    copied: Copied,
}

/// `WM_FASTPAD_COPY_DONE`'s payload.
#[derive(Debug)]
pub(crate) struct CopyDone {
    items: Vec<ItemDone>,
    reloads: Vec<Reload>,
    notices: Vec<String>,
    dirty_tab: Option<String>,
    root: PathBuf,
    folder: PathBuf,
}

#[derive(Debug)]
struct Reload {
    id: DocumentId,
    path: PathBuf,
    mark: TabMark,
    stamp: Option<library::DiskStamp>,
    loaded: Option<crate::file::loader::LoadedFile>,
}

/// The one copy worker a window has, started by its first copy. Its queue is a channel: a second
/// drop waits behind the first. Dropping it (the window closing) stops the worker after the file
/// in hand and waits for that file to finish, so the process never exits with one half written;
/// nothing more is copied or posted.
#[derive(Default)]
pub(crate) struct CopyWorker {
    sender: Option<Sender<CopyJob>>,
    thread: Option<std::thread::JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    /// Drops queued whose result has not been applied yet, for `wait_for_copies`.
    #[cfg(test)]
    pending: Arc<std::sync::atomic::AtomicUsize>,
}

impl std::fmt::Debug for CopyWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CopyWorker").finish_non_exhaustive()
    }
}

impl CopyWorker {
    /// Queues `job`, starting the worker thread on the first one.
    fn send(&mut self, job: CopyJob) {
        let sender = self.sender.get_or_insert_with(|| {
            let (sender, receiver) = channel();
            let cancel = Arc::clone(&self.cancel);
            self.thread = Some(std::thread::spawn(move || run_worker(receiver, cancel)));
            sender
        });
        let _ = sender.send(job);
    }
}

impl Drop for CopyWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // The worker's `recv` ends with the channel, and the file in hand is waited for.
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_worker(jobs: Receiver<CopyJob>, cancel: Arc<AtomicBool>) {
    while let Ok(job) = jobs.recv() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let items = job
            .items
            .into_iter()
            .take_while(|_| !cancel.load(Ordering::Relaxed))
            .map(|(source, destination)| ItemDone {
                is_folder: source.is_dir(),
                copied: files::copy_tree(&source, &destination, &cancel),
                destination,
            })
            .collect();
        // Cancelled mid-copy: the window is closing, so there is nobody to tell.
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let reloads = job
            .reloads
            .into_iter()
            .map(|(id, path, mark)| {
                let stamp = library::disk_stamp(&path);
                let loaded = crate::file::loader::load(&path)
                    .ok()
                    .filter(|loaded| std::ffi::CString::new(loaded.text.as_str()).is_ok());
                Reload {
                    id,
                    path,
                    mark,
                    stamp,
                    loaded,
                }
            })
            .collect();
        let done = Box::into_raw(Box::new(CopyDone {
            items,
            reloads,
            notices: job.notices,
            dirty_tab: job.dirty_tab,
            root: job.root,
            folder: job.folder,
        }));
        let posted = unsafe {
            PostMessageW(
                job.target as HWND,
                crate::window::WM_FASTPAD_COPY_DONE,
                0,
                done as isize,
            )
        } != 0;
        if !posted {
            drop(unsafe { Box::from_raw(done) });
        }
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_RECYCLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next Recycle Bin step fail, as a network share would.
#[cfg(test)]
pub(crate) fn fail_next_recycle() {
    FAIL_RECYCLE.with(|fail| fail.set(true));
}

fn recycle(hwnd: HWND, path: &Path) -> bool {
    #[cfg(test)]
    if FAIL_RECYCLE.with(|fail| fail.replace(false)) {
        return false;
    }
    let result = if path.is_dir() {
        files::recycle_folder(hwnd, path)
    } else {
        files::recycle(hwnd, path)
    };
    result.is_ok()
}

/// Why the item at `destination`, which a clash is about to recycle, must stay, by file identity
/// rather than by the spelling the plan's lexical checks go by: it is `source` itself, or one of
/// the folders above `source` as spelled or above where `source` really is (`files::final_path`,
/// which resolves junctions, symlinks, `subst` and mapped drives). When the real location can't
/// be resolved, only the spelled path's folders are checked. When `destination` can't be
/// identified, it isn't proven safe, and it stays with the Recycle Bin notice. `None` when it
/// may go.
fn kept_by_identity(source: &Path, destination: &Path, name: &str) -> Option<String> {
    let Some(id) = files::file_id(destination) else {
        return Some(tree_copy::recycle_failed_notice(name));
    };
    let holds = |path: &Path| {
        path.ancestors()
            .skip(1)
            .any(|ancestor| files::file_id(ancestor) == Some(id))
    };
    let refusal = if files::file_id(source) == Some(id) {
        Refusal::SamePlace
    } else if holds(source) || files::final_path(source).is_some_and(|real| holds(&real)) {
        Refusal::HoldsSource
    } else {
        return None;
    };
    Some(tree_copy::refused_notice(name, refusal))
}

/// Copies `sources` (absolute) into `folder` (relative to the notebook; empty is the root):
/// plans, asks about each clash, recycles each answered replace, and queues the copies.
/// `dirty_tab` names the tab whose saved version is copied: its notice comes once that copy
/// worked.
pub(crate) fn copy_into(
    hwnd: HWND,
    sources: Vec<PathBuf>,
    folder: &Path,
    dirty_tab: Option<String>,
) {
    let Some(root) = library_host::folder(hwnd) else {
        return;
    };
    let planned = tree_copy::plan(&sources, &root, folder, &|path: &Path| path.exists());
    let folder_label = match folder.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => library_host::notebook_name(&root),
    };
    let mut notices = Vec::new();
    let mut items = Vec::new();
    let mut replaced = Vec::new();
    for Planned {
        source,
        destination,
        outcome,
    } in planned
    {
        let name = tree_copy::item_name(&source);
        match outcome {
            // A refused item's destination is never acted on: it may be a placeholder.
            Outcome::Refused(refusal) => notices.push(tree_copy::refused_notice(&name, refusal)),
            Outcome::Clash => {
                let question = tree_copy::replace_question(&name, &folder_label);
                if !library_host::confirmed(hwnd, &question) {
                    continue;
                }
                if let Some(notice) = kept_by_identity(&source, &destination, &name) {
                    notices.push(notice);
                    continue;
                }
                if !recycle(hwnd, &destination) {
                    notices.push(tree_copy::recycle_failed_notice(&name));
                    continue;
                }
                replaced.push(destination.clone());
                items.push((source, destination));
            }
            Outcome::Copy => items.push((source, destination)),
        }
    }
    if items.is_empty() {
        for notice in notices {
            push_notice(hwnd, notice);
        }
        return;
    }
    let reloads = clean_tabs_on(hwnd, &replaced);
    queue(
        hwnd,
        CopyJob {
            target: hwnd as isize,
            items,
            reloads,
            notices,
            dirty_tab,
            root,
            folder: folder.to_path_buf(),
        },
    );
}

/// An Open Editors row dropped on `folder`: its file on disk is copied. A dirty tab's saved
/// version goes, and the notice says so.
pub(crate) fn copy_tab_into(hwnd: HWND, id: DocumentId, path: &Path, folder: &Path) {
    let dirty = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            unsafe { app.as_ref() }
                .tabs
                .document(id)
                .map(|document| document.dirty)
        })
        .unwrap_or(false);
    let dirty_tab = dirty.then(|| tree_copy::item_name(path));
    copy_into(hwnd, vec![path.to_path_buf()], folder, dirty_tab);
}

/// `WM_FASTPAD_PANEL_DROPPED`'s payload: what an Explorer drop on the panel does.
pub(crate) struct PanelDrop {
    paths: Vec<PathBuf>,
    /// `None` opens the paths, as a drop on the window does.
    folder: Option<PathBuf>,
}

/// Posts an Explorer drop on the panel to the window, so Drop returns before anything is asked
/// (open editors spec §6). False when the post failed; its payload is freed here then.
pub(crate) fn post_panel_drop(hwnd: HWND, paths: Vec<PathBuf>, folder: Option<PathBuf>) -> bool {
    let payload = Box::into_raw(Box::new(PanelDrop { paths, folder }));
    let posted = unsafe {
        PostMessageW(
            hwnd,
            crate::window::WM_FASTPAD_PANEL_DROPPED,
            0,
            payload as isize,
        )
    } != 0;
    if !posted {
        drop(unsafe { Box::from_raw(payload) });
    }
    posted
}

/// `WM_FASTPAD_PANEL_DROPPED`: opens or copies. Ignored while a modal dialog runs, as
/// `WM_DROPFILES` is for a disabled window.
pub(crate) fn panel_dropped(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let drop = *unsafe { Box::from_raw(lparam as *mut PanelDrop) };
    if super::modal::modal_active(hwnd) {
        return;
    }
    match drop.folder {
        Some(folder) => copy_into(hwnd, drop.paths, &folder, None),
        None => library_host::files_dropped(hwnd, drop.paths),
    }
}

/// Clean tabs open on `replaced` files or on files under `replaced` folders.
fn clean_tabs_on(hwnd: HWND, replaced: &[PathBuf]) -> Vec<(DocumentId, PathBuf, TabMark)> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| !document.dirty)
                .filter_map(|document| {
                    let path = document.path.as_ref()?;
                    replaced
                        .iter()
                        .any(|gone| library::at_or_under(path, gone))
                        .then(|| {
                            let mark = TabMark {
                                generation: document.generation,
                                disk_stamp: document.disk_stamp,
                            };
                            (document.id, path.clone(), mark)
                        })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn queue(hwnd: HWND, job: CopyJob) {
    library_host::with_copy_worker(hwnd, |worker| {
        #[cfg(test)]
        worker.pending.fetch_add(1, Ordering::SeqCst);
        worker.send(job);
    });
}

/// `WM_DESTROY`: stops the copy worker after the file in hand and waits for it, so the process
/// never exits partway through a file. It is taken out of the window first, so nothing of the
/// App is borrowed while the UI thread waits.
pub(crate) fn stop(hwnd: HWND) {
    let worker = library_host::with_copy_worker(hwnd, std::mem::take);
    drop(worker);
}

/// `WM_FASTPAD_COPY_DONE`: frees the result and applies it.
pub(crate) fn copy_done(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let done = *unsafe { Box::from_raw(lparam as *mut CopyDone) };
    #[cfg(test)]
    library_host::with_copy_worker(hwnd, |worker| worker.pending.fetch_sub(1, Ordering::SeqCst));
    apply(hwnd, done);
}

fn apply(hwnd: HWND, mut done: CopyDone) {
    let mut notices = std::mem::take(&mut done.notices);
    // The dirty tab's drop is its one item: the notice goes with a copy that worked.
    if let Some(name) = done.dirty_tab.take()
        && done.items.iter().all(|item| item.copied.error.is_none())
    {
        notices.push(tree_copy::dirty_notice(&name));
    }
    let mut hidden = Vec::new();
    let mut rescan = false;
    let mut listed = Vec::new();
    for item in &done.items {
        let name = tree_copy::item_name(&item.destination);
        if let Some((_, error)) = &item.copied.error {
            let before = item.is_folder.then_some(item.copied.files);
            notices.push(tree_copy::failed_notice(&name, &error.to_string(), before));
            rescan = true;
            continue;
        }
        let relative = library::record_path(&done.root, &item.destination);
        if item.is_folder {
            rescan = true;
            listed.push(RowKind::Folder(relative));
            continue;
        }
        let is_note = item.destination.extension().is_some_and(|extension| {
            library::title::is_note_extension(&extension.to_string_lossy())
        });
        if is_note {
            listed.push(RowKind::Note(relative));
        } else {
            hidden.push(name);
        }
    }
    if !hidden.is_empty() {
        notices.push(tree_copy::hidden_notice(&hidden));
    }
    // The tree is left alone once another notebook is open: its rows are not these.
    if library_host::folder(hwnd).is_some_and(|open| same_path(&open, &done.root)) {
        show_in_tree(hwnd, &done, &listed, rescan);
    }
    // A reload swaps documents: inside a modal loop or a file population the tab is left as it
    // is, and its disk stamp pauses its autosave as for any change made outside FastPad.
    let busy = super::main_window::file_population_active(hwnd) || super::modal::modal_active(hwnd);
    if !busy {
        for reload in done.reloads {
            if let Some(loaded) = reload.loaded {
                super::main_window::reload_clean_document(
                    hwnd,
                    reload.id,
                    &reload.path,
                    reload.mark,
                    &loaded,
                    reload.stamp,
                );
            }
        }
    }
    for notice in notices {
        push_notice(hwnd, notice);
    }
}

/// Indexes the copied notes and folders, expands the folder copied into, and selects the row of
/// a drop's single item, as after a tree drag. Folders and failures are rescanned for what is in
/// them.
fn show_in_tree(hwnd: HWND, done: &CopyDone, listed: &[RowKind], rescan: bool) {
    library_host::with_state(hwnd, |state| {
        for row in listed {
            match row {
                RowKind::Note(relative) => {
                    state.add_note(&done.root.join(relative));
                }
                RowKind::Folder(relative) => state.add_folder(relative),
                _ => {}
            }
        }
    });
    library_host::set_root_expanded(hwnd, true);
    let mut folders = tree::ancestors(&done.folder);
    if !done.folder.as_os_str().is_empty() {
        folders.push(done.folder.clone());
    }
    for folder in folders {
        library_host::set_expanded(hwnd, &folder, true);
    }
    library_host::schedule_write(hwnd);
    if rescan {
        library_host::request_rescan(hwnd);
    }
    let single = match (done.items.as_slice(), listed) {
        ([_], [row]) => Some(row),
        _ => None,
    };
    super::side_panel::with_accessible_events(hwnd, || {
        super::side_panel::refresh(hwnd);
        if let Some(row) = single {
            super::notebook_view::select_row(hwnd, row);
        }
    });
}

/// Pumps the window's messages until every queued copy has come back and been applied.
#[cfg(test)]
pub(crate) fn wait_for_copies(hwnd: HWND) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let pending =
            library_host::with_copy_worker(hwnd, |worker| worker.pending.load(Ordering::SeqCst))
                .unwrap_or(0);
        if pending == 0 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the copy never came back"
        );
        super::main_window::pump_posted_messages(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_worker_waits_for_the_file_in_hand_and_starts_no_other() {
        // Break caught: the window closing while a copy runs, and the process exiting partway
        // through a file, which is left in the notebook half written (open editors spec §5).
        let dir = std::env::temp_dir().join(format!("fastpad-copy-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (from, to) = (dir.join("from"), dir.join("to"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        let block = vec![7u8; 8 * 1024 * 1024];
        let names: Vec<String> = (0..20).map(|index| format!("f{index:02}.bin")).collect();
        for name in &names {
            std::fs::write(from.join(name), &block).unwrap();
        }
        let mut worker = CopyWorker::default();
        worker.send(CopyJob {
            target: 0,
            items: names
                .iter()
                .map(|name| (from.join(name), to.join(name)))
                .collect(),
            reloads: Vec::new(),
            notices: Vec::new(),
            dirty_tab: None,
            root: dir.clone(),
            folder: PathBuf::new(),
        });
        let started = std::time::Instant::now();
        while !to.join(&names[0]).exists() {
            assert!(started.elapsed().as_secs() < 10, "the copy never started");
            std::thread::yield_now();
        }
        let cancel = Arc::clone(&worker.cancel);

        drop(worker);
        assert_eq!(
            Arc::strong_count(&cancel),
            1,
            "Drop returned before the worker thread ended"
        );
        let copied: Vec<_> = std::fs::read_dir(&to)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        assert!(
            !copied.is_empty() && copied.len() < names.len(),
            "{}",
            copied.len()
        );
        for entry in copied {
            // Compared whole: a copy in progress may already have its final size, filled later.
            assert!(
                std::fs::read(entry.path()).unwrap() == block,
                "{:?} is half written",
                entry.file_name()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
