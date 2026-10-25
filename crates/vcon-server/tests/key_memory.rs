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

/// The address range and `VmFlags` of every mapping that has the flag
/// `flag`: `lo` for locked pages, `dd` for pages core dumps leave out.
fn mappings_with(flag: &str) -> Vec<(String, String)> {
    let smaps = std::fs::read_to_string("/proc/self/smaps").expect("read /proc/self/smaps");
    let mut range = String::new();
    let mut found = Vec::new();
    for line in smaps.lines() {
        if let Some(flags) = line.strip_prefix("VmFlags:") {
            if flags.split_whitespace().any(|f| f == flag) {
                found.push((range.clone(), flags.to_owned()));
            }
        } else if let Some((first, _)) = line.split_once(' ')
            && first.contains('-')
            && first.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        {
            range = first.to_owned();
        }
    }
    found
}

fn page_kb() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(page).expect("a page size") / 1024
}

/// A page core dumps leave out that is not a key's, as some kernels map for
/// their own use (GitHub's x86_64 runners have two at start).
fn foreign_page_left_out() {
    let len = usize::try_from(page_kb() * 1024).expect("a page");
    // SAFETY: a fresh private anonymous mapping, never freed: the test
    // process ends with it.
    unsafe {
        let addr = libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );
        assert_ne!(addr, libc::MAP_FAILED, "map a page");
        assert_eq!(
            libc::madvise(addr, len, libc::MADV_DONTDUMP),
            0,
            "leave it out"
        );
    }
}

#[test]
fn each_key_has_its_own_locked_page_left_out_of_core_dumps() {
    foreign_page_left_out();
    assert_eq!(status_kb("VmLck"), 0, "nothing is locked before");
    let left_out_before = mappings_with("dd");
    let first = Key::generate().expect("first key");
    let second = Key::generate().expect("second key");
    assert_eq!(status_kb("VmLck"), 2 * page_kb(), "one locked page per key");
    let locked = mappings_with("lo");
    assert!(!locked.is_empty(), "the keys' mappings are listed");
    for (range, flags) in &locked {
        assert!(
            flags.split_whitespace().any(|f| f == "dd"),
            "not left out of core dumps: {range} {flags}"
        );
        assert!(
            !left_out_before.iter().any(|(r, _)| r == range),
            "a key's page is new: {range}"
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
        left_out_before,
        "both pages are unmapped"
    );
}
