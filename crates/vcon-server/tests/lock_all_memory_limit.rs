//! Locking all memory fails, and says so, when the lock limit is too low.
//! This lowers the process's lock limit, so this file is its own test
//! binary. It needs a user without CAP_IPC_LOCK, which ignores the limit.

#[test]
fn locking_all_memory_fails_under_a_zero_lock_limit() {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &none) }, 0);
    assert!(vcon_server::envelope::lock_all_memory().is_err());
}
