//! Startup distributions: printing and comparing milestone percentiles, reading JSONL records,
//! and the statistics behind them (percentiles, regression checks, bootstrap intervals).

use super::*;

pub(super) fn print_distribution(records: &[BenchmarkRecord]) {
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

pub(super) type MetricAccessor = fn(&BenchmarkRecord) -> u64;

pub(super) fn milestone_columns(records: &[BenchmarkRecord]) -> Vec<(&'static str, Vec<u64>)> {
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

pub(super) fn compare_distributions(
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

pub(super) fn read_records(path: &std::path::Path) -> Result<Vec<BenchmarkRecord>, String> {
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

pub(super) fn record_from_json(value: &serde_json::Value) -> Result<BenchmarkRecord, String> {
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

pub(super) fn percentile(sorted: &[u64], percentile: f64) -> u64 {
    let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
    sorted[index]
}

pub(super) fn reference_thresholds_pass(tti_p50_us: u64, tti_p95_us: u64) -> bool {
    tti_p50_us < 25_000 && tti_p95_us < 40_000
}

pub(super) fn is_regression(baseline: &[u64], candidate: &[u64]) -> bool {
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

pub(super) fn bootstrap_p95_delta_ci(baseline: &[u64], candidate: &[u64]) -> (i128, i128) {
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

pub(super) fn signed_percentile(sorted: &[i128], percentile: f64) -> i128 {
    let index = ((sorted.len() - 1) as f64 * percentile).ceil() as usize;
    sorted[index]
}

pub(super) struct DeterministicRng(u64);

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
