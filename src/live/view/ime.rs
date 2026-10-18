//! IME composition in the Live view (phase 1 plan, Task 10): the IME draws its own composition
//! and candidate windows, placed at the caret here. The result arrives as `WM_IME_CHAR`, which
//! `DefWindowProcW` turns into the `WM_CHAR`s the view forwards to Scintilla, so Scintilla's own
//! IME code never runs. `imm32.dll` is reached through `GetProcAddress` on the first composition,
//! like Direct2D, so nothing new enters FastPad.exe's import table.

use super::*;
use crate::platform::OwnedModule;
use crate::preview::dwrite::{load_system_library, resolve};
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::LOGFONTW;
use windows_sys::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_EXCLUDE, CFS_POINT, COMPOSITIONFORM, HIMC,
};

type GetContextFn = unsafe extern "system" fn(HWND) -> HIMC;
type ReleaseContextFn = unsafe extern "system" fn(HWND, HIMC) -> i32;
type SetCompositionWindowFn = unsafe extern "system" fn(HIMC, *const COMPOSITIONFORM) -> i32;
type SetCandidateWindowFn = unsafe extern "system" fn(HIMC, *const CANDIDATEFORM) -> i32;
type SetCompositionFontFn = unsafe extern "system" fn(HIMC, *const LOGFONTW) -> i32;

struct Imm {
    get_context: GetContextFn,
    release_context: ReleaseContextFn,
    set_composition_window: SetCompositionWindowFn,
    set_candidate_window: SetCandidateWindowFn,
    set_composition_font: SetCompositionFontFn,
    /// Kept loaded for the process's life; the pointers above live in it.
    _module: OwnedModule,
}

// SAFETY: the function pointers and the module handle are plain values valid on every thread.
unsafe impl Send for Imm {}
unsafe impl Sync for Imm {}

fn imm() -> Option<&'static Imm> {
    static IMM: OnceLock<Option<Imm>> = OnceLock::new();
    IMM.get_or_init(|| {
        let module = load_system_library("imm32.dll").ok()?;
        // SAFETY: each type is the export's documented signature.
        unsafe {
            Some(Imm {
                get_context: resolve(&module, c"ImmGetContext").ok()?,
                release_context: resolve(&module, c"ImmReleaseContext").ok()?,
                set_composition_window: resolve(&module, c"ImmSetCompositionWindow").ok()?,
                set_candidate_window: resolve(&module, c"ImmSetCandidateWindow").ok()?,
                set_composition_font: resolve(&module, c"ImmSetCompositionFontW").ok()?,
                _module: module,
            })
        }
    })
    .as_ref()
}

/// Where the caret is, in client pixels: its top-left and its height; and the font size in
/// pixels.
pub(super) struct CaretPlace {
    pub x: i32,
    pub y: i32,
    pub height: i32,
    pub font_pixels: i32,
    pub family: String,
}

/// `WM_IME_STARTCOMPOSITION`: puts the IME's composition window at the caret in the view's font
/// and keeps its candidate list off the caret's row.
pub(super) fn place(hwnd: HWND, caret: &CaretPlace) {
    let Some(imm) = imm() else {
        return;
    };
    let context = unsafe { (imm.get_context)(hwnd) };
    if context.is_null() {
        return;
    }
    let point = POINT {
        x: caret.x,
        y: caret.y,
    };
    let mut font = LOGFONTW {
        lfHeight: -caret.font_pixels.max(1),
        ..Default::default()
    };
    for (slot, unit) in font
        .lfFaceName
        .iter_mut()
        .take(31)
        .zip(caret.family.encode_utf16())
    {
        *slot = unit;
    }
    let composition = COMPOSITIONFORM {
        dwStyle: CFS_POINT,
        ptCurrentPos: point,
        rcArea: RECT::default(),
    };
    let candidate = CANDIDATEFORM {
        dwIndex: 0,
        dwStyle: CFS_EXCLUDE,
        ptCurrentPos: POINT {
            x: caret.x,
            y: caret.y + caret.height,
        },
        rcArea: RECT {
            left: caret.x,
            top: caret.y,
            right: caret.x + 1,
            bottom: caret.y + caret.height,
        },
    };
    unsafe {
        (imm.set_composition_font)(context, &font);
        (imm.set_composition_window)(context, &composition);
        (imm.set_candidate_window)(context, &candidate);
        (imm.release_context)(hwnd, context);
    }
}
