//! The local date and time, for daily notes and template placeholders.

use crate::library::dates::{Date, Time};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// Now, in the PC's time zone.
pub fn local_now() -> (Date, Time) {
    let mut now = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    let date = Date::new(now.wYear, now.wMonth as u8, now.wDay as u8)
        .unwrap_or_else(|| Date::new(2000, 1, 1).expect("a real date"));
    let time = Time {
        hour: now.wHour as u8,
        minute: now.wMinute as u8,
    };
    (date, time)
}

/// The second of the minute now, for names that need it (a pasted image's).
pub fn local_second() -> u8 {
    let mut now = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    now.wSecond as u8
}
