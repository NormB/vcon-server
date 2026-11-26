//! The key-wrapping backend: what holds the root of the key hierarchy and
//! wraps each tenant key under it. The registry asks it for the root, from
//! which the server catalog key is derived, and has it wrap and unwrap
//! tenant keys; it never sees a data key or a body.
//!
//! [`FileWrap`] is the backend every release so far has used: the master key
//! from its file is the root, and a tenant key is wrapped under it with
//! AES-256-GCM bound to the tenant id ([`envelope::wrap`] with
//! [`Context::Tenant`]). What it writes is byte for byte what the code before
//! this trait wrote, so data made by an older release opens unchanged.

use std::{fmt, io};

use crate::envelope::{self, Context, Key, KeyError};
use crate::ids::{KeyId, TenantId};

pub mod blob;

/// Why a backend could not wrap or unwrap a key.
#[derive(Debug)]
#[non_exhaustive]
pub enum WrapError {
    /// The wrapped key did not open under the backend's key for this
    /// tenant: it was changed, cut short, moved from another tenant's row,
    /// or made under another key. It does not say which part was wrong.
    Damaged,
    /// The key's memory could not be locked or kept out of core dumps.
    Memory(io::Error),
}

/// The I/O error itself is given by `source`, not repeated here.
impl fmt::Display for WrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WrapError::Damaged => "a wrapped tenant key did not open",
            WrapError::Memory(_) => {
                "the key's memory could not be locked or kept out of core dumps"
            }
        })
    }
}

impl std::error::Error for WrapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WrapError::Memory(e) => Some(e),
            WrapError::Damaged => None,
        }
    }
}

/// A key-wrapping backend. Its methods are synchronous: the registry calls
/// them from blocking threads.
pub trait KeyWrap: Send + Sync + fmt::Debug {
    /// The id of the key that wraps tenant keys. The registry counts each
    /// wrap against it, and a key rotation records when its use began.
    fn key_id(&self) -> KeyId;

    /// The root of the hierarchy: the key the server catalog key is derived
    /// from ([`envelope::catalog_key`]).
    fn root(&self) -> &Key;

    /// Wraps `key`, the tenant key of `tenant`, bound to that tenant.
    ///
    /// # Errors
    ///
    /// A [`WrapError`] when the backend cannot wrap it.
    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Vec<u8>, WrapError>;

    /// Unwraps a key [`KeyWrap::wrap`] made for `tenant`, into a locked page
    /// of its own.
    ///
    /// # Errors
    ///
    /// [`WrapError::Damaged`] when `wrapped` does not open for `tenant`
    /// under the backend's key, and [`WrapError::Memory`] when the key's page
    /// cannot be locked.
    fn unwrap(&self, tenant: TenantId, wrapped: &[u8]) -> Result<Key, WrapError>;
}

/// The master key file's backend: the master key is the root, and tenant
/// keys are wrapped under it locally. Debug shows only the key's id.
#[derive(Debug)]
pub struct FileWrap {
    master: Key,
}

impl FileWrap {
    /// The backend over `master`, as loaded by [`Key::load_master`].
    #[must_use]
    pub fn new(master: Key) -> FileWrap {
        FileWrap { master }
    }
}

impl KeyWrap for FileWrap {
    fn key_id(&self) -> KeyId {
        self.master.id()
    }

    fn root(&self) -> &Key {
        &self.master
    }

    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Vec<u8>, WrapError> {
        Ok(envelope::wrap(
            &self.master,
            &Context::Tenant { tenant },
            key,
        ))
    }

    fn unwrap(&self, tenant: TenantId, wrapped: &[u8]) -> Result<Key, WrapError> {
        // `envelope::unwrap` fails only with these two (envelope.rs, `unwrap`).
        envelope::unwrap(&self.master, &Context::Tenant { tenant }, wrapped).map_err(|e| match e {
            KeyError::Memory(e) => WrapError::Memory(e),
            KeyError::Damaged | KeyError::Read(_) | KeyError::Refused(_) | KeyError::WrongSize => {
                WrapError::Damaged
            }
        })
    }
}

#[cfg(test)]
mod tests;
