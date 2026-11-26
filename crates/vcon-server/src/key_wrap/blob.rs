//! The `key-wrap` format: a key as a key backend wrapped it, with the
//! backend's kind and the reference of the key that wrapped it, so a reader
//! knows which backend and key opens it before it calls any backend.
//!
//! The layout, version 1 (every length big-endian):
//!
//! | Bytes | What |
//! |---|---|
//! | 1 | the format, [`FORMAT`] |
//! | 1 | the backend's kind ([`BackendKind`]) |
//! | 2 | the length of the key reference, 1 to [`MAX_KEY_REF`] |
//! | that many | the key reference, printable ASCII with no space |
//! | 4 | the length of the ciphertext, 1 to [`MAX_CIPHERTEXT`] |
//! | that many | the ciphertext, as the backend returned it |
//!
//! Nothing follows the ciphertext. A reader refuses a format it does not
//! know, and names it, before it reads anything else.

use std::fmt;

/// The version of the `key-wrap` format this build writes and reads.
pub const FORMAT: u8 = 1;

/// The longest key reference a blob holds, in bytes.
pub const MAX_KEY_REF: usize = 1024;

/// The longest ciphertext a blob holds, in bytes.
pub const MAX_CIPHERTEXT: usize = 8192;

/// The kind of key backend that wrapped a key. The file backend wraps
/// nothing in this format (a tenant key under the master key file is an
/// `envelope` blob), so it has no kind here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BackendKind {
    /// AWS Key Management Service.
    AwsKms,
    /// Google Cloud Key Management.
    GcpKms,
    /// Azure Key Vault.
    AzureKeyVault,
    /// HashiCorp Vault's transit engine.
    VaultTransit,
    /// OpenBao's transit engine.
    OpenbaoTransit,
    /// A PKCS#11 token or hardware security module.
    Pkcs11,
}

impl BackendKind {
    /// Every kind, in the order of their bytes.
    pub const ALL: [BackendKind; 6] = [
        BackendKind::AwsKms,
        BackendKind::GcpKms,
        BackendKind::AzureKeyVault,
        BackendKind::VaultTransit,
        BackendKind::OpenbaoTransit,
        BackendKind::Pkcs11,
    ];

    /// The kind's name, as the configuration and a key reference's text
    /// give it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::AwsKms => "aws-kms",
            BackendKind::GcpKms => "gcp-kms",
            BackendKind::AzureKeyVault => "azure-key-vault",
            BackendKind::VaultTransit => "vault-transit",
            BackendKind::OpenbaoTransit => "openbao-transit",
            BackendKind::Pkcs11 => "pkcs11",
        }
    }

    /// The kind named `name`, if there is one. Names are lowercase.
    #[must_use]
    pub fn from_name(name: &str) -> Option<BackendKind> {
        BackendKind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// The kind's byte in a blob: its place in [`BackendKind::ALL`], from 1.
    fn byte(self) -> u8 {
        match self {
            BackendKind::AwsKms => 1,
            BackendKind::GcpKms => 2,
            BackendKind::AzureKeyVault => 3,
            BackendKind::VaultTransit => 4,
            BackendKind::OpenbaoTransit => 5,
            BackendKind::Pkcs11 => 6,
        }
    }

    /// The kind whose byte is `byte`, if there is one.
    fn from_byte(byte: u8) -> Option<BackendKind> {
        BackendKind::ALL.into_iter().find(|k| k.byte() == byte)
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a `key-wrap` blob, or a part of one, was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlobError {
    /// The blob has no bytes.
    Empty,
    /// The blob's first byte is a format this build does not know.
    UnknownFormat(u8),
    /// The blob names a backend kind this build does not know.
    UnknownKind(u8),
    /// The blob ends before its last field does.
    Truncated,
    /// Bytes follow the blob's ciphertext.
    TrailingBytes,
    /// The key reference is empty, too long, or not printable ASCII.
    KeyRef,
    /// The ciphertext is empty or too long.
    Ciphertext,
    /// A key reference's text does not start with a known kind's name and
    /// a colon.
    RefText,
}

impl fmt::Display for BlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlobError::Empty => f.write_str("a key-wrap blob is empty"),
            BlobError::UnknownFormat(n) => {
                write!(f, "key-wrap format {n} is not one this build knows")
            }
            BlobError::UnknownKind(n) => write!(
                f,
                "a key-wrap blob names key backend {n}, which this build does not know"
            ),
            BlobError::Truncated => f.write_str("a key-wrap blob is cut short"),
            BlobError::TrailingBytes => f.write_str("a key-wrap blob has bytes after its end"),
            BlobError::KeyRef => write!(
                f,
                "a key reference is empty, longer than {MAX_KEY_REF} bytes, or not printable ASCII"
            ),
            BlobError::Ciphertext => write!(
                f,
                "a wrapped key's ciphertext is empty or longer than {MAX_CIPHERTEXT} bytes"
            ),
            BlobError::RefText => {
                f.write_str("a key reference does not start with a key backend's name and a colon")
            }
        }
    }
}

impl std::error::Error for BlobError {}

/// The key a backend wrapped under: its backend's kind and the backend's
/// own reference of the key and its version (an ARN, a key URL with its
/// version, a transit mount and key, a token object's id).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WrapRef {
    kind: BackendKind,
    key: String,
}

impl WrapRef {
    /// The reference of `key` at a backend of `kind`.
    ///
    /// # Errors
    ///
    /// [`BlobError::KeyRef`] when `key` is empty, longer than
    /// [`MAX_KEY_REF`] bytes, or holds a byte that is not printable ASCII
    /// (a space included).
    pub fn new(kind: BackendKind, key: impl Into<String>) -> Result<WrapRef, BlobError> {
        let key = key.into();
        if !valid_key(key.as_bytes()) {
            return Err(BlobError::KeyRef);
        }
        Ok(WrapRef { kind, key })
    }

    /// The reference as [`WrapRef`]'s `Display` writes it: the kind's name,
    /// a colon, the key.
    ///
    /// # Errors
    ///
    /// [`BlobError::RefText`] when the text does not start with a known
    /// kind's name and a colon, and [`BlobError::KeyRef`] when the key after
    /// it is not one [`WrapRef::new`] takes.
    pub fn parse(text: &str) -> Result<WrapRef, BlobError> {
        let (name, key) = text.split_once(':').ok_or(BlobError::RefText)?;
        let kind = BackendKind::from_name(name).ok_or(BlobError::RefText)?;
        WrapRef::new(kind, key)
    }

    /// The backend's kind.
    #[must_use]
    pub fn kind(&self) -> BackendKind {
        self.kind
    }

    /// The backend's reference of the key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Display for WrapRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.key)
    }
}

/// A key as a backend wrapped it. Debug shows the reference and the
/// ciphertext's length, never its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Blob {
    reference: WrapRef,
    ciphertext: Vec<u8>,
}

impl Blob {
    /// The blob of `ciphertext`, made by the key `reference` names.
    ///
    /// # Errors
    ///
    /// [`BlobError::Ciphertext`] when `ciphertext` is empty or longer than
    /// [`MAX_CIPHERTEXT`] bytes.
    pub fn new(reference: WrapRef, ciphertext: Vec<u8>) -> Result<Blob, BlobError> {
        if !(1..=MAX_CIPHERTEXT).contains(&ciphertext.len()) {
            return Err(BlobError::Ciphertext);
        }
        Ok(Blob {
            reference,
            ciphertext,
        })
    }

    /// The blob's bytes, in the layout of format [`FORMAT`].
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let key = self.reference.key.as_bytes();
        // Both lengths fit: `WrapRef::new` and `Blob::new` hold them to
        // their limits, which fit in two and four bytes.
        let key_len = u16::try_from(key.len()).unwrap_or(u16::MAX);
        let ct_len = u32::try_from(self.ciphertext.len()).unwrap_or(u32::MAX);
        let mut out = Vec::with_capacity(8 + key.len() + self.ciphertext.len());
        out.push(FORMAT);
        out.push(self.reference.kind.byte());
        out.extend_from_slice(&key_len.to_be_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&ct_len.to_be_bytes());
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// The blob `bytes` hold.
    ///
    /// # Errors
    ///
    /// A [`BlobError`] naming the first thing wrong, the format first.
    pub fn decode(bytes: &[u8]) -> Result<Blob, BlobError> {
        let (&format, rest) = bytes.split_first().ok_or(BlobError::Empty)?;
        if format != FORMAT {
            return Err(BlobError::UnknownFormat(format));
        }
        let (&kind, rest) = rest.split_first().ok_or(BlobError::Truncated)?;
        let kind = BackendKind::from_byte(kind).ok_or(BlobError::UnknownKind(kind))?;
        let (key_len, rest) = take::<2>(rest)?;
        let key_len = usize::from(u16::from_be_bytes(key_len));
        if !(1..=MAX_KEY_REF).contains(&key_len) {
            return Err(BlobError::KeyRef);
        }
        let (key, rest) = rest.split_at_checked(key_len).ok_or(BlobError::Truncated)?;
        if !valid_key(key) {
            return Err(BlobError::KeyRef);
        }
        let (ct_len, rest) = take::<4>(rest)?;
        let ct_len = usize::try_from(u32::from_be_bytes(ct_len)).unwrap_or(usize::MAX);
        if !(1..=MAX_CIPHERTEXT).contains(&ct_len) {
            return Err(BlobError::Ciphertext);
        }
        let (ciphertext, rest) = rest.split_at_checked(ct_len).ok_or(BlobError::Truncated)?;
        if !rest.is_empty() {
            return Err(BlobError::TrailingBytes);
        }
        // `valid_key` took only ASCII bytes, so the key is UTF-8.
        let key = String::from_utf8(key.to_vec()).map_err(|_| BlobError::KeyRef)?;
        Ok(Blob {
            reference: WrapRef { kind, key },
            ciphertext: ciphertext.to_vec(),
        })
    }

    /// The key that wrapped it.
    #[must_use]
    pub fn reference(&self) -> &WrapRef {
        &self.reference
    }

    /// The backend's ciphertext.
    #[must_use]
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }
}

impl fmt::Debug for Blob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("reference", &self.reference)
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

/// Whether `key` is a key reference a blob may hold: 1 to [`MAX_KEY_REF`]
/// bytes, each printable ASCII other than the space (`!` to `~`), so it is
/// safe in a log line and in a TEXT column.
fn valid_key(key: &[u8]) -> bool {
    (1..=MAX_KEY_REF).contains(&key.len()) && key.iter().all(u8::is_ascii_graphic)
}

/// The first `N` bytes of `bytes` and the rest.
fn take<const N: usize>(bytes: &[u8]) -> Result<([u8; N], &[u8]), BlobError> {
    let (head, rest) = bytes.split_at_checked(N).ok_or(BlobError::Truncated)?;
    let head = head.try_into().map_err(|_| BlobError::Truncated)?;
    Ok((head, rest))
}

#[cfg(test)]
mod tests;
