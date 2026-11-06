//! The worker that moves a folder to a notebook on another drive (multi-root move spec §2.3):
//! it copies the folder with progress, checks the copy against the original, and posts the
//! outcome. The original is recycled on the UI thread, once the copy proved good. A cancelled,
//! failed or mismatched copy is removed here, so the original is the only one left.

use crate::platform::files;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// How often progress is posted at most.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// One folder to move.
pub(crate) struct Job {
    pub(crate) target: isize,
    pub(crate) generation: u64,
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) cancel: Arc<AtomicBool>,
}

/// How a job ended.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    /// Every file copied and checked: the original may go, once it still holds exactly
    /// `before` (a file added since the copy read its folder is in neither).
    Copied {
        files: usize,
        before: files::Snapshot,
    },
    Cancelled,
    Failed(String),
    /// A file of the original changed while it was copied.
    Changed,
}

/// `WM_FASTPAD_MOVE_DONE`'s payload.
#[derive(Debug)]
pub(crate) struct Done {
    pub(crate) generation: u64,
    pub(crate) outcome: Outcome,
}

/// Packs `copied` of `total` files into a progress post's `lParam`.
pub(crate) fn pack(copied: usize, total: usize) -> isize {
    (((copied as u64) << 32) | (total as u64 & 0xffff_ffff)) as isize
}

/// `pack`'s inverse.
pub(crate) fn unpack(lparam: isize) -> (usize, usize) {
    let value = lparam as u64;
    ((value >> 32) as usize, (value & 0xffff_ffff) as usize)
}

static PAUSE_BEFORE_CHECK: AtomicBool = AtomicBool::new(false);
static FAIL_NEXT_COPY: AtomicBool = AtomicBool::new(false);

/// Holds every job after its copy, before its check, while `on`.
#[cfg(test)]
pub(crate) fn pause_before_check(on: bool) {
    PAUSE_BEFORE_CHECK.store(on, Ordering::SeqCst);
}

/// Makes the next job's copy fail, as a full drive would.
#[cfg(test)]
pub(crate) fn fail_next_copy() {
    FAIL_NEXT_COPY.store(true, Ordering::SeqCst);
}

/// Runs one job here, posting nowhere, for the worker's own tests.
#[cfg(test)]
pub(crate) fn run_for_test(source: &std::path::Path, destination: &std::path::Path) -> Outcome {
    run(&Job {
        target: 0,
        generation: 0,
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        cancel: Arc::new(AtomicBool::new(false)),
    })
}

/// Starts `job` on its own thread. The thread returns whether it posted a checked copy, so a
/// window that goes away before reading the post can remove that copy (final review 4).
pub(crate) fn spawn(job: Job) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || {
        let outcome = run(&job);
        let copied = matches!(outcome, Outcome::Copied { .. });
        post_done(job.target, job.generation, outcome);
        copied
    })
}

fn post_progress(target: isize, generation: u64, copied: usize, total: usize) {
    unsafe {
        PostMessageW(
            target as HWND,
            crate::window::WM_FASTPAD_MOVE_PROGRESS,
            generation as usize,
            pack(copied, total),
        );
    }
}

fn post_done(target: isize, generation: u64, outcome: Outcome) {
    let done = Box::into_raw(Box::new(Done {
        generation,
        outcome,
    }));
    let posted = unsafe {
        PostMessageW(
            target as HWND,
            crate::window::WM_FASTPAD_MOVE_DONE,
            0,
            done as isize,
        )
    } != 0;
    if !posted {
        drop(unsafe { Box::from_raw(done) });
    }
}

fn run(job: &Job) -> Outcome {
    let before = match files::snapshot(&job.source) {
        Ok(before) => before,
        Err(error) => return Outcome::Failed(error.to_string()),
    };
    let total = before.0.len();
    post_progress(job.target, job.generation, 0, total);
    if FAIL_NEXT_COPY.swap(false, Ordering::SeqCst) {
        return Outcome::Failed("the destination refused the copy".to_owned());
    }
    // Something already at the destination is never written into, and never removed.
    if job.destination.exists() {
        return Outcome::Failed(format!(
            "{} already exists there",
            job.destination.display()
        ));
    }
    let mut last = Instant::now();
    let copied = files::copy_tree_reporting(&job.source, &job.destination, &job.cancel, &mut |n| {
        if last.elapsed() >= PROGRESS_EVERY {
            last = Instant::now();
            post_progress(job.target, job.generation, n, total);
        }
    });
    // Only a folder this job created is ever removed (final review 1).
    let created = copied.created;
    let remove = || {
        if created {
            let _ = std::fs::remove_dir_all(&job.destination);
        }
    };
    if job.cancel.load(Ordering::SeqCst) {
        remove();
        return Outcome::Cancelled;
    }
    if let Some((_, error)) = copied.error {
        remove();
        return Outcome::Failed(error.to_string());
    }
    post_progress(job.target, job.generation, copied.files, total);
    while PAUSE_BEFORE_CHECK.load(Ordering::SeqCst) && !job.cancel.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(2));
    }
    if job.cancel.load(Ordering::SeqCst) {
        remove();
        return Outcome::Cancelled;
    }
    match files::verify_copy(&job.source, &job.destination, &before) {
        Ok(()) => Outcome::Copied {
            files: copied.files,
            before,
        },
        Err(files::Mismatch::Changed(_)) => {
            remove();
            Outcome::Changed
        }
        Err(files::Mismatch::Missing(path) | files::Mismatch::Size(path)) => {
            remove();
            Outcome::Failed(format!("{} didn't copy whole", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_packs_and_unpacks() {
        assert_eq!(unpack(pack(120, 340)), (120, 340));
        assert_eq!(unpack(pack(0, 0)), (0, 0));
    }
}
