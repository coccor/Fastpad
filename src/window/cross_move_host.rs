//! Moving notes and folders between open notebooks (multi-root move spec §2.2): the checks, the
//! dirty tabs saved first, the plan and its question, then the move itself (one rename on one
//! drive, `move_file` for a note going to another drive) and the hand-off that updates both
//! notebooks, the tabs, the pins, the expanded folders and the tree.

use super::library_host::{self, NotebookId};
use super::main_window::push_notice;
use super::tree_move::{self, MoveError};
use super::{notebook_view, side_panel};
use crate::library::cross_move::{self, Answer, Item, MovePlan, Side};
use crate::library::tree::{self, RowKind};
use crate::library::{self, capture, capture_file};
use crate::platform::files;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::HWND;

/// Where a move goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Destination {
    /// A folder (relative; empty is the root) of an open notebook.
    Open { id: NotebookId, folder: PathBuf },
    /// The root of a notebook that isn't open.
    #[allow(dead_code, reason = "Move to notebook… sends it from a later commit")]
    Closed { root: PathBuf },
}

/// A folder on its way to another drive (spec §2.3, §3.3).
pub(crate) struct Moving {
    generation: u64,
    started: Started,
    plan: MovePlan,
    /// Files copied of the total, as the worker last said.
    progress: (usize, usize),
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Moving {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Moving")
            .field("generation", &self.generation)
            .field("item", &self.started.item)
            .field("progress", &self.progress)
            .finish_non_exhaustive()
    }
}

/// What a folder move to another drive needs when it lands.
#[derive(Clone, Debug)]
struct Started {
    from: NotebookId,
    dest: NotebookId,
    source_root: PathBuf,
    dest_root: PathBuf,
    dest_label: String,
    item: Item,
    landing: PathBuf,
    with_images: bool,
}

impl Started {
    fn source(&self) -> PathBuf {
        self.source_root.join(self.item.path())
    }

    fn destination(&self) -> PathBuf {
        self.dest_root.join(&self.landing)
    }

    fn name(&self) -> String {
        name_of(self.item.path())
    }
}

fn with_moving<R>(hwnd: HWND, f: impl FnOnce(&Moving) -> R) -> Option<R> {
    library_host::with_moving(hwnd, |moving, _| moving.as_ref().map(f)).flatten()
}

/// The name of what the notebook `id` is moving to another drive, while it does.
pub(crate) fn busy(hwnd: HWND, id: NotebookId) -> Option<String> {
    with_moving(hwnd, |moving| {
        (moving.started.from == id).then(|| moving.started.name())
    })
    .flatten()
}

/// Whether `path` (absolute) is inside, or is, the folder moving to another drive: its tabs'
/// writes wait, and it can't be renamed, deleted, dragged or moved meanwhile (spec §3.3).
pub(crate) fn holds(hwnd: HWND, path: &Path) -> bool {
    with_moving(hwnd, |moving| {
        let source = moving.started.source();
        library::model::same_path(&source, path) || library::is_inside(&source, path)
    })
    .unwrap_or(false)
}

/// `holds`, saying so: "Wait for the move of *work* to finish." True when it refused.
pub(crate) fn refuse_if_moving(hwnd: HWND, path: &Path) -> bool {
    if !holds(hwnd, path) {
        return false;
    }
    if let Some(name) = with_moving(hwnd, |moving| moving.started.name()) {
        push_notice(hwnd, format!("Wait for the move of {name} to finish."));
    }
    true
}

/// The folder (relative) of the notebook `id` moving to another drive and its progress row's
/// text: "Moving to *B*… *k* of *n* — Cancel" (spec §3.3).
pub(crate) fn moving_row(hwnd: HWND, id: NotebookId) -> Option<(PathBuf, String)> {
    with_moving(hwnd, |moving| {
        (moving.started.from == id).then(|| {
            let (copied, total) = moving.progress;
            (
                moving.started.item.path().to_path_buf(),
                format!(
                    "Moving to {}… {copied} of {total} — Cancel",
                    library_host::notebook_name(&moving.started.dest_root)
                ),
            )
        })
    })
    .flatten()
}

/// The running move's files copied and total.
#[cfg(test)]
pub(crate) fn moving_progress(hwnd: HWND) -> Option<(usize, usize)> {
    with_moving(hwnd, |moving| moving.progress)
}

/// Pumps the window's messages until no folder move is running.
#[cfg(test)]
pub(crate) fn wait_for_moves(hwnd: HWND) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while with_moving(hwnd, |_| ()).is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "the move never came back"
        );
        super::main_window::pump_posted_messages(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Cancels the folder moving out of the notebook `from`: the worker removes its partial copy.
pub(crate) fn cancel_move(hwnd: HWND, from: NotebookId) {
    with_moving(hwnd, |moving| {
        if moving.started.from == from {
            moving.cancel.store(true, Ordering::SeqCst);
        }
    });
}

/// Whether the window (`closing` `None`) or the notebook `closing` may close with a folder
/// moving to another drive (spec §3.3): a move out of it, or any move when the window closes,
/// asks "A move to *B* is still running. Cancel it and close?" and is cancelled on yes; a move
/// into it is cancelled.
pub(crate) fn may_close(hwnd: HWND, closing: Option<NotebookId>) -> bool {
    let Some((from, dest, dest_root)) = with_moving(hwnd, |moving| {
        (
            moving.started.from,
            moving.started.dest,
            moving.started.dest_root.clone(),
        )
    }) else {
        return true;
    };
    let cancel = |hwnd: HWND| {
        with_moving(hwnd, |moving| moving.cancel.store(true, Ordering::SeqCst));
    };
    if closing == Some(dest) {
        cancel(hwnd);
        return true;
    }
    if closing.is_some_and(|id| id != from) {
        return true;
    }
    let question = format!(
        "A move to {} is still running. Cancel it and close?",
        library_host::notebook_name(&dest_root)
    );
    let Some(identity) = (unsafe { super::main_window::window_identity(hwnd) }) else {
        return false;
    };
    let chosen =
        crate::window::modal::choose(hwnd, &question, &["Cancel move and close", "Keep running"]);
    if !identity.is_live_for(hwnd) || chosen != 0 {
        return false;
    }
    cancel(hwnd);
    true
}

/// `WM_DESTROY`: a running move is cancelled and waited for; its partial copy goes.
pub(crate) fn stop(hwnd: HWND) {
    let moving = library_host::with_moving(hwnd, |moving, _| moving.take()).flatten();
    if let Some(mut moving) = moving {
        moving.cancel.store(true, Ordering::SeqCst);
        if let Some(thread) = moving.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts the worker for a folder going to another drive, unless one is already moving.
fn start_folder_move(hwnd: HWND, started: Started, plan: MovePlan) {
    if let Some(name) = with_moving(hwnd, |moving| moving.started.name()) {
        push_notice(hwnd, format!("Wait for the move of {name} to finish."));
        return;
    }
    let (source, destination) = (started.source(), started.destination());
    library_host::begin_own_writes(hwnd, [source.as_path(), destination.as_path()]);
    let cancel = Arc::new(AtomicBool::new(false));
    library_host::with_moving(hwnd, |moving, generation| {
        *generation += 1;
        let job = super::move_worker::Job {
            target: hwnd as isize,
            generation: *generation,
            source,
            destination,
            cancel: Arc::clone(&cancel),
        };
        *moving = Some(Moving {
            generation: *generation,
            started,
            plan,
            progress: (0, 0),
            cancel,
            thread: Some(super::move_worker::spawn(job)),
        });
    });
    side_panel::refresh(hwnd);
}

/// `WM_FASTPAD_MOVE_PROGRESS`: the progress row follows.
pub(crate) fn move_progress(hwnd: HWND, wparam: usize, lparam: isize) {
    let updated = library_host::with_moving(hwnd, |moving, _| {
        let moving = moving
            .as_mut()
            .filter(|moving| moving.generation == wparam as u64)?;
        moving.progress = super::move_worker::unpack(lparam);
        Some(())
    })
    .flatten();
    if updated.is_some() {
        notebook_view::rebuild(hwnd);
    }
}

/// `WM_FASTPAD_MOVE_DONE`: frees the outcome and applies it (spec §3.3).
pub(crate) fn move_done(hwnd: HWND, lparam: isize) {
    if lparam == 0 {
        return;
    }
    let done = *unsafe { Box::from_raw(lparam as *mut super::move_worker::Done) };
    let moving = library_host::with_moving(hwnd, |moving, _| {
        if moving
            .as_ref()
            .is_some_and(|moving| moving.generation == done.generation)
        {
            moving.take()
        } else {
            None
        }
    })
    .flatten();
    let Some(mut moving) = moving else {
        return;
    };
    if let Some(thread) = moving.thread.take() {
        let _ = thread.join();
    }
    let started = moving.started.clone();
    let (source, destination) = (started.source(), started.destination());
    library_host::end_own_writes(hwnd, [source.as_path(), destination.as_path()]);
    let name = started.name();
    let open = |id| library_host::notebook_folder(hwnd, id).is_some();
    match done.outcome {
        super::move_worker::Outcome::Copied { .. }
            if !open(started.from) || !open(started.dest) =>
        {
            // A notebook closed after the check: the move is dropped, and the copy with it, so
            // the original is the only one.
            let _ = std::fs::remove_dir_all(&destination);
        }
        super::move_worker::Outcome::Copied { .. } => {
            land(hwnd, &started, &moving.plan);
            side_panel::refresh(hwnd);
            return;
        }
        super::move_worker::Outcome::Cancelled => {
            push_notice(
                hwnd,
                format!("Move of {name} cancelled. Nothing was moved."),
            );
        }
        super::move_worker::Outcome::Failed(error) => {
            push_notice(
                hwnd,
                format!("Couldn't move {name}: {error}. Nothing was moved."),
            );
        }
        super::move_worker::Outcome::Changed => {
            push_notice(
                hwnd,
                format!("{name} changed while it was being moved. Nothing was moved."),
            );
        }
    }
    // Writes held during the move go where the files still are.
    save_held(hwnd, &source);
    side_panel::refresh(hwnd);
}

/// A checked copy landed: the original goes to the Recycle Bin, then the tabs, the libraries
/// and the tree follow, as for a move on one drive. An original that can't be recycled stays,
/// and both notebooks list what they have.
fn land(hwnd: HWND, started: &Started, plan: &MovePlan) {
    let (source, destination) = (started.source(), started.destination());
    let name = started.name();
    if !super::copy_host::recycle_path(hwnd, &source) {
        let folders = folders_on_disk(&started.dest_root, &started.landing);
        let notes: Vec<PathBuf> = plan.notes.iter().map(|(_, new)| new.clone()).collect();
        library_host::with_notebook(hwnd, started.dest, |state| {
            state.arrive(&folders, &notes, &[]);
        });
        library_host::request_index_of(hwnd, started.dest);
        push_notice(
            hwnd,
            format!(
                "{name} was copied to {}, but the original in {} couldn't be removed.",
                library_host::notebook_name(&started.dest_root),
                library_host::notebook_name(&started.source_root),
            ),
        );
        save_held(hwnd, &source);
        return;
    }
    for (id, path, _) in library_host::tabs_under(hwnd, &source) {
        let target = destination.join(library::record_path(&source, &path));
        if let Some(mut app) = unsafe { super::main_window::app_ptr(hwnd) } {
            let _ = unsafe { app.as_mut() }.tabs.rebind_path(id, target);
        }
    }
    library_host::reroot_save_folders(hwnd, &source, &destination);
    hand_off(
        hwnd,
        started.from,
        Some(started.dest),
        &started.dest_root,
        plan,
        &started.item,
        &started.landing,
    );
    let done = [(started.item.clone(), started.landing.clone())];
    // The images stayed in the source: they come along now, as on one drive.
    let failed = if started.with_images {
        Facts::of(hwnd, started.from, &started.source_root).map_or(0, |source_facts| {
            bring_images(
                hwnd,
                started.dest,
                &started.source_root,
                &started.dest_root,
                &source_facts,
                plan,
                &done,
            )
        })
    } else {
        0
    };
    finish(
        hwnd,
        Some(started.dest),
        &started.dest_label,
        plan,
        &done,
        failed,
    );
    save_held(hwnd, &destination);
}

/// Saves the dirty tabs under `folder` (absolute) whose writes waited for a move.
fn save_held(hwnd: HWND, folder: &Path) {
    for (_, path, dirty) in library_host::tabs_under(hwnd, folder) {
        if dirty {
            library_host::save_before_move(hwnd, &path);
        }
    }
}

/// One notebook's facts, owned, for `cross_move::plan`.
struct Facts {
    notes: Vec<PathBuf>,
    index: Option<std::sync::Arc<library::index::NoteIndex>>,
    resolver: library::links::Resolver,
    attachments: PathBuf,
}

impl Facts {
    fn of(hwnd: HWND, id: NotebookId, root: &Path) -> Option<Self> {
        let notes = library_host::with_notebook(hwnd, id, |state| {
            state.notes.iter().map(|note| note.path.clone()).collect()
        })?;
        let index =
            library_host::index_loaded_of(hwnd, id).then(|| library_host::note_index_of(hwnd, id));
        let resolver = library_host::resolver_of(hwnd, id)?;
        Some(Self {
            notes,
            index,
            resolver,
            attachments: attachments_folder(hwnd, id, root, Path::new("note.md")),
        })
    }

    fn side(&self) -> Side<'_> {
        Side {
            notes: &self.notes,
            index: self.index.as_deref(),
            resolver: &self.resolver,
            attachments: &self.attachments,
        }
    }
}

/// The attachments folder (relative) of the notebook `id` at `root`, for the note `note`
/// (relative): `notebook.ini`, then Obsidian's setting, then `attachments`.
pub(crate) fn attachments_folder(hwnd: HWND, id: NotebookId, root: &Path, note: &Path) -> PathBuf {
    let settings = library_host::notebook_settings_of(hwnd, id);
    let obsidian = capture_file::obsidian_attachments_of(root);
    capture::attachments_folder(
        settings.attachments_folder.as_deref(),
        obsidian.as_ref(),
        note,
    )
}

/// "*Notebook* › *folder*", or the notebook's name for its root.
fn place_label(root: &Path, folder: &Path) -> String {
    library::roots::notebook_prefixed(
        &library_host::notebook_name(root),
        &folder.to_string_lossy(),
        true,
    )
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Moves `items` (rows of the notebook `from`) to `dest` (multi-root move spec §2.2).
pub(crate) fn move_items(hwnd: HWND, from: NotebookId, items: Vec<RowKind>, dest: Destination) {
    let Some(source_root) = library_host::notebook_folder(hwnd, from) else {
        return;
    };
    let (dest_id, dest_root, dest_folder) = match dest {
        Destination::Open { id, folder } => {
            let Some(root) = library_host::notebook_folder(hwnd, id) else {
                return;
            };
            (Some(id), root, folder)
        }
        Destination::Closed { root } => (None, root, PathBuf::new()),
    };
    if !library_host::ready_library_of(hwnd, Some(from))
        || dest_id.is_some_and(|id| !library_host::ready_library_of(hwnd, Some(id)))
    {
        return;
    }
    if let Some(name) = busy(hwnd, from) {
        push_notice(hwnd, format!("Wait for the move of {name} to finish."));
        return;
    }
    let items: Vec<Item> = items
        .into_iter()
        .filter_map(|kind| match kind {
            RowKind::Note(path) => Some(Item::Note(path)),
            RowKind::Folder(path) if tree::is_plain_relative_folder(&path) => {
                Some(Item::Folder(path))
            }
            _ => None,
        })
        .collect();
    // Nothing goes into a folder that is itself moving to another drive.
    if refuse_if_moving(hwnd, &dest_root.join(&dest_folder)) {
        return;
    }
    if items.is_empty() {
        return;
    }
    // Unsaved edits go with the files (spec §2.2 step 2).
    for item in &items {
        let absolute = source_root.join(item.path());
        let notes: Vec<PathBuf> = match item {
            Item::Note(_) => vec![absolute],
            Item::Folder(_) => library_host::tabs_under(hwnd, &absolute)
                .into_iter()
                .map(|(_, path, _)| path)
                .collect(),
        };
        for note in notes {
            if !library_host::save_before_move(hwnd, &note) {
                return;
            }
        }
    }
    let Some(source) = Facts::of(hwnd, from, &source_root) else {
        return;
    };
    let dest_facts = dest_id.and_then(|id| Facts::of(hwnd, id, &dest_root));
    let dest_exists = |relative: &Path| dest_root.join(relative).exists();
    let source_exists = |relative: &Path| source_root.join(relative).is_file();
    let plan = cross_move::plan(
        &items,
        &dest_folder,
        &source.side(),
        dest_facts.as_ref().map(Facts::side).as_ref(),
        &dest_exists,
        &source_exists,
    );
    let dest_label = place_label(&dest_root, &dest_folder);
    if let Some(name) = plan.clashes.first() {
        push_notice(
            hwnd,
            format!("{name} already exists in {dest_label}. Nothing was moved."),
        );
        return;
    }
    let label = cross_move::item_label(&items);
    let source_name = library_host::notebook_name(&source_root);
    let dest_name = library_host::notebook_name(&dest_root);
    let answer = match cross_move::question(&plan, &label, &source_name, &dest_name) {
        None => Answer::Move,
        Some(question) => {
            let Some(identity) = (unsafe { super::main_window::window_identity(hwnd) }) else {
                return;
            };
            let buttons: Vec<&str> = question.buttons.iter().map(|(text, _)| *text).collect();
            let chosen = crate::window::modal::choose(hwnd, &question.text, &buttons);
            if !identity.is_live_for(hwnd) {
                return;
            }
            question
                .buttons
                .get(chosen)
                .map_or(Answer::Cancel, |(_, answer)| *answer)
        }
    };
    if answer == Answer::Cancel {
        return;
    }
    let same_drive = files::same_volume(&source_root, &dest_root);
    if !same_drive && items.iter().any(|item| matches!(item, Item::Folder(_))) {
        // A folder to another drive goes on the worker (spec §2.3), one at a time.
        if let (Some(dest_id), [(item @ Item::Folder(_), landing)]) = (dest_id, &plan.moves[..]) {
            let started = Started {
                from,
                dest: dest_id,
                source_root,
                dest_root,
                dest_label,
                item: item.clone(),
                landing: landing.clone(),
                with_images: answer == Answer::MoveWithImages,
            };
            start_folder_move(hwnd, started, plan);
        } else {
            push_notice(
                hwnd,
                format!(
                    "Couldn't move {label}: move one folder at a time to another drive, into an open notebook."
                ),
            );
        }
        return;
    }
    let paths: Vec<PathBuf> = plan
        .moves
        .iter()
        .flat_map(|(item, landing)| [source_root.join(item.path()), dest_root.join(landing)])
        .collect();
    library_host::begin_own_writes(hwnd, paths.iter().map(PathBuf::as_path));
    let mut done: Vec<(Item, PathBuf)> = Vec::new();
    for (item, landing) in &plan.moves {
        let name = name_of(item.path());
        let (old, new) = (source_root.join(item.path()), dest_root.join(landing));
        let is_folder = matches!(item, Item::Folder(_));
        let moved = if same_drive {
            tree_move::rename_item(hwnd, &old, &new, is_folder)
                .map(|moved| moved.stuck)
                .map_err(Some)
        } else {
            move_note_across(hwnd, &old, &new).map(|()| Vec::new())
        };
        match moved {
            Ok(stuck) => {
                if !stuck.is_empty() {
                    push_notice(
                        hwnd,
                        library_host::rename_undo_failed_notice(
                            &name,
                            &landing.to_string_lossy(),
                            &stuck,
                        ),
                    );
                }
                hand_off(hwnd, from, dest_id, &dest_root, &plan, item, landing);
                done.push((item.clone(), landing.clone()));
            }
            Err(Some(error)) => {
                report_failure(hwnd, from, &name, &dest_label, error);
                break;
            }
            // Copied, but the original stays: the source keeps it, the destination lists it.
            Err(None) => {
                if let Some(id) = dest_id {
                    library_host::with_notebook(hwnd, id, |state| state.add_note(&new));
                    library_host::request_index_of(hwnd, id);
                }
                break;
            }
        }
    }
    if done.is_empty() {
        side_panel::refresh(hwnd);
        return;
    }
    let failed = match dest_id {
        Some(id) if answer == Answer::MoveWithImages => {
            bring_images(hwnd, id, &source_root, &dest_root, &source, &plan, &done)
        }
        _ => 0,
    };
    finish(hwnd, dest_id, &dest_label, &plan, &done, failed);
}

/// A byte range of a note and its new text.
type Edit = (std::ops::Range<usize>, String);

/// The notes of `plan` that moved with `done`, before and after.
fn moved_notes<'a>(
    plan: &'a MovePlan,
    done: &'a [(Item, PathBuf)],
) -> impl Iterator<Item = &'a (PathBuf, PathBuf)> {
    plan.notes.iter().filter(move |(old, _)| {
        done.iter().any(|(item, _)| match item {
            Item::Note(path) => library::model::same_path(path, old),
            Item::Folder(folder) => library::is_inside(folder, old),
        })
    })
}

/// Move with images (spec §4.2): each image only moving notes use moves, any other is copied,
/// into the destination's attachments folder for the note, under a free name, and the moved
/// notes' links to it are rewritten. An image that fails keeps its links. Returns how many
/// failed.
fn bring_images(
    hwnd: HWND,
    dest_id: NotebookId,
    source_root: &Path,
    dest_root: &Path,
    source: &Facts,
    plan: &MovePlan,
    done: &[(Item, PathBuf)],
) -> usize {
    let notes: Vec<&(PathBuf, PathBuf)> = moved_notes(plan, done).collect();
    let exists = |relative: &Path| source_root.join(relative).is_file();
    // Each moved note's text now, read before any image moves: the links resolve against the
    // source as it still is.
    let texts: Vec<(&(PathBuf, PathBuf), String, Option<library::DiskStamp>)> = notes
        .iter()
        .filter_map(|pair| {
            let path = dest_root.join(&pair.1);
            let stamp = library::disk_stamp(&path);
            let text = crate::file::loader::load(&path).ok()?.text;
            Some((*pair, text, stamp))
        })
        .collect();
    let mut chosen: Vec<PathBuf> = Vec::new();
    let mut edits: Vec<(usize, Vec<Edit>)> = Vec::new();
    let mut failed = 0;
    for image in &plan.images {
        // Only images a moved note uses come along.
        let Some(first) = image.uses.iter().find_map(|usage| {
            notes
                .iter()
                .find(|(old, _)| library::model::same_path(old, &usage.note))
        }) else {
            continue;
        };
        let folder = attachments_folder(hwnd, dest_id, dest_root, &first.1);
        let taken = |relative: &Path| {
            dest_root.join(relative).exists()
                || chosen
                    .iter()
                    .any(|other| library::model::same_path(other, relative))
        };
        let landing = cross_move::image_destination(
            &folder,
            image.source.file_name().unwrap_or_default(),
            &taken,
        );
        chosen.push(landing.clone());
        let pair = [(image.source.clone(), landing.clone())];
        let image_edits: Vec<(usize, Vec<_>)> = texts
            .iter()
            .enumerate()
            .map(|(index, ((old, new), text, _))| {
                (
                    index,
                    cross_move::image_link_edits(
                        text,
                        old,
                        new,
                        &source.attachments,
                        &exists,
                        &pair,
                    ),
                )
            })
            .filter(|(_, edits)| !edits.is_empty())
            .collect();
        let (from, to) = (source_root.join(&image.source), dest_root.join(&landing));
        library_host::begin_own_writes(hwnd, [from.as_path(), to.as_path()]);
        let arrived = to
            .parent()
            .is_some_and(|parent| std::fs::create_dir_all(parent).is_ok())
            && if image.users_only_moving {
                files::move_file(&from, &to).is_ok()
            } else {
                files::copy_file_no_replace(&from, &to).is_ok()
            };
        if !arrived {
            failed += 1;
            continue;
        }
        edits.extend(image_edits);
    }
    let mut note_edits: Vec<library::links::NoteEdits> = Vec::new();
    for (index, ((_, new), _, stamp)) in texts.iter().enumerate() {
        let mut ranges: Vec<_> = edits
            .iter()
            .filter(|(at, _)| *at == index)
            .flat_map(|(_, edits)| edits.iter().cloned())
            .collect();
        if ranges.is_empty() {
            continue;
        }
        let Some(stamp) = stamp else {
            continue;
        };
        ranges.sort_by_key(|(range, _)| range.start);
        note_edits.push(library::links::NoteEdits {
            note: new.clone(),
            size: stamp.size,
            mtime: library::filetime_ticks(stamp.modified),
            own: true,
            edits: ranges,
        });
    }
    if !note_edits.is_empty() {
        library_host::write_link_edits_of(hwnd, dest_id, &note_edits);
    }
    failed
}

/// A note going to another drive: `move_file` copies then deletes. `Err(None)` when the copy
/// landed but the original couldn't be removed (said here), `Err(Some(error))` when nothing
/// moved.
fn move_note_across(hwnd: HWND, old: &Path, new: &Path) -> Result<(), Option<MoveError>> {
    files::move_file(old, new).map_err(|error| Some(MoveError::Failed(error)))?;
    if old.exists() {
        push_notice(
            hwnd,
            format!(
                "{} was copied to {}, but the original could not be removed.",
                library::title::note_title(old),
                library_host::notebook_name(new.parent().unwrap_or(Path::new(""))),
            ),
        );
        return Err(None);
    }
    match library_host::rebind_open_tab(hwnd, old, new.to_path_buf()) {
        Ok(_) => {}
        Err(()) => library_host::report_rebind_failure(hwnd, old, new),
    }
    Ok(())
}

fn report_failure(hwnd: HWND, from: NotebookId, name: &str, dest_label: &str, error: MoveError) {
    match error {
        MoveError::Taken => push_notice(
            hwnd,
            format!("{name} already exists in {dest_label}. Nothing was moved."),
        ),
        MoveError::Missing(_) => {
            push_notice(hwnd, format!("{name} no longer exists."));
            library_host::request_rescan_of(hwnd, from);
        }
        MoveError::TargetGone(error) | MoveError::Failed(error) => {
            push_notice(hwnd, format!("Couldn't move {name}: {error}"));
        }
        MoveError::TabCantFollow => push_notice(
            hwnd,
            format!("Couldn't move {name}: another tab already has that file open."),
        ),
    }
}

/// The folders (relative to `root`) at and under `folder` (relative) on disk, parents first.
fn folders_on_disk(root: &Path, folder: &Path) -> Vec<PathBuf> {
    let mut found = vec![folder.to_path_buf()];
    let mut index = 0;
    while index < found.len() {
        if let Ok(entries) = std::fs::read_dir(root.join(&found[index])) {
            let parent = found[index].clone();
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    found.push(parent.join(entry.file_name()));
                }
            }
        }
        index += 1;
    }
    found
}

/// One moved item's hand-off (spec §4.1): the source forgets it and what it took (records,
/// pins, expanded folders); an open destination lists it, pins it and opens the same folders;
/// tabs that followed stop being previews.
fn hand_off(
    hwnd: HWND,
    from: NotebookId,
    dest_id: Option<NotebookId>,
    dest_root: &Path,
    plan: &MovePlan,
    item: &Item,
    landing: &Path,
) {
    let is_folder = matches!(item, Item::Folder(_));
    let departure =
        library_host::with_notebook(hwnd, from, |state| state.depart(item.path(), is_folder))
            .unwrap_or_default();
    library_host::request_index_of(hwnd, from);
    library_host::save_local_soon_of(hwnd, from);
    library_host::schedule_write(hwnd);
    let rerooted = |path: &Path| -> PathBuf {
        if is_folder {
            library::reroot(path, item.path(), landing).unwrap_or_else(|| path.to_path_buf())
        } else {
            landing.to_path_buf()
        }
    };
    if let Some(id) = dest_id {
        let folders = if is_folder {
            folders_on_disk(dest_root, landing)
        } else {
            Vec::new()
        };
        let notes: Vec<PathBuf> = plan
            .notes
            .iter()
            .filter(|(old, _)| {
                if is_folder {
                    library::is_inside(item.path(), old)
                } else {
                    library::model::same_path(old, item.path())
                }
            })
            .map(|(_, new)| new.clone())
            .collect();
        let expanded: Vec<PathBuf> = departure
            .expanded
            .iter()
            .map(|path| rerooted(path))
            .collect();
        library_host::with_notebook(hwnd, id, |state| state.arrive(&folders, &notes, &expanded));
        for pinned in &departure.pinned {
            library_host::pin_in(hwnd, id, &dest_root.join(rerooted(pinned)));
        }
        library_host::request_index_of(hwnd, id);
        library_host::save_local_soon_of(hwnd, id);
    }
    let new = dest_root.join(landing);
    if is_folder {
        for (_, path, _) in library_host::tabs_under(hwnd, &new) {
            library_host::promote_tab_for(hwnd, &path);
        }
    } else {
        library_host::promote_tab_for(hwnd, &new);
    }
}

/// After the moves: the destination opens down to the last moved row and selects it, the tab
/// strip repaints and a notice says what moved.
fn finish(
    hwnd: HWND,
    dest_id: Option<NotebookId>,
    dest_label: &str,
    plan: &MovePlan,
    done: &[(Item, PathBuf)],
    failed_images: usize,
) {
    super::main_window::invalidate_title_strip(hwnd);
    let mut notice = match done {
        [(Item::Note(path), _)] => format!("Moved {} to {dest_label}.", name_of(path)),
        [(Item::Folder(path), _)] => {
            let count = plan
                .notes
                .iter()
                .filter(|(old, _)| library::is_inside(path, old))
                .count();
            let notes = if count == 1 {
                "1 note".to_owned()
            } else {
                format!("{count} notes")
            };
            format!("Moved {} ({notes}) to {dest_label}.", name_of(path))
        }
        _ => format!("Moved {} items to {dest_label}.", done.len()),
    };
    match failed_images {
        0 => {}
        1 => notice.push_str(" 1 image couldn't be brought along."),
        count => notice.push_str(&format!(" {count} images couldn't be brought along.")),
    }
    match (dest_id, done.last()) {
        (Some(id), Some((item, landing))) => {
            for ancestor in tree::ancestors(landing) {
                library_host::set_expanded_of(hwnd, id, &ancestor, true);
            }
            let row = match item {
                Item::Note(_) => RowKind::Note(landing.clone()),
                Item::Folder(_) => RowKind::Folder(landing.clone()),
            };
            side_panel::with_accessible_events(hwnd, || {
                side_panel::refresh(hwnd);
                notebook_view::select_row(hwnd, id, &row);
            });
        }
        _ => side_panel::refresh(hwnd),
    }
    push_notice(hwnd, notice);
}
