use crate::Result;
use crate::platform::{last_error, wide_null};
use crate::window::commands::CommandId;
use crate::window::menu_band::{self, MENU_TITLES};
use crate::window::modal::ModalScope;
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_LEFT, VK_RIGHT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    ACCEL, AppendMenuW, CallNextHookEx, CheckMenuItem, CreateAcceleratorTableW, CreateMenu,
    CreatePopupMenu, DestroyAcceleratorTable, DestroyMenu, EnableMenuItem, EndMenu, FVIRTKEY,
    GetMenuItemCount, GetMenuState, GetSubMenu, HACCEL, HMENU, MF_BYCOMMAND, MF_BYPOSITION,
    MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_HILITE, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    MSG, MSGF_MENU, SetWindowsHookExW, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN,
    TPM_VERTICAL, TPMPARAMS, TrackPopupMenuEx, TranslateAcceleratorW, UnhookWindowsHookEx,
    WH_MSGFILTER, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE,
};

#[derive(Debug)]
pub(crate) struct AcceleratorTable(HACCEL);

impl AcceleratorTable {
    /// The table for `keymap`'s bindings, in its precedence order: `TranslateAcceleratorW`
    /// takes the first entry that matches, so a user binding shadows a default on its key.
    pub(crate) fn create(keymap: &crate::window::keymap::Keymap) -> Result<Self> {
        let accelerators = keymap
            .bindings()
            .iter()
            .map(|binding| ACCEL {
                fVirt: FVIRTKEY | binding.stroke.accel_flags(),
                key: binding.stroke.vk,
                cmd: binding.command as u16,
            })
            .collect::<Vec<_>>();
        // Windows refuses a table of no entries; with every command unbound there is none.
        if accelerators.is_empty() {
            return Err(crate::FastPadError::Invariant(
                "no keyboard shortcuts are bound",
            ));
        }
        // `ACCEL` alone aligns to 2 bytes, but `CreateAcceleratorTableW` rejects a buffer that
        // is not on a 4-byte boundary with `ERROR_NOACCESS`. A `u32` buffer pins the alignment.
        let bytes = std::mem::size_of_val(accelerators.as_slice());
        let mut aligned = vec![0u32; bytes.div_ceil(4).max(1)];
        unsafe {
            std::ptr::copy_nonoverlapping(
                accelerators.as_ptr().cast::<u8>(),
                aligned.as_mut_ptr().cast::<u8>(),
                bytes,
            );
        }
        let handle = unsafe {
            CreateAcceleratorTableW(aligned.as_ptr().cast::<ACCEL>(), accelerators.len() as i32)
        };
        if handle.is_null() {
            Err(last_error())
        } else {
            Ok(Self(handle))
        }
    }

    pub(crate) fn raw(&self) -> HACCEL {
        self.0
    }

    /// The table's entries, as Windows stores them.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> Vec<ACCEL> {
        use windows_sys::Win32::UI::WindowsAndMessaging::CopyAcceleratorTableW;
        let count = unsafe { CopyAcceleratorTableW(self.0, std::ptr::null_mut(), 0) };
        let mut entries = vec![
            ACCEL {
                fVirt: 0,
                key: 0,
                cmd: 0
            };
            count.max(0) as usize
        ];
        unsafe { CopyAcceleratorTableW(self.0, entries.as_mut_ptr(), count) };
        entries
    }
}

impl Drop for AcceleratorTable {
    fn drop(&mut self) {
        unsafe {
            DestroyAcceleratorTable(self.0);
        }
    }
}

#[derive(Debug)]
pub(crate) struct MenuBar(HMENU);

/// View ▸ Editor Layout (split editors spec §7).
fn editor_layout() -> Vec<MenuEntry> {
    vec![
        MenuEntry::command("Split &Right", CommandId::SplitRight),
        MenuEntry::command("Split &Down", CommandId::SplitDown),
        MenuEntry::Separator,
        MenuEntry::command("Move to &Next Group", CommandId::MoveTabToNextGroup),
        MenuEntry::command("Move to &Previous Group", CommandId::MoveTabToPreviousGroup),
        MenuEntry::Separator,
        MenuEntry::command("&Close Group", CommandId::CloseGroup),
    ]
}

impl MenuBar {
    pub(crate) fn create(keymap: &crate::window::keymap::Keymap) -> Result<Self> {
        let root = unsafe { CreateMenu() };
        if root.is_null() {
            return Err(last_error());
        }
        let result = (|| {
            let file = create_popup(
                &[
                    MenuEntry::command("&New", CommandId::New),
                    MenuEntry::command("&Open...", CommandId::Open),
                    MenuEntry::command("Open &Notebook...", CommandId::OpenFolder),
                    MenuEntry::command("&Go to note\u{2026}", CommandId::QuickOpen),
                    MenuEntry::command("&Save", CommandId::Save),
                    MenuEntry::command("Save &As...", CommandId::SaveAs),
                    MenuEntry::command("&Close tab", CommandId::CloseTab),
                    MenuEntry::command("Close a&ll tabs", CommandId::CloseAllTabs),
                    MenuEntry::command("Close &group", CommandId::CloseGroup),
                    MenuEntry::Separator,
                    MenuEntry::command("Se&ttings...", CommandId::OpenSettings),
                    MenuEntry::command(
                        "&Restore session on startup",
                        CommandId::ToggleRestoreSession,
                    ),
                    MenuEntry::Separator,
                    MenuEntry::command("E&xit", CommandId::Exit),
                ],
                keymap,
            )?;
            append_popup(root, MENU_TITLES[0], file)?;
            let edit = create_popup(
                &[
                    MenuEntry::command("&Undo", CommandId::Undo),
                    MenuEntry::command("&Redo", CommandId::Redo),
                    MenuEntry::Separator,
                    MenuEntry::command("Cu&t", CommandId::Cut),
                    MenuEntry::command("&Copy", CommandId::Copy),
                    MenuEntry::command("&Paste", CommandId::Paste),
                    MenuEntry::Separator,
                    MenuEntry::command("&Format JSON", CommandId::FormatJson),
                ],
                keymap,
            )?;
            append_popup(root, MENU_TITLES[1], edit)?;
            let search = create_popup(
                &[
                    MenuEntry::command("&Find", CommandId::Find),
                    MenuEntry::command("Find &next", CommandId::FindNext),
                    MenuEntry::command("Find pre&vious", CommandId::FindPrevious),
                    MenuEntry::command("&Replace", CommandId::Replace),
                ],
                keymap,
            )?;
            append_popup(root, MENU_TITLES[2], search)?;
            let view = create_popup(
                &[
                    MenuEntry::Submenu(
                        "&Language",
                        crate::languages::LANGUAGES
                            .iter()
                            .map(|row| {
                                MenuEntry::command(row.name, CommandId::for_language(row.language))
                            })
                            .collect(),
                    ),
                    MenuEntry::Separator,
                    MenuEntry::command("Zoom &in", CommandId::ZoomIn),
                    MenuEntry::command("Zoom &out", CommandId::ZoomOut),
                    MenuEntry::command("Reset &zoom", CommandId::ZoomReset),
                    MenuEntry::Separator,
                    MenuEntry::command("&Word wrap", CommandId::ToggleWordWrap),
                    MenuEntry::command("Line &numbers", CommandId::ToggleLineNumbers),
                    MenuEntry::Separator,
                    MenuEntry::command("Side&bar", CommandId::ToggleSidebar),
                    MenuEntry::Submenu("Editor &Layout", editor_layout()),
                    MenuEntry::Separator,
                    MenuEntry::command(
                        "Markdown preview &side by side",
                        CommandId::MarkdownPreviewSide,
                    ),
                    MenuEntry::command("Markdown preview f&ull", CommandId::MarkdownPreviewFull),
                    MenuEntry::command("Close Markdown pre&view", CommandId::MarkdownPreviewClose),
                    MenuEntry::Separator,
                    MenuEntry::command("Command &palette...", CommandId::CommandPalette),
                ],
                keymap,
            )?;
            append_popup(root, MENU_TITLES[menu_band::VIEW_MENU_INDEX], view)?;
            let help = create_popup(
                &[MenuEntry::command("&About FastPad", CommandId::About)],
                keymap,
            )?;
            append_popup(root, MENU_TITLES[menu_band::HELP_MENU_INDEX], help)
        })();
        match result {
            Ok(()) => Ok(Self(root)),
            Err(error) => {
                unsafe {
                    DestroyMenu(root);
                }
                Err(error)
            }
        }
    }

    /// The dropdown under heading `index` of `MENU_TITLES`.
    pub(crate) fn dropdown(&self, index: usize) -> HMENU {
        unsafe { GetSubMenu(self.0, index as i32) }
    }
}

pub(crate) fn translate_accelerator(handle: HACCEL, hwnd: HWND, message: &MSG) -> bool {
    unsafe { TranslateAcceleratorW(hwnd, handle, message) != 0 }
}

/// Grays the View menu's preview entries while the active tab is not Markdown.
pub(crate) fn set_markdown_preview_enabled(menu: HMENU, enabled: bool) {
    let state = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
    for command in [
        CommandId::MarkdownPreviewSide,
        CommandId::MarkdownPreviewFull,
        CommandId::MarkdownPreviewClose,
    ] {
        unsafe { EnableMenuItem(menu, command as u32, state) };
    }
}

/// Grays the commands that need text while an image tab is active (image preview spec §5).
pub(crate) fn set_text_commands_enabled(menu: HMENU, enabled: bool) {
    let state = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
    for command in crate::window::commands::TEXT_COMMANDS {
        unsafe { EnableMenuItem(menu, command as u32, state) };
    }
}

/// Checks the active tab's language in View → Language and clears every other language.
pub(crate) fn set_checked_language(menu: HMENU, active: crate::document::Language) {
    for row in crate::languages::LANGUAGES.iter() {
        let check = if row.language == active {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        unsafe {
            CheckMenuItem(
                menu,
                CommandId::for_language(row.language) as u32,
                MF_BYCOMMAND | check,
            )
        };
    }
}

/// Grays the View menu's Sidebar entry while notes mode is off and there is no sidebar.
pub(crate) fn set_sidebar_enabled(menu: HMENU, enabled: bool) {
    let state = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
    unsafe { EnableMenuItem(menu, CommandId::ToggleSidebar as u32, state) };
}

impl Drop for MenuBar {
    fn drop(&mut self) {
        unsafe {
            DestroyMenu(self.0);
        }
    }
}

pub(crate) enum MenuEntry {
    Command(&'static str, CommandId),
    /// A command id reused for a local action: it shows no key, since the global key does
    /// something else.
    Local(&'static str, CommandId),
    Submenu(&'static str, Vec<MenuEntry>),
    Separator,
}

impl MenuEntry {
    pub(crate) const fn command(label: &'static str, command: CommandId) -> Self {
        Self::Command(label, command)
    }

    pub(crate) const fn local(label: &'static str, command: CommandId) -> Self {
        Self::Local(label, command)
    }
}

fn create_popup(entries: &[MenuEntry], keymap: &crate::window::keymap::Keymap) -> Result<HMENU> {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return Err(last_error());
    }
    for entry in entries {
        let ok = match entry {
            MenuEntry::Command(label, command) => {
                // A label that spells its own key (the tree's F2, Del) keeps it; every other
                // command shows its first key from the keymap (keyboard shortcuts spec §5).
                let text = match keymap.first_text(*command) {
                    Some(key) if !label.contains('\t') => format!("{label}\t{key}"),
                    _ => (*label).to_owned(),
                };
                let text = wide_null(&text);
                unsafe { AppendMenuW(menu, MF_STRING, *command as usize, text.as_ptr()) }
            }
            MenuEntry::Local(label, command) => {
                let text = wide_null(label);
                unsafe { AppendMenuW(menu, MF_STRING, *command as usize, text.as_ptr()) }
            }
            MenuEntry::Submenu(label, children) => {
                let child = match create_popup(children, keymap) {
                    Ok(child) => child,
                    Err(error) => {
                        unsafe { DestroyMenu(menu) };
                        return Err(error);
                    }
                };
                let label = wide_null(label);
                // Once appended, the child belongs to `menu` and is destroyed with it.
                let ok = unsafe { AppendMenuW(menu, MF_POPUP, child as usize, label.as_ptr()) };
                if ok == 0 {
                    unsafe { DestroyMenu(child) };
                }
                ok
            }
            MenuEntry::Separator => unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null()) },
        };
        if ok == 0 {
            unsafe {
                DestroyMenu(menu);
            }
            return Err(last_error());
        }
    }
    Ok(menu)
}

fn append_popup(root: HMENU, label: &str, popup: HMENU) -> Result<()> {
    let label = wide_null(label);
    if unsafe { AppendMenuW(root, MF_POPUP, popup as usize, label.as_ptr()) } == 0 {
        unsafe {
            DestroyMenu(popup);
        }
        Err(last_error())
    } else {
        Ok(())
    }
}

/// The context menu of a tab, at client coordinates `x`, `y` (split editors plan amendment 12).
pub(crate) fn show_tab_menu(hwnd: HWND, x: i32, y: i32) -> Option<CommandId> {
    let entries = [
        MenuEntry::command("&Close tab", CommandId::CloseTab),
        MenuEntry::command("Close a&ll tabs", CommandId::CloseAllTabs),
        MenuEntry::Separator,
        MenuEntry::command("Split &Right", CommandId::SplitRight),
        MenuEntry::command("Split &Down", CommandId::SplitDown),
        MenuEntry::command("Move to &Next Group", CommandId::MoveTabToNextGroup),
    ];
    track_popup(hwnd, &entries, POINT { x, y })
}

/// The context menu of the empty tab-strip space, at client coordinates `x`, `y`.
pub(crate) fn show_tab_strip_menu(hwnd: HWND, x: i32, y: i32, has_tabs: bool) -> Option<CommandId> {
    let mut entries = vec![
        MenuEntry::command("New tab", CommandId::New),
        MenuEntry::command("Open...", CommandId::Open),
    ];
    if has_tabs {
        entries.extend([
            MenuEntry::Separator,
            MenuEntry::command("Close all tabs", CommandId::CloseAllTabs),
        ]);
    }
    entries.extend([
        MenuEntry::Separator,
        MenuEntry::command("Split Right", CommandId::SplitRight),
        MenuEntry::command("Split Down", CommandId::SplitDown),
        MenuEntry::command("Close group", CommandId::CloseGroup),
    ]);
    track_popup(hwnd, &entries, POINT { x, y })
}

pub(crate) fn track_popup(hwnd: HWND, entries: &[MenuEntry], client: POINT) -> Option<CommandId> {
    // TrackPopupMenuEx runs a nested modal loop that reenters the window procedure, exactly as the
    // file dialogs do. Hold deferred, IPC and snapshot work for its duration so the command the
    // user picks still acts on the document that was active when they opened the menu.
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = POPUP_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd);
    }
    let keymap = crate::window::main_window::keymap(hwnd);
    let menu = create_popup(entries, &keymap).ok()?;
    let mut point = client;
    unsafe {
        ClientToScreen(hwnd, &mut point);
    }
    let selected = unsafe {
        TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            hwnd,
            std::ptr::null(),
        )
    };
    unsafe {
        DestroyMenu(menu);
    }
    u16::try_from(selected)
        .ok()
        .and_then(|value| CommandId::try_from(value).ok())
}

/// How a menu-band dropdown closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DropdownExit {
    Command(CommandId),
    /// Left/Right, or the pointer moving onto another heading: open that heading's dropdown next.
    Switch(usize),
    /// Escape closes just the dropdown and leaves its heading highlighted.
    Escape,
    /// A click outside, or on the open heading itself, leaves menu mode.
    Dismissed,
}

struct DropdownTracking {
    current: usize,
    /// The dropdown being tracked, read for which item and submenu are highlighted.
    menu: HMENU,
    /// Heading rectangles in screen coordinates.
    headings: Vec<RECT>,
    exit: Option<DropdownExit>,
}

/// Where the keyboard highlight is inside a dropdown that may hold submenus.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HighlightState {
    /// The dropdown's highlighted item opens a submenu.
    pub(crate) on_popup_item: bool,
    /// That submenu is open with one of its own items highlighted.
    pub(crate) in_submenu: bool,
}

/// Reads `MF_HILITE` from `menu`'s items and, for a highlighted submenu item, from its children.
pub(crate) fn highlight_state(menu: HMENU) -> HighlightState {
    let highlighted = |menu: HMENU| {
        let count = unsafe { GetMenuItemCount(menu) }.max(0) as u32;
        (0..count).find(|&position| {
            let state = unsafe { GetMenuState(menu, position, MF_BYPOSITION) };
            state != u32::MAX && state & MF_HILITE != 0
        })
    };
    let Some(position) = highlighted(menu) else {
        return HighlightState::default();
    };
    let submenu = unsafe { GetSubMenu(menu, position as i32) };
    if submenu.is_null() {
        return HighlightState::default();
    }
    HighlightState {
        on_popup_item: true,
        in_submenu: highlighted(submenu).is_some(),
    }
}

/// What Left/Right does in a dropdown: Right on a submenu item opens it and Left inside a submenu
/// closes it (both left to Windows, `None`); otherwise the arrows move to the neighboring heading.
pub(crate) fn arrow_exit(
    right: bool,
    current: usize,
    highlight: HighlightState,
) -> Option<DropdownExit> {
    let submenu_owns_the_key = if right {
        highlight.on_popup_item && !highlight.in_submenu
    } else {
        highlight.in_submenu
    };
    if submenu_owns_the_key {
        None
    } else {
        Some(DropdownExit::Switch(menu_band::neighbor(current, right)))
    }
}

thread_local! {
    static DROPDOWN: RefCell<Option<DropdownTracking>> = const { RefCell::new(None) };
}

/// Opens `menu` below heading `current` (`headings` are client rectangles) and reports how it
/// closed. Moving between headings needs a message filter: a popup menu's modal loop otherwise
/// swallows Left/Right and pointer movement over the band.
pub(crate) fn track_dropdown(
    hwnd: HWND,
    menu: HMENU,
    current: usize,
    headings: &[RECT],
) -> DropdownExit {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = DROPDOWN_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd, current);
    }
    let headings = headings
        .iter()
        .map(|rect| client_to_screen(hwnd, *rect))
        .collect::<Vec<_>>();
    let Some(anchor) = headings.get(current).copied() else {
        return DropdownExit::Dismissed;
    };
    DROPDOWN.with(|tracking| {
        *tracking.borrow_mut() = Some(DropdownTracking {
            current,
            menu,
            headings,
            exit: None,
        });
    });
    let hook = unsafe {
        SetWindowsHookExW(
            WH_MSGFILTER,
            Some(dropdown_filter),
            std::ptr::null_mut(),
            GetCurrentThreadId(),
        )
    };
    let params = TPMPARAMS {
        cbSize: std::mem::size_of::<TPMPARAMS>() as u32,
        rcExclude: anchor,
    };
    let selected = unsafe {
        TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN | TPM_VERTICAL,
            anchor.left,
            anchor.bottom,
            hwnd,
            &params,
        )
    };
    if !hook.is_null() {
        unsafe {
            UnhookWindowsHookEx(hook);
        }
    }
    let exit = DROPDOWN.with(|tracking| tracking.borrow_mut().take().and_then(|state| state.exit));
    u16::try_from(selected)
        .ok()
        .and_then(|value| CommandId::try_from(value).ok())
        .map(DropdownExit::Command)
        .or(exit)
        .unwrap_or(DropdownExit::Dismissed)
}

fn client_to_screen(hwnd: HWND, rect: RECT) -> RECT {
    let mut top_left = POINT {
        x: rect.left,
        y: rect.top,
    };
    let mut bottom_right = POINT {
        x: rect.right,
        y: rect.bottom,
    };
    unsafe {
        ClientToScreen(hwnd, &mut top_left);
        ClientToScreen(hwnd, &mut bottom_right);
    }
    RECT {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
    }
}

/// The `WH_MSGFILTER` hook installed for one dropdown: decides from each message of the popup's
/// modal loop whether to close it in favor of a neighboring heading.
unsafe extern "system" fn dropdown_filter(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == MSGF_MENU as i32 && lparam != 0 {
        let message = unsafe { &*(lparam as *const MSG) };
        let exit = DROPDOWN.with(|tracking| {
            let mut tracking = tracking.borrow_mut();
            let state = tracking.as_mut()?;
            let under_pointer = menu_band::heading_at(&state.headings, message.pt.x, message.pt.y);
            let exit = match message.message {
                WM_KEYDOWN
                    if message.wParam == VK_LEFT as usize
                        || message.wParam == VK_RIGHT as usize =>
                {
                    arrow_exit(
                        message.wParam == VK_RIGHT as usize,
                        state.current,
                        highlight_state(state.menu),
                    )?
                }
                WM_KEYDOWN if message.wParam == VK_ESCAPE as usize => DropdownExit::Escape,
                WM_MOUSEMOVE => match under_pointer {
                    Some(index) if index != state.current => DropdownExit::Switch(index),
                    _ => return None,
                },
                WM_LBUTTONDOWN if under_pointer == Some(state.current) => DropdownExit::Dismissed,
                _ => return None,
            };
            state.exit = Some(exit);
            Some(exit)
        });
        match exit {
            // The popup's own Escape handling closes it; only the reason is recorded.
            Some(DropdownExit::Escape) | None => {}
            Some(_) => {
                unsafe {
                    EndMenu();
                }
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

#[cfg(test)]
type PopupAnswer = Box<dyn FnOnce(HWND) -> Option<CommandId>>;

#[cfg(test)]
thread_local! {
    static POPUP_ANSWERS: std::cell::RefCell<std::collections::VecDeque<PopupAnswer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

#[cfg(test)]
type DropdownAnswer = Box<dyn FnOnce(HWND, usize) -> DropdownExit>;

#[cfg(test)]
thread_local! {
    static DROPDOWN_ANSWERS: std::cell::RefCell<std::collections::VecDeque<DropdownAnswer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Answers the next menu-band dropdown (given its heading) instead of tracking a real popup.
#[cfg(test)]
pub(crate) fn answer_next_dropdown(answer: impl FnOnce(HWND, usize) -> DropdownExit + 'static) {
    DROPDOWN_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Answers the next popup menu from inside its modal scope instead of tracking a real popup.
#[cfg(test)]
pub(crate) fn answer_next_popup_menu(answer: impl FnOnce(HWND) -> Option<CommandId> + 'static) {
    POPUP_ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

#[cfg(test)]
mod tests {
    use crate::window::commands::CommandId;

    fn label(
        menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
        command: CommandId,
    ) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuStringW, MF_BYCOMMAND};
        let mut buffer = [0u16; 128];
        let length = unsafe {
            GetMenuStringW(
                menu,
                command as u32,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                MF_BYCOMMAND,
            )
        };
        String::from_utf16_lossy(&buffer[..length.max(0) as usize])
    }

    #[test]
    fn menu_labels_spell_the_keymaps_first_key() {
        // Break caught: a rebound command whose menu still shows the old key, or an unbound one
        // still showing a key that does nothing.
        use super::MenuBar;
        use crate::window::keymap::{KeyStroke, Keymap};
        let defaults = MenuBar::create(&Keymap::defaults()).unwrap();
        assert_eq!(
            label(defaults.dropdown(0), CommandId::Save),
            "&Save\tCtrl+S"
        );
        assert_eq!(
            label(defaults.dropdown(0), CommandId::CloseAllTabs),
            "Close a&ll tabs"
        );
        let view = crate::window::menu_band::VIEW_MENU_INDEX;
        assert_eq!(
            label(defaults.dropdown(view), CommandId::ZoomIn),
            "Zoom &in\tCtrl+="
        );

        let keymap = Keymap::defaults()
            .with_keys(
                CommandId::Save,
                vec![KeyStroke::parse("Ctrl+Alt+S").unwrap()],
            )
            .with_keys(CommandId::Undo, vec![]);
        let custom = MenuBar::create(&keymap).unwrap();
        assert_eq!(
            label(custom.dropdown(0), CommandId::Save),
            "&Save\tCtrl+Alt+S"
        );
        assert_eq!(label(custom.dropdown(1), CommandId::Undo), "&Undo");
    }

    #[test]
    fn a_local_entry_shows_no_key_even_when_its_command_has_one() {
        // Break caught: a context-menu "Open in new tab" reading Ctrl+O, which opens the dialog.
        let entries = [
            super::MenuEntry::local("Open in new tab", CommandId::Open),
            super::MenuEntry::command("&Save", CommandId::Save),
        ];
        let menu =
            super::create_popup(&entries, &crate::window::keymap::Keymap::defaults()).unwrap();
        assert_eq!(label(menu, CommandId::Open), "Open in new tab");
        assert_eq!(label(menu, CommandId::Save), "&Save\tCtrl+S");
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyMenu(menu) };
    }

    #[derive(Clone, Copy)]
    struct Spec {
        modifiers: u8,
        key: u16,
        command: CommandId,
    }

    fn accelerator_specs() -> Vec<Spec> {
        crate::window::keymap::Keymap::defaults()
            .bindings()
            .iter()
            .map(|binding| Spec {
                modifiers: binding.stroke.accel_flags(),
                key: binding.stroke.vk,
                command: binding.command,
            })
            .collect()
    }

    #[test]
    fn ctrl_w_closes_the_tab() {
        // Break caught: Ctrl+W unbound, or bound to Close all tabs (quick-open spec §4).
        use windows_sys::Win32::UI::WindowsAndMessaging::FCONTROL;
        let bound = accelerator_specs()
            .into_iter()
            .find(|spec| spec.modifiers == FCONTROL && spec.key == u16::from(b'W'))
            .map(|spec| spec.command);
        assert_eq!(bound, Some(CommandId::CloseTab));
    }

    #[test]
    fn shortcut_and_menu_commands_share_command_ids() {
        let specs = accelerator_specs();
        assert!(specs.iter().any(|item| item.command == CommandId::New));
        assert!(specs.iter().any(|item| item.command == CommandId::SaveAs));
        assert!(
            specs
                .iter()
                .any(|item| item.command == CommandId::FormatJson)
        );
        assert_eq!(specs.len(), 66);
    }

    #[test]
    fn the_native_accelerator_table_is_created_from_a_four_byte_aligned_buffer() {
        // Break caught: release builds where the table buffer landed off a 4-byte boundary, so
        // table creation failed with ERROR_NOACCESS and every keyboard shortcut was silently dead.
        let table = super::AcceleratorTable::create(&crate::window::keymap::Keymap::defaults())
            .expect("accelerator table");
        assert_eq!(table.entries().len(), 66);
    }

    #[test]
    fn ctrl_p_opens_quick_open_and_ctrl_shift_p_stays_the_palette() {
        // Break caught: Ctrl+P unbound, or taking Ctrl+Shift+P from the command palette.
        use windows_sys::Win32::UI::WindowsAndMessaging::{FCONTROL, FSHIFT};
        let bound = |modifiers: u8| {
            accelerator_specs()
                .into_iter()
                .find(|spec| spec.modifiers == modifiers && spec.key == u16::from(b'P'))
                .map(|spec| spec.command)
        };
        assert_eq!(bound(FCONTROL), Some(CommandId::QuickOpen));
        assert_eq!(bound(FCONTROL | FSHIFT), Some(CommandId::CommandPalette));
    }

    #[test]
    fn every_shortcut_chord_maps_to_exactly_one_command() {
        let specs = accelerator_specs();
        for (index, spec) in specs.iter().enumerate() {
            assert!(
                specs[index + 1..]
                    .iter()
                    .all(|other| (other.modifiers, other.key) != (spec.modifiers, spec.key)),
                "duplicate chord for {:?}",
                spec.command
            );
        }
    }

    #[test]
    fn tab_and_zoom_shortcuts_are_bound_and_text_direction_has_none() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_F3, VK_NUMPAD9, VK_OEM_MINUS, VK_OEM_PLUS, VK_TAB,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};
        let bound = |modifiers: u8, key: u16| {
            accelerator_specs()
                .into_iter()
                .find(|spec| spec.modifiers == modifiers && spec.key == key)
                .map(|spec| spec.command)
        };
        assert_eq!(bound(FCONTROL, u16::from(b'T')), Some(CommandId::New));
        assert_eq!(bound(FCONTROL, VK_TAB), Some(CommandId::NextTab));
        assert_eq!(
            bound(FCONTROL | FSHIFT, VK_TAB),
            Some(CommandId::PreviousTab)
        );
        // Split editors spec §6: Ctrl+digits focus groups, Alt+digits select tabs.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_LEFT, VK_NUMPAD0, VK_NUMPAD2, VK_RIGHT,
        };
        assert_eq!(
            bound(FCONTROL, u16::from(b'1')),
            Some(CommandId::FocusGroup1)
        );
        assert_eq!(
            bound(FCONTROL, u16::from(b'9')),
            Some(CommandId::FocusLastGroup)
        );
        assert_eq!(bound(FCONTROL, VK_NUMPAD2), Some(CommandId::FocusGroup2));
        assert_eq!(bound(FALT, u16::from(b'1')), Some(CommandId::SelectTab1));
        // Alt with numpad digits types Alt codes (Alt+0233 is é): an accelerator there would
        // swallow each digit before TranslateMessage composes the character.
        for key in VK_NUMPAD0..=VK_NUMPAD9 {
            assert_eq!(bound(FALT, key), None, "Alt+numpad {key}");
        }
        assert_eq!(
            bound(FCONTROL | FALT, VK_RIGHT),
            Some(CommandId::MoveTabToNextGroup)
        );
        assert_eq!(
            bound(FCONTROL | FALT, VK_LEFT),
            Some(CommandId::MoveTabToPreviousGroup)
        );
        assert_eq!(bound(FCONTROL, VK_OEM_PLUS), Some(CommandId::ZoomIn));
        assert_eq!(bound(FCONTROL, VK_OEM_MINUS), Some(CommandId::ZoomOut));
        assert_eq!(bound(FCONTROL, u16::from(b'0')), Some(CommandId::ZoomReset));
        assert_eq!(bound(FCONTROL, u16::from(b'L')), None);
        assert_eq!(bound(FCONTROL, u16::from(b'R')), None);
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'P')),
            Some(CommandId::CommandPalette)
        );
        assert_eq!(
            bound(FALT, u16::from(b'Z')),
            Some(CommandId::ToggleWordWrap)
        );
        assert_eq!(
            bound(FCONTROL, u16::from(b'B')),
            Some(CommandId::ToggleSidebar)
        );
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'E')),
            Some(CommandId::ShowNotebookView)
        );
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'F')),
            Some(CommandId::ShowSearchView)
        );
        assert_eq!(
            bound(FSHIFT | FALT, u16::from(b'F')),
            Some(CommandId::FormatJson)
        );
        // Break caught: Ctrl+K still showing Search after Search moved to Ctrl+Shift+F.
        assert_eq!(bound(FCONTROL, u16::from(b'K')), None);
        // The option toggles are palette-only (spec §5).
        assert!(
            accelerator_specs()
                .iter()
                .all(|spec| spec.command.search_option().is_none())
        );
        // Break caught: F3 unbound, so opening a Search result can't step on (spec §8).
        assert_eq!(bound(0, VK_F3), Some(CommandId::FindNext));
        assert_eq!(bound(FSHIFT, VK_F3), Some(CommandId::FindPrevious));
        // Break caught: Ctrl+Shift+H unbound, or taking Ctrl+H from the find bar's Replace
        // (spec §11).
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'H')),
            Some(CommandId::ReplaceInNotes)
        );
        assert_eq!(bound(FCONTROL, u16::from(b'H')), Some(CommandId::Replace));
        // Split editors spec §6: Ctrl+\ splits right, Ctrl+Shift+\ down.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_OEM_5;
        assert_eq!(bound(FCONTROL, VK_OEM_5), Some(CommandId::SplitRight));
        assert_eq!(
            bound(FCONTROL | FSHIFT, VK_OEM_5),
            Some(CommandId::SplitDown)
        );
    }

    #[test]
    fn the_file_menu_closes_all_tabs_and_the_edit_menu_formats_json() {
        // Break caught: commands lost with the title bar's and the strip's "…" menus, which the
        // menu band now carries instead.
        use super::MenuBar;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND};
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let has = |menu: usize, command: CommandId| unsafe {
            GetMenuState(bar.dropdown(menu), command as u32, MF_BYCOMMAND) != u32::MAX
        };
        assert!(has(0, CommandId::CloseAllTabs), "File: Close all tabs");
        assert!(has(1, CommandId::FormatJson), "Edit: Format JSON");
    }

    #[test]
    fn the_help_menu_is_last_and_opens_about() {
        // Break caught: a Help heading whose dropdown is missing, so the band shows a heading
        // that opens nothing, or About filed under another heading.
        use super::MenuBar;
        use crate::window::menu_band::{HELP_MENU_INDEX, MENU_TITLES};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND};
        assert_eq!(MENU_TITLES[HELP_MENU_INDEX], "&Help");
        assert_eq!(HELP_MENU_INDEX, MENU_TITLES.len() - 1);
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let state = unsafe {
            GetMenuState(
                bar.dropdown(HELP_MENU_INDEX),
                CommandId::About as u32,
                MF_BYCOMMAND,
            )
        };
        assert_ne!(state, u32::MAX, "Help: About FastPad");
    }

    #[test]
    fn the_file_menu_opens_settings() {
        // Break caught: Settings missing from the menus, so the only mouse route is the gear,
        // which is hidden with notes mode off.
        use super::MenuBar;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND};
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let state = unsafe {
            GetMenuState(
                bar.dropdown(0),
                CommandId::OpenSettings as u32,
                MF_BYCOMMAND,
            )
        };
        assert_ne!(state, u32::MAX, "File: Settings");
    }

    #[test]
    fn the_view_menu_toggles_the_sidebar_and_grays_it_without_notes_mode() {
        // Break caught: a Sidebar entry that stays enabled with notes mode off, where it does
        // nothing, or no entry at all.
        use super::{MenuBar, set_sidebar_enabled};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND, MF_GRAYED};
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let view = bar.dropdown(crate::window::menu_band::VIEW_MENU_INDEX);
        let state = || unsafe { GetMenuState(view, CommandId::ToggleSidebar as u32, MF_BYCOMMAND) };
        assert_ne!(state(), u32::MAX, "the View menu has a Sidebar entry");
        set_sidebar_enabled(view, false);
        assert_ne!(state() & MF_GRAYED, 0);
        set_sidebar_enabled(view, true);
        assert_eq!(state() & MF_GRAYED, 0);
    }

    #[test]
    fn arrows_open_and_close_a_submenu_before_switching_headings() {
        // Break caught: Right on View → Language jumped to the next heading instead of opening
        // the submenu, and Left inside the submenu left View instead of closing the submenu.
        use super::{DropdownExit, HighlightState, arrow_exit};
        use crate::window::menu_band::neighbor;
        let plain = HighlightState::default();
        let on_popup = HighlightState {
            on_popup_item: true,
            in_submenu: false,
        };
        let inside = HighlightState {
            on_popup_item: true,
            in_submenu: true,
        };

        assert_eq!(arrow_exit(true, 2, on_popup), None);
        assert_eq!(arrow_exit(false, 2, inside), None);
        assert_eq!(
            arrow_exit(true, 2, plain),
            Some(DropdownExit::Switch(neighbor(2, true)))
        );
        assert_eq!(
            arrow_exit(false, 2, plain),
            Some(DropdownExit::Switch(neighbor(2, false)))
        );
        assert_eq!(
            arrow_exit(false, 2, on_popup),
            Some(DropdownExit::Switch(neighbor(2, false)))
        );
        // Right on a leaf inside the submenu moves on, as Windows' own menu bar does.
        assert_eq!(
            arrow_exit(true, 2, inside),
            Some(DropdownExit::Switch(neighbor(2, true)))
        );
    }

    #[test]
    fn highlight_state_reads_the_dropdown_and_its_open_submenu() {
        use super::{HighlightState, MenuBar, highlight_state};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSubMenu, MENUITEMINFOW, MFS_HILITE, MIIM_STATE, SetMenuItemInfoW,
        };
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let view = bar.dropdown(crate::window::menu_band::VIEW_MENU_INDEX);
        let hilite = |menu, position: u32| {
            let info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_STATE,
                fState: MFS_HILITE,
                ..unsafe { std::mem::zeroed() }
            };
            assert_ne!(unsafe { SetMenuItemInfoW(menu, position, 1, &info) }, 0);
        };

        assert_eq!(highlight_state(view), HighlightState::default());
        // View's first item is the Language submenu.
        hilite(view, 0);
        assert_eq!(
            highlight_state(view),
            HighlightState {
                on_popup_item: true,
                in_submenu: false
            }
        );
        hilite(unsafe { GetSubMenu(view, 0) }, 3);
        assert_eq!(
            highlight_state(view),
            HighlightState {
                on_popup_item: true,
                in_submenu: true
            }
        );
    }

    #[test]
    fn language_submenu_lists_every_language_and_checks_the_active_one() {
        use super::{MenuBar, set_checked_language};
        use crate::document::Language;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND, MF_CHECKED};
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let view = bar.dropdown(crate::window::menu_band::VIEW_MENU_INDEX);
        let state = |language| unsafe {
            GetMenuState(view, CommandId::for_language(language) as u32, MF_BYCOMMAND)
        };
        for row in crate::languages::LANGUAGES.iter() {
            assert_ne!(
                state(row.language),
                u32::MAX,
                "{} missing from the menu",
                row.name
            );
        }

        set_checked_language(view, Language::Xml);
        assert_ne!(state(Language::Xml) & MF_CHECKED, 0);
        assert_eq!(state(Language::Json) & MF_CHECKED, 0);

        set_checked_language(view, Language::Json);
        assert_eq!(state(Language::Xml) & MF_CHECKED, 0);
        assert_ne!(state(Language::Json) & MF_CHECKED, 0);
    }

    #[test]
    fn preview_entries_gray_out_and_re_enable() {
        use super::{MenuBar, set_markdown_preview_enabled};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND, MF_GRAYED};
        let bar = MenuBar::create(&crate::window::keymap::Keymap::defaults()).unwrap();
        let view = bar.dropdown(crate::window::menu_band::VIEW_MENU_INDEX);
        set_markdown_preview_enabled(view, false);
        let state =
            unsafe { GetMenuState(view, CommandId::MarkdownPreviewSide as u32, MF_BYCOMMAND) };
        assert_ne!(state & MF_GRAYED, 0);
        set_markdown_preview_enabled(view, true);
        let state =
            unsafe { GetMenuState(view, CommandId::MarkdownPreviewSide as u32, MF_BYCOMMAND) };
        assert_eq!(state & MF_GRAYED, 0);
    }
}
