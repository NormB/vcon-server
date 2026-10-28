//! Core dumps are turned off for the whole process, so this file is its own
//! test binary.

fn dumpable() -> libc::c_int {
    // SAFETY: PR_GET_DUMPABLE takes no further arguments.
    unsafe { libc::prctl(libc::PR_GET_DUMPABLE) }
}

#[test]
fn turning_core_dumps_off_clears_the_dumpable_flag_and_the_core_limit() {
    assert_eq!(dumpable(), 1, "dumpable before");
    vcon_server::envelope::disable_core_dumps().expect("core dumps off");
    assert_eq!(dumpable(), 0, "dumpable after");
    let mut limit = libc::rlimit {
        rlim_cur: 1,
        rlim_max: 1,
    };
    // SAFETY: `limit` is a valid rlimit to write into.
    let read = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) };
    assert_eq!(read, 0, "getrlimit succeeds for a valid resource");
    assert_eq!((limit.rlim_cur, limit.rlim_max), (0, 0), "core size limit");
}
