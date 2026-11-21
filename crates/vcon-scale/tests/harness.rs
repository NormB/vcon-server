//! The scale harness's logic. Each test is marked success, failure or edge.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Value, json};
use vcon_scale::args::{self, ArgError, Options};
use vcon_scale::guard::{self, PlanError, Projection};
use vcon_scale::mix::{DEFAULT_MIX, MixError, SizeMix};
use vcon_scale::report::{self, Hardware, Phase, Run, TABLE_HEADER};
use vcon_scale::rng::{self, SplitMix64};
use vcon_scale::stats::{self, Summary};
use vcon_scale::{host, synth};

const MAX: usize = 16 << 20;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn minimal() -> Vec<&'static str> {
    vec![
        "--url",
        "https://localhost:8443",
        "--key-file",
        "key.txt",
        "--count",
        "100",
        "--out",
        "result.json",
    ]
}

fn with(extra: &[&'static str]) -> Vec<String> {
    let mut all = minimal();
    all.extend_from_slice(extra);
    strings(&all)
}

fn refused(extra: &[&'static str]) -> ArgError {
    args::parse(&with(extra)).expect_err("these arguments must be refused")
}

fn is_v4(id: &str) -> bool {
    let parts: Vec<&str> = id.split('-').collect();
    let lengths: Vec<usize> = parts.iter().map(|p| p.len()).collect();
    lengths == [8, 4, 4, 4, 12]
        && id
            .chars()
            .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
        && parts[2].starts_with('4')
        && matches!(parts[3].chars().next(), Some('8' | '9' | 'a' | 'b'))
}

fn summary(n: u64) -> Summary {
    Summary {
        count: n,
        min: 100,
        p50: 1_250,
        p95: 3_000,
        p99: 4_560,
        p999: 9_049,
        max: 12_000,
    }
}

fn run() -> Run {
    Run {
        label: "pilot-10k".to_owned(),
        commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        count: 10_000,
        concurrency: 8,
        seed: 1,
        mix: DEFAULT_MIX.to_owned(),
        start_load: 12.5,
        ingest: Phase {
            requests: 10_000,
            errors: 0,
            micros: 12_500_000,
            latency: Some(summary(10_000)),
        },
        reads: Phase {
            requests: 2_000,
            errors: 0,
            micros: 1_000_000,
            latency: Some(summary(2_000)),
        },
        lists: Phase {
            requests: 10,
            errors: 0,
            micros: 50_000,
            latency: Some(summary(10)),
        },
        data_bytes: Some(204_800_000),
        peak_rss_kib: Some(524_288),
        cpu_ns: Some(30_000_000_000),
        hardware: Hardware {
            machine: "Example board".to_owned(),
            arch: "aarch64".to_owned(),
            cpus: 14,
            max_mhz: Some(2601),
            memory_kib: 128_790_920,
            kernel: "6.8.12".to_owned(),
            disk: "NVMe SSD".to_owned(),
        },
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch directory is made");
    dir
}

// ---- rng: success 3, edge 2

#[test]
fn success_the_same_seed_gives_the_same_numbers() {
    let mut a = SplitMix64::new(42);
    let mut b = SplitMix64::new(42);
    let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
    let second: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
    assert_eq!(first, second, "one seed makes one sequence");
}

#[test]
fn success_splitmix_matches_its_reference_values() {
    // The reference implementation's first outputs for seed 0.
    let mut g = SplitMix64::new(0);
    assert_eq!(
        g.next_u64(),
        0xe220_a839_7b1d_cdaf,
        "the first output for seed 0"
    );
    assert_eq!(
        g.next_u64(),
        0x6e78_9e6a_a1b9_65f4,
        "the second output for seed 0"
    );
}

#[test]
fn success_below_stays_in_range() {
    let mut g = SplitMix64::new(7);
    for _ in 0..10_000 {
        assert!(g.below(10) < 10, "below(10) answers 0..10");
    }
}

#[test]
fn edge_below_zero_is_zero() {
    let mut g = SplitMix64::new(7);
    assert_eq!(g.below(0), 0, "below(0) answers 0");
}

#[test]
fn edge_below_one_is_always_zero() {
    let mut g = SplitMix64::new(9);
    assert!((0..100).all(|_| g.below(1) == 0), "below(1) answers only 0");
}

#[test]
fn edge_finalize_differs_for_neighbouring_inputs() {
    assert_ne!(
        rng::finalize(1),
        rng::finalize(2),
        "finalize mixes its input"
    );
    assert_eq!(rng::finalize(5), rng::finalize(5), "finalize is a function");
}

// ---- mix: success 4, failure 6, edge 3

#[test]
fn success_the_default_mix_reads() {
    let mix = SizeMix::parse(DEFAULT_MIX, MAX).expect("the default mix reads");
    assert_eq!(mix.total_weight(), 100, "70 + 25 + 5");
}

#[test]
fn success_pick_follows_the_weights() {
    let mix = SizeMix::parse(DEFAULT_MIX, MAX).expect("the default mix reads");
    assert_eq!(mix.pick(0), 2048, "draw 0 is in the first class");
    assert_eq!(mix.pick(69), 2048, "draw 69 is the first class's last");
    assert_eq!(mix.pick(70), 16384, "draw 70 is the second class's first");
    assert_eq!(mix.pick(94), 16384, "draw 94 is the second class's last");
    assert_eq!(mix.pick(95), 262_144, "draw 95 is the third class's first");
    assert_eq!(mix.pick(99), 262_144, "draw 99 is the last");
}

#[test]
fn success_the_mean_is_weighted() {
    let mix = SizeMix::parse(DEFAULT_MIX, MAX).expect("the default mix reads");
    // (70 * 2048 + 25 * 16384 + 5 * 262144) / 100 = 18636.8
    assert_eq!(mix.mean_bytes(), 18_636, "the weighted mean, rounded down");
}

#[test]
fn success_spaces_around_parts_are_read() {
    let mix = SizeMix::parse(" 1:100 , 3:300 ", MAX).expect("spaces are allowed");
    assert_eq!(mix.total_weight(), 4, "1 + 3");
    assert_eq!(mix.pick(1), 300, "draw 1 is in the second class");
}

#[test]
fn failure_an_empty_mix_is_refused() {
    assert_eq!(SizeMix::parse("", MAX), Err(MixError::Empty), "no class");
}

#[test]
fn failure_a_part_without_a_colon_is_refused() {
    assert_eq!(
        SizeMix::parse("70:2048,25", MAX),
        Err(MixError::Malformed("25".to_owned())),
        "the part without a colon is named"
    );
}

#[test]
fn failure_a_part_that_is_not_a_number_is_refused() {
    assert_eq!(
        SizeMix::parse("x:2048", MAX),
        Err(MixError::Malformed("x:2048".to_owned())),
        "a weight that is not a number"
    );
}

#[test]
fn failure_a_zero_weight_is_refused() {
    assert_eq!(
        SizeMix::parse("0:2048", MAX),
        Err(MixError::ZeroWeight("0:2048".to_owned())),
        "a weight of 0"
    );
}

#[test]
fn failure_a_zero_size_is_refused() {
    assert_eq!(
        SizeMix::parse("1:0", MAX),
        Err(MixError::ZeroSize("1:0".to_owned())),
        "a size of 0"
    );
}

#[test]
fn failure_a_size_above_the_body_limit_is_refused() {
    assert_eq!(
        SizeMix::parse("1:1025", 1024),
        Err(MixError::TooLarge {
            bytes: 1025,
            max: 1024
        }),
        "a size above the limit"
    );
}

#[test]
fn failure_errors_say_what_is_wrong() {
    let text = MixError::TooLarge { bytes: 9, max: 8 }.to_string();
    assert!(
        text.contains('9') && text.contains('8'),
        "the message names both sizes: {text}"
    );
    let text = MixError::Malformed("q".to_owned()).to_string();
    assert!(
        text.contains("\"q\""),
        "the message quotes the part: {text}"
    );
}

#[test]
fn edge_a_size_at_the_limit_is_allowed() {
    let mix = SizeMix::parse("1:1024", 1024).expect("the limit itself is allowed");
    assert_eq!(mix.pick(0), 1024, "one class");
}

#[test]
fn edge_a_draw_past_the_total_falls_on_the_last_class() {
    let mix = SizeMix::parse("1:10,1:20", MAX).expect("the mix reads");
    assert_eq!(mix.pick(u64::MAX), 20, "a draw past the total");
}

#[test]
fn edge_a_trailing_comma_is_refused() {
    assert_eq!(
        SizeMix::parse("1:10,", MAX),
        Err(MixError::Malformed(String::new())),
        "an empty part"
    );
}

// ---- synth: success 5, edge 3

#[test]
fn success_a_uuid_is_version_4_text() {
    let mut g = SplitMix64::new(3);
    for _ in 0..200 {
        let id = synth::random_id(&mut g);
        assert!(is_v4(&id), "a version 4 uuid in RFC 9562 text form: {id}");
    }
}

#[test]
fn success_a_vcon_is_one_the_server_accepts() {
    let mix = SizeMix::parse(DEFAULT_MIX, MAX).expect("the default mix reads");
    for index in 0..50 {
        let made = synth::vcon(11, index, &mix);
        let seen = vcon::read::inspect(
            &made.body,
            Some("application/vcon"),
            &vcon::read::Limits::new(64 << 20),
            &BTreeSet::new(),
        )
        .expect("the server's own check accepts the vCon");
        assert_eq!(seen.uuid.to_string(), made.id, "the vCon holds its own id");
    }
}

#[test]
fn success_a_vcon_is_the_size_the_mix_picked() {
    let mix = SizeMix::parse("1:4096", MAX).expect("the mix reads");
    for index in 0..20 {
        assert_eq!(
            synth::vcon(5, index, &mix).body.len(),
            4096,
            "exactly 4096 bytes"
        );
    }
}

#[test]
fn success_the_same_seed_and_index_make_the_same_vcon() {
    let mix = SizeMix::parse(DEFAULT_MIX, MAX).expect("the mix reads");
    assert_eq!(
        synth::vcon(8, 77, &mix),
        synth::vcon(8, 77, &mix),
        "made again"
    );
}

#[test]
fn success_only_reserved_values_appear() {
    let mix = SizeMix::parse("1:2048", MAX).expect("the mix reads");
    let made: Value =
        serde_json::from_slice(&synth::vcon(1, 0, &mix).body).expect("the body is JSON");
    for party in made["parties"].as_array().expect("parties is an array") {
        let mail = party["mailto"].as_str().expect("a mail address");
        assert!(mail.ends_with("@example.com"), "a reserved domain: {mail}");
        let tel = party["tel"].as_str().expect("a number");
        assert!(tel.starts_with("+1-555-01"), "a reserved number: {tel}");
    }
}

#[test]
fn success_the_id_alone_is_the_vcons_id() {
    let mix = SizeMix::parse(DEFAULT_MIX, MAX).expect("the mix reads");
    for index in [0, 1, 99, 1_000_000] {
        assert_eq!(
            synth::id(4, index),
            synth::vcon(4, index, &mix).id,
            "index {index}"
        );
    }
}

#[test]
fn edge_neighbouring_indexes_make_different_ids() {
    let mix = SizeMix::parse("1:2048", MAX).expect("the mix reads");
    let ids: BTreeSet<String> = (0..1000).map(|i| synth::vcon(1, i, &mix).id).collect();
    assert_eq!(ids.len(), 1000, "1000 indexes, 1000 ids");
}

#[test]
fn edge_other_seeds_make_other_ids() {
    let mix = SizeMix::parse("1:2048", MAX).expect("the mix reads");
    assert_ne!(
        synth::vcon(1, 0, &mix).id,
        synth::vcon(2, 0, &mix).id,
        "seed 1 and 2"
    );
}

#[test]
fn edge_a_size_below_the_smallest_vcon_gives_the_smallest() {
    let mix = SizeMix::parse("1:1", MAX).expect("the mix reads");
    let small = synth::vcon(1, 0, &mix);
    let parsed: Value = serde_json::from_slice(&small.body).expect("still JSON");
    assert_eq!(parsed["dialog"][0]["body"], "", "an empty dialog body");
    assert!(small.body.len() > 1, "the smallest vCon of this shape");
}

#[test]
fn success_reads_stay_among_the_stored() {
    for j in 0..5000 {
        assert!(
            synth::read_index(3, j, 1000) < 1000,
            "read {j} names a stored index"
        );
    }
}

#[test]
fn success_odd_reads_are_of_the_newest_hundredth() {
    for j in (1..2000).step_by(2) {
        assert!(
            synth::read_index(3, j, 10_000) >= 9_900,
            "read {j} is of the newest 1%"
        );
    }
}

#[test]
fn success_even_reads_spread_over_all() {
    let low = (0..2000)
        .step_by(2)
        .filter(|&j| synth::read_index(3, j, 10_000) < 5_000)
        .count();
    assert!(
        (400..=600).contains(&low),
        "about half of 1000 below the middle: {low}"
    );
}

#[test]
fn edge_one_stored_vcon_is_every_read() {
    assert!(
        (0..50).all(|j| synth::read_index(9, j, 1) == 0),
        "only index 0"
    );
}

#[test]
fn edge_no_stored_vcon_reads_index_zero() {
    assert_eq!(synth::read_index(9, 0, 0), 0, "nothing stored");
}

#[test]
fn success_the_machine_is_the_device_tree_model() {
    assert_eq!(
        host::machine(Some("Example Board Model\0"), LSCPU),
        "Example Board Model",
        "the trailing NUL is dropped"
    );
}

#[test]
fn edge_without_a_device_tree_the_cpu_model_names_the_machine() {
    let lscpu = "Model name:   Example CPU 9000\n";
    assert_eq!(
        host::machine(None, lscpu),
        "Example CPU 9000",
        "lscpu's model"
    );
    assert_eq!(
        host::machine(Some("\0"), lscpu),
        "Example CPU 9000",
        "an empty model"
    );
}

#[test]
fn failure_with_neither_the_machine_is_unknown() {
    assert_eq!(
        host::machine(None, LSCPU),
        "unknown machine",
        "no model anywhere"
    );
}

// ---- stats: success 4, failure 2, edge 4

#[test]
fn success_nearest_rank_of_one_to_a_hundred() {
    let sorted: Vec<u64> = (1..=100).collect();
    assert_eq!(stats::nearest_rank(&sorted, 500), Some(50), "p50");
    assert_eq!(stats::nearest_rank(&sorted, 950), Some(95), "p95");
    assert_eq!(stats::nearest_rank(&sorted, 990), Some(99), "p99");
    assert_eq!(
        stats::nearest_rank(&sorted, 999),
        Some(100),
        "p99.9 rounds the rank up"
    );
}

#[test]
fn success_summarize_sorts_first() {
    let mut samples = vec![5, 1, 4, 2, 3];
    let s = stats::summarize(&mut samples).expect("five samples");
    assert_eq!(
        (s.min, s.p50, s.max, s.count),
        (1, 3, 5, 5),
        "min, median, max, count"
    );
}

#[test]
fn success_p999_of_a_thousand_is_the_999th() {
    let mut samples: Vec<u64> = (1..=1000).rev().collect();
    let s = stats::summarize(&mut samples).expect("a thousand samples");
    assert_eq!((s.p99, s.p999), (990, 999), "ranks 990 and 999");
}

#[test]
fn success_per_second_is_count_over_time() {
    assert_eq!(
        stats::per_second(500, 250_000),
        Some(2000.0),
        "500 in a quarter second"
    );
}

#[test]
fn failure_no_samples_has_no_summary() {
    assert_eq!(stats::summarize(&mut []), None, "nothing to summarize");
    assert_eq!(stats::nearest_rank(&[], 500), None, "no rank in nothing");
}

#[test]
fn failure_a_rank_past_a_thousand_is_refused() {
    assert_eq!(
        stats::nearest_rank(&[1, 2], 1001),
        None,
        "per mille above 1000"
    );
}

#[test]
fn edge_no_time_has_no_rate() {
    assert_eq!(stats::per_second(10, 0), None, "zero microseconds");
}

#[test]
fn edge_one_sample_is_every_percentile() {
    let s = stats::summarize(&mut [42]).expect("one sample");
    assert_eq!(
        [s.min, s.p50, s.p95, s.p99, s.p999, s.max],
        [42; 6],
        "a single sample"
    );
}

#[test]
fn edge_rank_zero_is_the_smallest() {
    assert_eq!(stats::nearest_rank(&[3, 9], 0), Some(3), "p0 is the first");
    assert_eq!(
        stats::nearest_rank(&[3, 9], 1000),
        Some(9),
        "p100 is the last"
    );
}

#[test]
fn edge_huge_samples_do_not_overflow() {
    let s = stats::summarize(&mut [u64::MAX, 0]).expect("two samples");
    assert_eq!((s.min, s.max), (0, u64::MAX), "the extremes");
}

// ---- guard: success 4, failure 4, edge 4

#[test]
fn success_load_reads_from_loadavg() {
    assert_eq!(
        guard::load_one("11.68 17.25 18.85 3/1500 4242\n"),
        Some(11.68),
        "first field"
    );
}

#[test]
fn success_a_load_below_the_cap_may_start() {
    assert!(guard::may_start(11.0, 28.0), "11 under 28");
}

#[test]
fn success_projection_multiplies_and_divides() {
    assert_eq!(
        guard::project(10_000_000, 5120, 1000, u64::MAX),
        Ok(Projection {
            disk_bytes: 51_200_000_000,
            seconds: 10_000
        }),
        "10M x 5 KiB, at 1000 a second"
    );
}

#[test]
fn success_projection_rounds_time_up() {
    assert_eq!(
        guard::project(1001, 1, 1000, u64::MAX).map(|p| p.seconds),
        Ok(2),
        "1001 at 1000 a second is 2 s"
    );
}

#[test]
fn failure_a_load_above_the_cap_may_not_start() {
    assert!(!guard::may_start(28.04, 28.0), "28.04 is above 28");
}

#[test]
fn failure_a_rate_of_zero_is_refused() {
    assert_eq!(
        guard::project(1, 1, 0, u64::MAX),
        Err(PlanError::ZeroRate),
        "no rate"
    );
}

#[test]
fn failure_disk_over_the_cap_is_refused() {
    assert_eq!(
        guard::project(1000, 1000, 1, 999_999),
        Err(PlanError::DiskOverCap {
            need: 1_000_000,
            cap: 999_999
        }),
        "1 MB needed, 1 byte less allowed"
    );
}

#[test]
fn failure_an_overflow_is_refused() {
    assert_eq!(
        guard::project(u64::MAX, 2, 1, u64::MAX),
        Err(PlanError::Overflow),
        "the product does not fit"
    );
}

#[test]
fn edge_the_cap_itself_may_start() {
    assert!(guard::may_start(28.0, 28.0), "exactly at the cap");
}

#[test]
fn edge_disk_at_the_cap_is_allowed() {
    assert!(guard::project(10, 10, 1, 100).is_ok(), "exactly the cap");
}

#[test]
fn edge_bad_loadavg_text_has_no_load() {
    assert_eq!(guard::load_one(""), None, "empty");
    assert_eq!(guard::load_one("high 1 2"), None, "not a number");
}

#[test]
fn edge_a_load_that_is_not_a_number_may_not_start() {
    assert!(!guard::may_start(f64::NAN, 28.0), "NaN never starts");
}

#[test]
fn failure_plan_errors_say_what_is_wrong() {
    let text = PlanError::DiskOverCap { need: 7, cap: 3 }.to_string();
    assert!(
        text.contains('7') && text.contains('3'),
        "both numbers: {text}"
    );
}

// ---- host: success 4, failure 2, edge 3

const LSCPU: &str =
    "Architecture:   aarch64\nCPU(s):    14\nModel name:   -\nCPU max MHz:    2601.0000\n";

#[test]
fn success_lscpu_fields_read() {
    assert_eq!(
        host::lscpu_field(LSCPU, "Architecture"),
        Some("aarch64".to_owned()),
        "arch"
    );
    assert_eq!(
        host::lscpu_field(LSCPU, "CPU(s)"),
        Some("14".to_owned()),
        "cpus"
    );
}

#[test]
fn success_kib_fields_read() {
    let meminfo = "MemTotal:       128790920 kB\nMemFree:  10 kB\n";
    assert_eq!(
        host::kib_field(meminfo, "MemTotal"),
        Some(128_790_920),
        "MemTotal"
    );
    let status = "Name:\tvcon-server\nVmHWM:\t  524288 kB\n";
    assert_eq!(host::kib_field(status, "VmHWM"), Some(524_288), "VmHWM");
}

/// `/proc/<pid>/stat` with utime 250 and stime 50 (fields 14 and 15), and a
/// command name holding a space and a parenthesis.
const STAT: &str = "4242 (vcon (server) x) S 1 4242 4242 0 -1 4194560 900 0 0 0 250 50 0 0 20 0 9 0 100 1000 200\n";

#[test]
fn success_cpu_time_is_user_plus_system_of_every_thread() {
    assert_eq!(
        host::stat_cpu_ns(STAT, 100),
        Some(3_000_000_000),
        "300 ticks at 100 a second"
    );
}

#[test]
fn edge_cpu_time_follows_the_tick_rate() {
    assert_eq!(
        host::stat_cpu_ns(STAT, 1000),
        Some(300_000_000),
        "300 ticks at 1000 a second"
    );
}

#[test]
fn failure_cpu_time_needs_a_tick_rate_and_whole_fields() {
    assert_eq!(host::stat_cpu_ns(STAT, 0), None, "no tick rate");
    assert_eq!(
        host::stat_cpu_ns("4242 (short) S 1 2", 100),
        None,
        "too few fields"
    );
    assert_eq!(
        host::stat_cpu_ns("no parenthesis at all", 100),
        None,
        "no command name"
    );
}

#[test]
fn success_tree_bytes_sums_every_file() {
    let dir = scratch("tree-sum");
    std::fs::write(dir.join("a"), [0u8; 10]).expect("a is written");
    std::fs::create_dir(dir.join("sub")).expect("sub is made");
    std::fs::write(dir.join("sub").join("b"), [0u8; 32]).expect("b is written");
    assert_eq!(
        host::tree_bytes(&dir).expect("the tree reads"),
        42,
        "10 + 32"
    );
    std::fs::remove_dir_all(&dir).expect("the scratch directory is removed");
}

#[test]
fn failure_a_missing_tree_is_an_error() {
    let dir = scratch("tree-missing");
    std::fs::remove_dir_all(&dir).expect("the scratch directory is removed");
    assert!(host::tree_bytes(&dir).is_err(), "no such directory");
}

#[test]
fn failure_missing_fields_are_none() {
    assert_eq!(
        host::lscpu_field(LSCPU, "Vendor ID"),
        None,
        "not in the text"
    );
    assert_eq!(
        host::kib_field("VmHWM: lots kB\n", "VmHWM"),
        None,
        "not a number"
    );
    assert_eq!(host::stat_cpu_ns("", 100), None, "empty");
}

#[test]
fn edge_a_dash_value_is_none() {
    assert_eq!(
        host::lscpu_field(LSCPU, "Model name"),
        None,
        "lscpu writes - for unknown"
    );
}

#[test]
fn edge_a_field_name_must_match_whole() {
    assert_eq!(host::lscpu_field(LSCPU, "CPU"), None, "CPU is not CPU(s)");
    assert_eq!(
        host::kib_field("MemTotalX: 5 kB\n", "MemTotal"),
        None,
        "a longer name"
    );
}

#[test]
fn edge_a_symbolic_link_is_not_followed() {
    let dir = scratch("tree-link");
    let outside = scratch("tree-link-target");
    std::fs::write(outside.join("big"), [0u8; 1000]).expect("big is written");
    std::os::unix::fs::symlink(&outside, dir.join("link")).expect("the link is made");
    std::fs::write(dir.join("small"), [0u8; 3]).expect("small is written");
    assert_eq!(
        host::tree_bytes(&dir).expect("the tree reads"),
        3,
        "the link's target is not counted"
    );
    std::fs::remove_dir_all(&dir).expect("the scratch directory is removed");
    std::fs::remove_dir_all(&outside).expect("the target directory is removed");
}

// ---- args: success 4, failure 8, edge 3

#[test]
fn success_the_minimal_command_line_reads_with_defaults() {
    let o: Options = args::parse(&with(&[])).expect("the minimal line reads");
    assert_eq!(o.url, "https://localhost:8443", "url");
    assert_eq!(o.count, 100, "count");
    assert_eq!(o.concurrency, 8, "the default concurrency");
    assert_eq!(o.mix, DEFAULT_MIX, "the default mix");
    assert_eq!(o.max_load, args::DEFAULT_MAX_LOAD, "the default load cap");
    assert_eq!(o.reads, 1000, "the default reads");
    assert_eq!(o.list_pages, 10, "the default list pages");
}

#[test]
fn success_every_flag_reads() {
    let o = args::parse(&with(&[
        "--ca",
        "ca.pem",
        "--concurrency",
        "4",
        "--reads",
        "50",
        "--list-pages",
        "3",
        "--seed",
        "9",
        "--mix",
        "1:100",
        "--max-load",
        "14.5",
        "--data-dir",
        "d",
        "--server-pid",
        "77",
        "--commit",
        "abc",
        "--disk",
        "NVMe",
        "--label",
        "t",
    ]))
    .expect("every flag reads");
    assert_eq!(o.ca, Some(PathBuf::from("ca.pem")), "ca");
    assert_eq!(
        (o.concurrency, o.reads, o.list_pages, o.seed),
        (4, 50, 3, 9),
        "numbers"
    );
    assert_eq!(o.max_load, 14.5, "max load");
    assert_eq!(o.server_pid, Some(77), "pid");
    assert_eq!(
        (o.commit.as_str(), o.disk.as_str(), o.label.as_str()),
        ("abc", "NVMe", "t"),
        "texts"
    );
}

#[test]
fn success_zero_reads_are_allowed() {
    assert_eq!(
        args::parse(&with(&["--reads", "0"])).map(|o| o.reads),
        Ok(0),
        "reads 0"
    );
}

#[test]
fn success_flags_may_come_in_any_order() {
    let o = args::parse(&strings(&[
        "--out",
        "r.json",
        "--count",
        "5",
        "--key-file",
        "k",
        "--url",
        "https://h",
    ]))
    .expect("any order");
    assert_eq!(o.count, 5, "count read");
}

#[test]
fn failure_an_unknown_flag_is_refused() {
    assert_eq!(
        refused(&["--fast"]),
        ArgError::Unknown("--fast".to_owned()),
        "unknown"
    );
}

#[test]
fn failure_a_flag_without_a_value_is_refused() {
    assert_eq!(
        refused(&["--seed"]),
        ArgError::NoValue("--seed".to_owned()),
        "no value"
    );
}

#[test]
fn failure_a_missing_url_is_refused() {
    let line = strings(&["--key-file", "k", "--count", "1", "--out", "o"]);
    assert_eq!(
        args::parse(&line),
        Err(ArgError::Missing("--url")),
        "no url"
    );
}

#[test]
fn failure_a_plain_http_url_is_refused() {
    let line = strings(&[
        "--url",
        "http://h",
        "--key-file",
        "k",
        "--count",
        "1",
        "--out",
        "o",
    ]);
    assert_eq!(
        args::parse(&line),
        Err(ArgError::Bad {
            flag: "--url".to_owned(),
            value: "http://h".to_owned()
        }),
        "only https"
    );
}

#[test]
fn failure_a_count_of_zero_is_refused() {
    let line = strings(&[
        "--url",
        "https://h",
        "--key-file",
        "k",
        "--count",
        "0",
        "--out",
        "o",
    ]);
    assert_eq!(
        args::parse(&line),
        Err(ArgError::Range {
            flag: "--count".to_owned(),
            value: "0".to_owned()
        }),
        "count 0"
    );
}

#[test]
fn failure_a_concurrency_above_the_most_is_refused() {
    assert_eq!(
        refused(&["--concurrency", "257"]),
        ArgError::Range {
            flag: "--concurrency".to_owned(),
            value: "257".to_owned()
        },
        "257 clients"
    );
}

#[test]
fn failure_a_value_of_the_wrong_type_is_refused() {
    assert_eq!(
        refused(&["--seed", "-1"]),
        ArgError::Bad {
            flag: "--seed".to_owned(),
            value: "-1".to_owned()
        },
        "a negative seed"
    );
}

#[test]
fn failure_a_flag_given_twice_is_refused() {
    assert_eq!(
        refused(&["--count", "5"]),
        ArgError::Twice("--count".to_owned()),
        "count twice"
    );
}

#[test]
fn failure_a_load_cap_that_is_not_positive_is_refused() {
    assert_eq!(
        refused(&["--max-load", "0"]),
        ArgError::Range {
            flag: "--max-load".to_owned(),
            value: "0".to_owned()
        },
        "cap 0"
    );
    assert!(
        matches!(refused(&["--max-load", "inf"]), ArgError::Range { .. }),
        "an infinite cap"
    );
}

#[test]
fn edge_the_largest_count_is_allowed() {
    let line = strings(&[
        "--url",
        "https://h",
        "--key-file",
        "k",
        "--count",
        "100000000",
        "--out",
        "o",
    ]);
    assert_eq!(
        args::parse(&line).map(|o| o.count),
        Ok(args::MAX_COUNT),
        "the most"
    );
}

#[test]
fn edge_one_past_the_largest_count_is_refused() {
    let line = strings(&[
        "--url",
        "https://h",
        "--key-file",
        "k",
        "--count",
        "100000001",
        "--out",
        "o",
    ]);
    assert!(
        matches!(args::parse(&line), Err(ArgError::Range { .. })),
        "one too many"
    );
}

#[test]
fn edge_arg_errors_name_the_flag() {
    let text = ArgError::Range {
        flag: "--count".to_owned(),
        value: "0".to_owned(),
    }
    .to_string();
    assert!(
        text.contains("--count") && text.contains('0'),
        "flag and value: {text}"
    );
    assert!(
        ArgError::Missing("--out").to_string().contains("--out"),
        "the missing flag"
    );
}

// ---- report: success 5, failure 4, edge 4

#[test]
fn success_bytes_per_vcon_rounds_up() {
    assert_eq!(report::bytes_per_vcon(10, 3), Some(4), "10 / 3 rounded up");
    assert_eq!(report::bytes_per_vcon(9, 3), Some(3), "exact");
}

#[test]
fn success_the_result_file_holds_every_measure() {
    let j = report::to_json(&run());
    assert_eq!(j["format"], "vcon-scale/1", "the format");
    assert_eq!(j["count"], 10_000, "count");
    assert_eq!(
        j["phases"]["ingest"]["per_second"], 800.0,
        "10000 in 12.5 s"
    );
    assert_eq!(j["phases"]["reads"]["latency_us"]["p99"], 4560, "p99");
    assert_eq!(j["disk"]["bytes_per_vcon"], 20_480, "bytes per vCon");
    assert_eq!(j["server"]["peak_rss_kib"], 524_288, "peak memory");
    assert_eq!(j["hardware"]["cpus"], 14, "cpus");
}

#[test]
fn success_a_run_without_errors_passes() {
    assert_eq!(report::verdict(&run()), Ok(()), "no errors");
}

#[test]
fn success_the_table_row_shows_the_numbers() {
    let row = report::table_row(&report::to_json(&run())).expect("a full result");
    assert_eq!(
        row,
        "| pilot-10k | Example board, aarch64, 14 CPUs, 123 GiB, NVMe SSD | 10,000 | 8 | 800 | \
         1.3 ms | 4.6 ms | 9.0 ms | 0 | 20,480 | 512 MiB | `0123456` |",
        "one row"
    );
}

/// zola writes an aligned column (`---:`) as a style attribute, which the
/// site check (scripts/check-site.sh) refuses on every page.
#[test]
fn edge_the_header_aligns_no_column() {
    let rule = TABLE_HEADER.lines().nth(1).expect("the header's rule line");
    assert!(!rule.contains(':'), "no column alignment: {rule}");
}

#[test]
fn success_the_header_has_as_many_columns_as_a_row() {
    let row = report::table_row(&report::to_json(&run())).expect("a full result");
    for line in TABLE_HEADER.lines() {
        assert_eq!(
            line.matches('|').count(),
            row.matches('|').count(),
            "columns: {line}"
        );
    }
    assert_eq!(TABLE_HEADER.lines().count(), 2, "a header and its rule");
}

#[test]
fn failure_errors_fail_the_run() {
    let mut r = run();
    r.reads.errors = 2;
    let refusal = report::verdict(&r).expect_err("two read errors");
    assert!(
        refusal.contains("reads") && refusal.contains('2'),
        "names the phase: {refusal}"
    );
}

#[test]
fn failure_errors_in_every_phase_are_all_named() {
    let mut r = run();
    r.ingest.errors = 1;
    r.lists.errors = 3;
    let refusal = report::verdict(&r).expect_err("errors");
    assert!(
        refusal.contains("ingest") && refusal.contains("lists"),
        "both: {refusal}"
    );
}

#[test]
fn failure_a_result_without_reads_has_no_row() {
    let mut j = report::to_json(&run());
    j["phases"]["reads"]["latency_us"] = Value::Null;
    let refusal = report::table_row(&j).expect_err("no read latency");
    assert!(
        refusal.contains("reads"),
        "names what is missing: {refusal}"
    );
}

#[test]
fn failure_a_result_of_another_format_has_no_row() {
    let refusal = report::table_row(&json!({"format": "other/9"})).expect_err("other format");
    assert!(refusal.contains("format"), "names the format: {refusal}");
}

#[test]
fn edge_no_count_has_no_bytes_per_vcon() {
    assert_eq!(report::bytes_per_vcon(10, 0), None, "count 0");
}

#[test]
fn edge_a_run_without_disk_facts_writes_nulls() {
    let mut r = run();
    r.data_bytes = None;
    r.peak_rss_kib = None;
    let j = report::to_json(&r);
    assert!(j["disk"]["bytes_per_vcon"].is_null(), "no disk measure");
    assert!(j["server"]["peak_rss_kib"].is_null(), "no memory measure");
    let row = report::table_row(&j).expect("a row without them");
    assert!(row.contains("| n/a | n/a |"), "shown as n/a: {row}");
}

#[test]
fn edge_a_phase_of_no_time_has_a_null_rate() {
    let mut r = run();
    r.ingest.micros = 0;
    assert!(
        report::to_json(&r)["phases"]["ingest"]["per_second"].is_null(),
        "no rate"
    );
}

#[test]
fn success_the_page_table_is_the_header_then_a_row_per_result() {
    let one = report::to_json(&run());
    let table = report::page_table(&[one.clone(), one.clone()]).expect("two results");
    let row = report::table_row(&one).expect("a row");
    assert_eq!(
        table,
        format!("{TABLE_HEADER}\n{row}\n{row}\n"),
        "header, then rows"
    );
}

#[test]
fn failure_a_page_table_with_a_broken_result_is_refused() {
    let refusal = report::page_table(&[report::to_json(&run()), json!({})]).expect_err("broken");
    assert!(
        refusal.contains("format"),
        "the broken result is named: {refusal}"
    );
}

#[test]
fn failure_a_result_with_errors_has_no_place_on_the_page() {
    let mut r = run();
    r.ingest.errors = 1;
    let refusal = report::page_table(&[report::to_json(&r)]).expect_err("errors");
    assert!(
        refusal.contains("error"),
        "a failed run is not published: {refusal}"
    );
}

#[test]
fn edge_no_results_is_no_table() {
    assert_eq!(
        report::page_table(&[]),
        Err("no result to publish".to_owned()),
        "none"
    );
}

#[test]
fn edge_the_result_file_round_trips_through_text() {
    let j = report::to_json(&run());
    let text = serde_json::to_string_pretty(&j).expect("serialized");
    let back: Value = serde_json::from_str(&text).expect("parsed");
    assert_eq!(
        report::table_row(&back),
        report::table_row(&j),
        "the same row"
    );
}
