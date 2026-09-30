//! Probing a launched FastPad: waiting for its window, Scintilla and events, typing and
//! verifying the benchmark character, and measuring its private working set.

use super::*;

#[cfg(windows)]
pub(super) fn wait_for_main_window(
    guard: &ChildGuard,
) -> Result<windows_sys::Win32::Foundation::HWND, String> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId};

    struct Search {
        pid: u32,
        hwnd: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> windows_sys::core::BOOL {
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut pid = 0_u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let mut class = [0_u16; 64];
        let class_len = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
                hwnd,
                class.as_mut_ptr(),
                class.len() as i32,
            )
        };
        let is_main = class_len > 0
            && is_fastpad_main_window_class(&String::from_utf16_lossy(
                &class[..class_len as usize],
            ));
        if pid == search.pid && is_main {
            search.hwnd = hwnd;
            0
        } else {
            1
        }
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let mut search = Search {
            pid: guard.pid,
            hwnd: std::ptr::null_mut(),
        };
        unsafe {
            EnumWindows(Some(visit), (&mut search as *mut Search) as LPARAM);
        }
        if !search.hwnd.is_null() {
            return Ok(search.hwnd);
        }
        if unsafe { WaitForSingleObject(guard.process.as_raw(), 0) } == WAIT_OBJECT_0 {
            return Err("FastPad exited before creating its main window".to_owned());
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for FastPad main window".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(windows)]
pub(super) fn wait_for_scintilla(
    parent: windows_sys::Win32::Foundation::HWND,
    guard: &ChildGuard,
) -> Result<windows_sys::Win32::Foundation::HWND, String> {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowExW;
    let group_class = fastpad::platform::wide_null("FastPadEditorGroup");
    let class = fastpad::platform::wide_null("Scintilla");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        // The editor sits in the editor group window, a child of the main window.
        let group = unsafe {
            FindWindowExW(
                parent,
                std::ptr::null_mut(),
                group_class.as_ptr(),
                std::ptr::null(),
            )
        };
        let hwnd = if group.is_null() {
            std::ptr::null_mut()
        } else {
            unsafe {
                FindWindowExW(
                    group,
                    std::ptr::null_mut(),
                    class.as_ptr(),
                    std::ptr::null(),
                )
            }
        };
        if !hwnd.is_null() {
            return Ok(hwnd);
        }
        if unsafe { WaitForSingleObject(guard.process.as_raw(), 0) } == WAIT_OBJECT_0 {
            return Err("FastPad exited before creating Scintilla".to_owned());
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for Scintilla".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(windows)]
pub(super) fn send_benchmark_char(
    editor: windows_sys::Win32::Foundation::HWND,
    character: usize,
) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CHAR,
    };
    let mut result = 0_usize;
    if unsafe {
        SendMessageTimeoutW(
            editor,
            WM_CHAR,
            character,
            1,
            SMTO_ABORTIFHUNG,
            5_000,
            &mut result,
        )
    } == 0
    {
        Err(fastpad::platform::last_error().to_string())
    } else {
        Ok(())
    }
}

pub(super) fn verify_benchmark_utf8(
    length: usize,
    mut get_byte: impl FnMut(usize) -> Result<u8, String>,
) -> Result<(), String> {
    let expected = "\u{E000}".as_bytes();
    if length != expected.len() {
        return Err(format!(
            "Scintilla benchmark text length was {length}, expected {}",
            expected.len()
        ));
    }
    for (index, expected_byte) in expected.iter().copied().enumerate() {
        let actual = get_byte(index)?;
        if actual != expected_byte {
            return Err(format!(
                "Scintilla benchmark byte {index} was {actual:#04x}, expected {expected_byte:#04x}"
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn wait_for_event(
    event: windows_sys::Win32::Foundation::HANDLE,
    guard: &ChildGuard,
) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match unsafe { WaitForSingleObject(event, 10) } {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT => {}
            _ => return Err(fastpad::platform::last_error().to_string()),
        }
        if unsafe { WaitForSingleObject(guard.process.as_raw(), 0) } == WAIT_OBJECT_0 {
            return Err("FastPad exited before signaling rendered input".to_owned());
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for rendered input".to_owned());
        }
    }
}

#[cfg(windows)]
pub(super) fn verify_benchmark_char(
    editor: windows_sys::Win32::Foundation::HWND,
) -> Result<(), String> {
    use fastpad::editor::scintilla_constants::SCI_GETLENGTH;
    const SCI_GETCHARAT: u32 = 2007;
    let length = send_scintilla_scalar(editor, SCI_GETLENGTH, 0)? as isize;
    if length < 0 {
        return Err("Scintilla did not retain the benchmark character".to_owned());
    }
    verify_benchmark_utf8(length as usize, |index| {
        send_scintilla_scalar(editor, SCI_GETCHARAT, index).map(|value| value as u8)
    })
}

#[cfg(windows)]
pub(super) fn send_scintilla_scalar(
    editor: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
) -> Result<usize, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, SendMessageTimeoutW,
    };
    let mut result = 0_usize;
    if unsafe {
        SendMessageTimeoutW(
            editor,
            message,
            wparam,
            0,
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            5_000,
            &mut result,
        )
    } == 0
    {
        Err(fastpad::platform::last_error().to_string())
    } else {
        Ok(result)
    }
}

#[cfg(windows)]
pub(super) fn wait_for_fully_ready(
    view: *const u8,
    guard: &ChildGuard,
) -> Result<BenchmarkRecord, String> {
    use fastpad::perf::protocol::read_shared_record;
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(record) = unsafe { read_shared_record(view) }.map_err(str::to_owned)?
            && record.fully_ready_us != 0
        {
            return Ok(record);
        }
        if unsafe { WaitForSingleObject(guard.process.as_raw(), 0) } == WAIT_OBJECT_0 {
            return Err("FastPad exited before reaching FullyReady".to_owned());
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for FullyReady".to_owned());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[cfg(windows)]
pub(super) fn private_working_set(
    process: windows_sys::Win32::Foundation::HANDLE,
) -> Result<u64, String> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    let ex2_value = (unsafe {
        GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        )
    } != 0
        && counters.PrivateWorkingSetSize != 0)
        .then_some(counters.PrivateWorkingSetSize as u64);
    select_private_working_set(ex2_value, || private_working_set_via_page_query(process))
}

pub(super) fn select_private_working_set(
    ex2_value: Option<u64>,
    fallback: impl FnOnce() -> Result<u64, String>,
) -> Result<u64, String> {
    ex2_value.map_or_else(fallback, Ok)
}

pub(super) fn private_bytes_from_working_set_flags(flags: &[usize], page_size: u64) -> u64 {
    const VALID: usize = 1;
    const SHARED: usize = 1 << 15;
    flags
        .iter()
        .filter(|flags| **flags & VALID != 0 && **flags & SHARED == 0)
        .count() as u64
        * page_size
}

#[cfg(windows)]
pub(super) fn private_working_set_via_page_query(
    process: windows_sys::Win32::Foundation::HANDLE,
) -> Result<u64, String> {
    use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, GetLastError};
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQueryEx,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32QueryWorkingSetEx, PSAPI_WORKING_SET_EX_INFORMATION,
    };
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

    const QUERY_BATCH_PAGES: usize = 4096;
    let mut system_info = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system_info) };
    let page_size = u64::from(system_info.dwPageSize);
    if page_size == 0 {
        return Err("GetSystemInfo returned a zero page size".to_owned());
    }

    let maximum_address = system_info.lpMaximumApplicationAddress as usize;
    let mut address = 0_usize;
    let mut private_bytes = 0_u64;
    let mut pages = Vec::with_capacity(QUERY_BATCH_PAGES);

    let query_pages = |pages: &mut Vec<PSAPI_WORKING_SET_EX_INFORMATION>,
                       private_bytes: &mut u64|
     -> Result<(), String> {
        if pages.is_empty() {
            return Ok(());
        }
        let byte_len = pages
            .len()
            .checked_mul(std::mem::size_of::<PSAPI_WORKING_SET_EX_INFORMATION>())
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "working-set query batch exceeded DWORD size".to_owned())?;
        if unsafe { K32QueryWorkingSetEx(process, pages.as_mut_ptr().cast(), byte_len) } == 0 {
            return Err(fastpad::platform::last_error().to_string());
        }
        let flags = pages
            .iter()
            .map(|page| unsafe { page.VirtualAttributes.Flags })
            .collect::<Vec<_>>();
        *private_bytes = private_bytes
            .checked_add(private_bytes_from_working_set_flags(&flags, page_size))
            .ok_or_else(|| "private working-set byte count overflowed".to_owned())?;
        pages.clear();
        Ok(())
    };

    while address < maximum_address {
        let mut information = MEMORY_BASIC_INFORMATION::default();
        let queried = unsafe {
            VirtualQueryEx(
                process,
                address as *const core::ffi::c_void,
                &mut information,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if queried == 0 {
            if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                break;
            }
            return Err(fastpad::platform::last_error().to_string());
        }

        let base = information.BaseAddress as usize;
        let next = base
            .checked_add(information.RegionSize)
            .ok_or_else(|| "virtual-memory region address overflowed".to_owned())?;
        if information.State == MEM_COMMIT {
            let mut page = base;
            while page < next {
                pages.push(PSAPI_WORKING_SET_EX_INFORMATION {
                    VirtualAddress: page as *mut core::ffi::c_void,
                    ..Default::default()
                });
                if pages.len() == QUERY_BATCH_PAGES {
                    query_pages(&mut pages, &mut private_bytes)?;
                }
                page = page
                    .checked_add(page_size as usize)
                    .ok_or_else(|| "virtual page address overflowed".to_owned())?;
            }
        }
        if next <= address {
            return Err("VirtualQueryEx did not advance the address".to_owned());
        }
        address = next;
    }
    query_pages(&mut pages, &mut private_bytes)?;
    Ok(private_bytes)
}
