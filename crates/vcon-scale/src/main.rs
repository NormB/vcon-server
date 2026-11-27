//! `vcon-scale run ...`: stores synthetic vCons in a live server over HTTPS,
//! reads some back, follows list pages, and writes a result file. The rules
//! it applies are in the library, where tests drive them; this file makes
//! the HTTP calls and reads /proc. See website/content/docs/scale.md.

use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::blocking::Client;
use vcon_scale::args::{self, Options};
use vcon_scale::mix::SizeMix;
use vcon_scale::report::{self, Hardware, Phase, Run};
use vcon_scale::{guard, host, stats, synth};
use vcon_server::stderr;

type Failure = Box<dyn std::error::Error>;

/// The server's default body limit (config.rs `BODY_LIMIT`): no size in the
/// mix may pass it.
const BODY_LIMIT: usize = 16 << 20;

const USAGE: &str = "usage: vcon-scale run --url https://HOST:PORT --key-file FILE --count N \
--out FILE [--ca PEM] [--concurrency C] [--reads R] [--list-pages P] [--seed S] [--mix SPEC] \
[--max-load L] [--data-dir DIR] [--server-pid PID] [--commit SHA] [--disk TEXT] [--label NAME]";

/// Runs `call(i)` for every `i` in `0..total` on `clients` threads; `call`
/// answers whether the request succeeded, or the error's text.
fn phase(clients: usize, total: u64, call: impl Fn(u64) -> Result<(), String> + Sync) -> Phase {
    let next = AtomicU64::new(0);
    let errors = AtomicU64::new(0);
    let first_error: Mutex<Option<String>> = Mutex::new(None);
    let latencies: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..clients {
            scope.spawn(|| {
                let mut mine = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total {
                        break;
                    }
                    let sent = Instant::now();
                    let answer = call(i);
                    mine.push(u64::try_from(sent.elapsed().as_micros()).unwrap_or(u64::MAX));
                    if let Err(why) = answer {
                        errors.fetch_add(1, Ordering::Relaxed);
                        if let Ok(mut first) = first_error.lock() {
                            first.get_or_insert(why);
                        }
                    }
                }
                if let Ok(mut all) = latencies.lock() {
                    all.extend(mine);
                }
            });
        }
    });
    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    if let Some(why) = first_error.into_inner().ok().flatten() {
        stderr::line(&format!("vcon-scale: first error: {why}"));
    }
    let mut all = latencies.into_inner().unwrap_or_default();
    Phase {
        requests: total,
        errors: errors.into_inner(),
        micros,
        latency: stats::summarize(&mut all),
    }
}

/// Whether an answer has the status a step expects.
fn expect_status(
    what: &str,
    answer: reqwest::Result<reqwest::blocking::Response>,
    want: StatusCode,
) -> Result<reqwest::blocking::Response, String> {
    let answer = answer.map_err(|e| format!("{what}: {e}"))?;
    if answer.status() == want {
        Ok(answer)
    } else {
        Err(format!("{what}: {}", answer.status().as_u16()))
    }
}

/// The list pages: `pages` of them, one after another, following `next`
/// from the newest page and from the newest again after the last
/// (`pages::follow`).
fn lists(http: &Client, o: &Options, bearer: &str, pages: u64) -> Phase {
    let started = Instant::now();
    let mut latencies = Vec::new();
    let followed = vcon_scale::pages::follow(pages, |cursor| {
        let path = match cursor {
            Some(c) => format!("{}/vcons?limit=100&cursor={c}", o.url),
            None => format!("{}/vcons?limit=100", o.url),
        };
        let sent = Instant::now();
        let page = expect_status(
            "GET /vcons",
            http.get(path).bearer_auth(bearer).send(),
            StatusCode::OK,
        )
        .and_then(|answer| {
            let body = answer.bytes().map_err(|e| format!("GET /vcons: {e}"))?;
            serde_json::from_slice::<serde_json::Value>(&body)
                .map_err(|e| format!("GET /vcons: {e}"))
        });
        latencies.push(u64::try_from(sent.elapsed().as_micros()).unwrap_or(u64::MAX));
        page.map(|page| page["next"].as_str().map(str::to_owned))
    });
    if let Some(why) = &followed.error {
        stderr::line(&format!("vcon-scale: {why}"));
    }
    Phase {
        requests: followed.asked,
        errors: u64::from(followed.error.is_some()),
        micros: u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        latency: stats::summarize(&mut latencies),
    }
}

/// The clock ticks a second that /proc counts CPU time in.
fn ticks_per_second() -> Option<u64> {
    let out = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

fn read_text(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn hardware(disk: &str) -> Hardware {
    let lscpu = std::process::Command::new("lscpu")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_default();
    let meminfo = read_text("/proc/meminfo").unwrap_or_default();
    Hardware {
        machine: host::machine(read_text("/proc/device-tree/model").as_deref(), &lscpu),
        arch: host::lscpu_field(&lscpu, "Architecture")
            .unwrap_or_else(|| std::env::consts::ARCH.to_owned()),
        cpus: host::lscpu_field(&lscpu, "CPU(s)")
            .and_then(|n| n.parse().ok())
            .unwrap_or_default(),
        max_mhz: host::lscpu_field(&lscpu, "CPU max MHz")
            .and_then(|n| n.parse::<f64>().ok())
            .map(|mhz| mhz.round() as u64),
        memory_kib: host::kib_field(&meminfo, "MemTotal").unwrap_or_default(),
        kernel: read_text("/proc/sys/kernel/osrelease")
            .map(|k| k.trim().to_owned())
            .unwrap_or_default(),
        disk: disk.to_owned(),
    }
}

fn run(rest: &[String]) -> Result<bool, Failure> {
    let o = args::parse(rest)?;
    let loadavg = std::fs::read_to_string("/proc/loadavg")?;
    let start_load = guard::load_one(&loadavg).ok_or("no load in /proc/loadavg")?;
    if !guard::may_start(start_load, o.max_load) {
        return Err(format!(
            "the 1-minute load is {start_load}, above the cap of {}: not started",
            o.max_load
        )
        .into());
    }
    let mix = SizeMix::parse(&o.mix, BODY_LIMIT)?;
    let bearer = std::fs::read_to_string(&o.key_file)?.trim().to_owned();
    let mut builder = Client::builder().timeout(Duration::from_secs(120));
    if let Some(ca) = &o.ca {
        builder =
            builder.add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(ca)?)?);
    }
    let http = builder.build()?;
    let stores = format!("{}/vcons", o.url);

    let ingest = phase(o.concurrency, o.count, |i| {
        let made = synth::vcon(o.seed, i, &mix);
        let sent = http
            .post(&stores)
            .bearer_auth(&bearer)
            .header("Content-Type", "application/vcon")
            .body(made.body)
            .send();
        expect_status("POST /vcons", sent, StatusCode::CREATED).map(|_| ())
    });
    let data_bytes = o.data_dir.as_deref().map(host::tree_bytes).transpose()?;
    let reads = phase(o.concurrency, o.reads, |j| {
        let index = synth::read_index(o.seed, j, o.count);
        let path = format!("{stores}/{}", synth::id(o.seed, index));
        let answer = expect_status(
            "GET /vcons/{uuid}",
            http.get(path).bearer_auth(&bearer).send(),
            StatusCode::OK,
        )?;
        answer
            .bytes()
            .map(|_| ())
            .map_err(|e| format!("GET /vcons/{{uuid}}: {e}"))
    });
    let lists = lists(&http, &o, &bearer, o.list_pages);

    let proc_of = |file: &str| {
        o.server_pid
            .and_then(|pid| read_text(&format!("/proc/{pid}/{file}")))
    };
    let result = Run {
        label: o.label.clone(),
        commit: o.commit.clone(),
        count: o.count,
        concurrency: o.concurrency as u64,
        seed: o.seed,
        mix: o.mix.clone(),
        start_load,
        ingest,
        reads,
        lists,
        data_bytes,
        peak_rss_kib: proc_of("status").and_then(|s| host::kib_field(&s, "VmHWM")),
        cpu_ns: proc_of("stat").and_then(|s| host::stat_cpu_ns(&s, ticks_per_second()?)),
        hardware: hardware(&o.disk),
    };
    let json = report::to_json(&result);
    std::fs::write(&o.out, serde_json::to_string_pretty(&json)? + "\n")?;
    println!(
        "vcon-scale: {} stored at {:.0} a second; reads p99 {} us; result in {}",
        result.count,
        stats::per_second(result.ingest.requests, result.ingest.micros).unwrap_or_default(),
        result.reads.latency.map_or(0, |s| s.p99),
        o.out.display()
    );
    match report::verdict(&result) {
        Ok(()) => Ok(true),
        Err(failed) => {
            stderr::line(&format!("vcon-scale: {failed}"));
            Ok(false)
        }
    }
}

fn main() -> ExitCode {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let code = match all.split_first() {
        Some((command, rest)) if command == "run" => match run(rest) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(failure) => {
                stderr::line(&format!("vcon-scale: {failure}"));
                ExitCode::from(2)
            }
        },
        _ => {
            stderr::line(USAGE);
            ExitCode::from(2)
        }
    };
    stderr::drain(stderr::EXIT_DRAIN);
    code
}
