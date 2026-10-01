//! The chrome's font faces. Windows 11 gets Segoe UI Variable and Segoe Fluent Icons (the icon face
//! keeps Segoe MDL2 Assets' codepoints); everything else keeps Segoe UI and Segoe MDL2 Assets.
//! The set is chosen lazily the first time a font is made and cached, so startup does no work.

use std::sync::OnceLock;

/// The faces the chrome draws with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Faces {
    /// Every text style except `Title`.
    pub(crate) text: &'static str,
    /// `Title`.
    pub(crate) display: &'static str,
    /// Glyph fonts.
    pub(crate) icons: &'static str,
}

const LEGACY: Faces = Faces {
    text: "Segoe UI",
    display: "Segoe UI",
    icons: "Segoe MDL2 Assets",
};
const WINDOWS_11: Faces = Faces {
    text: "Segoe UI Variable Text",
    display: "Segoe UI Variable Display",
    icons: "Segoe Fluent Icons",
};
/// The first Windows 11 build.
const WINDOWS_11_BUILD: u32 = 22000;

/// Pure: the Windows 11 faces on build 22000 or later when `probe` accepts the text face, the
/// legacy faces otherwise. `probe` is only called on a Windows 11 build.
pub(crate) fn choose(build: Option<u32>, probe: impl Fn(&str) -> bool) -> Faces {
    match build {
        Some(build) if build >= WINDOWS_11_BUILD && probe(WINDOWS_11.text) => WINDOWS_11,
        _ => LEGACY,
    }
}

/// The set for this process, computed on first use.
#[allow(dead_code, reason = "used by the font sites in the next tasks")]
pub(crate) fn current() -> Faces {
    static CURRENT: OnceLock<Faces> = OnceLock::new();
    *CURRENT.get_or_init(|| choose(os_build(), face_is_mapped))
}

/// The OS build from `RtlGetVersion` (in ntdll, which every process has loaded), or `None` when it
/// cannot be reached.
fn os_build() -> Option<u32> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    #[repr(C)]
    struct VersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    type RtlGetVersion = unsafe extern "system" fn(*mut VersionInfo) -> i32;
    unsafe {
        let ntdll = GetModuleHandleW(crate::platform::wide_null("ntdll.dll").as_ptr());
        if ntdll.is_null() {
            return None;
        }
        let function = GetProcAddress(ntdll, c"RtlGetVersion".as_ptr().cast())?;
        let function: RtlGetVersion = std::mem::transmute(function);
        let mut info: VersionInfo = std::mem::zeroed();
        info.size = std::mem::size_of::<VersionInfo>() as u32;
        (function(&mut info) == 0).then_some(info.build)
    }
}

/// Whether GDI maps `face` to itself: a font made with it, selected into the screen DC, reports
/// the same face name. A missing face is substituted by GDI and reports another name.
fn face_is_mapped(face: &str) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, FW_NORMAL, GetDC, GetTextFaceW, ReleaseDC, SelectObject,
    };
    let font = crate::window::titlebar::create_ui_font(13, face, FW_NORMAL as i32, false);
    if font.is_null() {
        return false;
    }
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            DeleteObject(font);
            return false;
        }
        let previous = SelectObject(dc, font);
        let mut name = [0u16; 64];
        let written = GetTextFaceW(dc, name.len() as i32, name.as_mut_ptr());
        SelectObject(dc, previous);
        ReleaseDC(std::ptr::null_mut(), dc);
        DeleteObject(font);
        let length = usize::try_from(written).unwrap_or(0).saturating_sub(1);
        length > 0
            && String::from_utf16_lossy(&name[..length.min(name.len())]).eq_ignore_ascii_case(face)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: Faces = Faces {
        text: "Segoe UI",
        display: "Segoe UI",
        icons: "Segoe MDL2 Assets",
    };
    const WINDOWS_11: Faces = Faces {
        text: "Segoe UI Variable Text",
        display: "Segoe UI Variable Display",
        icons: "Segoe Fluent Icons",
    };

    #[test]
    fn windows_10_keeps_the_legacy_faces_and_never_probes() {
        // Break caught: Windows 10 running the probe (startup cost) or getting Windows 11 faces.
        let probe = |_: &str| -> bool { panic!("Windows 10 must not probe") };
        assert_eq!(choose(Some(19045), probe), LEGACY);
        assert_eq!(choose(Some(21999), probe), LEGACY);
    }

    #[test]
    fn windows_11_uses_the_variable_faces_when_the_probe_accepts() {
        assert_eq!(choose(Some(22000), |_| true), WINDOWS_11);
        assert_eq!(choose(Some(26100), |_| true), WINDOWS_11);
    }

    #[test]
    fn a_rejected_probe_falls_back_to_the_legacy_faces() {
        // Break caught: a Windows 11 machine without the face rendering a substituted font.
        assert_eq!(choose(Some(22621), |_| false), LEGACY);
    }

    #[test]
    fn an_unknown_build_falls_back_to_the_legacy_faces() {
        let probe = |_: &str| -> bool { panic!("an unknown build must not probe") };
        assert_eq!(choose(None, probe), LEGACY);
    }

    #[test]
    fn the_current_faces_are_a_valid_set_and_stable() {
        let first = current();
        assert!(!first.text.is_empty() && !first.display.is_empty() && !first.icons.is_empty());
        assert_eq!(current(), first);
    }
}
