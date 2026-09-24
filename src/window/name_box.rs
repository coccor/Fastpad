//! The inline name box: one text field with Save and Browse… buttons, shown above the editor for
//! a first save and for renaming a note. Enter saves, Esc cancels, Tab moves between the field
//! and the buttons.

use crate::document::DocumentId;
use crate::platform::{last_error, wide_null};
use crate::window::palette::Palette;
use crate::window::panel::{
    create_child, create_child_with_id, create_panel, fill, inset, scale, text_height,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, EndPaint, GetDC, GetTextExtentPoint32W, HBRUSH, HDC, HFONT,
    InvalidateRect, PAINTSTRUCT, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RedrawWindow,
    ReleaseDC, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::EM_SETSEL;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, SetFocus, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetParent, GetWindowTextLengthW,
    GetWindowTextW, HWND_TOP, IsWindowVisible, MoveWindow, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow, WM_CHAR, WM_KEYDOWN,
    WM_KILLFOCUS, WM_NCDESTROY, WM_SETFOCUS, WM_SETFONT, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

pub(crate) const NAME_BOX_SAVE_ID: u16 = 1;
pub(crate) const NAME_BOX_BROWSE_ID: u16 = 2;
const NAME_BOX_HOOK_ID: usize = 0x4650_4E42;

const BOX_HEIGHT_AT_96_DPI: i32 = 36;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const PADDING_AT_96_DPI: i32 = 6;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const BUTTON_WIDTH_AT_96_DPI: i32 = 72;

/// Height of the box, reserved above the editor whenever it's visible.
pub(crate) const fn name_box_height(dpi: u32) -> i32 {
    scale(BOX_HEIGHT_AT_96_DPI, dpi)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NamePurpose {
    FirstSave(DocumentId),
    RenameNote(DocumentId),
    /// A new folder in this folder, relative to the notebook (empty for its root).
    NewFolder(PathBuf),
}

impl NamePurpose {
    /// The tab this purpose acts on: the box closes when that tab goes away.
    pub(crate) fn document(&self) -> Option<DocumentId> {
        match self {
            Self::FirstSave(id) | Self::RenameNote(id) => Some(*id),
            Self::NewFolder(_) => None,
        }
    }

    /// For a folder box, the folder that must stay in the tree for the box to stay open: a new
    /// folder's parent (empty for the notebook root). A folder box belongs to no tab.
    pub(crate) fn folder(&self) -> Option<&Path> {
        match self {
            Self::FirstSave(_) | Self::RenameNote(_) => None,
            Self::NewFolder(parent) => Some(parent),
        }
    }
}

/// The box's parts in box coordinates: the field (and its borderless `Edit`), the suffix or
/// error text beside it, and the buttons at the right end.
#[derive(Clone, Copy)]
struct BoxLayout {
    field: RECT,
    edit: RECT,
    note: RECT,
    save: RECT,
    browse: Option<RECT>,
}

fn box_layout(
    width: i32,
    dpi: u32,
    text_height: i32,
    note_width: i32,
    show_browse: bool,
) -> BoxLayout {
    let padding = scale(PADDING_AT_96_DPI, dpi);
    let field_height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    let button_width = scale(BUTTON_WIDTH_AT_96_DPI, dpi);
    let top = (name_box_height(dpi) - 1 - field_height) / 2;
    let bottom = top + field_height;
    let button = |right: i32| RECT {
        left: (right - button_width).max(0),
        top,
        right: right.max(0),
        bottom,
    };
    let browse = show_browse.then(|| button(width - padding));
    let save = button(browse.map_or(width - padding, |browse| browse.left - padding));
    // The note never takes more than half of what is left, so the field stays usable.
    let room = (save.left - 3 * padding).max(0);
    let note_width = note_width.clamp(0, room / 2);
    let note_right = save.left - padding;
    let note = RECT {
        left: (note_right - note_width).max(0),
        top,
        right: note_right.max(0),
        bottom,
    };
    let field_right = if note_width > 0 {
        note.left - padding
    } else {
        save.left - padding
    };
    let field = RECT {
        left: padding,
        top,
        right: field_right.max(padding),
        bottom,
    };
    let text_height = text_height.clamp(1, (field_height - 2).max(1));
    let edit_top = top + (field_height - text_height) / 2;
    BoxLayout {
        field,
        edit: RECT {
            left: field.left + inset_x,
            top: edit_top,
            right: (field.right - inset_x).max(field.left + inset_x),
            bottom: edit_top + text_height,
        },
        note,
        save,
        browse,
    }
}

#[derive(Debug)]
pub(crate) struct NameBox {
    panel: HWND,
    edit: HWND,
    save: HWND,
    browse: HWND,
    purpose: Option<NamePurpose>,
    /// Painted after the field, e.g. "in Notes". Replaced by `error` while one is set.
    suffix: String,
    error: Option<String>,
    show_browse: bool,
    visible: bool,
    colors: Palette,
    field_brush: HBRUSH,
    strip_brush: HBRUSH,
}

impl NameBox {
    pub(crate) fn create(parent: HWND) -> crate::Result<Self> {
        let panel = create_panel(parent)?;
        let controls = (|| {
            // The label before the field is its accessible name.
            let label = create_child(panel, &wide_null("Static"), WS_CHILD)?;
            set_control_text(label, "Name");
            let edit = create_child_with_id(
                panel,
                &wide_null("Edit"),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | (ES_AUTOHSCROLL as u32),
                0,
            )?;
            let button = |label: &str, id: u16| {
                let hwnd = create_child_with_id(
                    panel,
                    &wide_null("BUTTON"),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | (BS_PUSHBUTTON as u32),
                    id,
                )?;
                set_control_text(hwnd, label);
                crate::Result::Ok(hwnd)
            };
            let save = button("Save", NAME_BOX_SAVE_ID)?;
            let browse = button("Browse\u{2026}", NAME_BOX_BROWSE_ID)?;
            let controls = [edit, save, browse];
            for (role, hwnd) in [Role::Edit, Role::Save, Role::Browse]
                .into_iter()
                .zip(controls)
            {
                install_hook(hwnd, parent, role, controls)?;
            }
            Ok((edit, save, browse))
        })();
        let (edit, save, browse) = match controls {
            Ok(controls) => controls,
            Err(error) => {
                unsafe {
                    DestroyWindow(panel);
                }
                return Err(error);
            }
        };
        let colors = Palette::neutral();
        Ok(Self {
            panel,
            edit,
            save,
            browse,
            purpose: None,
            suffix: String::new(),
            error: None,
            show_browse: true,
            visible: false,
            colors,
            field_brush: unsafe { CreateSolidBrush(colors.editor_background) },
            strip_brush: unsafe { CreateSolidBrush(colors.strip_background) },
        })
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.visible
    }

    pub(crate) fn owns(&self, hwnd: HWND) -> bool {
        !hwnd.is_null()
            && (hwnd == self.panel || hwnd == self.edit || hwnd == self.save || hwnd == self.browse)
    }

    pub(crate) fn text(&self) -> String {
        control_text(self.edit)
    }

    pub(crate) fn purpose(&self) -> Option<&NamePurpose> {
        self.purpose.as_ref()
    }

    #[cfg_attr(not(test), allow(dead_code, reason = "read by the window tests"))]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn show(
        &mut self,
        purpose: NamePurpose,
        text: &str,
        suffix: String,
        browse: bool,
        colors: Palette,
    ) {
        self.set_colors(colors);
        self.purpose = Some(purpose);
        self.suffix = suffix;
        self.show_browse = browse;
        self.error = None;
        set_control_text(self.edit, text);
        self.visible = true;
        unsafe {
            ShowWindow(self.browse, if browse { SW_SHOWNA } else { SW_HIDE });
        }
    }

    pub(crate) fn hide(&mut self) {
        self.visible = false;
        self.purpose = None;
        self.error = None;
        unsafe {
            ShowWindow(self.panel, SW_HIDE);
        }
    }

    /// Shows `error` in place of the suffix, or the suffix again for `None`. Only stores it: the
    /// caller lays the box out again (the note's width changes) once it holds no `App` borrow.
    pub(crate) fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    /// Recolors for a theme change; the caller repaints with `invalidate`.
    pub(crate) fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        unsafe {
            DeleteObject(self.field_brush);
            DeleteObject(self.strip_brush);
            self.field_brush = CreateSolidBrush(colors.editor_background);
            self.strip_brush = CreateSolidBrush(colors.strip_background);
        }
        self.colors = colors;
    }

    pub(crate) fn invalidate(&self) {
        unsafe {
            RedrawWindow(
                self.panel,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
            );
        }
    }

    /// Places the box across `width` from `left`, at `top`, and its controls inside it.
    pub(crate) fn layout(&self, left: i32, width: i32, top: i32, dpi: u32, font: HFONT) {
        if !self.visible {
            return;
        }
        unsafe {
            if !font.is_null() {
                for control in [self.edit, self.save, self.browse] {
                    SendMessageW(control, WM_SETFONT, font as WPARAM, 0);
                }
            }
        }
        let layout = box_layout(
            width,
            dpi,
            text_height(self.edit, font),
            self.note_width(font),
            self.show_browse,
        );
        let move_to = |hwnd, rect: RECT| unsafe {
            MoveWindow(
                hwnd,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                1,
            );
        };
        move_to(self.edit, layout.edit);
        move_to(self.save, layout.save);
        if let Some(browse) = layout.browse {
            move_to(self.browse, browse);
        }
        unsafe {
            SetWindowPos(
                self.panel,
                HWND_TOP,
                left,
                top,
                width.max(0),
                name_box_height(dpi),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    /// Focuses the field with the name selected up to its extension, so typing replaces the stem.
    /// A folder name has no extension: it is selected in full.
    pub(crate) fn focus(&self) {
        let text = self.text();
        let folder = self
            .purpose
            .as_ref()
            .is_some_and(|purpose| purpose.folder().is_some());
        let end = if folder {
            -1
        } else {
            text.rfind('.')
                .map_or(-1, |dot| text[..dot].encode_utf16().count() as isize)
        };
        unsafe {
            SetFocus(self.edit);
            SendMessageW(self.edit, EM_SETSEL, 0, end);
        }
    }

    /// `WM_CTLCOLOREDIT` for the field.
    pub(crate) fn control_color(&self, dc: HDC) -> HBRUSH {
        unsafe {
            SetTextColor(dc, self.colors.editor_foreground);
            SetBkColor(dc, self.colors.editor_background);
        }
        self.field_brush
    }

    /// `WM_CTLCOLORBTN` for the buttons: the strip color around their rounded corners.
    pub(crate) fn button_color(&self, dc: HDC) -> HBRUSH {
        unsafe {
            SetTextColor(dc, self.colors.strip_foreground);
            SetBkColor(dc, self.colors.strip_background);
        }
        self.strip_brush
    }

    fn note_text(&self) -> &str {
        self.error.as_deref().unwrap_or(&self.suffix)
    }

    /// The pixel width of the suffix or error in `font`.
    fn note_width(&self, font: HFONT) -> i32 {
        let text = self.note_text().encode_utf16().collect::<Vec<_>>();
        if text.is_empty() {
            return 0;
        }
        unsafe {
            let dc = GetDC(self.panel);
            if dc.is_null() {
                return 0;
            }
            let previous = (!font.is_null()).then(|| SelectObject(dc, font as _));
            let mut size = SIZE::default();
            let measured = GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
            if let Some(previous) = previous {
                SelectObject(dc, previous);
            }
            ReleaseDC(self.panel, dc);
            if measured != 0 { size.cx } else { 0 }
        }
    }

    /// `WM_PAINT` for the box: strip background, a hairline above the editor, the field's box
    /// (outlined with the accent while it has the focus), and the suffix or error beside it.
    pub(crate) fn paint_panel(&self, panel: HWND, font: HFONT) {
        let note_width = self.note_width(font);
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(panel, &mut paint) };
        if dc.is_null() {
            return;
        }
        let mut client = RECT::default();
        unsafe {
            GetClientRect(panel, &mut client);
        }
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
        let colors = self.colors;
        let layout = box_layout(client.right, dpi, 0, note_width, self.show_browse);
        let outline = if unsafe { GetFocus() } == self.edit {
            colors.selection_background
        } else {
            colors.pressed_background
        };
        unsafe {
            fill(dc, client, colors.strip_background);
            fill(
                dc,
                RECT {
                    top: client.bottom - 1,
                    ..client
                },
                colors.pressed_background,
            );
            fill(dc, layout.field, outline);
            fill(dc, inset(layout.field, 1), colors.editor_background);
            let note = self.note_text();
            if !note.is_empty() && !font.is_null() {
                let previous = SelectObject(dc, font as _);
                SetBkMode(dc, TRANSPARENT as i32);
                // The palette has no error color; an error reads in the full text color.
                SetTextColor(
                    dc,
                    if self.error.is_some() {
                        colors.strip_foreground
                    } else {
                        colors.muted_foreground
                    },
                );
                let mut text = note.encode_utf16().collect::<Vec<_>>();
                let mut rect = layout.note;
                DrawTextW(
                    dc,
                    text.as_mut_ptr(),
                    text.len() as i32,
                    &mut rect,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
                SelectObject(dc, previous);
            }
            EndPaint(panel, &paint);
        }
    }

    #[cfg(test)]
    pub(crate) fn edit_hwnd(&self) -> HWND {
        self.edit
    }
}

impl Drop for NameBox {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.field_brush);
            DeleteObject(self.strip_brush);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Edit,
    Save,
    Browse,
}

struct NameBoxHook {
    parent: HWND,
    role: Role,
    /// The field, Save and Browse, in Tab order.
    controls: [HWND; 3],
}

fn install_hook(hwnd: HWND, parent: HWND, role: Role, controls: [HWND; 3]) -> crate::Result<()> {
    let data = Rc::into_raw(Rc::new(NameBoxHook {
        parent,
        role,
        controls,
    })) as usize;
    if unsafe { SetWindowSubclass(hwnd, Some(name_box_proc), NAME_BOX_HOOK_ID, data) } == 0 {
        unsafe {
            drop(Rc::from_raw(data as *const NameBoxHook));
        }
        return Err(last_error());
    }
    Ok(())
}

unsafe extern "system" fn name_box_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> isize {
    let raw = ref_data as *const NameBoxHook;
    unsafe {
        Rc::increment_strong_count(raw);
    }
    let hook = unsafe { Rc::from_raw(raw) };
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(name_box_proc), NAME_BOX_HOOK_ID);
            Rc::decrement_strong_count(raw);
        }
    }
    // A single-line Edit beeps at Enter, Escape and Tab characters; all are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b | 0x09) {
        return 0;
    }
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    match message {
        WM_KEYDOWN => {
            let key = wparam as u16;
            if key == VK_RETURN {
                match hook.role {
                    Role::Edit | Role::Save => {
                        crate::window::library_host::name_box_submit(hook.parent)
                    }
                    Role::Browse => crate::window::library_host::name_box_browse(hook.parent),
                }
            } else if key == VK_ESCAPE {
                crate::window::library_host::close_name_box(hook.parent);
            } else if key == VK_TAB {
                let backward = unsafe { GetAsyncKeyState(VK_SHIFT as i32) } < 0;
                let next = next_in_tab_order(&hook, backward);
                unsafe {
                    SetFocus(next);
                }
            }
        }
        // The focused field carries the accent outline the box paints.
        WM_SETFOCUS | WM_KILLFOCUS if hook.role == Role::Edit => unsafe {
            InvalidateRect(GetParent(hwnd), std::ptr::null(), 0);
        },
        _ => {}
    }
    result
}

/// Field → Save → Browse (while shown) → field, or the reverse.
fn next_in_tab_order(hook: &NameBoxHook, backward: bool) -> HWND {
    let [edit, save, browse] = hook.controls;
    let browse_shown = unsafe { IsWindowVisible(browse) } != 0;
    match (hook.role, backward) {
        (Role::Edit, false) | (Role::Browse, true) => save,
        (Role::Save, false) if browse_shown => browse,
        (Role::Save, false) | (Role::Browse, false) => edit,
        (Role::Save, true) => edit,
        (Role::Edit, true) if browse_shown => browse,
        (Role::Edit, true) => save,
    }
}

fn control_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(copied.max(0) as usize);
        String::from_utf16_lossy(&buffer)
    }
}

fn set_control_text(hwnd: HWND, text: &str) {
    let wide = wide_null(text);
    unsafe {
        SetWindowTextW(hwnd, wide.as_ptr());
    }
}

#[cfg(test)]
mod tests {
    use super::{box_layout, name_box_height};

    #[test]
    fn the_field_note_and_buttons_sit_side_by_side_inside_the_box() {
        // Break caught: the suffix drawn over the field or the buttons, or a button hanging off
        // the right edge.
        for dpi in [96, 144] {
            let height = name_box_height(dpi);
            let layout = box_layout(800, dpi, 16, 60, true);
            let browse = layout.browse.unwrap();
            assert!(layout.field.right < layout.note.left);
            assert!(layout.note.right < layout.save.left);
            assert!(layout.save.right < browse.left);
            assert!(browse.right < 800);
            assert!(layout.field.top > 0 && layout.field.bottom < height - 1);
            assert_eq!(
                layout.save.right - layout.save.left,
                browse.right - browse.left
            );

            let without = box_layout(800, dpi, 16, 0, false);
            assert!(without.browse.is_none());
            assert!(without.field.right < without.save.left);
            assert!(without.save.right < 800);
        }
    }

    #[test]
    fn a_long_note_never_takes_more_than_half_the_room_left_for_the_field() {
        let layout = box_layout(400, 96, 16, 10_000, true);
        assert!(layout.field.right - layout.field.left >= layout.note.right - layout.note.left);
    }
}
