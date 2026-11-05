//! Nested modal loops (common file dialogs, the themed prompt) re-enter the window procedure. While one
//! runs, deferred startup units, the IPC drain, and recovery snapshots are held back so they cannot
//! change which document the modal operation ends up acting on.

use super::main_window::{app_ptr, current_palette, window_identity};
use super::prompt;
use crate::app::WindowIdentity;
use crate::document::CloseDecision;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Raises `App::modal_depth` for its lifetime; leaving the outermost scope re-posts held messages.
pub(super) struct ModalScope {
    hwnd: HWND,
    identity: Option<WindowIdentity>,
}

impl ModalScope {
    pub(super) fn enter(hwnd: HWND) -> Self {
        let identity = unsafe { window_identity(hwnd) };
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            app.modal_depth = app.modal_depth.saturating_add(1);
        }
        Self { hwnd, identity }
    }
}

impl Drop for ModalScope {
    fn drop(&mut self) {
        if !self
            .identity
            .as_ref()
            .is_some_and(|identity| identity.is_live_for(self.hwnd))
        {
            return;
        }
        let held = unsafe { app_ptr(self.hwnd) }
            .map(|mut app| {
                let app = unsafe { app.as_mut() };
                app.modal_depth = app.modal_depth.saturating_sub(1);
                if app.modal_depth == 0 {
                    std::mem::take(&mut app.held_messages)
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        for message in held {
            unsafe {
                PostMessageW(self.hwnd, message, 0, 0);
            }
        }
    }
}

pub(super) fn modal_active(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.modal_depth > 0)
}

/// Returns true when `message` was held for re-posting after the outermost modal loop ends.
pub(super) fn hold_while_modal(hwnd: HWND, message: u32) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.modal_depth == 0 {
            return false;
        }
        if !app.held_messages.contains(&message) {
            app.held_messages.push(message);
        }
        true
    })
}

/// Message, button labels (primary first, Cancel last) and quick keys of the close prompt.
fn close_spec(title: &str) -> (String, [&'static str; 3], [(u16, usize); 2]) {
    (
        format!("Save changes to {title}?"),
        ["Save", "Don't save", "Cancel"],
        [(u16::from(b'S'), 0), (u16::from(b'D'), 1)],
    )
}

/// Button labels of a confirmation: the action, then Cancel.
fn confirm_labels(action: &str) -> [String; 2] {
    [action.to_owned(), "Cancel".to_owned()]
}

/// The only modal prompt FastPad shows outside startup-fatal errors.
pub(super) fn prompt_close_decision(hwnd: HWND, title: &str) -> CloseDecision {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = CLOSE_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd);
    }
    let (message, buttons, quick) = close_spec(title);
    let spec = prompt::Spec {
        message: &message,
        buttons: &buttons,
        quick_keys: &quick,
    };
    match prompt::show(hwnd, current_palette(hwnd), &spec) {
        0 => CloseDecision::Save,
        1 => CloseDecision::Discard,
        _ => CloseDecision::Cancel,
    }
}

pub(super) fn choose_save_path(
    hwnd: HWND,
    suggested_name: &str,
    folder: Option<&std::path::Path>,
) -> crate::Result<Option<PathBuf>> {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    LAST_SAVE_REQUEST.with(|last| {
        *last.borrow_mut() = Some((suggested_name.to_owned(), folder.map(|f| f.to_path_buf())))
    });
    #[cfg(test)]
    if let Some(answer) = SAVE_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return Ok(answer(hwnd));
    }
    #[cfg(test)]
    real_dialog_or_panic("choose_save_path", "answer_next_save_dialog");
    crate::window::commands::choose_save_path(hwnd, suggested_name, folder)
}

pub(super) fn choose_open_path(hwnd: HWND) -> crate::Result<Option<PathBuf>> {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = OPEN_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return Ok(answer(hwnd));
    }
    #[cfg(test)]
    real_dialog_or_panic("choose_open_path", "answer_next_open_dialog");
    crate::window::commands::choose_open_path(hwnd)
}

pub(crate) fn choose_folder(hwnd: HWND) -> crate::Result<Option<PathBuf>> {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = FOLDER_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return Ok(answer(hwnd));
    }
    #[cfg(test)]
    real_dialog_or_panic("choose_folder", "answer_next_folder_dialog");
    crate::window::commands::choose_folder_path(hwnd)
}

#[cfg(test)]
thread_local! {
    static ALLOW_REAL_DIALOGS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Lets this thread's modal choosers open the real Windows dialogs again until the guard drops.
/// Only for tests that drive the native dialog through automation.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "used by the in_process open_file test, not by the lib window tests"
)]
pub(crate) fn allow_real_dialogs() -> RealDialogsGuard {
    ALLOW_REAL_DIALOGS.with(|allow| allow.set(true));
    RealDialogsGuard
}

#[cfg(test)]
pub(crate) struct RealDialogsGuard;

#[cfg(test)]
impl Drop for RealDialogsGuard {
    fn drop(&mut self) {
        ALLOW_REAL_DIALOGS.with(|allow| allow.set(false));
    }
}

/// A real file or folder dialog blocks forever in an unattended test run, so tests must queue an
/// answer instead of falling through to one.
#[cfg(test)]
fn real_dialog_or_panic(function: &str, seam: &str) {
    if ALLOW_REAL_DIALOGS.with(std::cell::Cell::get) {
        return;
    }
    panic!("{function} reached the real dialog in a test; queue an answer with {seam} first")
}

/// Themed two-button confirmation: `action` is the primary button, Cancel the other. Returns
/// whether the user chose `action`.
pub(crate) fn confirm(hwnd: HWND, text: &str, action: &str) -> bool {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    {
        LAST_CONFIRM.with(|last| *last.borrow_mut() = Some(text.to_owned()));
        LAST_CONFIRM_ACTION.with(|last| *last.borrow_mut() = Some(action.to_owned()));
    }
    #[cfg(test)]
    if let Some(answer) = CONFIRM_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd);
    }
    let [primary, cancel] = confirm_labels(action);
    let buttons = [primary.as_str(), cancel.as_str()];
    let spec = prompt::Spec {
        message: text,
        buttons: &buttons,
        quick_keys: &[],
    };
    prompt::show(hwnd, current_palette(hwnd), &spec) == 0
}

/// Themed prompt with `buttons`, the first the primary one and the last what Esc picks. Returns
/// the index of the button chosen.
pub(crate) fn choose(hwnd: HWND, text: &str, buttons: &[&str]) -> usize {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    {
        LAST_CHOICE.with(|last| {
            *last.borrow_mut() = Some((
                text.to_owned(),
                buttons.iter().map(|button| (*button).to_owned()).collect(),
            ))
        });
        if let Some(answer) = CHOICE_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
            return answer(hwnd);
        }
    }
    let spec = prompt::Spec {
        message: text,
        buttons,
        quick_keys: &[],
    };
    prompt::show(hwnd, current_palette(hwnd), &spec)
}

/// The question and buttons the last `choose` showed.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn take_last_choice() -> Option<(String, Vec<String>)> {
    LAST_CHOICE.with(|last| last.borrow_mut().take())
}

/// Answers the next `choose` with a button index.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn answer_next_choice(answer: impl FnOnce(HWND) -> usize + 'static) {
    CHOICE_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

#[cfg(test)]
thread_local! {
    static CHOICE_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<usize>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static LAST_CHOICE: std::cell::RefCell<Option<(String, Vec<String>)>> =
        const { std::cell::RefCell::new(None) };
}
/// The New notebook dialog, starting in `folder`: the path of the folder to create, or `None`
/// for Cancel.
pub(crate) fn choose_new_notebook_path(
    hwnd: HWND,
    folder: Option<&std::path::Path>,
) -> crate::Result<Option<PathBuf>> {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    LAST_NEW_NOTEBOOK_START.with(|last| *last.borrow_mut() = folder.map(|f| f.to_path_buf()));
    #[cfg(test)]
    if let Some(answer) = NEW_NOTEBOOK_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return Ok(answer(hwnd));
    }
    crate::window::commands::choose_new_notebook_path(hwnd, folder)
}

#[cfg(test)]
type Answer<T> = Box<dyn FnOnce(HWND) -> T>;

#[cfg(test)]
thread_local! {
    static OPEN_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<Option<PathBuf>>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static CLOSE_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<CloseDecision>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static SAVE_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<Option<PathBuf>>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static FOLDER_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<Option<PathBuf>>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static CONFIRM_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<bool>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static NEW_NOTEBOOK_ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer<Option<PathBuf>>>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    static LAST_NEW_NOTEBOOK_START: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
    static LAST_SAVE_REQUEST: std::cell::RefCell<Option<(String, Option<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
    static LAST_CONFIRM: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static LAST_CONFIRM_ACTION: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// The question the last confirm asked.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn take_last_confirm() -> Option<String> {
    LAST_CONFIRM.with(|last| last.borrow_mut().take())
}

/// The primary-button label the last confirm was shown with.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn take_last_confirm_action() -> Option<String> {
    LAST_CONFIRM_ACTION.with(|last| last.borrow_mut().take())
}

/// The suggested name and starting folder the last Save As dialog was opened with.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn take_last_save_request() -> Option<(String, Option<PathBuf>)> {
    LAST_SAVE_REQUEST.with(|last| last.borrow_mut().take())
}

/// Answers the next close prompt from inside its modal scope instead of showing the themed prompt.
#[cfg(test)]
pub(crate) fn answer_next_close_prompt(answer: impl FnOnce(HWND) -> CloseDecision + 'static) {
    CLOSE_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Answers the next Save As dialog from inside its modal scope; `None` is Cancel.
#[cfg(test)]
pub(crate) fn answer_next_save_dialog(answer: impl FnOnce(HWND) -> Option<PathBuf> + 'static) {
    SAVE_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Answers the next Open dialog from inside its modal scope; `None` is Cancel.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn answer_next_open_dialog(answer: impl FnOnce(HWND) -> Option<PathBuf> + 'static) {
    OPEN_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Answers the next folder picker from inside its modal scope; `None` is Cancel.
#[cfg(test)]
pub(crate) fn answer_next_folder_dialog(answer: impl FnOnce(HWND) -> Option<PathBuf> + 'static) {
    FOLDER_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Answers the next New notebook dialog from inside its modal scope; `None` is Cancel.
#[cfg(test)]
pub(crate) fn answer_next_new_notebook_dialog(
    answer: impl FnOnce(HWND) -> Option<PathBuf> + 'static,
) {
    NEW_NOTEBOOK_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// The folder the last New notebook dialog started in.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn take_last_new_notebook_start() -> Option<PathBuf> {
    LAST_NEW_NOTEBOOK_START.with(|last| last.borrow_mut().take())
}

/// Answers the next OK/Cancel confirmation from inside its modal scope.
#[cfg(test)]
pub(crate) fn answer_next_confirm(answer: impl FnOnce(HWND) -> bool + 'static) {
    CONFIRM_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_spec_offers_save_dont_save_cancel() {
        let (message, buttons, quick) = close_spec("notes.txt");
        assert_eq!(message, "Save changes to notes.txt?");
        assert_eq!(buttons, ["Save", "Don't save", "Cancel"]);
        assert_eq!(quick, [(u16::from(b'S'), 0), (u16::from(b'D'), 1)]);
    }

    #[test]
    fn confirm_labels_name_the_action_then_cancel() {
        assert_eq!(confirm_labels("Replace"), ["Replace", "Cancel"]);
        assert_eq!(confirm_labels("Delete"), ["Delete", "Cancel"]);
    }

    // The tests below queue no CLOSE_ANSWERS or CONFIRM_ANSWERS, so the real themed prompt opens
    // and `prompt::answer_next` posts its input.

    use crate::platform::wide_null;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, IsWindow, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage,
        WM_KEYDOWN, WM_QUIT, WS_OVERLAPPEDWINDOW,
    };

    /// A plain top-level window (not a main window) to own the prompt; the caller destroys it.
    fn owner() -> HWND {
        let class = wide_null("STATIC");
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                800,
                600,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null(), "the test owner window");
        hwnd
    }

    fn key(dialog: HWND, key: u16) {
        unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(key), 0) };
    }

    /// The close prompt's decision when `answer` drives the real prompt.
    fn close_decision(answer: impl FnOnce(HWND) + 'static) -> CloseDecision {
        let hwnd = owner();
        prompt::answer_next(answer);
        let decision = prompt_close_decision(hwnd, "notes.txt");
        if unsafe { IsWindow(hwnd) } != 0 {
            unsafe { DestroyWindow(hwnd) };
        }
        decision
    }

    fn confirmed(answer: impl FnOnce(HWND) + 'static) -> bool {
        let hwnd = owner();
        prompt::answer_next(answer);
        let confirmed = confirm(hwnd, "Delete it?", "Delete");
        unsafe { DestroyWindow(hwnd) };
        confirmed
    }

    #[test]
    fn the_close_prompt_maps_its_buttons_to_decisions() {
        // Break caught: the prompt's button indexes mapped to the wrong decision (Discard for
        // the primary, or anything but Cancel for Esc).
        assert_eq!(
            close_decision(|dialog| key(dialog, VK_ESCAPE)),
            CloseDecision::Cancel
        );
        assert_eq!(
            close_decision(|dialog| key(dialog, u16::from(b'D'))),
            CloseDecision::Discard
        );
        assert_eq!(
            close_decision(|dialog| key(dialog, VK_RETURN)),
            CloseDecision::Save
        );
    }

    #[test]
    fn confirm_is_true_only_for_the_action() {
        // Break caught: Esc confirming a Delete or Replace, or Enter not confirming it.
        assert!(!confirmed(|dialog| key(dialog, VK_ESCAPE)));
        assert!(confirmed(|dialog| key(dialog, VK_RETURN)));
    }

    #[test]
    fn a_destroyed_owner_cancels_the_close_prompt() {
        // Break caught: a prompt torn down with its owner answering Save or Discard.
        assert_eq!(
            close_decision(|dialog| {
                let owner = unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::GetWindow(
                        dialog,
                        windows_sys::Win32::UI::WindowsAndMessaging::GW_OWNER,
                    )
                };
                unsafe { DestroyWindow(owner) };
            }),
            CloseDecision::Cancel
        );
    }

    #[test]
    fn a_quit_cancels_the_close_prompt_and_is_passed_on() {
        // Break caught: WM_QUIT answering Save or Discard, or being swallowed by the prompt's
        // loop instead of reaching the outer one.
        let decision = close_decision(|_| unsafe { PostQuitMessage(0) });
        // Drained before any assert, so a failure cannot leak WM_QUIT into other tests.
        let mut message = MSG::default();
        let quit = unsafe {
            PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                WM_QUIT,
                WM_QUIT,
                PM_REMOVE,
            )
        };
        assert_eq!(decision, CloseDecision::Cancel);
        assert_ne!(quit, 0, "WM_QUIT re-posted for the outer loop");
        assert_eq!(message.message, WM_QUIT);
    }

    #[test]
    #[should_panic(expected = "choose_folder reached the real dialog in a test")]
    fn choosing_a_folder_with_no_queued_answer_panics_instead_of_blocking() {
        // Break caught: a test with no queued FOLDER_ANSWERS falling through to the real
        // IFileOpenDialog, which hangs the run forever.
        let _ = choose_folder(std::ptr::null_mut());
    }
}
