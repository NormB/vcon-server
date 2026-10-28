//! A key that cannot be locked in memory is refused, not handed out in
//! memory that could reach swap, whether it is generated or unwrapped. This
//! lowers the process's lock limit, so this file is its own test binary.
//! It needs a user without CAP_IPC_LOCK, which ignores the limit.

use vcon_server::envelope::{Context, Key, KeyError, unwrap, wrap};
use vcon_server::ids::TenantId;

#[test]
fn a_key_is_refused_when_its_memory_cannot_be_locked() {
    let ctx = Context::Tenant {
        tenant: TenantId([1; 16]),
    };
    let wrapping = Key::generate().expect("wrapping key");
    let wrapped = wrap(&wrapping, &ctx, &Key::generate().expect("key"));
    assert!(unwrap(&wrapping, &ctx, &wrapped).is_ok(), "unwraps before");

    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit.
    let set = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &none) };
    assert_eq!(set, 0, "a process may always lower its own limit");
    assert!(
        matches!(Key::generate(), Err(KeyError::Memory(_))),
        "generated in memory that is not locked"
    );
    assert!(
        matches!(unwrap(&wrapping, &ctx, &wrapped), Err(KeyError::Memory(_))),
        "unwrapped into memory that is not locked"
    );
}
