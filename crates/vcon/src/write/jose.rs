//! The signed form (a General JWS JSON Serialization, core-04 5.2) and the
//! encrypted form (a General JWE JSON Serialization, core-04 5.3) of a
//! vCon. The cryptography is the caller's: it comes through [`Signer`],
//! [`KeyEncrypter`] and [`ContentEncrypter`], so this module holds no key
//! beyond the content key of one JWE and draws no random numbers itself.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Map, Value};
use uuid::Uuid;
pub use zeroize::Zeroizing;

use super::emit::{Node, Obj, render};
use super::{Date, Vcon, WriteError, gzip, is_https};
use crate::Form;
use crate::form::form_of;
use crate::gzip::inflate;
use crate::identity::{parse_uuid_text, unsigned_identity};
use crate::json::parse_object;
use crate::read::{Limits, MAX_SIGNED_LAYERS};

/// The first two bytes of every gzip member (RFC 1952, section 2.3.1).
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Whether a payload or plaintext is gzipped first.
///
/// # Examples
///
/// ```
/// use vcon::write::Compression;
///
/// assert_ne!(Compression::None, Compression::Gzip, "two choices");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Compression {
    /// The bytes as they are, marked `application/vcon`.
    None,
    /// The bytes gzipped, marked `application/vcon+gzip`.
    Gzip,
}

/// The certificates that identify a signer (core-04 5.2.2): DER
/// certificates written in `x5c`, or an https URL written in `x5u`.
///
/// # Examples
///
/// ```
/// use vcon::write::Chain;
///
/// let at = Chain::X5u("https://example.com/chain.pem".to_owned());
/// assert_ne!(at, Chain::X5c(vec![vec![0x30, 0x03]]), "a URL, not certificates");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Chain {
    /// DER certificates, the signer's first; written standard base64.
    X5c(Vec<Vec<u8>>),
    /// An https URL where the chain is.
    X5u(String),
}

/// The output of a [`ContentEncrypter`]: the initialization vector, the
/// ciphertext and the authentication tag.
///
/// # Examples
///
/// ```
/// let sealed = vcon::write::Sealed::new(vec![0; 16], vec![1, 2, 3], vec![9; 32]);
/// assert_eq!(sealed.clone(), sealed, "a sealed plaintext");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sealed {
    iv: Vec<u8>,
    ciphertext: Vec<u8>,
    tag: Vec<u8>,
}

impl Sealed {
    /// The three outputs of an authenticated encryption.
    ///
    /// # Examples
    ///
    /// ```
    /// let sealed = vcon::write::Sealed::new(vec![0; 16], vec![1, 2, 3], vec![9; 32]);
    /// assert_eq!(sealed.clone(), sealed, "a sealed plaintext");
    /// ```
    #[must_use]
    pub fn new(iv: Vec<u8>, ciphertext: Vec<u8>, tag: Vec<u8>) -> Sealed {
        Sealed {
            iv,
            ciphertext,
            tag,
        }
    }
}

/// Why a [`Signer`], [`KeyEncrypter`] or [`ContentEncrypter`] failed, in its
/// own words.
///
/// # Examples
///
/// ```
/// let e = vcon::write::CryptoError::new("the key is not loaded");
/// assert_eq!(e.to_string(), "the key is not loaded", "its message");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[must_use]
pub struct CryptoError(String);

impl CryptoError {
    /// A failure described by `message`.
    ///
    /// # Examples
    ///
    /// ```
    /// let e = vcon::write::CryptoError::new("the key is not loaded");
    /// assert_eq!(e.clone(), e, "an error");
    /// ```
    pub fn new(message: impl Into<String>) -> CryptoError {
        CryptoError(message.into())
    }
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CryptoError {}

/// Signs JWS signing input with the caller's key (core-04 5.2.1).
///
/// # Examples
///
/// ```
/// use vcon::write::{Chain, CryptoError, Signer};
///
/// /// A signer for this example only: it does not sign anything.
/// struct Example(Chain);
///
/// impl Signer for Example {
///     fn alg(&self) -> &str {
///         "RS256"
///     }
///     fn chain(&self) -> &Chain {
///         &self.0
///     }
///     fn sign(&self, input: &[u8]) -> Result<Vec<u8>, CryptoError> {
///         Ok(input.iter().rev().copied().collect())
///     }
/// }
///
/// let s = Example(Chain::X5u("https://example.com/chain.pem".to_owned()));
/// assert_eq!(s.alg(), "RS256", "the algorithm written in the header");
/// ```
pub trait Signer {
    /// The JWS `alg` of the signatures it makes, such as `RS256`.
    fn alg(&self) -> &str;

    /// The certificates that identify it.
    fn chain(&self) -> &Chain;

    /// The signature of `input`, ASCII(protected) "." ASCII(payload).
    ///
    /// # Errors
    ///
    /// A [`CryptoError`] in the signer's words if it cannot sign.
    fn sign(&self, input: &[u8]) -> Result<Vec<u8>, CryptoError>;
}

/// Wraps a JWE's content key for one recipient (core-04 5.3.2).
///
/// # Examples
///
/// ```
/// use vcon::write::{CryptoError, KeyEncrypter};
///
/// /// A key encrypter for this example only: it does not encrypt.
/// struct Example;
///
/// impl KeyEncrypter for Example {
///     fn alg(&self) -> &str {
///         "RSA-OAEP"
///     }
///     fn encrypt_key(&self, cek: &[u8]) -> Result<Vec<u8>, CryptoError> {
///         Ok(cek.to_vec())
///     }
/// }
///
/// assert_eq!(Example.alg(), "RSA-OAEP", "the algorithm written for the recipient");
/// ```
pub trait KeyEncrypter {
    /// The JWE `alg`, such as `RSA-OAEP`.
    fn alg(&self) -> &str;

    /// The content key `cek`, encrypted for this recipient.
    ///
    /// # Errors
    ///
    /// A [`CryptoError`] in the encrypter's words if it cannot.
    fn encrypt_key(&self, cek: &[u8]) -> Result<Vec<u8>, CryptoError>;
}

/// Encrypts a JWE's plaintext under a content key it makes (core-04 5.3).
///
/// # Examples
///
/// ```
/// use vcon::write::{ContentEncrypter, CryptoError, Sealed, Zeroizing};
///
/// /// A content encrypter for this example only: it does not encrypt.
/// struct Example;
///
/// impl ContentEncrypter for Example {
///     fn enc(&self) -> &str {
///         "A256CBC-HS512"
///     }
///     fn new_key(&self) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
///         Ok(Zeroizing::new(vec![7; 64]))
///     }
///     fn seal(&self, _cek: &[u8], plaintext: &[u8], _aad: &[u8]) -> Result<Sealed, CryptoError> {
///         Ok(Sealed::new(vec![0; 16], plaintext.to_vec(), vec![0; 32]))
///     }
/// }
///
/// assert_eq!(Example.enc(), "A256CBC-HS512", "the enc written in unprotected");
/// ```
pub trait ContentEncrypter {
    /// The JWE `enc`, such as `A256CBC-HS512`.
    fn enc(&self) -> &str;

    /// A new content key for one JWE; it is zeroed when dropped.
    ///
    /// # Errors
    ///
    /// A [`CryptoError`] in the encrypter's words if it cannot make one.
    fn new_key(&self) -> Result<Zeroizing<Vec<u8>>, CryptoError>;

    /// `plaintext` encrypted under `cek`, authenticating `aad` too.
    ///
    /// # Errors
    ///
    /// A [`CryptoError`] in the encrypter's words if it cannot.
    fn seal(&self, cek: &[u8], plaintext: &[u8], aad: &[u8]) -> Result<Sealed, CryptoError>;
}

/// The signed form of `vcon` (core-04 5.2): a General JWS JSON
/// Serialization whose payload is the vCon with `updated_at` set to
/// `signed_at`, gzipped first if `compression` says so, with one signature
/// per signer. The caller's model is not changed.
///
/// # Errors
///
/// [`WriteError::NoSigner`] if `signers` is empty, [`WriteError::BadChain`]
/// if a signer's chain is empty or not https, [`WriteError::Crypto`] if a
/// signer fails, and the errors of [`Vcon::to_json`].
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use vcon::read::{Limits, inspect};
/// use vcon::write::{Chain, Compression, CryptoError, Signer, Vcon, sign};
///
/// /// A signer for this example only: its signature proves nothing.
/// struct Example(Chain);
///
/// impl Signer for Example {
///     fn alg(&self) -> &str {
///         "RS256"
///     }
///     fn chain(&self) -> &Chain {
///         &self.0
///     }
///     fn sign(&self, input: &[u8]) -> Result<Vec<u8>, CryptoError> {
///         Ok(input.len().to_be_bytes().to_vec())
///     }
/// }
///
/// let signer = Example(Chain::X5u("https://example.com/chain.pem".to_owned()));
/// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
/// let v = Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
/// let at = "2026-10-27T01:00:00Z".parse().expect("RFC 3339");
/// let signed = sign(&v, at, Compression::None, &[&signer]).expect("signed");
/// let limits = Limits::new(1 << 20);
/// let found = inspect(&signed, None, &limits, &BTreeSet::new()).expect("a vCon");
/// assert_eq!(found.uuid, uuid, "the signed vCon's uuid");
/// ```
pub fn sign(
    vcon: &Vcon,
    signed_at: Date,
    compression: Compression,
    signers: &[&dyn Signer],
) -> Result<Vec<u8>, WriteError> {
    check_signers(signers)?;
    let mut signed = vcon.clone();
    signed.set_updated_at(signed_at);
    let payload = packed(signed.to_json()?, compression);
    jws(&payload, vcon.uuid(), signers)
}

/// A signed vCon signed again, as the payload of a new JWS, for archives
/// kept longer than their signing certificates (core-04 5.2.3). The new
/// header's `uuid` is the innermost vCon's. The inner signatures are not
/// verified: that is the caller's step before signing again.
///
/// # Errors
///
/// [`WriteError::NotSigned`] if `signed` is not a signed vCon read under
/// `limits`, [`WriteError::TooManyLayers`] if the result would have more
/// than [`crate::read::MAX_SIGNED_LAYERS`] signed layers,
/// [`WriteError::BadChain`] and [`WriteError::Crypto`] as for [`sign`].
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use vcon::read::{Limits, inspect};
/// use vcon::write::{Chain, Compression, CryptoError, Signer, Vcon, sign, sign_again};
///
/// /// A signer for this example only: its signature proves nothing.
/// struct Example(Chain);
///
/// impl Signer for Example {
///     fn alg(&self) -> &str {
///         "RS256"
///     }
///     fn chain(&self) -> &Chain {
///         &self.0
///     }
///     fn sign(&self, input: &[u8]) -> Result<Vec<u8>, CryptoError> {
///         Ok(input.len().to_be_bytes().to_vec())
///     }
/// }
///
/// let signer = Example(Chain::X5u("https://example.com/chain.pem".to_owned()));
/// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
/// let v = Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
/// let at = "2026-10-27T01:00:00Z".parse().expect("RFC 3339");
/// let signed = sign(&v, at, Compression::None, &[&signer]).expect("signed");
/// let limits = Limits::new(1 << 20);
/// let archived = sign_again(&signed, Compression::Gzip, &signer, limits).expect("signed again");
/// let found = inspect(&archived, None, &limits, &BTreeSet::new()).expect("a vCon");
/// assert_eq!(found.uuid, uuid, "the innermost uuid, through both layers");
/// ```
pub fn sign_again(
    signed: &[u8],
    compression: Compression,
    signer: &dyn Signer,
    limits: Limits,
) -> Result<Vec<u8>, WriteError> {
    let inner = read_signed(signed, limits)?;
    if inner.layers >= MAX_SIGNED_LAYERS {
        return Err(WriteError::TooManyLayers);
    }
    check_signers(&[signer])?;
    let payload = packed(signed.to_vec(), compression);
    jws(&payload, inner.uuid, &[signer])
}

/// The encrypted form of a signed vCon (core-04 5.3): a General JWE JSON
/// Serialization whose plaintext is `signed`, gzipped first if
/// `compression` says so, under one new content key wrapped for each
/// recipient. There is no protected header, so the additional
/// authenticated data is empty.
///
/// # Errors
///
/// [`WriteError::NotSigned`] if `signed` is not a signed vCon read under
/// `limits`, [`WriteError::NoRecipient`] if `recipients` is empty, and
/// [`WriteError::Crypto`] if an encrypter fails.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeSet;
///
/// use vcon::read::{Limits, inspect};
/// use vcon::write::{Chain, Compression, CryptoError, ContentEncrypter, KeyEncrypter, Sealed, Signer, Vcon, Zeroizing, encrypt, sign};
///
/// /// A signer for this example only: its signature proves nothing.
/// struct Example(Chain);
///
/// impl Signer for Example {
///     fn alg(&self) -> &str {
///         "RS256"
///     }
///     fn chain(&self) -> &Chain {
///         &self.0
///     }
///     fn sign(&self, input: &[u8]) -> Result<Vec<u8>, CryptoError> {
///         Ok(input.len().to_be_bytes().to_vec())
///     }
/// }
///
/// let signer = Example(Chain::X5u("https://example.com/chain.pem".to_owned()));
/// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
/// let v = Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
/// let at = "2026-10-27T01:00:00Z".parse().expect("RFC 3339");
/// let signed = sign(&v, at, Compression::None, &[&signer]).expect("signed");
///
/// /// An encrypter for this example only: it does not encrypt.
/// struct Plain;
///
/// impl ContentEncrypter for Plain {
///     fn enc(&self) -> &str {
///         "A256CBC-HS512"
///     }
///     fn new_key(&self) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
///         Ok(Zeroizing::new(vec![0; 64]))
///     }
///     fn seal(&self, _cek: &[u8], plaintext: &[u8], _aad: &[u8]) -> Result<Sealed, CryptoError> {
///         Ok(Sealed::new(vec![0; 16], plaintext.to_vec(), vec![0; 32]))
///     }
/// }
///
/// impl KeyEncrypter for Plain {
///     fn alg(&self) -> &str {
///         "RSA-OAEP"
///     }
///     fn encrypt_key(&self, cek: &[u8]) -> Result<Vec<u8>, CryptoError> {
///         Ok(cek.to_vec())
///     }
/// }
///
/// let limits = Limits::new(1 << 20);
/// let jwe = encrypt(&signed, Compression::Gzip, &Plain, &[&Plain], limits).expect("encrypted");
/// let found = inspect(&jwe, None, &limits, &BTreeSet::new()).expect("a vCon");
/// assert_eq!(found.form, vcon::Form::Encrypted, "the encrypted form");
/// ```
pub fn encrypt(
    signed: &[u8],
    compression: Compression,
    content: &dyn ContentEncrypter,
    recipients: &[&dyn KeyEncrypter],
    limits: Limits,
) -> Result<Vec<u8>, WriteError> {
    let inner = read_signed(signed, limits)?;
    if recipients.is_empty() {
        return Err(WriteError::NoRecipient);
    }
    let plaintext = packed(signed.to_vec(), compression);
    let cek = content.new_key().map_err(WriteError::Crypto)?;
    // No protected header: the additional authenticated data is
    // ASCII(""), RFC 7516 5.1 step 14.
    let sealed = content
        .seal(&cek, &plaintext.0, b"")
        .map_err(WriteError::Crypto)?;
    let mut wrapped = Vec::new();
    for recipient in recipients {
        let key = recipient.encrypt_key(&cek).map_err(WriteError::Crypto)?;
        let mut header = Obj::default();
        header.put("alg", Node::str(recipient.alg()));
        let mut entry = Obj::default();
        entry.put("header", header.done());
        entry.put("encrypted_key", Node::Str(URL_SAFE_NO_PAD.encode(key)));
        wrapped.push(entry.done());
    }
    let mut unprotected = Obj::default();
    unprotected.put("cty", Node::str(plaintext.cty()));
    unprotected.put("enc", Node::str(content.enc()));
    unprotected.put("uuid", Node::str(&inner.uuid.hyphenated().to_string()));
    let mut top = Obj::default();
    top.put("unprotected", unprotected.done());
    top.put("recipients", Node::Arr(wrapped));
    top.put("iv", Node::Str(URL_SAFE_NO_PAD.encode(&sealed.iv)));
    top.put(
        "ciphertext",
        Node::Str(URL_SAFE_NO_PAD.encode(&sealed.ciphertext)),
    );
    top.put("tag", Node::Str(URL_SAFE_NO_PAD.encode(&sealed.tag)));
    render(&top.done())
}

/// Bytes to sign or encrypt, gzipped if asked, with the media type that
/// says what they are.
struct Packed(Vec<u8>);

impl Packed {
    /// `application/vcon+gzip` exactly when the bytes are gzip, whether
    /// this crate gzipped them or they arrived so.
    fn cty(&self) -> &'static str {
        if self.0.starts_with(&GZIP_MAGIC) {
            "application/vcon+gzip"
        } else {
            "application/vcon"
        }
    }
}

fn packed(bytes: Vec<u8>, compression: Compression) -> Packed {
    match compression {
        Compression::None => Packed(bytes),
        Compression::Gzip => Packed(gzip(&bytes)),
    }
}

/// At least one signer, each with a usable chain, before anything is
/// signed.
fn check_signers(signers: &[&dyn Signer]) -> Result<(), WriteError> {
    if signers.is_empty() {
        return Err(WriteError::NoSigner);
    }
    for signer in signers {
        match signer.chain() {
            Chain::X5c(certificates) if certificates.is_empty() => {
                return Err(WriteError::BadChain);
            }
            Chain::X5u(url) if !is_https(url) => return Err(WriteError::BadChain),
            _ => {}
        }
    }
    Ok(())
}

/// A General JWS JSON Serialization of `payload`, one signature per
/// signer, each header naming `uuid` (core-04 5.2.1, 5.2.2).
fn jws(payload: &Packed, uuid: Uuid, signers: &[&dyn Signer]) -> Result<Vec<u8>, WriteError> {
    let mut cty = Obj::default();
    cty.put("cty", Node::str(payload.cty()));
    let protected = URL_SAFE_NO_PAD.encode(render(&cty.done())?);
    let encoded = URL_SAFE_NO_PAD.encode(&payload.0);
    let input = format!("{protected}.{encoded}");
    let mut signatures = Vec::new();
    for signer in signers {
        let signature = signer.sign(input.as_bytes()).map_err(WriteError::Crypto)?;
        let mut header = Obj::default();
        header.put("alg", Node::str(signer.alg()));
        match signer.chain() {
            Chain::X5c(certificates) => header.put(
                "x5c",
                Node::Arr(
                    certificates
                        .iter()
                        .map(|der| Node::Str(STANDARD.encode(der)))
                        .collect(),
                ),
            ),
            Chain::X5u(url) => header.put("x5u", Node::str(url)),
        }
        header.put("uuid", Node::str(&uuid.hyphenated().to_string()));
        let mut entry = Obj::default();
        entry.put("protected", Node::str(&protected));
        entry.put("header", header.done());
        entry.put("signature", Node::Str(URL_SAFE_NO_PAD.encode(signature)));
        signatures.push(entry.done());
    }
    let mut top = Obj::default();
    top.put("payload", Node::Str(encoded));
    top.put("signatures", Node::Arr(signatures));
    render(&top.done())
}

/// What a signed vCon holds: the innermost uuid and how many signed layers
/// lead to it.
struct Inner {
    uuid: Uuid,
    layers: usize,
}

/// Reads `signed` as a signed vCon: every layer a JWS whose payload is the
/// next, gzipped or not, under `limits`, down to an unsigned vCon, with
/// every header `uuid` that vCon's. Unlike `read::inspect` it does not
/// check `critical`: signing or encrypting does not process the vCon.
fn read_signed(signed: &[u8], limits: Limits) -> Result<Inner, WriteError> {
    let not_signed = |_| WriteError::NotSigned;
    let mut budget = limits.inflated;
    let mut object = open(signed, &mut budget).map_err(not_signed)?;
    if form_of(&object).map_err(not_signed)? != Form::Signed {
        return Err(WriteError::NotSigned);
    }
    let mut header_uuids = Vec::new();
    let mut layers = 0;
    loop {
        layers += 1;
        if layers > MAX_SIGNED_LAYERS {
            return Err(WriteError::NotSigned);
        }
        header_uuids.extend(header_uuids_of(&object)?);
        let Some(Value::String(payload)) = object.get("payload") else {
            return Err(WriteError::NotSigned);
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| WriteError::NotSigned)?;
        object = open(&bytes, &mut budget).map_err(not_signed)?;
        match form_of(&object).map_err(not_signed)? {
            Form::Signed => {}
            Form::Unsigned => break,
            _ => return Err(WriteError::NotSigned),
        }
    }
    let uuid = unsigned_identity(&object).map_err(not_signed)?;
    for header_uuid in header_uuids {
        match header_uuid {
            Value::String(text) if parse_uuid_text(&text) == Ok(uuid) => {}
            _ => return Err(WriteError::NotSigned),
        }
    }
    Ok(Inner { uuid, layers })
}

/// One JSON object, inflated first if it is gzip, counting the inflated
/// bytes against `budget`.
fn open(bytes: &[u8], budget: &mut u64) -> Result<Map<String, Value>, crate::read::Rejection> {
    let (inner, was_gzip) = inflate(bytes, None, *budget)?;
    if was_gzip {
        *budget = budget.saturating_sub(inner.len() as u64);
    }
    parse_object(&inner)
}

/// The `uuid` of each signature's header that has one.
fn header_uuids_of(object: &Map<String, Value>) -> Result<Vec<Value>, WriteError> {
    let Some(Value::Array(signatures)) = object.get("signatures") else {
        return Err(WriteError::NotSigned);
    };
    let mut uuids = Vec::new();
    for signature in signatures {
        match signature.get("header") {
            None => {}
            Some(Value::Object(header)) => uuids.extend(header.get("uuid").cloned()),
            Some(_) => return Err(WriteError::NotSigned),
        }
    }
    Ok(uuids)
}
