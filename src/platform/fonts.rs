//! The installed font families, for the Settings dialog's Font dropdown (settings dialog spec
//! §3.5). Enumerated each time the dialog opens, never at startup.

use windows_sys::Win32::Foundation::LPARAM;
use windows_sys::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, FIXED_PITCH, GetDC, LOGFONTW, ReleaseDC, TEXTMETRICW,
};

/// One installed family, as GDI enumerates it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFamily {
    pub name: String,
    pub fixed_pitch: bool,
}

/// Every family GDI lists for `DEFAULT_CHARSET`, once per charset it supports: unsorted and with
/// duplicates, which `dropdown_names` removes.
pub fn installed_font_families() -> Vec<FontFamily> {
    let mut families = Vec::new();
    let query = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    // SAFETY: the enumeration is synchronous, so the `&raw mut families` pointer it hands
    // `collect_family` outlives every callback, and nothing else touches `families` meanwhile.
    // The screen DC is released on the only path out after `GetDC` succeeds.
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return families;
        }
        EnumFontFamiliesExW(
            dc,
            &query,
            Some(collect_family),
            (&raw mut families) as LPARAM,
            0,
        );
        ReleaseDC(std::ptr::null_mut(), dc);
    }
    families
}

unsafe extern "system" fn collect_family(
    logfont: *const LOGFONTW,
    _metrics: *const TEXTMETRICW,
    _font_type: u32,
    lparam: LPARAM,
) -> i32 {
    // SAFETY: `lparam` is the `Vec` `installed_font_families` passed, alive for the whole call.
    let families = unsafe { &mut *(lparam as *mut Vec<FontFamily>) };
    let Some(logfont) = (unsafe { logfont.as_ref() }) else {
        return 1;
    };
    let face = &logfont.lfFaceName;
    let length = face.iter().position(|&c| c == 0).unwrap_or(face.len());
    families.push(FontFamily {
        name: String::from_utf16_lossy(&face[..length]),
        fixed_pitch: logfont.lfPitchAndFamily & 0x03 == FIXED_PITCH,
    });
    1
}

/// The dropdown's font names: fixed-pitch families first, then the rest, each group sorted by
/// name ignoring case. Empty names, duplicates and vertical (`@`) families are dropped.
/// `current` is listed first when no family has its name, so the dropdown can still show it.
pub fn dropdown_names(families: Vec<FontFamily>, current: &str) -> Vec<String> {
    let mut families = families
        .into_iter()
        .filter(|family| !family.name.is_empty() && !family.name.starts_with('@'))
        .collect::<Vec<_>>();
    families.sort_by(|a, b| {
        b.fixed_pitch
            .cmp(&a.fixed_pitch)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut seen = std::collections::HashSet::new();
    let mut names = families
        .into_iter()
        .filter(|family| seen.insert(family.name.to_lowercase()))
        .map(|family| family.name)
        .collect::<Vec<_>>();
    if !current.is_empty() && !names.iter().any(|name| name.eq_ignore_ascii_case(current)) {
        names.insert(0, current.to_owned());
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(name: &str, fixed_pitch: bool) -> FontFamily {
        FontFamily {
            name: name.to_owned(),
            fixed_pitch,
        }
    }

    #[test]
    fn fixed_pitch_families_come_first_sorted_without_duplicates_or_vertical_fonts() {
        // Break caught: a code font buried among hundreds of proportional ones, the same face
        // listed once per charset, or "@MS Gothic" (a vertical font) offered for an editor.
        let names = dropdown_names(
            vec![
                family("Segoe UI", false),
                family("consolas", true),
                family("@MS Gothic", true),
                family("Arial", false),
                family("Cascadia Mono", true),
                family("Arial", false),
                family("", false),
            ],
            "Consolas",
        );
        assert_eq!(names, ["Cascadia Mono", "consolas", "Arial", "Segoe UI"]);
    }

    #[test]
    fn a_face_that_is_not_installed_is_listed_first() {
        // Break caught: a hand-edited font_face the dropdown cannot show, so it reads as blank.
        let names = dropdown_names(vec![family("Consolas", true)], "Iosevka");
        assert_eq!(names, ["Iosevka", "Consolas"]);
        assert_eq!(dropdown_names(Vec::new(), ""), Vec::<String>::new());
    }

    #[test]
    fn this_machine_lists_consolas_as_fixed_pitch() {
        // Break caught: an enumeration that returns nothing, or reads the pitch bits wrong.
        let families = installed_font_families();
        assert!(
            families
                .iter()
                .any(|family| family.name == "Consolas" && family.fixed_pitch),
            "{} families, no fixed-pitch Consolas",
            families.len()
        );
    }
}
