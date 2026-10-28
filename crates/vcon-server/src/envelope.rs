//! Keys and the encryption that protects every stored body.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::digest::{SHA256, digest};
use aws_lc_rs::hkdf::{HKDF_SHA256, KeyType, Salt};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::ids::{KeyId, TenantId, VersionId};

mod locked;

use locked::LockedPage;

/// The length of every key, in bytes.
const KEY_LEN: usize = 32;

/// The first byte of every blob, naming its layout and algorithm:
/// AES-256-GCM with a 96-bit nonce and a 128-bit tag.
const FORMAT: u8 = 1;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// The start of every associated data.
const LABEL: &[u8] = b"vcon-server/v1";

/// The HKDF info that makes a catalog key.
const CATALOG_LABEL: &[u8] = b"vcon-server/v1 catalog";

/// A 256-bit key. Its bytes live only in a page of their own that is locked
/// in memory, so they never reach swap, and that core dumps leave out; they
/// are zeroed when the key is dropped. A key cannot be copied, and Debug
/// shows only its id.
pub struct Key {
    page: LockedPage,
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("id", &format_args!("{}", self.id()))
            .finish_non_exhaustive()
    }
}

impl Key {
    /// Makes a new random key, refusing if its memory cannot be locked.
    ///
    /// # Errors
    ///
    /// [`KeyError::Memory`] if the key's page cannot be locked or kept out
    /// of core dumps.
    ///
    /// # Panics
    ///
    /// Never: AWS-LC aborts the process rather than fail to give random
    /// bytes.
    pub fn generate() -> Result<Key, KeyError> {
        let mut page = LockedPage::new().map_err(KeyError::Memory)?;
        // AWS-LC's RAND_bytes aborts the process when it cannot get enough
        // randomness, so `fill` never returns its error.
        aws_lc_rs::rand::fill(&mut page.bytes_mut()[..KEY_LEN])
            .expect("AWS-LC aborts instead of failing");
        Ok(Key { page })
    }

    /// Loads the master key: the file must hold exactly 32 bytes. They are
    /// read straight into the key's own locked page.
    ///
    /// # Errors
    ///
    /// [`KeyError::Read`] if the file cannot be opened or read,
    /// [`KeyError::WrongSize`] if it holds other than 32 bytes, and
    /// [`KeyError::Memory`] if the key's page cannot be locked.
    pub fn load_master(path: &Path) -> Result<Key, KeyError> {
        let mut page = LockedPage::new().map_err(KeyError::Memory)?;
        let mut file = File::open(path).map_err(KeyError::Read)?;
        let buf = page.bytes_mut();
        file.read_exact(&mut buf[..KEY_LEN])
            .map_err(|e| match e.kind() {
                io::ErrorKind::UnexpectedEof => KeyError::WrongSize,
                _ => KeyError::Read(e),
            })?;
        if file
            .read(&mut buf[KEY_LEN..=KEY_LEN])
            .map_err(KeyError::Read)?
            != 0
        {
            return Err(KeyError::WrongSize);
        }
        Ok(Key { page })
    }

    /// The key's id: the first 8 bytes of its SHA-256 hash.
    ///
    /// # Panics
    ///
    /// Never: a SHA-256 hash is 32 bytes.
    #[must_use]
    pub fn id(&self) -> KeyId {
        let hash = digest(&SHA256, self.bytes());
        KeyId(hash.as_ref()[..8].try_into().expect("SHA-256 is 32 bytes"))
    }

    fn bytes(&self) -> &[u8] {
        &self.page.bytes()[..KEY_LEN]
    }

    /// The key as SQLCipher's raw key literal, `x'` and 64 hex digits and
    /// `'`, in a buffer that is zeroed when dropped. It is built at its
    /// final size, so no shorter copy is left behind by a reallocation.
    pub(crate) fn sqlcipher_raw(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut literal = Zeroizing::new(String::with_capacity(3 + 2 * KEY_LEN));
        literal.push_str("x'");
        for byte in self.bytes() {
            literal.push(char::from(HEX[usize::from(byte >> 4)]));
            literal.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        literal.push('\'');
        literal
    }
}

/// Why a key could not be made.
#[derive(Debug)]
pub enum KeyError {
    /// The key's memory could not be locked or kept out of core dumps.
    Memory(io::Error),
    /// The master key file could not be opened or read.
    Read(io::Error),
    /// The master key file does not hold exactly 32 bytes.
    WrongSize,
    /// A wrapped key did not open. Like [`OpenError`], it does not say which
    /// part was wrong.
    Damaged,
}

/// The I/O error itself is given by `source`, not repeated here.
impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KeyError::Memory(_) => "the key's memory could not be locked or kept out of core dumps",
            KeyError::Read(_) => "the master key file could not be read",
            KeyError::WrongSize => "the master key file does not hold exactly 32 bytes",
            KeyError::Damaged => "a wrapped key did not open",
        })
    }
}

impl std::error::Error for KeyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KeyError::Memory(e) | KeyError::Read(e) => Some(e),
            KeyError::WrongSize | KeyError::Damaged => None,
        }
    }
}

/// Locks every page of this process in memory, now and as it grows, so
/// nothing it holds can reach swap: not AWS-LC's expanded keys, which live
/// in its own allocations for the length of one call, and not plaintext
/// bodies. It fails if the lock limit is too low; the operator then raises
/// it (for example LimitMEMLOCK=infinity under systemd).
///
/// # Errors
///
/// The error mlockall(2) gives: ENOMEM when the limit is too low, EPERM
/// when it is zero and the process may not lock memory.
pub fn lock_all_memory() -> io::Result<()> {
    // SAFETY: mlockall takes only flags.
    if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Stops this process from writing core dumps: the core size limit is set
/// to zero and the process is marked not dumpable, which also stops other
/// processes of the same user from reading its memory. Neither is enough on
/// its own: the limit is not enforced when core dumps are piped to a
/// program, and a host with `suid_dumpable` set to 2 still dumps processes
/// that are not dumpable. Each key's page is left out of core dumps as well.
///
/// # Errors
///
/// The error setrlimit(2) or prctl(2) gives.
pub fn disable_core_dumps() -> io::Result<()> {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: PR_SET_DUMPABLE takes one integer argument.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// What a sealed or wrapped value belongs to. It is bound into the
/// encryption, so a value moved to another tenant, uuid or version does
/// not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// A tenant key, wrapped under the master key.
    Tenant {
        /// The tenant the key belongs to.
        tenant: TenantId,
    },
    /// One version's data key, wrapped under its tenant key, and the body
    /// sealed under that data key.
    Version {
        /// The tenant the vCon belongs to.
        tenant: TenantId,
        /// The vCon's uuid.
        uuid: Uuid,
        /// The version.
        version: VersionId,
    },
}

impl Context {
    /// The label, the blob's format byte, then the context in fixed-length
    /// fields after a byte naming its kind, so no two contexts give the
    /// same bytes.
    fn associated_data(&self, format: u8) -> Vec<u8> {
        let mut ad = LABEL.to_vec();
        ad.push(format);
        match self {
            Context::Tenant { tenant } => {
                ad.push(1);
                ad.extend(tenant.0);
            }
            Context::Version {
                tenant,
                uuid,
                version,
            } => {
                ad.push(2);
                ad.extend(tenant.0);
                ad.extend(uuid.as_bytes());
                ad.extend(version.0);
            }
        }
        ad
    }
}

/// A sealed body or wrapped key did not open. It carries no detail, so a
/// caller cannot tell which part was wrong.
#[derive(Debug, PartialEq, Eq)]
pub struct OpenError;

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the blob did not open under this key and context")
    }
}

impl std::error::Error for OpenError {}

/// Encrypts `plaintext` under `data_key`, bound to `ctx`, with a random
/// nonce. The blob is the format byte, the nonce, the ciphertext and the
/// tag.
///
/// # Panics
///
/// Never: AWS-LC aborts the process rather than fail to give random
/// bytes.
#[must_use]
pub fn seal(data_key: &Key, ctx: &Context, plaintext: &[u8]) -> Vec<u8> {
    // Drawn whole from the system generator, so no fixed value is ever a
    // nonce, even for a moment.
    let nonce: [u8; NONCE_LEN] = aws_lc_rs::rand::generate(&aws_lc_rs::rand::SystemRandom::new())
        .expect("AWS-LC aborts instead of failing")
        .expose();
    seal_with_nonce(data_key.bytes(), nonce, ctx, plaintext)
}

/// Decrypts a blob made by [`seal`] with the same key and context. Any
/// change to the blob, the context or the key is an error, never partial
/// plaintext.
///
/// # Errors
///
/// [`OpenError`] for a blob that does not open under this key and context.
pub fn open(data_key: &Key, ctx: &Context, blob: &[u8]) -> Result<Vec<u8>, OpenError> {
    let mut text = vec![0; blob.len().saturating_sub(1 + NONCE_LEN)];
    let len = open_into(data_key.bytes(), ctx, blob, &mut text)?;
    text.truncate(len);
    Ok(text)
}

/// Wraps `key` under `wrapping`, bound to `ctx`: a blob like [`seal`]'s,
/// whose plaintext is the key.
#[must_use]
pub fn wrap(wrapping: &Key, ctx: &Context, key: &Key) -> Vec<u8> {
    seal(wrapping, ctx, key.bytes())
}

/// Unwraps a key made by [`wrap`] with the same wrapping key and context.
/// It is opened straight into the new key's own locked page, so the plain
/// key is never anywhere else; on any error that page is dropped, and so
/// zeroed.
///
/// # Errors
///
/// [`KeyError::Damaged`] for a blob that does not open under this key and
/// context to exactly 32 bytes, and [`KeyError::Memory`] if the new key's
/// page cannot be locked.
pub fn unwrap(wrapping: &Key, ctx: &Context, wrapped: &[u8]) -> Result<Key, KeyError> {
    let mut page = LockedPage::new().map_err(KeyError::Memory)?;
    match open_into(wrapping.bytes(), ctx, wrapped, page.bytes_mut()) {
        Ok(KEY_LEN) => Ok(Key { page }),
        _ => Err(KeyError::Damaged),
    }
}

/// Moves a wrapped key from `old` to `new`, keeping its context: the
/// rotation step. The plain key exists only in its own locked page in
/// between, and what it protects is never re-encrypted.
///
/// # Errors
///
/// Any error [`unwrap`] gives for `wrapped` under `old`.
pub fn rewrap(old: &Key, new: &Key, ctx: &Context, wrapped: &[u8]) -> Result<Vec<u8>, KeyError> {
    let key = unwrap(old, ctx, wrapped)?;
    Ok(wrap(new, ctx, &key))
}

/// Whether a key that has already wrapped `wraps` keys may wrap another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrapLimit {
    /// Wrap.
    Allow,
    /// Wrap, and warn that the key needs rotating.
    Warn,
    /// Refuse the write until the key is rotated.
    Refuse,
}

/// The wrap-count limit for one wrapping key. Each wrap uses a random
/// nonce, and NIST SP 800-38D 8.3 allows at most 2^32 such encryptions
/// under one key, so a key that has made 2^32 wraps makes no more. The
/// warning starts at half that.
#[must_use]
pub fn wrap_limit(wraps: u64) -> WrapLimit {
    const REFUSE_AT: u64 = 1 << 32;
    const WARN_AT: u64 = 1 << 31;
    if wraps >= REFUSE_AT {
        WrapLimit::Refuse
    } else if wraps >= WARN_AT {
        WrapLimit::Warn
    } else {
        WrapLimit::Allow
    }
}

/// Decrypts `blob` into the start of `out` and returns the plaintext's
/// length. `out` must hold the blob's ciphertext and tag.
fn open_into(key: &[u8], ctx: &Context, blob: &[u8], out: &mut [u8]) -> Result<usize, OpenError> {
    let (&format, rest) = blob.split_first().ok_or(OpenError)?;
    let (nonce, sealed) = rest.split_first_chunk::<NONCE_LEN>().ok_or(OpenError)?;
    let text = out.get_mut(..sealed.len()).ok_or(OpenError)?;
    text.copy_from_slice(sealed);
    gcm_open(key, *nonce, &ctx.associated_data(format), text)
}

/// The key of the catalog that `parent` protects: the master key's for
/// server.sqlite, a tenant key's for that tenant's catalog. It is HKDF-SHA256
/// of the parent with no salt and the info `vcon-server/v1 catalog`, made in
/// a locked page of its own and never stored.
///
/// # Errors
///
/// [`KeyError::Memory`] if the key's page cannot be locked or kept out
/// of core dumps.
pub fn catalog_key(parent: &Key) -> Result<Key, KeyError> {
    let mut page = LockedPage::new().map_err(KeyError::Memory)?;
    hkdf_sha256(
        &[],
        parent.bytes(),
        CATALOG_LABEL,
        &mut page.bytes_mut()[..KEY_LEN],
    );
    Ok(Key { page })
}

/// HKDF (RFC 5869) with SHA-256, filling `out`.
fn hkdf_sha256(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) {
    struct Len(usize);
    impl KeyType for Len {
        fn len(&self) -> usize {
            self.0
        }
    }
    // HKDF refuses only outputs over 255 hashes (8160 bytes); every caller
    // asks for far less.
    Salt::new(HKDF_SHA256, salt)
        .extract(ikm)
        .expand(&[info], Len(out.len()))
        .and_then(|okm| okm.fill(out))
        .expect("within HKDF's output limit");
}

fn seal_with_nonce(key: &[u8], nonce: [u8; NONCE_LEN], ctx: &Context, plaintext: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(1 + NONCE_LEN + plaintext.len() + TAG_LEN);
    blob.push(FORMAT);
    blob.extend(nonce);
    blob.extend(plaintext);
    let ad = ctx.associated_data(FORMAT);
    let tag = gcm_seal(key, nonce, &ad, &mut blob[1 + NONCE_LEN..]);
    blob.extend(tag);
    blob
}

fn aead_key(key: &[u8]) -> LessSafeKey {
    LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).expect("every key is 32 bytes"))
}

/// Encrypts `in_out` in place and returns the tag.
fn gcm_seal(key: &[u8], nonce: [u8; NONCE_LEN], ad: &[u8], in_out: &mut [u8]) -> [u8; TAG_LEN] {
    // AWS-LC refuses only inputs over 2^36 - 32 bytes, short output buffers
    // and nonces of another size. Keys are 32 bytes, and the request size
    // limit must keep bodies far below 64 GiB.
    let tag = aead_key(key)
        .seal_in_place_separate_tag(Nonce::assume_unique_for_key(nonce), Aad::from(ad), in_out)
        .expect("within GCM's limits");
    tag.as_ref().try_into().expect("a 16-byte tag")
}

/// Decrypts ciphertext followed by its tag in place and returns the length
/// of the plaintext now at the start of `in_out`.
fn gcm_open(
    key: &[u8],
    nonce: [u8; NONCE_LEN],
    ad: &[u8],
    in_out: &mut [u8],
) -> Result<usize, OpenError> {
    aead_key(key)
        .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::from(ad), in_out)
        .map(|plaintext| plaintext.len())
        .map_err(|_| OpenError)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_of_a_key_is_random() {
        let keys: Vec<Key> = (0..8).map(|_| Key::generate().expect("key")).collect();
        for i in 0..KEY_LEN {
            let first = keys[0].bytes()[i];
            assert!(
                keys.iter().any(|k| k.bytes()[i] != first),
                "byte {i} is {first} in all eight keys"
            );
        }
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
    }

    /// Test Case 16 of McGrew and Viega, "The Galois/Counter Mode of
    /// Operation (GCM)": AES-256, a 96-bit IV and additional data. The same
    /// values are in AWS-LC's gcm_tests.txt.
    #[test]
    fn aes_256_gcm_matches_the_published_vector() {
        let key = hex("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
        let nonce: [u8; 12] = hex("cafebabefacedbaddecaf888")
            .try_into()
            .expect("12 bytes");
        let ad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let mut text = hex(concat!(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72",
            "1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39"
        ));
        let tag = gcm_seal(&key, nonce, &ad, &mut text);
        assert_eq!(
            text,
            hex(concat!(
                "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa",
                "8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662"
            )),
            "the ciphertext is Test Case 16's"
        );
        assert_eq!(
            tag.to_vec(),
            hex("76fc6ece0f4e1768cddf8853bb2d551b"),
            "the tag is Test Case 16's"
        );
    }

    const TENANT: TenantId = TenantId([0x11; 16]);
    const VERSION: VersionId = VersionId([0x22; 16]);
    const UUID: Uuid = Uuid::from_bytes([0x33; 16]);

    fn version_ctx() -> Context {
        Context::Version {
            tenant: TENANT,
            uuid: UUID,
            version: VERSION,
        }
    }

    /// The blob layout and the associated data are written out here byte by
    /// byte, so a change to either, which would make stored bodies
    /// unreadable, fails this test.
    #[test]
    fn a_blob_is_the_format_byte_nonce_ciphertext_and_tag() {
        let key = [0x44; 32];
        let nonce = [0x55; 12];
        let mut ad = b"vcon-server/v1".to_vec();
        ad.extend([1, 2]);
        ad.extend([0x11; 16]);
        ad.extend([0x33; 16]);
        ad.extend([0x22; 16]);
        let mut ciphertext = b"body".to_vec();
        let tag = gcm_seal(&key, nonce, &ad, &mut ciphertext);
        let mut expected = vec![1];
        expected.extend(nonce);
        expected.extend(ciphertext);
        expected.extend(tag);
        assert_eq!(
            seal_with_nonce(&key, nonce, &version_ctx(), b"body"),
            expected,
            "format byte, nonce, ciphertext, tag, under a version's associated data"
        );

        let mut tenant_ad = b"vcon-server/v1".to_vec();
        tenant_ad.extend([1, 1]);
        tenant_ad.extend([0x11; 16]);
        let mut ciphertext = b"key".to_vec();
        let tag = gcm_seal(&key, nonce, &tenant_ad, &mut ciphertext);
        let mut expected = vec![1];
        expected.extend(nonce);
        expected.extend(ciphertext);
        expected.extend(tag);
        let tenant = Context::Tenant { tenant: TENANT };
        assert_eq!(
            seal_with_nonce(&key, nonce, &tenant, b"key"),
            expected,
            "a tenant blob: the same layout under the tenant's associated data"
        );
    }

    #[test]
    fn a_sealed_body_opens_with_the_same_key_and_context() {
        let key = Key::generate().expect("key");
        for body in [&b""[..], b"x", &[0xa5; 100_000]] {
            let blob = seal(&key, &version_ctx(), body);
            assert_eq!(
                blob.len(),
                1 + 12 + body.len() + 16,
                "a format byte, a 12-byte nonce, the ciphertext as long as the body, a 16-byte tag"
            );
            assert_eq!(
                open(&key, &version_ctx(), &blob).as_deref(),
                Ok(body),
                "the same key and context open what they sealed"
            );
        }
    }

    /// Every byte of the nonce is random: over 1000 seals no nonce repeats
    /// and no byte position keeps one value. For 96 random bits either
    /// event has a probability below 2^-76.
    #[test]
    fn every_byte_of_a_nonce_is_random() {
        let key = Key::generate().expect("key");
        let nonces: Vec<[u8; 12]> = (0..1000)
            .map(|_| {
                let blob = seal(&key, &version_ctx(), b"same");
                blob[1..13].try_into().expect("12 bytes")
            })
            .collect();
        let distinct: std::collections::HashSet<_> = nonces.iter().collect();
        assert_eq!(distinct.len(), nonces.len(), "a nonce repeated");
        for i in 0..12 {
            assert!(
                nonces.iter().any(|n| n[i] != nonces[0][i]),
                "byte {i} of the nonce never changes"
            );
        }
    }

    #[test]
    fn changing_any_byte_of_a_blob_fails() {
        let key = Key::generate().expect("key");
        let blob = seal(&key, &version_ctx(), b"a body");
        assert_eq!(
            open(&key, &version_ctx(), &blob).as_deref(),
            Ok(&b"a body"[..]),
            "unchanged"
        );
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x01;
            assert_eq!(open(&key, &version_ctx(), &bad), Err(OpenError), "byte {i}");
        }
        for len in 0..blob.len() {
            assert_eq!(
                open(&key, &version_ctx(), &blob[..len]),
                Err(OpenError),
                "first {len} bytes"
            );
        }
        let mut longer = blob.clone();
        longer.push(0);
        assert_eq!(
            open(&key, &version_ctx(), &longer),
            Err(OpenError),
            "one byte more"
        );
    }

    #[test]
    fn another_context_or_key_fails() {
        let key = Key::generate().expect("key");
        let blob = seal(&key, &version_ctx(), b"a body");
        assert_eq!(
            open(&key, &version_ctx(), &blob).as_deref(),
            Ok(&b"a body"[..]),
            "unchanged"
        );
        let others = [
            Context::Version {
                tenant: TenantId([0x12; 16]),
                uuid: UUID,
                version: VERSION,
            },
            Context::Version {
                tenant: TENANT,
                uuid: Uuid::from_bytes([0x34; 16]),
                version: VERSION,
            },
            Context::Version {
                tenant: TENANT,
                uuid: UUID,
                version: VersionId([0x23; 16]),
            },
            Context::Tenant { tenant: TENANT },
        ];
        for other in others {
            assert_eq!(open(&key, &other, &blob), Err(OpenError), "{other:?}");
        }
        let other_key = Key::generate().expect("key");
        assert_eq!(
            open(&other_key, &version_ctx(), &blob),
            Err(OpenError),
            "the tag fails under any other key"
        );
    }

    const TENANT_CTX: Context = Context::Tenant { tenant: TENANT };

    #[test]
    fn a_wrapped_key_unwraps_to_the_same_key() {
        let wrapping = Key::generate().expect("key");
        let key = Key::generate().expect("key");
        for ctx in [TENANT_CTX, version_ctx()] {
            let wrapped = wrap(&wrapping, &ctx, &key);
            assert_eq!(
                wrapped.len(),
                1 + 12 + KEY_LEN + 16,
                "a wrapped key is a sealed blob of the key's 32 bytes"
            );
            let unwrapped = unwrap(&wrapping, &ctx, &wrapped).expect("unwraps");
            assert_eq!(
                unwrapped.bytes(),
                key.bytes(),
                "unwrap gives back the key wrapped"
            );
        }
    }

    #[test]
    fn changing_any_byte_of_a_wrapped_key_fails() {
        let wrapping = Key::generate().expect("key");
        let key = Key::generate().expect("key");
        let wrapped = wrap(&wrapping, &TENANT_CTX, &key);
        assert!(
            unwrap(&wrapping, &TENANT_CTX, &wrapped).is_ok(),
            "unchanged"
        );
        for i in 0..wrapped.len() {
            let mut bad = wrapped.clone();
            bad[i] ^= 0x01;
            assert!(unwrap(&wrapping, &TENANT_CTX, &bad).is_err(), "byte {i}");
        }
        for len in 0..wrapped.len() {
            assert!(
                unwrap(&wrapping, &TENANT_CTX, &wrapped[..len]).is_err(),
                "first {len} bytes"
            );
        }
    }

    #[test]
    fn another_context_or_wrapping_key_fails() {
        let wrapping = Key::generate().expect("key");
        let key = Key::generate().expect("key");
        let wrapped = wrap(&wrapping, &TENANT_CTX, &key);
        assert!(
            unwrap(&wrapping, &TENANT_CTX, &wrapped).is_ok(),
            "unchanged"
        );
        let other_tenant = Context::Tenant {
            tenant: TenantId([0x12; 16]),
        };
        assert!(
            unwrap(&wrapping, &other_tenant, &wrapped).is_err(),
            "other tenant"
        );
        assert!(
            unwrap(&wrapping, &version_ctx(), &wrapped).is_err(),
            "other kind"
        );
        let other = Key::generate().expect("key");
        assert!(
            unwrap(&other, &TENANT_CTX, &wrapped).is_err(),
            "other wrapping key"
        );
    }

    #[test]
    fn only_a_32_byte_key_unwraps() {
        let wrapping = Key::generate().expect("key");
        for len in [0, 31, 33, 64] {
            let sealed = seal(&wrapping, &TENANT_CTX, &vec![7; len]);
            assert!(
                unwrap(&wrapping, &TENANT_CTX, &sealed).is_err(),
                "{len} bytes"
            );
        }
        let sealed = seal(&wrapping, &TENANT_CTX, &[7; KEY_LEN]);
        let key = unwrap(&wrapping, &TENANT_CTX, &sealed).expect("32 bytes unwrap");
        assert_eq!(key.bytes(), [7; KEY_LEN], "the key is the 32 bytes sealed");
    }

    /// A key with the given bytes, for tests that need a known key.
    fn key_of(bytes: [u8; KEY_LEN]) -> Key {
        let mut page = LockedPage::new().expect("page");
        page.bytes_mut()[..KEY_LEN].copy_from_slice(&bytes);
        Key { page }
    }

    /// RFC 5869, appendix A, test cases 1 to 3 (SHA-256).
    #[test]
    fn hkdf_sha256_matches_rfc_5869() {
        let cases = [
            (
                "000102030405060708090a0b0c",
                "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
                "f0f1f2f3f4f5f6f7f8f9",
                concat!(
                    "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf",
                    "34007208d5b887185865"
                ),
            ),
            (
                concat!(
                    "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
                    "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
                    "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf"
                ),
                concat!(
                    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                    "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                    "404142434445464748494a4b4c4d4e4f"
                ),
                concat!(
                    "b0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
                    "d0d1d2d3d4d5d6d7d8d9dadbdcdddedfe0e1e2e3e4e5e6e7e8e9eaebecedeeef",
                    "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"
                ),
                concat!(
                    "b11e398dc80327a1c8e7f78c596a49344f012eda2d4efad8a050cc4c19afa97c",
                    "59045a99cac7827271cb41c65e590e09da3275600c2f09b8367793a9aca3db71",
                    "cc30c58179ec3e87c14c01d5c1f3434f1d87"
                ),
            ),
            (
                "",
                "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
                "",
                concat!(
                    "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d",
                    "9d201395faa4b61a96c8"
                ),
            ),
        ];
        for (n, (salt, ikm, info, okm)) in cases.into_iter().enumerate() {
            let expected = hex(okm);
            let mut out = vec![0; expected.len()];
            hkdf_sha256(&hex(salt), &hex(ikm), &hex(info), &mut out);
            assert_eq!(out, expected, "test case {}", n + 1);
        }
    }

    /// The expected key was computed with Python's hmac and hashlib: HKDF
    /// with no salt and the info "vcon-server/v1 catalog". Changing the
    /// label or the salt would lock every catalog out, so this pins both.
    #[test]
    fn a_catalog_key_is_hkdf_of_its_parent_with_the_catalog_label() {
        let catalog = catalog_key(&key_of([0x44; KEY_LEN])).expect("derives");
        assert_eq!(
            catalog.bytes(),
            hex("c0df4ea4f63e64687ca93c494fff4ca00b3ae13112766d8ea24cca35c6b1031b"),
            "the value Python's HKDF gives for this parent and label"
        );
    }

    #[test]
    fn distinct_parents_or_labels_give_distinct_keys() {
        let parent = Key::generate().expect("key");
        let again = catalog_key(&parent).expect("derives");
        assert_eq!(
            catalog_key(&parent).expect("derives").bytes(),
            again.bytes(),
            "derivation is deterministic: the same parent gives the same catalog key"
        );
        let other = catalog_key(&Key::generate().expect("key")).expect("derives");
        assert_ne!(
            other.bytes(),
            again.bytes(),
            "another parent gives another catalog key"
        );
        let mut a = [0; KEY_LEN];
        let mut b = [0; KEY_LEN];
        hkdf_sha256(&[], parent.bytes(), b"label a", &mut a);
        hkdf_sha256(&[], parent.bytes(), b"label b", &mut b);
        assert_ne!(a, b, "another label gives another key from the same parent");
    }

    #[test]
    fn a_rewrapped_key_opens_under_the_new_key_only() {
        let old = Key::generate().expect("key");
        let new = Key::generate().expect("key");
        let key = Key::generate().expect("key");
        let wrapped = wrap(&old, &version_ctx(), &key);
        let rewrapped = rewrap(&old, &new, &version_ctx(), &wrapped).expect("rewraps");
        let opened = unwrap(&new, &version_ctx(), &rewrapped).expect("opens under new");
        assert_eq!(
            opened.bytes(),
            key.bytes(),
            "rewrapping keeps the wrapped key itself"
        );
        assert!(unwrap(&old, &version_ctx(), &rewrapped).is_err(), "old key");
        assert!(
            unwrap(&new, &TENANT_CTX, &rewrapped).is_err(),
            "context kept"
        );
    }

    #[test]
    fn rewrapping_needs_the_right_old_key_and_context() {
        let old = Key::generate().expect("key");
        let new = Key::generate().expect("key");
        let wrapped = wrap(&old, &TENANT_CTX, &Key::generate().expect("key"));
        assert!(
            matches!(
                rewrap(&new, &new, &TENANT_CTX, &wrapped),
                Err(KeyError::Damaged)
            ),
            "the key was not wrapped under new, so new cannot unwrap it to rewrap"
        );
        assert!(
            matches!(
                rewrap(&old, &new, &version_ctx(), &wrapped),
                Err(KeyError::Damaged)
            ),
            "the key was wrapped for the tenant context, not a version's"
        );
    }

    #[test]
    fn wrapping_warns_from_2_to_the_31_and_stops_at_2_to_the_32() {
        let cases = [
            (0, WrapLimit::Allow),
            ((1 << 31) - 1, WrapLimit::Allow),
            (1 << 31, WrapLimit::Warn),
            ((1 << 32) - 1, WrapLimit::Warn),
            (1 << 32, WrapLimit::Refuse),
            (u64::MAX, WrapLimit::Refuse),
        ];
        for (wraps, expected) in cases {
            assert_eq!(wrap_limit(wraps), expected, "{wraps} wraps");
        }
    }

    #[test]
    fn the_sqlcipher_literal_spells_the_key_in_hex() {
        let mut bytes = [0; KEY_LEN];
        bytes[0] = 0x0a;
        bytes[31] = 0xff;
        let literal = key_of(bytes).sqlcipher_raw();
        let expected = format!("x'0a{}ff'", "00".repeat(30));
        assert_eq!(
            *literal, expected,
            "SQLCipher's raw key literal is x'' around two lower-case hex digits per byte"
        );
        assert_eq!(literal.capacity(), literal.len(), "built at its final size");
    }

    /// Debug is how a key reaches a log line or a panic message, so it must
    /// name the key without any rendering of its bytes.
    #[test]
    fn a_keys_debug_shows_its_id_and_none_of_its_bytes() {
        let bytes: [u8; KEY_LEN] = std::array::from_fn(|i| 0xa0 ^ (5 * i as u8));
        let key = key_of(bytes);
        let shown = format!("{key:?}");
        assert!(
            shown.contains(&key.id().to_string()),
            "the id tells keys apart in a log: {shown}"
        );
        for run in bytes.windows(4) {
            let lower: String = run.iter().map(|b| format!("{b:02x}")).collect();
            let upper = lower.to_uppercase();
            let decimal = run.iter().map(u8::to_string).collect::<Vec<_>>().join(", ");
            for rendering in [lower, upper, decimal] {
                assert!(
                    !shown.contains(&rendering),
                    "no run of the key's bytes may be printed, found {rendering:?} in {shown}"
                );
            }
        }
    }

    #[test]
    fn keys_can_be_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Key>();
    }
}
