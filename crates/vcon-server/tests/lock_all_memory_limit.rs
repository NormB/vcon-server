//! Locking all memory fails, and says why, when the lock limit is too low.
//! This lowers the process's lock limit, so this file is its own test
//! binary. It needs a user without CAP_IPC_LOCK, which ignores the limit.

fn lock_under(limit: libc::rlim_t) -> std::io::Error {
    let lowered = libc::rlimit {
        rlim_cur: limit,
        rlim_max: limit,
    };
    // SAFETY: `lowered` is a valid rlimit.
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &lowered) },
        0
    );
    vcon_server::envelope::lock_all_memory().expect_err("the lock is refused")
}

/// mlockall(2): ENOMEM when the limit is above zero but smaller than the
/// memory to lock, EPERM when it is zero. The limit is only lowered, so the
/// small limit is tried first.
#[test]
fn locking_all_memory_fails_under_a_low_lock_limit_and_says_why() {
    assert_eq!(lock_under(64 * 1024).raw_os_error(), Some(libc::ENOMEM));
    assert_eq!(lock_under(0).raw_os_error(), Some(libc::EPERM));
}
