use fastpad::perf::protocol::BenchmarkRecord;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const BOOTSTRAP_RESAMPLES: usize = 10_000;
const BOOTSTRAP_SEED: u64 = 0xFA57_0A0D;

#[derive(Debug, Eq, PartialEq)]
enum Action {
    Run {
        runs: usize,
        warmup: usize,
        output: PathBuf,
        enforce_reference: bool,
        launch_file: Option<PathBuf>,
        notes_folder: Option<PathBuf>,
        sidebar_view: Option<String>,
    },
    Compare {
        baseline: PathBuf,
        candidate: PathBuf,
    },
    LibraryScan {
        folder: PathBuf,
        count: Option<usize>,
        enforce_reference: bool,
    },
}

const USAGE: &str = "usage: fastpad-bench [--runs N] [--warmup N] [--output FILE] [--launch-file FILE] \
                     [--notes-folder DIR] [--sidebar-view notebook|search|favorites|none] \
                     [--enforce-reference]\n       \
                     fastpad-bench compare BASELINE.jsonl CANDIDATE.jsonl\n       \
                     fastpad-bench library-scan DIR [--count N] [--enforce-reference]";

fn parse_args<I, S>(args: I) -> Result<Action, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    if args.first().and_then(|arg| arg.to_str()) == Some("compare") {
        if args.len() != 3 {
            return Err("usage: fastpad-bench compare BASELINE.jsonl CANDIDATE.jsonl".to_owned());
        }
        return Ok(Action::Compare {
            baseline: PathBuf::from(&args[1]),
            candidate: PathBuf::from(&args[2]),
        });
    }
    if args.first().and_then(|arg| arg.to_str()) == Some("library-scan") {
        return parse_library_scan(&args[1..]);
    }

    let mut runs = 100_usize;
    let mut warmup = 10_usize;
    let mut output = PathBuf::from("benchmarks/latest.jsonl");
    let mut enforce_reference = false;
    let mut launch_file = None;
    let mut notes_folder = None;
    let mut sidebar_view = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| "benchmark options must be valid Unicode".to_owned())?;
        match flag {
            "--runs" | "--warmup" | "--output" | "--launch-file" | "--notes-folder"
            | "--sidebar-view" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                match flag {
                    "--runs" => runs = parse_count(value, flag)?,
                    "--warmup" => warmup = parse_count(value, flag)?,
                    "--output" => output = PathBuf::from(value),
                    "--launch-file" => launch_file = Some(PathBuf::from(value)),
                    "--notes-folder" => notes_folder = Some(PathBuf::from(value)),
                    "--sidebar-view" => {
                        let view = value
                            .to_str()
                            .filter(|view| {
                                matches!(*view, "notebook" | "search" | "favorites" | "none")
                            })
                            .ok_or_else(|| {
                                "--sidebar-view must be notebook, search, favorites or none"
                                    .to_owned()
                            })?;
                        sidebar_view = Some(view.to_owned());
                    }
                    _ => unreachable!(),
                }
                index += 2;
            }
            "--enforce-reference" => {
                enforce_reference = true;
                index += 1;
            }
            _ => {
                return Err(format!("unknown benchmark option: {flag}\n{USAGE}"));
            }
        }
    }
    if runs == 0 {
        return Err("--runs must be greater than zero".to_owned());
    }
    Ok(Action::Run {
        runs,
        warmup,
        output,
        enforce_reference,
        launch_file,
        notes_folder,
        sidebar_view,
    })
}

/// `library-scan DIR [--count N] [--enforce-reference]`, after the action name.
fn parse_library_scan(args: &[OsString]) -> Result<Action, String> {
    let (folder, options) = args.split_first().ok_or_else(|| USAGE.to_owned())?;
    let mut count = None;
    let mut enforce_reference = false;
    let mut index = 0;
    while index < options.len() {
        match options[index].to_str() {
            Some("--count") => {
                let value = options
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --count".to_owned())?;
                count = Some(parse_count(value, "--count")?);
                index += 2;
            }
            Some("--enforce-reference") => {
                enforce_reference = true;
                index += 1;
            }
            _ => {
                return Err(format!(
                    "unknown library-scan option: {}\n{USAGE}",
                    options[index].to_string_lossy()
                ));
            }
        }
    }
    Ok(Action::LibraryScan {
        folder: PathBuf::from(folder),
        count,
        enforce_reference,
    })
}

fn parse_count(value: &OsString, flag: &str) -> Result<usize, String> {
    value
        .to_str()
        .ok_or_else(|| format!("{flag} must be valid Unicode"))?
        .parse()
        .map_err(|_| format!("{flag} must be a nonnegative integer"))
}

fn record_to_json_line(record: &BenchmarkRecord) -> String {
    serde_json::json!({
        "version": record.version,
        "pid": record.pid,
        "process_start_us": record.process_start_us,
        "window_created_us": record.window_created_us,
        "editor_created_us": record.editor_created_us,
        "first_paint_us": record.first_paint_us,
        "first_input_accepted_us": record.first_input_accepted_us,
        "first_input_rendered_us": record.first_input_rendered_us,
        "settings_loaded_us": record.settings_loaded_us,
        "file_loaded_us": record.file_loaded_us,
        "fully_ready_us": record.fully_ready_us,
        "idle_private_working_set_bytes": record.idle_private_working_set_bytes,
    })
    .to_string()
}

fn validate_record(record: &BenchmarkRecord, expected_pid: u32) -> Result<(), String> {
    validate_record_fields(record)?;
    if record.pid != expected_pid {
        return Err(format!(
            "diagnostic PID {} did not match child PID {expected_pid}",
            record.pid
        ));
    }
    Ok(())
}

fn validate_record_fields(record: &BenchmarkRecord) -> Result<(), String> {
    if record.version != fastpad::perf::protocol::BENCHMARK_VERSION {
        return Err(format!(
            "unsupported benchmark record version {}",
            record.version
        ));
    }
    if record.pid == 0 {
        return Err("diagnostic record has a zero PID".to_owned());
    }
    let milestones = [
        record.process_start_us,
        record.window_created_us,
        record.editor_created_us,
        record.first_paint_us,
        record.first_input_accepted_us,
        record.first_input_rendered_us,
        record.settings_loaded_us,
        record.file_loaded_us,
        record.fully_ready_us,
    ];
    if milestones.contains(&0) {
        return Err("diagnostic frame is missing a startup milestone".to_owned());
    }
    if !(record.process_start_us <= record.window_created_us
        && record.window_created_us <= record.editor_created_us
        && record.editor_created_us <= record.first_paint_us
        && record.editor_created_us <= record.first_input_accepted_us
        && record.first_input_accepted_us <= record.first_input_rendered_us
        && record.first_paint_us <= record.settings_loaded_us
        && record.settings_loaded_us <= record.file_loaded_us
        && record.file_loaded_us <= record.fully_ready_us)
    {
        return Err("diagnostic milestones are out of order".to_owned());
    }
    if record.idle_private_working_set_bytes == 0 {
        return Err("diagnostic record is missing idle private working set".to_owned());
    }
    Ok(())
}

fn is_fastpad_main_window_class(class_name: &str) -> bool {
    class_name == "FastPadMainWindow"
}

fn main() {
    let exit_code = match run_main() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("fastpad-bench: {error}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run_main() -> Result<i32, String> {
    match parse_args(std::env::args_os().skip(1))? {
        Action::Run {
            runs,
            warmup,
            output,
            enforce_reference,
            launch_file,
            notes_folder,
            sidebar_view,
        } => run_distribution(
            runs,
            warmup,
            &output,
            enforce_reference,
            launch_file.as_deref(),
            notes_folder.as_deref(),
            sidebar_view.as_deref(),
        ),
        Action::Compare {
            baseline,
            candidate,
        } => compare_distributions(&baseline, &candidate),
        Action::LibraryScan {
            folder,
            count,
            enforce_reference,
        } => run_library_scan(&folder, count, enforce_reference),
    }
}

fn run_distribution(
    runs: usize,
    warmup: usize,
    output: &Path,
    enforce_reference: bool,
    launch_file: Option<&Path>,
    notes_folder: Option<&Path>,
    sidebar_view: Option<&str>,
) -> Result<i32, String> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let file = std::fs::File::create(output)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut records = Vec::with_capacity(runs);

    for index in 0..warmup + runs {
        let record = run_once(launch_file, notes_folder, sidebar_view)?;
        if index >= warmup {
            use std::io::Write;
            writeln!(writer, "{}", record_to_json_line(&record))
                .map_err(|error| format!("could not write {}: {error}", output.display()))?;
            records.push(record);
            eprintln!("record {}/{}: pid {}", records.len(), runs, record.pid);
        }
    }
    use std::io::Write;
    writer
        .flush()
        .map_err(|error| format!("could not flush {}: {error}", output.display()))?;

    print_distribution(&records);
    let mut tti = records
        .iter()
        .map(|record| record.first_input_rendered_us)
        .collect::<Vec<_>>();
    tti.sort_unstable();
    let p50 = percentile(&tti, 0.50);
    let p95 = percentile(&tti, 0.95);
    println!("valid_records={}", records.len());
    if enforce_reference && !reference_thresholds_pass(p50, p95) {
        eprintln!("reference threshold failed: warm_tti p50={p50}us p95={p95}us");
        return Ok(2);
    }
    Ok(0)
}

/// Pinned records written into a generated library, spread evenly over its notes.
const LIBRARY_SCAN_PINS: usize = 200;
const LIBRARY_SCAN_WARM_LOADS: usize = 5;
/// The warm-load median must stay below this on the reference machine.
const LIBRARY_SCAN_REFERENCE_MS: f64 = 500.0;
/// Spec §12 targets on the reference machine.
const TREE_BUILD_REFERENCE_MS: f64 = 20.0;
const TREE_ROWS_REFERENCE_MS: f64 = 16.0;
const NAME_SEARCH_REFERENCE_MS: f64 = 5.0;

/// Times one cold and several warm `library::load` calls of `folder`, first generating `count`
/// notes and a `library.ini` into it when asked.
fn run_library_scan(
    folder: &Path,
    count: Option<usize>,
    enforce_reference: bool,
) -> Result<i32, String> {
    if let Some(count) = count {
        // Generating into a folder that holds anything could overwrite someone's notes.
        let occupied = std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some());
        if occupied {
            return Err(format!(
                "{} is not empty; library-scan --count only generates into a new or empty folder",
                folder.display()
            ));
        }
        create_library_fixture(folder, count)?;
    }
    let local = ScratchFile(
        std::env::temp_dir().join(format!("fastpad-bench-library-{}.ini", std::process::id())),
    );
    let _ = std::fs::remove_file(&local.0);

    let load = || {
        let started = std::time::Instant::now();
        let state = fastpad::library::load(folder, &local.0, fastpad::library::now_unix())
            .map_err(|error| format!("could not load {}: {error}", folder.display()))?;
        Ok::<_, String>((state, started.elapsed().as_secs_f64() * 1_000.0))
    };
    // The load writes the per-PC cache itself, so the warm loads find it.
    let (state, cold_ms) = load()?;
    let mut warm_ms = Vec::with_capacity(LIBRARY_SCAN_WARM_LOADS);
    for _ in 0..LIBRARY_SCAN_WARM_LOADS {
        warm_ms.push(load()?.1);
    }
    warm_ms.sort_by(f64::total_cmp);
    let warm_median_ms = warm_ms[warm_ms.len() / 2];
    let index_bytes = state
        .notes
        .iter()
        .map(|note| {
            note.path.as_os_str().len() * 2 + std::mem::size_of::<fastpad::library::NoteEntry>()
        })
        .sum::<usize>();

    let paths = state
        .notes
        .iter()
        .map(|note| note.path.clone())
        .collect::<Vec<_>>();
    let pinned = state
        .library
        .notes
        .iter()
        .filter(|record| record.pinned)
        .map(|record| record.path.clone())
        .collect::<Vec<_>>();
    let tree_build_ms = median_ms(|| {
        std::hint::black_box(fastpad::library::tree::NoteTree::build(&paths, &pinned));
    });
    let tree = fastpad::library::tree::NoteTree::build(&paths, &pinned);
    // Every folder expanded: the fixture's folders hold 500 notes each.
    let tree_rows_expanded_ms = median_ms(|| {
        std::hint::black_box(tree.rows(&|_| true, &[]));
    });
    let name_search_ms = median_ms(|| {
        std::hint::black_box(fastpad::library::name_search::search(
            &paths, "note 12", 500,
        ));
    });

    println!("notes={}", state.notes.len());
    println!("cold_ms={cold_ms:.1}");
    println!("warm_median_ms={warm_median_ms:.1}");
    println!("index_bytes~{index_bytes}");
    println!("tree_build_ms={tree_build_ms:.2}");
    println!("tree_rows_expanded_ms={tree_rows_expanded_ms:.2}");
    println!("name_search_ms={name_search_ms:.2}");
    if enforce_reference {
        let failures = [
            (
                "library-scan warm median",
                warm_median_ms,
                LIBRARY_SCAN_REFERENCE_MS,
            ),
            ("tree build", tree_build_ms, TREE_BUILD_REFERENCE_MS),
            (
                "tree rows, all expanded",
                tree_rows_expanded_ms,
                TREE_ROWS_REFERENCE_MS,
            ),
            ("name search", name_search_ms, NAME_SEARCH_REFERENCE_MS),
        ]
        .into_iter()
        .filter(|(_, measured, limit)| measured >= limit)
        .collect::<Vec<_>>();
        for (what, measured, limit) in &failures {
            eprintln!("reference threshold failed: {what}={measured:.2}ms (limit {limit}ms)");
        }
        if !failures.is_empty() {
            return Ok(2);
        }
    }
    Ok(0)
}

/// The median of five timings of `work`, in milliseconds.
fn median_ms(mut work: impl FnMut()) -> f64 {
    let mut times = (0..LIBRARY_SCAN_WARM_LOADS)
        .map(|_| {
            let started = std::time::Instant::now();
            work();
            started.elapsed().as_secs_f64() * 1_000.0
        })
        .collect::<Vec<_>>();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// Writes `count` notes as `batch{i / 500}\note{i}.md`, about 200 bytes each, and a
/// `.fastpad\library.ini` with pinned records spread across them.
fn create_library_fixture(folder: &Path, count: usize) -> Result<(), String> {
    use fastpad::library::ids::{IdSource, NoteId, fnv1a};
    use fastpad::library::model::{Library, NoteRef};
    use fastpad::library::ops::{PendingOp, apply};

    let relative = |index: usize| PathBuf::from(format!(r"batch{}\note{index}.md", index / 500));
    let text = |index: usize| {
        let mut text = format!("# Note {index}\r\n\r\n");
        while text.len() < 200 {
            text.push_str("The quick brown fox jumps over the lazy dog. ");
        }
        text.truncate(200);
        text
    };
    for index in 0..count {
        let path = folder.join(relative(index));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, text(index))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }

    let mut library = Library::default();
    let mut ids = IdSource::new(fastpad::library::now_unix(), std::process::id());
    let pins = LIBRARY_SCAN_PINS.min(count);
    for pin in 0..pins {
        let index = pin * count / pins;
        let note = NoteRef {
            id: NoteId(ids.next()),
            path: relative(index),
        };
        let content = text(index);
        for op in [
            PendingOp::SetPinned {
                note: note.clone(),
                value: true,
            },
            PendingOp::SetFingerprint {
                note,
                size: content.len() as u64,
                hash: fnv1a(content.as_bytes()),
            },
        ] {
            apply(&mut library, &op)
                .map_err(|error| format!("could not build library.ini: {error}"))?;
        }
    }
    let path = fastpad::library::store::library_file(folder);
    fastpad::library::store::write(&path, &library)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(())
}

/// A scratch file removed on drop, including on an early `?` return.
struct ScratchFile(PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn print_distribution(records: &[BenchmarkRecord]) {
    for (name, values) in milestone_columns(records) {
        let mut values = values;
        values.sort_unstable();
        println!(
            "{name}: p50={}us p95={}us",
            percentile(&values, 0.50),
            percentile(&values, 0.95)
        );
    }
    let mut memory = records
        .iter()
        .map(|record| record.idle_private_working_set_bytes)
        .collect::<Vec<_>>();
    memory.sort_unstable();
    println!(
        "idle_private_working_set_bytes: p50={} p95={}",
        percentile(&memory, 0.50),
        percentile(&memory, 0.95)
    );
}

type MetricAccessor = fn(&BenchmarkRecord) -> u64;

fn milestone_columns(records: &[BenchmarkRecord]) -> Vec<(&'static str, Vec<u64>)> {
    let fields: [(&str, MetricAccessor); 9] = [
        ("process_start", |record| record.process_start_us),
        ("window_created", |record| record.window_created_us),
        ("editor_created", |record| record.editor_created_us),
        ("first_paint", |record| record.first_paint_us),
        ("first_input_accepted", |record| {
            record.first_input_accepted_us
        }),
        ("first_input_rendered", |record| {
            record.first_input_rendered_us
        }),
        ("settings_loaded", |record| record.settings_loaded_us),
        ("file_loaded", |record| record.file_loaded_us),
        ("fully_ready", |record| record.fully_ready_us),
    ];
    fields
        .into_iter()
        .map(|(name, field)| (name, records.iter().map(field).collect()))
        .collect()
}

fn compare_distributions(
    baseline_path: &std::path::Path,
    candidate_path: &std::path::Path,
) -> Result<i32, String> {
    let baseline = read_records(baseline_path)?;
    let candidate = read_records(candidate_path)?;
    let baseline_columns = milestone_columns(&baseline);
    let candidate_columns = milestone_columns(&candidate);
    let mut regression = false;
    for ((name, baseline_values), (_, candidate_values)) in
        baseline_columns.into_iter().zip(candidate_columns)
    {
        let mut baseline_sorted = baseline_values.clone();
        let mut candidate_sorted = candidate_values.clone();
        baseline_sorted.sort_unstable();
        candidate_sorted.sort_unstable();
        let baseline_p95 = percentile(&baseline_sorted, 0.95);
        let candidate_p95 = percentile(&candidate_sorted, 0.95);
        let delta = candidate_p95 as i128 - baseline_p95 as i128;
        let (lower, upper) = bootstrap_p95_delta_ci(&baseline_values, &candidate_values);
        let regressed = is_regression(&baseline_values, &candidate_values);
        println!(
            "{name}: baseline_p95={baseline_p95}us candidate_p95={candidate_p95}us delta={delta}us bootstrap95=[{lower},{upper}]{}",
            if regressed { " REGRESSION" } else { "" }
        );
        regression |= regressed;
    }
    Ok(if regression { 2 } else { 0 })
}

fn read_records(path: &std::path::Path) -> Result<Vec<BenchmarkRecord>, String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in std::io::BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|error| format!("could not read {}: {error}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).map_err(|error| {
            format!(
                "{} line {} is not valid JSON: {error}",
                path.display(),
                index + 1
            )
        })?;
        let record = record_from_json(&value)
            .map_err(|error| format!("{} line {}: {error}", path.display(), index + 1))?;
        validate_record_fields(&record)?;
        records.push(record);
    }
    if records.is_empty() {
        return Err(format!("{} contains no records", path.display()));
    }
    Ok(records)
}

fn record_from_json(value: &serde_json::Value) -> Result<BenchmarkRecord, String> {
    let u64_field = |name| {
        value
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("missing unsigned integer field {name}"))
    };
    let version = u64_field("version")?;
    let pid = u64_field("pid")?;
    Ok(BenchmarkRecord {
        version: version
            .try_into()
            .map_err(|_| "version exceeds u32".to_owned())?,
        pid: pid.try_into().map_err(|_| "pid exceeds u32".to_owned())?,
        process_start_us: u64_field("process_start_us")?,
        window_created_us: u64_field("window_created_us")?,
        editor_created_us: u64_field("editor_created_us")?,
        first_paint_us: u64_field("first_paint_us")?,
        first_input_accepted_us: u64_field("first_input_accepted_us")?,
        first_input_rendered_us: u64_field("first_input_rendered_us")?,
        settings_loaded_us: u64_field("settings_loaded_us")?,
        file_loaded_us: u64_field("file_loaded_us")?,
        fully_ready_us: u64_field("fully_ready_us")?,
        idle_private_working_set_bytes: u64_field("idle_private_working_set_bytes")?,
    })
}

#[cfg(not(windows))]
fn run_once(
    _launch_file: Option<&Path>,
    _notes_folder: Option<&Path>,
    _sidebar_view: Option<&str>,
) -> Result<BenchmarkRecord, String> {
    Err("the startup benchmark requires Windows".to_owned())
}

fn diagnostic_handle_allowlist(
    mapping: windows_sys::Win32::Foundation::HANDLE,
    event: windows_sys::Win32::Foundation::HANDLE,
) -> [windows_sys::Win32::Foundation::HANDLE; 2] {
    [mapping, event]
}

#[cfg(windows)]
struct ProcThreadAttributeList {
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
fn create_cleanup_job() -> Result<fastpad::platform::OwnedHandle, String> {
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
fn run_once(
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
fn diagnostic_environment_block(
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
fn write_fixed_decimal(target: &mut [u16], value: i64) -> Result<(), String> {
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
struct ScratchLocalAppData(PathBuf);

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
struct ChildGuard {
    job: Option<fastpad::platform::OwnedHandle>,
    process: fastpad::platform::OwnedHandle,
    pid: u32,
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

#[cfg(windows)]
fn wait_for_main_window(
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
fn wait_for_scintilla(
    parent: windows_sys::Win32::Foundation::HWND,
    guard: &ChildGuard,
) -> Result<windows_sys::Win32::Foundation::HWND, String> {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowExW;
    let class = fastpad::platform::wide_null("Scintilla");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let hwnd = unsafe {
            FindWindowExW(
                parent,
                std::ptr::null_mut(),
                class.as_ptr(),
                std::ptr::null(),
            )
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
fn send_benchmark_char(
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

fn verify_benchmark_utf8(
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
fn wait_for_event(
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
fn verify_benchmark_char(editor: windows_sys::Win32::Foundation::HWND) -> Result<(), String> {
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
fn send_scintilla_scalar(
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
fn wait_for_fully_ready(view: *const u8, guard: &ChildGuard) -> Result<BenchmarkRecord, String> {
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
fn private_working_set(process: windows_sys::Win32::Foundation::HANDLE) -> Result<u64, String> {
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

fn select_private_working_set(
    ex2_value: Option<u64>,
    fallback: impl FnOnce() -> Result<u64, String>,
) -> Result<u64, String> {
    ex2_value.map_or_else(fallback, Ok)
}

fn private_bytes_from_working_set_flags(flags: &[usize], page_size: u64) -> u64 {
    const VALID: usize = 1;
    const SHARED: usize = 1 << 15;
    flags
        .iter()
        .filter(|flags| **flags & VALID != 0 && **flags & SHARED == 0)
        .count() as u64
        * page_size
}

#[cfg(windows)]
fn private_working_set_via_page_query(
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

fn percentile(sorted: &[u64], percentile: f64) -> u64 {
    let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
    sorted[index]
}

fn reference_thresholds_pass(tti_p50_us: u64, tti_p95_us: u64) -> bool {
    tti_p50_us < 25_000 && tti_p95_us < 40_000
}

fn is_regression(baseline: &[u64], candidate: &[u64]) -> bool {
    if baseline.is_empty() || candidate.is_empty() {
        return false;
    }
    let mut baseline_sorted = baseline.to_vec();
    let mut candidate_sorted = candidate.to_vec();
    baseline_sorted.sort_unstable();
    candidate_sorted.sort_unstable();
    let baseline_p95 = percentile(&baseline_sorted, 0.95);
    let candidate_p95 = percentile(&candidate_sorted, 0.95);
    let delta = candidate_p95.saturating_sub(baseline_p95);
    let material_delta = 2_000_u64.max(baseline_p95.div_ceil(10));
    let (lower, _) = bootstrap_p95_delta_ci(baseline, candidate);
    delta >= material_delta && lower > 0
}

fn bootstrap_p95_delta_ci(baseline: &[u64], candidate: &[u64]) -> (i128, i128) {
    assert!(!baseline.is_empty());
    assert!(!candidate.is_empty());
    let mut rng = DeterministicRng::new(BOOTSTRAP_SEED);
    let mut baseline_sample = vec![0; baseline.len()];
    let mut candidate_sample = vec![0; candidate.len()];
    let mut deltas = Vec::with_capacity(BOOTSTRAP_RESAMPLES);

    for _ in 0..BOOTSTRAP_RESAMPLES {
        for value in &mut baseline_sample {
            *value = baseline[rng.index(baseline.len())];
        }
        for value in &mut candidate_sample {
            *value = candidate[rng.index(candidate.len())];
        }
        baseline_sample.sort_unstable();
        candidate_sample.sort_unstable();
        deltas.push(
            percentile(&candidate_sample, 0.95) as i128
                - percentile(&baseline_sample, 0.95) as i128,
        );
    }

    deltas.sort_unstable();
    (
        signed_percentile(&deltas, 0.025),
        signed_percentile(&deltas, 0.975),
    )
}

fn signed_percentile(sorted: &[i128], percentile: f64) -> i128 {
    let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
    sorted[index]
}

struct DeterministicRng(u64);

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn index(&mut self, upper: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)) % upper as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, bootstrap_p95_delta_ci, diagnostic_handle_allowlist, is_fastpad_main_window_class,
        is_regression, parse_args, percentile, private_bytes_from_working_set_flags,
        record_to_json_line, reference_thresholds_pass, select_private_working_set,
        validate_record, verify_benchmark_utf8,
    };
    use fastpad::perf::protocol::BenchmarkRecord;
    use std::path::PathBuf;

    #[test]
    fn percentile_uses_the_required_ceiling_rank() {
        // Break caught: rounding or flooring the rank understates p50/p95 for small distributions.
        assert_eq!(percentile(&[10, 20, 30, 40], 0.50), 30);
        assert_eq!(percentile(&[10, 20, 30, 40], 0.95), 40);
    }

    #[test]
    fn reference_thresholds_reject_values_at_the_limit() {
        // Break caught: using strict-greater threshold checks accepts the explicitly disallowed
        // 25 ms p50 or 40 ms p95 warm TTI boundary.
        assert!(!reference_thresholds_pass(25_000, 39_999));
        assert!(!reference_thresholds_pass(24_999, 40_000));
        assert!(reference_thresholds_pass(24_999, 39_999));
    }

    #[test]
    fn comparison_requires_material_delta_and_positive_bootstrap_interval() {
        // Break caught: reporting noise below the absolute/relative gate, or a p95 increase whose
        // confidence interval still includes zero, as a benchmark regression.
        let baseline = vec![10_000; 20];
        let material_candidate = vec![12_000; 20];
        let small_candidate = vec![11_999; 20];

        assert_eq!(
            bootstrap_p95_delta_ci(&baseline, &material_candidate),
            (2_000, 2_000)
        );
        assert!(is_regression(&baseline, &material_candidate));
        assert!(!is_regression(&baseline, &small_candidate));

        let high_baseline = vec![30_000; 20];
        let ten_percent_candidate = vec![33_000; 20];
        assert!(is_regression(&high_baseline, &ten_percent_candidate));
    }

    #[test]
    fn command_line_supports_the_sidebar_view() {
        // Break caught: a mistyped view silently measuring the default layout, or the flag
        // swallowing the next option.
        assert!(matches!(
            parse_args(["--notes-folder", r"C:\n", "--sidebar-view", "none"]).unwrap(),
            Action::Run { sidebar_view: Some(view), .. } if view == "none"
        ));
        assert!(parse_args(["--sidebar-view", "tree"]).is_err());
        assert!(parse_args(["--sidebar-view"]).is_err());
    }

    #[test]
    fn command_line_supports_run_and_compare_modes() {
        // Break caught: interpreting compare paths as run options, or silently ignoring explicit
        // warmup/output/reference settings, runs the wrong benchmark workload.
        assert_eq!(
            parse_args([
                "--runs",
                "20",
                "--warmup",
                "5",
                "--output",
                "sample.jsonl",
                "--enforce-reference"
            ])
            .unwrap(),
            Action::Run {
                runs: 20,
                warmup: 5,
                output: PathBuf::from("sample.jsonl"),
                enforce_reference: true,
                launch_file: None,
                notes_folder: None,
                sidebar_view: None,
            }
        );
        assert!(matches!(
            parse_args(["--launch-file", "notes.md"]).unwrap(),
            Action::Run { launch_file: Some(path), .. } if path == std::path::Path::new("notes.md")
        ));
        assert_eq!(
            parse_args(["compare", "baseline.jsonl", "candidate.jsonl"]).unwrap(),
            Action::Compare {
                baseline: PathBuf::from("baseline.jsonl"),
                candidate: PathBuf::from("candidate.jsonl"),
            }
        );
    }

    #[test]
    fn command_line_supports_the_notes_folder_and_library_scan() {
        // Break caught: dropping --notes-folder measures TTI without a library, or reading the
        // library-scan folder as a run option benchmarks the wrong thing.
        assert!(matches!(
            parse_args(["--runs", "3", "--notes-folder", r"C:\notes"]).unwrap(),
            Action::Run { runs: 3, notes_folder: Some(path), .. } if path == std::path::Path::new(r"C:\notes")
        ));
        assert_eq!(
            parse_args(["library-scan", r"C:\notes"]).unwrap(),
            Action::LibraryScan {
                folder: PathBuf::from(r"C:\notes"),
                count: None,
                enforce_reference: false,
            }
        );
        assert_eq!(
            parse_args([
                "library-scan",
                r"C:\notes",
                "--count",
                "10000",
                "--enforce-reference"
            ])
            .unwrap(),
            Action::LibraryScan {
                folder: PathBuf::from(r"C:\notes"),
                count: Some(10_000),
                enforce_reference: true,
            }
        );
        assert!(parse_args(["library-scan"]).is_err());
        assert!(parse_args(["library-scan", r"C:\notes", "--count"]).is_err());
        assert!(parse_args(["library-scan", r"C:\notes", "--runs", "3"]).is_err());
    }

    #[test]
    fn json_line_contains_every_fixed_record_field() {
        // Break caught: omitting or renaming a protocol field makes persisted distributions
        // impossible to compare with the fixed diagnostic frame.
        let line = record_to_json_line(&BenchmarkRecord {
            version: 1,
            pid: 42,
            process_start_us: 1,
            window_created_us: 2,
            editor_created_us: 3,
            first_paint_us: 4,
            first_input_accepted_us: 5,
            first_input_rendered_us: 6,
            settings_loaded_us: 7,
            file_loaded_us: 8,
            fully_ready_us: 9,
            idle_private_working_set_bytes: 10,
        });
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 12);
        assert_eq!(value["first_input_rendered_us"], 6);
        assert_eq!(value["idle_private_working_set_bytes"], 10);
    }

    #[test]
    fn record_validation_rejects_missing_or_misordered_milestones() {
        // Break caught: counting a partially written or internally inconsistent shared-memory
        // frame as a valid benchmark sample corrupts percentile distributions.
        let mut record = BenchmarkRecord {
            version: 1,
            pid: 42,
            process_start_us: 1,
            window_created_us: 2,
            editor_created_us: 3,
            first_paint_us: 4,
            first_input_accepted_us: 5,
            first_input_rendered_us: 6,
            settings_loaded_us: 7,
            file_loaded_us: 8,
            fully_ready_us: 9,
            idle_private_working_set_bytes: 10,
        };
        assert!(validate_record(&record, 42).is_ok());
        record.first_input_rendered_us = 0;
        assert!(validate_record(&record, 42).is_err());
        record.first_input_rendered_us = 4;
        assert!(validate_record(&record, 42).is_err());
        record.first_input_rendered_us = 6;
        assert!(validate_record(&record, 99).is_err());
        record.pid = 42;
        record.version = 2;
        assert!(validate_record(&record, 42).is_err());

        record.version = 1;
        record.editor_created_us = 6;
        assert!(validate_record(&record, 42).is_err());
        record.editor_created_us = 3;
        record.first_paint_us = 8;
        assert!(validate_record(&record, 42).is_err());
        record.first_paint_us = 4;
        record.pid = 0;
        assert!(validate_record(&record, 0).is_err());
    }

    #[test]
    fn comparison_delta_preserves_the_full_u64_timing_range() {
        // Break caught: narrowing arbitrary JSON u64 timings to i64 wraps large candidate values
        // and can hide a real positive regression.
        let expected = u64::MAX as i128 - 1;
        assert_eq!(
            bootstrap_p95_delta_ci(&[1], &[u64::MAX]),
            (expected, expected)
        );
    }

    #[test]
    fn main_window_discovery_rejects_process_owned_ime_helpers() {
        // Break caught: accepting the first top-level HWND for the child PID can select its IME
        // helper window, beneath which no Scintilla child exists.
        assert!(!is_fastpad_main_window_class("IME"));
        assert!(is_fastpad_main_window_class("FastPadMainWindow"));
    }

    #[test]
    fn benchmark_character_verification_uses_scalar_scintilla_reads() {
        // Break caught: passing a harness-process buffer pointer to child-process SCI_GETTEXT
        // cannot retrieve the inserted UTF-8 bytes across the process boundary.
        let expected = "\u{E000}".as_bytes();
        assert!(verify_benchmark_utf8(expected.len(), |index| Ok(expected[index])).is_ok());
        assert!(verify_benchmark_utf8(expected.len(), |_| Ok(0)).is_err());
    }

    #[test]
    fn benchmark_character_verification_propagates_scalar_read_timeout() {
        // Break caught: an unbounded or failed cross-process scalar read must not leave the
        // harness blocked forever after the rendered-input event was signaled.
        let error = verify_benchmark_utf8("\u{E000}".len(), |_| {
            Err("Scintilla scalar read timed out".to_owned())
        })
        .unwrap_err();
        assert_eq!(error, "Scintilla scalar read timed out");
    }

    #[test]
    fn private_working_set_uses_resident_private_pages_not_commit_charge() {
        // Break caught: persisting PROCESS_MEMORY_COUNTERS_EX::PrivateUsage reports private commit
        // charge rather than the resident private working set required by the benchmark contract.
        assert_eq!(
            select_private_working_set(Some(1_048_576), || -> Result<u64, String> {
                panic!("fallback must not run when EX2 supplied a resident value")
            })
            .unwrap(),
            1_048_576
        );
        assert_eq!(
            select_private_working_set(None, || Ok(524_288)).unwrap(),
            524_288
        );

        const VALID: usize = 1;
        const SHARED: usize = 1 << 15;
        assert_eq!(
            private_bytes_from_working_set_flags(&[VALID, VALID | SHARED, 0], 4096),
            4096
        );
    }

    #[test]
    fn diagnostic_handle_allowlist_contains_only_mapping_and_event() {
        // Break caught: CreateProcessW with broad inheritance leaks unrelated inheritable harness
        // handles into FastPad instead of limiting inheritance to its two transport handles.
        let mapping = 11_isize as windows_sys::Win32::Foundation::HANDLE;
        let event = 12_isize as windows_sys::Win32::Foundation::HANDLE;
        assert_eq!(
            diagnostic_handle_allowlist(mapping, event),
            [mapping, event]
        );
    }

    #[cfg(windows)]
    #[test]
    fn cleanup_job_is_configured_to_kill_children_when_harness_closes() {
        // Break caught: cleanup that depends on a Rust Drop or a borrowed process handle can leave
        // FastPad alive when the harness is terminated before normal teardown.
        use windows_sys::Win32::System::JobObjects::{
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, QueryInformationJobObject,
        };
        let job = super::create_cleanup_job().unwrap();
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    job.as_raw(),
                    JobObjectExtendedLimitInformation,
                    (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        assert_ne!(
            limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            0
        );
    }
}
