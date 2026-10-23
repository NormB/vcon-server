//! Each key lives in its own locked page that core dumps leave out, and the
//! page is released when the key is dropped. Locked memory is counted for
//! the whole process, so this file is its own test binary with one test.

use vcon_server::envelope::Key;

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

/// The `VmFlags` of every mapping that has the flag `flag`: `lo` for
/// locked pages, `dd` for pages core dumps leave out.
fn mappings_with(flag: &str) -> Vec<String> {
    let smaps = std::fs::read_to_string("/proc/self/smaps").expect("read /proc/self/smaps");
    smaps
        .lines()
        .filter_map(|line| line.strip_prefix("VmFlags:"))
        .filter(|flags| flags.split_whitespace().any(|f| f == flag))
        .map(str::to_owned)
        .collect()
}

fn page_kb() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(page).expect("a page size") / 1024
}

#[test]
fn each_key_has_its_own_locked_page_left_out_of_core_dumps() {
    assert_eq!(status_kb("VmLck"), 0, "nothing is locked before");
    assert_eq!(
        mappings_with("dd"),
        Vec::<String>::new(),
        "nothing is left out before"
    );
    let first = Key::generate().expect("first key");
    let second = Key::generate().expect("second key");
    assert_eq!(status_kb("VmLck"), 2 * page_kb(), "one locked page per key");
    let flags = mappings_with("lo");
    assert!(!flags.is_empty(), "the keys' mappings are listed");
    for f in &flags {
        assert!(
            f.split_whitespace().any(|f| f == "dd"),
            "not left out of core dumps: {f}"
        );
    }
    drop(first);
    assert_eq!(
        status_kb("VmLck"),
        page_kb(),
        "dropping one key keeps the other locked"
    );
    drop(second);
    assert_eq!(status_kb("VmLck"), 0, "dropping both releases both pages");
    assert_eq!(
        mappings_with("dd"),
        Vec::<String>::new(),
        "both pages are unmapped"
    );
}
