//! `fastpad-bench`: the external startup-distribution harness. This root parses arguments,
//! validates and serializes benchmark records, and runs the startup distribution.

use fastpad::perf::protocol::BenchmarkRecord;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod library_scan;
use library_scan::*;
mod distribution;
use distribution::*;
mod launch;
use launch::*;
mod probes;
use probes::*;

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
    fn text_search_notes_are_about_4_kb_and_one_in_fifty_mentions_the_invoice() {
        // Break caught: a fixture that measures 200-byte notes, or one where every note (or no
        // note) matches the rare phrase, so the full search is capped or finds nothing.
        for index in [0, 1, 49, 50, 9_999] {
            let text = super::text_search_note(index);
            assert!(
                (4_000..=4_200).contains(&text.len()),
                "{index}: {}",
                text.len()
            );
            assert!(text.contains("lazy dog"));
            assert_eq!(
                text.contains("invoice march"),
                index.is_multiple_of(50),
                "{index}"
            );
        }
    }

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
