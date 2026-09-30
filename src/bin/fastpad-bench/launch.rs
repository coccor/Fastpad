//! Launching one diagnostic FastPad run: the inherited handle list, cleanup job, scratch
//! `LOCALAPPDATA`, the diagnostic environment block, and the child process guard.

use super::*;

#[cfg(not(windows))]
pub(super) fn run_once(
    _launch_file: Option<&Path>,
    _notes_folder: Option<&Path>,
    _sidebar_view: Option<&str>,
) -> Result<BenchmarkRecord, String> {
    Err("the startup benchmark requires Windows".to_owned())
}

pub(super) fn diagnostic_handle_allowlist(
    mapping: windows_sys::Win32::Foundation::HANDLE,
    event: windows_sys::Win32::Foundation::HANDLE,
) -> [windows_sys::Win32::Foundation::HANDLE; 2] {
    [mapping, event]
}

#[cfg(windows)]
pub(super) struct ProcThreadAttributeList {
    _storage: Vec<usize>,
    pointer: windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
}

#[cfg(windows)]
impl ProcThreadAttributeList {
    fn with_diagnostic_resources(
        handles: &[windows_sys::Win32::Foundation::HANDLE],
        jobs: &[windows_sys::Win32::Foundation::HANDLE],
    ) -> Result<Self, String> {
        use windows_sys::Win32::System::Threading::{
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, UpdateProcThreadAttribute,
        };

        let handle_bytes = handles
            .len()
            .checked_mul(std::mem::size_of::<windows_sys::Win32::Foundation::HANDLE>())
            .ok_or_else(|| "diagnostic handle-list size overflowed".to_owned())?;
        let job_bytes = jobs
            .len()
            .checked_mul(std::mem::size_of::<windows_sys::Win32::Foundation::HANDLE>())
            .ok_or_else(|| "cleanup job-list size overflowed".to_owned())?;
        let mut byte_len = 0_usize;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut byte_len);
        }
        if byte_len == 0 {
            return Err("could not size process attribute list".to_owned());
        }
        let word_len = byte_len.div_ceil(std::mem::size_of::<usize>());
        let mut storage = vec![0_usize; word_len];
        let pointer = storage.as_mut_ptr().cast();
        if unsafe { InitializeProcThreadAttributeList(pointer, 2, 0, &mut byte_len) } == 0 {
            return Err(fastpad::platform::last_error().to_string());
        }
        if unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                handle_bytes,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            unsafe {
                windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(pointer);
            }
            return Err(fastpad::platform::last_error().to_string());
        }
        if unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                jobs.as_ptr().cast(),
                job_bytes,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            unsafe {
                windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(pointer);
            }
            return Err(fastpad::platform::last_error().to_string());
        }
        Ok(Self {
            _storage: storage,
            pointer,
        })
    }
}

#[cfg(windows)]
pub(super) fn create_cleanup_job() -> Result<fastpad::platform::OwnedHandle, String> {
    use fastpad::platform::OwnedHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    let job = unsafe { OwnedHandle::from_raw_owned(raw) }.map_err(|error| error.to_string())?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.as_raw(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(fastpad::platform::last_error().to_string());
    }
    Ok(job)
}

#[cfg(windows)]
impl Drop for ProcThreadAttributeList {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

#[cfg(windows)]
pub(super) fn run_once(
    launch_file: Option<&Path>,
    notes_folder: Option<&Path>,
    sidebar_view: Option<&str>,
) -> Result<BenchmarkRecord, String> {
    use fastpad::perf::protocol::{
        BENCHMARK_INPUT_CHAR, BENCHMARK_SHARED_FRAME_LEN, EVENT_HANDLE_ENV, MAPPING_HANDLE_ENV,
        QPC_ORIGIN_ENV,
    };
    use fastpad::platform::{OwnedHandle, last_error, wide_null};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile, PAGE_READWRITE,
        UnmapViewOfFile,
    };
    use windows_sys::Win32::System::Performance::QueryPerformanceCounter;
    use windows_sys::Win32::System::Threading::{
        CREATE_UNICODE_ENVIRONMENT, CreateEventW, CreateProcessW, EXTENDED_STARTUPINFO_PRESENT,
        PROCESS_INFORMATION, STARTUPINFOEXW,
    };

    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate benchmark executable: {error}"))?
        .with_file_name("fastpad.exe");
    if !executable.is_file() {
        return Err(format!(
            "{} is missing; build the release fastpad binary first",
            executable.display()
        ));
    }

    let mut name_counter = 0_i64;
    if unsafe { QueryPerformanceCounter(&mut name_counter) } == 0 {
        return Err(last_error().to_string());
    }
    let unique = format!("{}-{name_counter}", std::process::id());
    // The child is launched without `--new-window`, so it becomes the primary instance and would
    // otherwise resolve its settings/session paths from the real user profile via `LOCALAPPDATA`.
    // Each run gets its own scratch profile so the benchmark never reads or writes real user data
    // and never carries a session manifest from one run into the next.
    let local_app_data = ScratchLocalAppData::create(&unique)?;
    local_app_data.seed_notes_folder(notes_folder)?;
    local_app_data.seed_sidebar_view(sidebar_view)?;
    let mapping_name = wide_null(&format!("Local\\FastPadBenchMapping-{unique}"));
    let event_name = wide_null(&format!("Local\\FastPadBenchEvent-{unique}"));
    let security = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let mapping_raw = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            &security,
            PAGE_READWRITE,
            0,
            BENCHMARK_SHARED_FRAME_LEN as u32,
            mapping_name.as_ptr(),
        )
    };
    let mapping =
        unsafe { OwnedHandle::from_raw_owned(mapping_raw) }.map_err(|error| error.to_string())?;
    let event_raw = unsafe { CreateEventW(&security, 0, 0, event_name.as_ptr()) };
    let event =
        unsafe { OwnedHandle::from_raw_owned(event_raw) }.map_err(|error| error.to_string())?;
    let view = unsafe {
        MapViewOfFile(
            mapping.as_raw(),
            FILE_MAP_READ | FILE_MAP_WRITE,
            0,
            0,
            BENCHMARK_SHARED_FRAME_LEN,
        )
    };
    if view.Value.is_null() {
        return Err(last_error().to_string());
    }
    struct ViewGuard(windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS);
    impl Drop for ViewGuard {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(self.0);
            }
        }
    }
    let _view_guard = ViewGuard(view);

    let origin_width = 20;
    let origin_placeholder = "0".repeat(origin_width);
    let mut environment = diagnostic_environment_block(
        mapping.as_raw(),
        event.as_raw(),
        &origin_placeholder,
        MAPPING_HANDLE_ENV,
        EVENT_HANDLE_ENV,
        QPC_ORIGIN_ENV,
        local_app_data.path(),
    )?;
    let origin_marker = format!("{QPC_ORIGIN_ENV}={origin_placeholder}")
        .encode_utf16()
        .collect::<Vec<_>>();
    let origin_start = environment
        .windows(origin_marker.len())
        .position(|window| window == origin_marker)
        .ok_or_else(|| "could not locate QPC origin in environment block".to_owned())?
        + QPC_ORIGIN_ENV.encode_utf16().count()
        + 1;

    let application = executable
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let mut command_line = format!(
        "\"{}\" --diagnostic{}",
        executable.display(),
        launch_file
            .map(|path| format!(" \"{}\"", path.display()))
            .unwrap_or_default()
    )
    .encode_utf16()
    .chain([0])
    .collect::<Vec<_>>();
    let cleanup_job = create_cleanup_job()?;
    let inherited_handles = diagnostic_handle_allowlist(mapping.as_raw(), event.as_raw());
    let cleanup_jobs = [cleanup_job.as_raw()];
    let attributes =
        ProcThreadAttributeList::with_diagnostic_resources(&inherited_handles, &cleanup_jobs)?;
    let startup = STARTUPINFOEXW {
        StartupInfo: windows_sys::Win32::System::Threading::STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
            ..Default::default()
        },
        lpAttributeList: attributes.pointer,
    };
    let mut process_info = PROCESS_INFORMATION::default();
    let mut origin = 0_i64;
    if unsafe { QueryPerformanceCounter(&mut origin) } == 0 {
        return Err(last_error().to_string());
    }
    write_fixed_decimal(
        &mut environment[origin_start..origin_start + origin_width],
        origin,
    )?;
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            std::ptr::null(),
            (&startup as *const STARTUPINFOEXW).cast(),
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(last_error().to_string());
    }
    let thread = unsafe { OwnedHandle::from_raw_owned(process_info.hThread) }
        .map_err(|error| error.to_string())?;
    drop(thread);
    let process = unsafe { OwnedHandle::from_raw_owned(process_info.hProcess) }
        .map_err(|error| error.to_string())?;
    let mut assigned_to_cleanup_job = 0;
    if unsafe {
        windows_sys::Win32::System::JobObjects::IsProcessInJob(
            process.as_raw(),
            cleanup_job.as_raw(),
            &mut assigned_to_cleanup_job,
        )
    } == 0
        || assigned_to_cleanup_job == 0
    {
        return Err("FastPad was not atomically assigned to its cleanup job".to_owned());
    }
    let mut child = ChildGuard::new(cleanup_job, process, process_info.dwProcessId);

    let main_hwnd = wait_for_main_window(&child)?;
    let scintilla = wait_for_scintilla(main_hwnd, &child)?;
    send_benchmark_char(scintilla, BENCHMARK_INPUT_CHAR)?;
    wait_for_event(event.as_raw(), &child)?;
    // The rendered-input event is still required with a launch file, but the buffer check is not:
    // the character lands in the initial Untitled tab while the file opens in a tab of its own.
    if launch_file.is_none() {
        verify_benchmark_char(scintilla)?;
    }
    let mut record = wait_for_fully_ready(view.Value.cast(), &child)?;
    std::thread::sleep(std::time::Duration::from_secs(2));
    record.idle_private_working_set_bytes = private_working_set(child.process.as_raw())?;
    validate_record(&record, child.pid)?;
    // The benchmark character dirties the document; a save prompt would block WM_CLOSE. With a
    // launch file that document is the first (Untitled) tab, not the active file tab.
    if launch_file.is_some() {
        send_scintilla_scalar(
            main_hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND,
            fastpad::window::commands::CommandId::SelectTab1 as usize,
        )?;
    }
    send_scintilla_scalar(
        scintilla,
        fastpad::editor::scintilla_constants::SCI_SETSAVEPOINT,
        0,
    )?;
    child.close(main_hwnd)?;
    Ok(record)
}

#[cfg(windows)]
pub(super) fn diagnostic_environment_block(
    mapping: windows_sys::Win32::Foundation::HANDLE,
    event: windows_sys::Win32::Foundation::HANDLE,
    origin: &str,
    mapping_name: &str,
    event_name: &str,
    origin_name: &str,
    local_app_data: &Path,
) -> Result<Vec<u16>, String> {
    const LOCAL_APP_DATA_NAME: &str = "LOCALAPPDATA";
    let mut values = std::env::vars_os().collect::<Vec<_>>();
    values.retain(|(name, _)| {
        let name = name.to_string_lossy();
        !name.eq_ignore_ascii_case(mapping_name)
            && !name.eq_ignore_ascii_case(event_name)
            && !name.eq_ignore_ascii_case(origin_name)
            && !name.eq_ignore_ascii_case(LOCAL_APP_DATA_NAME)
    });
    values.push((mapping_name.into(), (mapping as usize).to_string().into()));
    values.push((event_name.into(), (event as usize).to_string().into()));
    values.push((origin_name.into(), origin.into()));
    values.push((
        LOCAL_APP_DATA_NAME.into(),
        local_app_data.as_os_str().to_owned(),
    ));
    values.sort_by(|(left, _), (right, _)| {
        left.to_string_lossy()
            .to_ascii_uppercase()
            .cmp(&right.to_string_lossy().to_ascii_uppercase())
    });
    let mut block = Vec::new();
    for (name, value) in values {
        use std::os::windows::ffi::OsStrExt;
        block.extend(name.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(windows)]
pub(super) fn write_fixed_decimal(target: &mut [u16], value: i64) -> Result<(), String> {
    if value <= 0 {
        return Err("QPC origin must be positive".to_owned());
    }
    let text = format!("{value:0width$}", width = target.len());
    if text.len() != target.len() {
        return Err("QPC origin exceeded environment field width".to_owned());
    }
    for (slot, byte) in target.iter_mut().zip(text.bytes()) {
        *slot = u16::from(byte);
    }
    Ok(())
}

/// A private `LOCALAPPDATA` for one benchmark run, so the primary-instance child under
/// measurement never reads or writes the real user's `fastpad.ini`/`session.ini`/`Recovery`.
/// Removed on drop, including on an early `?` return, so failed runs do not leak scratch
/// directories.
#[cfg(windows)]
pub(super) struct ScratchLocalAppData(PathBuf);

#[cfg(windows)]
impl ScratchLocalAppData {
    fn create(unique: &str) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!("fastpad-bench-{unique}"));
        std::fs::create_dir_all(root.join("FastPad"))
            .map_err(|error| format!("could not create {}: {error}", root.display()))?;
        Ok(Self(root))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `FastPad\folders.ini` naming `folder`, so the launch opens it as its library.
    /// Without one it names an empty `notes` folder in this scratch profile: with notes mode on,
    /// the launch would otherwise open the real `Documents\FastPad`.
    fn seed_notes_folder(&self, folder: Option<&Path>) -> Result<(), String> {
        let folder = match folder {
            Some(folder) => std::path::absolute(folder)
                .map_err(|error| format!("could not resolve {}: {error}", folder.display()))?,
            None => {
                let empty = self.0.join("notes");
                std::fs::create_dir_all(&empty)
                    .map_err(|error| format!("could not create {}: {error}", empty.display()))?;
                empty
            }
        };
        let recent = fastpad::library::local::RecentFolders {
            folders: vec![folder],
            ..Default::default()
        };
        let path = fastpad::library::local::folders_file(&self.0.join("FastPad"));
        std::fs::write(&path, recent.encode())
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    /// Writes `FastPad\fastpad.ini` with `sidebar_view=VIEW`, so a run can measure startup
    /// with the panel open or closed. Without a view the scratch profile keeps the default.
    fn seed_sidebar_view(&self, view: Option<&str>) -> Result<(), String> {
        let Some(view) = view else {
            return Ok(());
        };
        let path = self.0.join("FastPad").join("fastpad.ini");
        std::fs::write(&path, format!("sidebar_view={view}\r\n"))
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }
}

#[cfg(windows)]
impl Drop for ScratchLocalAppData {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(windows)]
pub(super) struct ChildGuard {
    job: Option<fastpad::platform::OwnedHandle>,
    pub(super) process: fastpad::platform::OwnedHandle,
    pub(super) pid: u32,
    closed: std::cell::Cell<bool>,
}

#[cfg(windows)]
impl ChildGuard {
    fn new(
        job: fastpad::platform::OwnedHandle,
        process: fastpad::platform::OwnedHandle,
        pid: u32,
    ) -> Self {
        Self {
            job: Some(job),
            process,
            pid,
            closed: std::cell::Cell::new(false),
        }
    }

    fn close(&mut self, hwnd: windows_sys::Win32::Foundation::HWND) -> Result<(), String> {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_CLOSE,
        };
        let mut result = 0_usize;
        if unsafe {
            SendMessageTimeoutW(hwnd, WM_CLOSE, 0, 0, SMTO_ABORTIFHUNG, 5_000, &mut result)
        } == 0
        {
            return Err(fastpad::platform::last_error().to_string());
        }
        if unsafe { WaitForSingleObject(self.process.as_raw(), 5_000) } != WAIT_OBJECT_0 {
            return Err("FastPad did not exit after WM_CLOSE".to_owned());
        }
        self.job.take();
        self.closed.set(true);
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
        if !self.closed.get()
            && unsafe { WaitForSingleObject(self.process.as_raw(), 0) } != WAIT_OBJECT_0
        {
            self.job.take();
            if unsafe { WaitForSingleObject(self.process.as_raw(), 5_000) } == WAIT_OBJECT_0 {
                return;
            }
            unsafe {
                TerminateProcess(self.process.as_raw(), 1);
                WaitForSingleObject(self.process.as_raw(), 5_000);
            }
        }
    }
}
