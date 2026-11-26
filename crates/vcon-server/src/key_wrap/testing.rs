//! A key backend for tests: it holds a key of its own, which never leaves
//! it, wraps under it with the envelope's AES-256-GCM bound to the context
//! asked, logs every call, and gives failures in the order a test queues
//! them. Clones share the key, the log and the queues, so a test keeps a
//! handle on a backend a registry holds.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::blob::{BackendKind, Blob, WrapRef};
use super::{Backend, WrapContext, WrapError};
use crate::envelope::{self, Context, Key};
use crate::ids::TenantId;

/// The tenant id the scripted backend binds the registry root to. No test
/// makes a tenant with it.
const REGISTRY_TENANT: TenantId = TenantId([0xff; 16]);

/// What was asked of the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Call {
    Wrap(WrapContext),
    Unwrap(WrapContext),
}

/// A failure the backend is told to give at its next call of a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fail {
    Damaged,
    Memory,
    /// A wrap that gives this many bytes of ciphertext.
    Ciphertext(usize),
}

#[derive(Debug)]
struct Inner {
    key: Key,
    reference: WrapRef,
    calls: Mutex<Vec<Call>>,
    wraps: Mutex<VecDeque<Option<Fail>>>,
    unwraps: Mutex<VecDeque<Option<Fail>>>,
}

#[derive(Debug, Clone)]
pub(crate) struct Scripted(Arc<Inner>);

impl Scripted {
    /// A backend of `kind` whose key `key` names, with a new key.
    pub(crate) fn new(kind: BackendKind, key: &str) -> Scripted {
        Scripted::with_key(kind, key, Key::generate().expect("a key"))
    }

    /// A backend of `kind` whose key `key` names, holding `held`.
    pub(crate) fn with_key(kind: BackendKind, key: &str, held: Key) -> Scripted {
        Scripted(Arc::new(Inner {
            key: held,
            reference: WrapRef::new(kind, key).expect("a valid key reference"),
            calls: Mutex::new(Vec::new()),
            wraps: Mutex::new(VecDeque::new()),
            unwraps: Mutex::new(VecDeque::new()),
        }))
    }

    /// The same key under the same reference, with a new log and queues:
    /// the backend as a process started again finds it.
    pub(crate) fn restarted(&self) -> Scripted {
        Scripted::with_key(
            self.0.reference.kind(),
            self.0.reference.key(),
            copy(&self.0.key),
        )
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.0.calls.lock().expect("calls").clone()
    }

    pub(crate) fn clear_calls(&self) {
        self.0.calls.lock().expect("calls").clear();
    }

    pub(crate) fn fail_wraps(&self, fails: &[Option<Fail>]) {
        self.0.wraps.lock().expect("wraps").extend(fails);
    }

    pub(crate) fn fail_unwraps(&self, fails: &[Option<Fail>]) {
        self.0.unwraps.lock().expect("unwraps").extend(fails);
    }
}

/// A second key with `key`'s bytes, in a page of its own (Key is not
/// Clone): `key` wrapped under itself, then unwrapped.
pub(crate) fn copy(key: &Key) -> Key {
    let ctx = context(WrapContext::Registry);
    envelope::unwrap(key, &ctx, &envelope::wrap(key, &ctx, key)).expect("a copy of the key")
}

fn context(ctx: WrapContext) -> Context {
    match ctx {
        WrapContext::Registry => Context::Tenant {
            tenant: REGISTRY_TENANT,
        },
        WrapContext::Tenant(tenant) => Context::Tenant { tenant },
    }
}

fn error(fail: Fail) -> WrapError {
    match fail {
        Fail::Damaged | Fail::Ciphertext(_) => WrapError::Damaged,
        Fail::Memory => WrapError::Memory(std::io::Error::from_raw_os_error(libc::ENOMEM)),
    }
}

impl Backend for Scripted {
    fn reference(&self) -> WrapRef {
        self.0.reference.clone()
    }

    fn wrap(&self, ctx: WrapContext, key: &Key) -> Result<Vec<u8>, WrapError> {
        self.0.calls.lock().expect("calls").push(Call::Wrap(ctx));
        match self.0.wraps.lock().expect("wraps").pop_front().flatten() {
            Some(Fail::Ciphertext(len)) => Ok(vec![7; len]),
            Some(fail) => Err(error(fail)),
            None => Ok(envelope::wrap(&self.0.key, &context(ctx), key)),
        }
    }

    fn unwrap(&self, ctx: WrapContext, blob: &Blob) -> Result<Key, WrapError> {
        self.0.calls.lock().expect("calls").push(Call::Unwrap(ctx));
        match self
            .0
            .unwraps
            .lock()
            .expect("unwraps")
            .pop_front()
            .flatten()
        {
            Some(fail) => Err(error(fail)),
            None => {
                envelope::unwrap(&self.0.key, &context(ctx), blob.ciphertext()).map_err(|e| match e
                {
                    envelope::KeyError::Memory(e) => WrapError::Memory(e),
                    _ => WrapError::Damaged,
                })
            }
        }
    }
}
