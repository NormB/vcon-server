//! A key that cannot be locked in memory is refused, not handed out in
//! memory that could reach swap. This lowers the process's lock limit, so
//! this file is its own test binary.

use vcon_server::envelope::{Key, KeyError};

#[test]
fn a_key_is_refused_when_its_memory_cannot_be_locked() {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &none) }, 0);
    match Key::generate() {
        Err(KeyError::Memory(_)) => {}
        Ok(_) => panic!("a key was made in memory that is not locked"),
    }
}
