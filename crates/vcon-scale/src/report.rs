//! The result file of a run, the verdict on it, and the table the scale page
//! shows of it.

use serde_json::{Value, json};

use crate::stats::{self, Summary};

/// The result file's format name; a change of its fields changes it.
pub const FORMAT: &str = "vcon-scale/1";

/// The scale page's table header, two lines.
pub const TABLE_HEADER: &str = "| Run | Machine | vCons | Clients | Stores a second | Read p50 | Read p99 | Read p99.9 | Errors | Bytes per vCon | Peak server memory | Commit |\n\
|---|---|---|---|---|---|---|---|---|---|---|---|";

/// One phase of a run: its requests, the errors among them, its wall time
/// and its latencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phase {
    /// Requests sent.
    pub requests: u64,
    /// Requests answered with an error, or not answered.
    pub errors: u64,
    /// Wall time, in microseconds.
    pub micros: u64,
    /// The latencies, when any request was sent.
    pub latency: Option<Summary>,
}

/// The machine a run was on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hardware {
    /// The board or machine model, as stated by the caller or the device tree.
    pub machine: String,
    /// The CPU architecture.
    pub arch: String,
    /// The number of CPUs.
    pub cpus: u64,
    /// The CPU's highest clock, in MHz, when known.
    pub max_mhz: Option<u64>,
    /// The memory, in KiB.
    pub memory_kib: u64,
    /// The kernel release.
    pub kernel: String,
    /// The disk the data directory is on.
    pub disk: String,
}

/// One measured run.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The run's name.
    pub label: String,
    /// The commit the server was built from.
    pub commit: String,
    /// The vCons stored.
    pub count: u64,
    /// The concurrent clients.
    pub concurrency: u64,
    /// The seed.
    pub seed: u64,
    /// The size mix.
    pub mix: String,
    /// The 1-minute load when the run started.
    pub start_load: f64,
    /// The stores.
    pub ingest: Phase,
    /// The reads.
    pub reads: Phase,
    /// The list pages.
    pub lists: Phase,
    /// The data directory's bytes after the stores.
    pub data_bytes: Option<u64>,
    /// The server's peak resident memory, in KiB.
    pub peak_rss_kib: Option<u64>,
    /// The server's CPU time, in nanoseconds.
    pub cpu_ns: Option<u64>,
    /// The machine.
    pub hardware: Hardware,
}

/// The bytes on disk per vCon, rounded up; `None` without a count.
#[must_use]
pub fn bytes_per_vcon(data_bytes: u64, count: u64) -> Option<u64> {
    (count > 0).then(|| data_bytes.div_ceil(count))
}

fn phase_json(phase: &Phase) -> Value {
    json!({
        "requests": phase.requests,
        "errors": phase.errors,
        "micros": phase.micros,
        "per_second": stats::per_second(phase.requests, phase.micros),
        "latency_us": phase.latency.map(|s| json!({
            "count": s.count,
            "min": s.min,
            "p50": s.p50,
            "p95": s.p95,
            "p99": s.p99,
            "p999": s.p999,
            "max": s.max,
        })),
    })
}

/// The run as the result file's JSON.
#[must_use]
pub fn to_json(run: &Run) -> Value {
    let hw = &run.hardware;
    json!({
        "format": FORMAT,
        "label": run.label,
        "commit": run.commit,
        "count": run.count,
        "concurrency": run.concurrency,
        "seed": run.seed,
        "mix": run.mix,
        "start_load": run.start_load,
        "phases": {
            "ingest": phase_json(&run.ingest),
            "reads": phase_json(&run.reads),
            "lists": phase_json(&run.lists),
        },
        "disk": {
            "data_bytes": run.data_bytes,
            "bytes_per_vcon": run.data_bytes.and_then(|b| bytes_per_vcon(b, run.count)),
        },
        "server": {
            "peak_rss_kib": run.peak_rss_kib,
            "cpu_ns": run.cpu_ns,
        },
        "hardware": {
            "machine": hw.machine,
            "arch": hw.arch,
            "cpus": hw.cpus,
            "max_mhz": hw.max_mhz,
            "memory_kib": hw.memory_kib,
            "kernel": hw.kernel,
            "disk": hw.disk,
        },
    })
}

/// Whether the run passes: every request answered without an error.
///
/// # Errors
///
/// A text naming each phase that had errors.
pub fn verdict(run: &Run) -> Result<(), String> {
    let failed: Vec<String> = [
        ("ingest", &run.ingest),
        ("reads", &run.reads),
        ("lists", &run.lists),
    ]
    .iter()
    .filter(|(_, phase)| phase.errors > 0)
    .map(|(name, phase)| format!("{name}: {} of {} failed", phase.errors, phase.requests))
    .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// `n` with a comma between each group of three digits.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Microseconds as milliseconds with one decimal, rounded half up.
fn millis(micros: u64) -> String {
    let tenths = micros.saturating_add(50) / 100;
    format!("{}.{} ms", tenths / 10, tenths % 10)
}

/// `n` over `unit`, rounded half up.
fn rounded(n: u64, unit: u64) -> u64 {
    n.saturating_add(unit / 2) / unit
}

fn field<'a>(result: &'a Value, path: &[&str]) -> Result<&'a Value, String> {
    let mut at = result;
    for key in path {
        at = at
            .get(key)
            .filter(|v| !v.is_null())
            .ok_or_else(|| format!("the result has no {}", path.join(".")))?;
    }
    Ok(at)
}

fn number(result: &Value, path: &[&str]) -> Result<u64, String> {
    field(result, path)?
        .as_u64()
        .ok_or_else(|| format!("{} is not a whole number", path.join(".")))
}

fn text<'a>(result: &'a Value, path: &[&str]) -> Result<&'a str, String> {
    field(result, path)?
        .as_str()
        .ok_or_else(|| format!("{} is not text", path.join(".")))
}

/// A measure that may be missing, shown as "n/a".
fn optional(result: &Value, path: &[&str], show: impl Fn(u64) -> String) -> String {
    number(result, path).map_or_else(|_| "n/a".to_owned(), show)
}

/// The scale page's whole table: the header, then a row for each result,
/// in the order given. Only a run without errors is published.
///
/// # Errors
///
/// A text naming the first result that cannot be shown or had errors, or
/// that there is none.
pub fn page_table(results: &[Value]) -> Result<String, String> {
    if results.is_empty() {
        return Err("no result to publish".to_owned());
    }
    let mut table = format!("{TABLE_HEADER}\n");
    for (i, result) in results.iter().enumerate() {
        let row = table_row(result).map_err(|why| format!("result {i}: {why}"))?;
        for phase in ["ingest", "reads", "lists"] {
            let errors = number(result, &["phases", phase, "errors"])?;
            if errors > 0 {
                return Err(format!(
                    "result {i}: {phase} had {errors} errors; a failed run is not published"
                ));
            }
        }
        table.push_str(&row);
        table.push('\n');
    }
    Ok(table)
}

/// The scale page's table row for a result file's JSON.
///
/// # Errors
///
/// A text naming the first field the JSON lacks.
pub fn table_row(result: &Value) -> Result<String, String> {
    if result.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(format!("the result's format is not {FORMAT}"));
    }
    let rate = field(result, &["phases", "ingest", "per_second"])?
        .as_f64()
        .ok_or("phases.ingest.per_second is not a number")?;
    let errors = ["ingest", "reads", "lists"]
        .iter()
        .map(|p| number(result, &["phases", p, "errors"]))
        .sum::<Result<u64, String>>()?;
    let read = |p: &str| number(result, &["phases", "reads", "latency_us", p]).map(millis);
    let commit = text(result, &["commit"])?;
    Ok(format!(
        "| {} | {}, {}, {} CPUs, {} GiB, {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | `{}` |",
        text(result, &["label"])?,
        text(result, &["hardware", "machine"])?,
        text(result, &["hardware", "arch"])?,
        number(result, &["hardware", "cpus"])?,
        rounded(number(result, &["hardware", "memory_kib"])?, 1 << 20),
        text(result, &["hardware", "disk"])?,
        grouped(number(result, &["count"])?),
        number(result, &["concurrency"])?,
        grouped(rate.round() as u64),
        read("p50")?,
        read("p99")?,
        read("p999")?,
        grouped(errors),
        optional(result, &["disk", "bytes_per_vcon"], grouped),
        optional(result, &["server", "peak_rss_kib"], |k| format!(
            "{} MiB",
            grouped(rounded(k, 1024))
        )),
        commit.get(..7).unwrap_or(commit),
    ))
}
