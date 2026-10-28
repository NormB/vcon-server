//! Locking all memory covers every page the process has and every page it
//! gets later. Locked memory is counted for the whole process, so this file
//! is its own test binary with one test.

/// A field of `/proc/self/status`, in kB.
fn status_kb(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    status
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix(field)?.strip_prefix(':')?;
            value.trim().strip_suffix(" kB")?.parse().ok()
        })
        .unwrap_or_else(|| panic!("{field} in /proc/self/status"))
}

#[test]
fn mlock_every_page_now_and_later_is_locked() {
    assert_eq!(status_kb("VmLck"), 0, "nothing is locked before");
    vcon_server::envelope::lock_all_memory().expect("locks all memory");
    let locked = status_kb("VmLck");
    assert!(
        locked >= status_kb("VmRSS"),
        "{locked} kB locked, less than the resident set"
    );
    let grown: Vec<u8> = std::hint::black_box(Vec::with_capacity(64 << 20));
    assert!(
        status_kb("VmLck") >= locked + (64 << 10),
        "a new 64 MiB allocation is locked"
    );
    drop(grown);
}

// A name no build holds: fn mlock_ghost_test
