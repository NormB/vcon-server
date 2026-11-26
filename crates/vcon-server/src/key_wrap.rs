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
//!
//! [`BackendWrap`] puts a key [`Backend`] (a KMS or an HSM, whose key never
//! leaves it) at the root instead. The root is a random key the backend
//! wraps, kept as a `key-wrap` blob ([`blob`]) where the registry is. Each
//! tenant key is wrapped locally under a key-encryption key of its own,
//! with the same AES-256-GCM binding to the tenant id, and the backend
//! wraps that key-encryption key.

use std::{fmt, io};

use crate::envelope::{self, Context, Key, KeyError};
use crate::ids::{KeyId, TenantId};

pub mod blob;

use blob::{Blob, BlobError, WrapRef};

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
    /// The wrapped key is under another key than this backend's, so
    /// nothing was asked of the backend: under the key backend key
    /// `wrap_ref` names, or, when it is None, under the master key file.
    Elsewhere {
        /// The key backend's key the wrapped key is under, as
        /// [`WrapRef`] writes it; None for the master key file.
        wrap_ref: Option<String>,
    },
    /// A `key-wrap` blob could not be read, or the backend's wrapped key
    /// does not fit in one.
    Blob(BlobError),
}

/// The I/O error and the blob's error are given by `source`, not repeated
/// here.
impl fmt::Display for WrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WrapError::Damaged => f.write_str("a wrapped tenant key did not open"),
            WrapError::Memory(_) => {
                f.write_str("the key's memory could not be locked or kept out of core dumps")
            }
            WrapError::Elsewhere { wrap_ref: None } => f.write_str(
                "the key is wrapped under the master key file, not under the key backend's key",
            ),
            WrapError::Elsewhere {
                wrap_ref: Some(wrap_ref),
            } => write!(
                f,
                "the key is wrapped under the key backend key {wrap_ref}, not under this \
                 backend's key"
            ),
            WrapError::Blob(_) => f.write_str("a key-wrap blob could not be read or made"),
        }
    }
}

impl std::error::Error for WrapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WrapError::Memory(e) => Some(e),
            WrapError::Blob(e) => Some(e),
            WrapError::Damaged | WrapError::Elsewhere { .. } => None,
        }
    }
}

/// The one rule for what an envelope failure is to a backend: a page that
/// cannot be locked is [`WrapError::Memory`], and every other failure means
/// the wrapped key did not open (`envelope::unwrap` fails only with
/// `Memory` and `Damaged`).
fn from_key_error(e: KeyError) -> WrapError {
    match e {
        KeyError::Memory(e) => WrapError::Memory(e),
        KeyError::Damaged | KeyError::Read(_) | KeyError::Refused(_) | KeyError::WrongSize => {
            WrapError::Damaged
        }
    }
}

/// A tenant's key-encryption key as a key backend wrapped it: the
/// backend's key as [`WrapRef`] writes it, and the `key-wrap` blob. A
/// registry keeps both or neither.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WrappedKek {
    /// The key backend's key, as [`WrapRef`] writes it.
    pub wrap_ref: String,
    /// The `key-wrap` blob of the key-encryption key.
    pub blob: Vec<u8>,
}

/// A tenant key as a backend wrapped it, as the registry keeps it: the
/// wrapped key, and the key-encryption key it is wrapped under, as the key
/// backend wrapped that; None when the wrapped key is under the master key
/// file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Wrapped {
    /// The tenant key, wrapped bound to its tenant.
    pub key: Vec<u8>,
    /// The key it is wrapped under, as the key backend wrapped that.
    pub kek: Option<WrappedKek>,
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
    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Wrapped, WrapError>;

    /// Unwraps a key [`KeyWrap::wrap`] made for `tenant`, into a locked page
    /// of its own: `wrapped` under the key-encryption key `kek`, or under
    /// the master key file when `kek` is None.
    ///
    /// # Errors
    ///
    /// [`WrapError::Elsewhere`] when the key is under another key than this
    /// backend's, [`WrapError::Damaged`] when `wrapped` does not open for
    /// `tenant` under the backend's key, [`WrapError::Blob`] when `kek`'s
    /// blob cannot be read, and [`WrapError::Memory`] when the key's page
    /// cannot be locked.
    fn unwrap(
        &self,
        tenant: TenantId,
        wrapped: &[u8],
        kek: Option<&WrappedKek>,
    ) -> Result<Key, WrapError>;
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

    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Wrapped, WrapError> {
        Ok(Wrapped {
            key: envelope::wrap(&self.master, &Context::Tenant { tenant }, key),
            kek: None,
        })
    }

    fn unwrap(
        &self,
        tenant: TenantId,
        wrapped: &[u8],
        kek: Option<&WrappedKek>,
    ) -> Result<Key, WrapError> {
        if let Some(kek) = kek {
            return Err(WrapError::Elsewhere {
                wrap_ref: Some(kek.wrap_ref.clone()),
            });
        }
        envelope::unwrap(&self.master, &Context::Tenant { tenant }, wrapped).map_err(from_key_error)
    }
}

/// What a key backend is asked to wrap or unwrap: the registry root, or
/// the key-encryption key of one tenant. A backend that can bind a context
/// to what it wraps binds this one, so a blob made for one does not open
/// as another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WrapContext {
    /// The root, from which the server catalog key is derived.
    Registry,
    /// The key-encryption key of this tenant's key.
    Tenant(TenantId),
}

/// A key backend: a service or a device that holds a key which never
/// leaves it, and wraps and unwraps keys under that key. Its methods are
/// synchronous: the registry calls them from blocking threads.
pub trait Backend: Send + Sync + fmt::Debug {
    /// The backend's key, as every blob it makes records it.
    fn reference(&self) -> WrapRef;

    /// Wraps `key` for `ctx`, and gives the backend's ciphertext.
    ///
    /// # Errors
    ///
    /// A [`WrapError`] when the backend does not wrap it.
    fn wrap(&self, ctx: WrapContext, key: &Key) -> Result<Vec<u8>, WrapError>;

    /// Unwraps the ciphertext of `blob`, made for `ctx`, into a locked page
    /// of its own.
    ///
    /// # Errors
    ///
    /// [`WrapError::Damaged`] when the ciphertext does not open for `ctx`,
    /// and any other [`WrapError`] when the backend does not unwrap it.
    fn unwrap(&self, ctx: WrapContext, blob: &Blob) -> Result<Key, WrapError>;
}

/// The backend over a key [`Backend`]: its root is a random key the
/// backend wraps, and each tenant key is wrapped locally under a new
/// key-encryption key of its own, bound to the tenant, which the backend
/// wraps. Debug shows the backend, its key and the root's id only.
#[derive(Debug)]
pub struct BackendWrap<B> {
    backend: B,
    reference: WrapRef,
    root: Key,
}

impl<B: Backend> BackendWrap<B> {
    /// A new root, wrapped by `backend`: the backend over it, and the
    /// root's `key-wrap` blob, to be kept where the registry's root is
    /// read from.
    ///
    /// # Errors
    ///
    /// A [`WrapError`] when the root cannot be made or the backend does not
    /// wrap it.
    pub fn create(backend: B) -> Result<(BackendWrap<B>, Vec<u8>), WrapError> {
        let root = Key::generate().map_err(from_key_error)?;
        let reference = backend.reference();
        let ciphertext = backend.wrap(WrapContext::Registry, &root)?;
        let blob = Blob::new(reference.clone(), ciphertext).map_err(WrapError::Blob)?;
        Ok((
            BackendWrap {
                backend,
                reference,
                root,
            },
            blob.encode(),
        ))
    }

    /// The backend over the root `root_blob` holds, as [`BackendWrap::create`]
    /// gave it; nothing is asked of the backend when the blob names another
    /// key than its own.
    ///
    /// # Errors
    ///
    /// [`WrapError::Blob`] when the blob cannot be read,
    /// [`WrapError::Elsewhere`] when it names another key, and any error
    /// of the backend's unwrap.
    pub fn open(backend: B, root_blob: &[u8]) -> Result<BackendWrap<B>, WrapError> {
        let reference = backend.reference();
        let blob = under(&reference, root_blob)?;
        let root = backend.unwrap(WrapContext::Registry, &blob)?;
        Ok(BackendWrap {
            backend,
            reference,
            root,
        })
    }
}

/// The blob `bytes` holds, when it is under the key `reference` names; a
/// blob under another key is [`WrapError::Elsewhere`], found before any
/// backend is asked.
fn under(reference: &WrapRef, bytes: &[u8]) -> Result<Blob, WrapError> {
    let blob = Blob::decode(bytes).map_err(WrapError::Blob)?;
    if blob.reference() != reference {
        return Err(WrapError::Elsewhere {
            wrap_ref: Some(blob.reference().to_string()),
        });
    }
    Ok(blob)
}

impl<B: Backend> KeyWrap for BackendWrap<B> {
    /// The root's id: each key-encryption key wraps one tenant key only, so
    /// the count of wraps against the root bounds nothing else.
    fn key_id(&self) -> KeyId {
        self.root.id()
    }

    fn root(&self) -> &Key {
        &self.root
    }

    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Wrapped, WrapError> {
        let kek = Key::generate().map_err(from_key_error)?;
        let wrapped = envelope::wrap(&kek, &Context::Tenant { tenant }, key);
        let ciphertext = self.backend.wrap(WrapContext::Tenant(tenant), &kek)?;
        let blob = Blob::new(self.reference.clone(), ciphertext).map_err(WrapError::Blob)?;
        Ok(Wrapped {
            key: wrapped,
            kek: Some(WrappedKek {
                wrap_ref: self.reference.to_string(),
                blob: blob.encode(),
            }),
        })
    }

    fn unwrap(
        &self,
        tenant: TenantId,
        wrapped: &[u8],
        kek: Option<&WrappedKek>,
    ) -> Result<Key, WrapError> {
        let kek = kek.ok_or(WrapError::Elsewhere { wrap_ref: None })?;
        let blob = under(&self.reference, &kek.blob)?;
        // The row keeps the reference beside the blob, which names it too:
        // two that disagree are a row no build writes.
        if blob.reference().to_string() != kek.wrap_ref {
            return Err(WrapError::Damaged);
        }
        let kek = self.backend.unwrap(WrapContext::Tenant(tenant), &blob)?;
        envelope::unwrap(&kek, &Context::Tenant { tenant }, wrapped).map_err(from_key_error)
    }
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod backend_tests;
