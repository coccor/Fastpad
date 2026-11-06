//! The "+" at the right of the Notebook view's title band: a menu with Open notebook…, New
//! notebook…, the recent notebooks that aren't open (up to five), and More recent notebooks…
//! when there are more, so a user can see that notebooks can be added beside the open ones.

use super::*;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, RECT};

/// The button's tooltip and its accessible description.
pub(crate) const ADD_NOTEBOOK_TIP: &str = "Open or create a notebook";

/// One item of the menu.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AddChoice {
    Open,
    New,
    /// A separator line.
    Separator,
    /// A recent notebook that isn't open, by its folder.
    Recent(PathBuf),
    MoreRecent,
}

/// How many recent notebooks the menu lists before "More recent notebooks…".
const RECENT_SHOWN: usize = 5;

impl NotebookView {
    /// The title band's "+", in panel client coordinates.
    pub(crate) fn add_button(&self) -> RECT {
        crate::window::notebook_layout::add_notebook_button(self.client(), self.dpi())
    }
}

/// The button, in panel client coordinates.
pub(crate) fn add_button_rect(hwnd: HWND) -> Option<RECT> {
    with_view(hwnd, |view| view.add_button())
}

/// Whether panel point `x`, `y` is on the button: the main window's caption hit test leaves it
/// to the panel, so a click there opens the menu rather than dragging the window.
pub(crate) fn header_hit(hwnd: HWND, x: i32, y: i32) -> bool {
    add_button_rect(hwnd).is_some_and(|rect| contains(rect, x, y))
}

/// The menu's items: Open notebook…, New notebook…, then, after a separator, up to five recent
/// notebooks that aren't open (most recent first), and "More recent notebooks…" when more are.
pub(crate) fn add_items(recent: &[PathBuf], open: &[PathBuf]) -> Vec<AddChoice> {
    let closed: Vec<&PathBuf> = recent
        .iter()
        .filter(|folder| {
            !open
                .iter()
                .any(|open| crate::library::model::same_path(open, folder))
        })
        .collect();
    let mut items = vec![AddChoice::Open, AddChoice::New];
    if !closed.is_empty() {
        items.push(AddChoice::Separator);
        items.extend(
            closed
                .iter()
                .take(RECENT_SHOWN)
                .map(|folder| AddChoice::Recent((*folder).clone())),
        );
        if closed.len() > RECENT_SHOWN {
            items.push(AddChoice::MoreRecent);
        }
    }
    items
}

/// Each item's label: a recent notebook by its name, with its parent folder when two share it.
fn add_labels(items: &[AddChoice]) -> Vec<String> {
    let recent: Vec<PathBuf> = items
        .iter()
        .filter_map(|item| match item {
            AddChoice::Recent(folder) => Some(folder.clone()),
            _ => None,
        })
        .collect();
    let mut names = crate::library::local::display_names(&recent).into_iter();
    items
        .iter()
        .map(|item| match item {
            AddChoice::Open => "Open notebook…\tCtrl+Shift+O".to_owned(),
            AddChoice::New => "New notebook…".to_owned(),
            AddChoice::Separator => String::new(),
            AddChoice::Recent(_) => match names.next() {
                Some((name, Some(hint))) => format!("{name} ({hint})"),
                Some((name, None)) => name,
                None => String::new(),
            },
            AddChoice::MoreRecent => "More recent notebooks…".to_owned(),
        })
        .collect()
}

/// A click on the button (or a screen reader's activate, which clicks it): the menu, under
/// the button, and what was picked.
pub(crate) fn add_notebook_menu(hwnd: HWND) {
    let Some((panel, button)) = with_view(hwnd, |view| (view.panel, view.add_button())) else {
        return;
    };
    let open: Vec<PathBuf> = crate::window::library_host::notebooks(hwnd)
        .into_iter()
        .map(|(_, folder)| folder)
        .collect();
    let recent = crate::window::library_host::recent_notebooks_now(hwnd);
    let items = add_items(&recent, &open);
    let labels = add_labels(&items);
    // A modal loop, as every menu's: timers that read the tabs wait until it ends.
    let picked = {
        let _modal = crate::window::modal::ModalScope::enter(hwnd);
        choose(hwnd, panel, button, &items, &labels)
    };
    match picked.and_then(|index| items.get(index)) {
        Some(AddChoice::Open) => crate::window::library_host::choose_and_open_folder(hwnd),
        Some(AddChoice::New) => crate::window::library_host::create_and_open_notebook(hwnd),
        Some(AddChoice::Recent(folder)) => {
            crate::window::library_host::open_listed_notebook(hwnd, folder);
        }
        Some(AddChoice::MoreRecent) => {
            crate::window::library_host::open_recent_folder_picker(hwnd);
        }
        Some(AddChoice::Separator) | None => {}
    }
}

/// The menu under test: answered by `answer_next_add_menu`, never shown.
#[cfg(test)]
fn choose(
    _hwnd: HWND,
    _panel: HWND,
    _button: RECT,
    items: &[AddChoice],
    labels: &[String],
) -> Option<usize> {
    LAST_ADD_MENU.with(|last| *last.borrow_mut() = labels.to_vec());
    let answer = ADD_ANSWERS
        .with(|answers| answers.borrow_mut().pop_front())
        .expect("a test opened the add notebook menu without queuing an answer");
    answer(items)
}

/// Shows the menu under `button` (panel coordinates) and returns the index of the item picked.
#[cfg(not(test))]
fn choose(
    hwnd: HWND,
    panel: HWND,
    button: RECT,
    items: &[AddChoice],
    labels: &[String],
) -> Option<usize> {
    use crate::platform::wide_null;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD,
        TPM_RIGHTALIGN, TPM_TOPALIGN, TrackPopupMenu,
    };
    // Right-aligned under the button, which sits at the panel's right edge.
    let mut at = POINT {
        x: button.right,
        y: button.bottom,
    };
    unsafe {
        ClientToScreen(panel, &mut at);
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        for (index, (item, label)) in items.iter().zip(labels).enumerate() {
            if *item == AddChoice::Separator {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            } else {
                let text = wide_null(label);
                AppendMenuW(menu, MF_STRING, index + 1, text.as_ptr());
            }
        }
        let picked = TrackPopupMenu(
            menu,
            TPM_RIGHTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
            at.x,
            at.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        usize::try_from(picked).ok()?.checked_sub(1)
    }
}

#[cfg(test)]
impl NotebookView {
    /// The panel's client rectangle and DPI, as paint and the accessibility tree use them.
    pub(crate) fn client_and_dpi_for_test(&self) -> (RECT, u32) {
        (self.client(), self.dpi())
    }
}

/// A queued answer to the add notebook menu.
#[cfg(test)]
type AddAnswer = Box<dyn FnOnce(&[AddChoice]) -> Option<usize>>;

#[cfg(test)]
thread_local! {
    static ADD_ANSWERS: std::cell::RefCell<std::collections::VecDeque<AddAnswer>> =
        std::cell::RefCell::new(std::collections::VecDeque::new());
    static LAST_ADD_MENU: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Answers the next add menu with the index of an item, or `None` to dismiss it.
#[cfg(test)]
pub(crate) fn answer_next_add_menu(answer: impl FnOnce(&[AddChoice]) -> Option<usize> + 'static) {
    ADD_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// The last add menu's labels, a separator as "".
#[cfg(test)]
pub(crate) fn last_add_menu() -> Vec<String> {
    LAST_ADD_MENU.with(|last| last.borrow().clone())
}
