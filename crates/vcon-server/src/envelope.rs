//! Keys and the encryption that protects every stored body.

use std::io;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use uuid::Uuid;

use crate::ids::{TenantId, VersionId};

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

/// A 256-bit key. Its bytes live only in a page of their own that is locked
/// in memory, so they never reach swap, and that core dumps leave out; they
/// are zeroed when the key is dropped. A key cannot be copied or printed.
pub struct Key {
    page: LockedPage,
}

impl Key {
    /// Makes a new random key, refusing if its memory cannot be locked.
    pub fn generate() -> Result<Key, KeyError> {
        let mut page = LockedPage::new().map_err(KeyError::Memory)?;
        // AWS-LC's RAND_bytes aborts the process when it cannot get enough
        // randomness, so `fill` never returns its error.
        aws_lc_rs::rand::fill(&mut page.bytes_mut()[..KEY_LEN])
            .expect("AWS-LC aborts instead of failing");
        Ok(Key { page })
    }

    fn bytes(&self) -> &[u8] {
        &self.page.bytes()[..KEY_LEN]
    }
}

/// Why a key could not be made.
#[derive(Debug)]
pub enum KeyError {
    /// The key's memory could not be locked or kept out of core dumps.
    Memory(io::Error),
    /// A wrapped key did not open. Like [`OpenError`], it does not say which
    /// part was wrong.
    Damaged,
}

/// Stops this process from writing core dumps: the core size limit is set
/// to zero and the process is marked not dumpable, which also stops other
/// processes of the same user from reading its memory. Neither is enough on
/// its own: the limit is not enforced when core dumps are piped to a
/// program, and a host with `suid_dumpable` set to 2 still dumps processes
/// that are not dumpable. Each key's page is left out of core dumps as well.
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
    Tenant { tenant: TenantId },
    /// One version's data key, wrapped under its tenant key, and the body
    /// sealed under that data key.
    Version {
        tenant: TenantId,
        uuid: Uuid,
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

/// Encrypts `plaintext` under `data_key`, bound to `ctx`, with a random
/// nonce. The blob is the format byte, the nonce, the ciphertext and the
/// tag.
pub fn seal(data_key: &Key, ctx: &Context, plaintext: &[u8]) -> Vec<u8> {
    let mut nonce = [0; NONCE_LEN];
    aws_lc_rs::rand::fill(&mut nonce).expect("AWS-LC aborts instead of failing");
    seal_with_nonce(data_key.bytes(), nonce, ctx, plaintext)
}

/// Decrypts a blob made by [`seal`] with the same key and context. Any
/// change to the blob, the context or the key is an error, never partial
/// plaintext.
pub fn open(data_key: &Key, ctx: &Context, blob: &[u8]) -> Result<Vec<u8>, OpenError> {
    let mut text = vec![0; blob.len().saturating_sub(1 + NONCE_LEN)];
    let len = open_into(data_key.bytes(), ctx, blob, &mut text)?;
    text.truncate(len);
    Ok(text)
}

/// Wraps `key` under `wrapping`, bound to `ctx`: a blob like [`seal`]'s,
/// whose plaintext is the key.
pub fn wrap(wrapping: &Key, ctx: &Context, key: &Key) -> Vec<u8> {
    seal(wrapping, ctx, key.bytes())
}

/// Unwraps a key made by [`wrap`] with the same wrapping key and context.
/// It is opened straight into the new key's own locked page, so the plain
/// key is never anywhere else; on any error that page is dropped, and so
/// zeroed.
pub fn unwrap(wrapping: &Key, ctx: &Context, wrapped: &[u8]) -> Result<Key, KeyError> {
    let mut page = LockedPage::new().map_err(KeyError::Memory)?;
    match open_into(wrapping.bytes(), ctx, wrapped, page.bytes_mut()) {
        Ok(KEY_LEN) => Ok(Key { page }),
        _ => Err(KeyError::Damaged),
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
            ))
        );
        assert_eq!(tag.to_vec(), hex("76fc6ece0f4e1768cddf8853bb2d551b"));
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
            expected
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
        assert_eq!(seal_with_nonce(&key, nonce, &tenant, b"key"), expected);
    }

    #[test]
    fn a_sealed_body_opens_with_the_same_key_and_context() {
        let key = Key::generate().expect("key");
        for body in [&b""[..], b"x", &[0xa5; 100_000]] {
            let blob = seal(&key, &version_ctx(), body);
            assert_eq!(blob.len(), 1 + 12 + body.len() + 16);
            assert_eq!(open(&key, &version_ctx(), &blob).as_deref(), Ok(body));
        }
    }

    #[test]
    fn each_seal_uses_a_new_nonce() {
        let key = Key::generate().expect("key");
        let a = seal(&key, &version_ctx(), b"same");
        let b = seal(&key, &version_ctx(), b"same");
        assert_ne!(a[1..13], b[1..13]);
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
        assert_eq!(open(&other_key, &version_ctx(), &blob), Err(OpenError));
    }

    const TENANT_CTX: Context = Context::Tenant { tenant: TENANT };

    #[test]
    fn a_wrapped_key_unwraps_to_the_same_key() {
        let wrapping = Key::generate().expect("key");
        let key = Key::generate().expect("key");
        for ctx in [TENANT_CTX, version_ctx()] {
            let wrapped = wrap(&wrapping, &ctx, &key);
            assert_eq!(wrapped.len(), 1 + 12 + KEY_LEN + 16);
            let unwrapped = unwrap(&wrapping, &ctx, &wrapped).expect("unwraps");
            assert_eq!(unwrapped.bytes(), key.bytes());
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
        assert_eq!(key.bytes(), [7; KEY_LEN]);
    }
}
