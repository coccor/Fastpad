//! Windows' text-size setting (Settings > Accessibility > Text size), 100 to 225 percent, and the
//! helper that applies it. The value is read lazily the first time something asks for it, never
//! at startup, and cached; `refresh` re-reads it when Windows says settings changed.

use std::sync::atomic::{AtomicU32, Ordering};

use super::metrics::scale;

const MIN: u32 = 100;
const MAX: u32 = 225;
/// `0` means "not read yet".
static FACTOR: AtomicU32 = AtomicU32::new(0);

/// The percentage for a raw registry value: missing is 100, anything else is clamped to 100..=225.
pub(crate) fn parse_factor(raw: Option<u32>) -> u32 {
    raw.map_or(MIN, |value| value.clamp(MIN, MAX))
}

fn read_registry() -> u32 {
    use std::ffi::c_void;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let subkey = crate::platform::wide_null(r"Software\Microsoft\Accessibility");
    let value_name = crate::platform::wide_null("TextScaleFactor");
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut data).cast::<c_void>(),
            &mut size,
        )
    };
    parse_factor((status == 0).then_some(data))
}

/// The text-size percentage, 100 to 225. The first call reads the registry.
pub(crate) fn factor() -> u32 {
    match FACTOR.load(Ordering::Relaxed) {
        0 => {
            let value = read_registry();
            FACTOR.store(value, Ordering::Relaxed);
            value
        }
        value => value,
    }
}

/// Re-reads the setting and returns whether it changed.
pub(crate) fn refresh() -> bool {
    let before = factor();
    let after = read_registry();
    FACTOR.store(after, Ordering::Relaxed);
    before != after
}

/// `value` (pixels at 96 DPI) scaled to `dpi`, then by the text-size factor, rounded half up.
/// At 100 percent it equals `scale`.
pub(crate) fn scale_text(value: i32, dpi: u32) -> i32 {
    let scaled = i64::from(scale(value, dpi));
    ((scaled * i64::from(factor()) + 50) / 100) as i32
}

/// Sets the cached factor, for tests.
#[cfg(test)]
pub(crate) fn set_factor_for_test(percent: u32) {
    FACTOR.store(percent.clamp(MIN, MAX), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factor_defaults_to_100_and_clamps_to_the_supported_range() {
        // Break caught: a missing value scaling text to 0, or a hostile value blowing up the UI.
        assert_eq!(parse_factor(None), 100);
        assert_eq!(parse_factor(Some(0)), 100);
        assert_eq!(parse_factor(Some(90)), 100);
        assert_eq!(parse_factor(Some(100)), 100);
        assert_eq!(parse_factor(Some(150)), 150);
        assert_eq!(parse_factor(Some(225)), 225);
        assert_eq!(parse_factor(Some(300)), 225);
        assert_eq!(parse_factor(Some(u32::MAX)), 225);
    }

    #[test]
    fn scale_text_equals_scale_at_100_percent() {
        set_factor_for_test(100);
        for dpi in [0, 96, 120, 144, 192] {
            for value in [1, 13, 22, 26, 38, 46] {
                assert_eq!(
                    scale_text(value, dpi),
                    scale(value, dpi),
                    "{value} at {dpi}"
                );
            }
        }
    }

    #[test]
    fn scale_text_multiplies_the_dpi_scaled_value_by_the_factor_rounding_half_up() {
        set_factor_for_test(150);
        assert_eq!(scale_text(13, 96), 20); // 13 * 1.5 = 19.5
        assert_eq!(scale_text(26, 96), 39);
        set_factor_for_test(225);
        assert_eq!(scale_text(13, 96), 29); // 13 * 2.25 = 29.25
        assert_eq!(scale_text(26, 96), 59); // 58.5
        assert_eq!(scale_text(13, 192), 59); // 26 * 2.25 = 58.5
        set_factor_for_test(100);
    }
}
