//! Writing a vCon: an object model of the vCon core draft
//! (draft-ietf-vcon-vcon-core-04, section 4) and its unsigned and gzip
//! forms. [`crate::read::inspect`] reads back whatever this module writes,
//! with the same form, gzip flag and uuid.
//!
//! The module does no I/O, reads no clock and draws no random numbers: the
//! caller passes every time and uuid. What core-04 requires of an object
//! is an argument of its constructor, and a closed set of values is an
//! enum; everything else is checked when the vCon is written, and a
//! refusal names the JSON path of the object at fault.
//!
//! Every public enum is `#[non_exhaustive]` and every struct has private
//! fields, so a later release can add a member without breaking a caller.
//!
//! # Examples
//!
//! A two-party call with an externally stored recording:
//!
//! ```
//! use vcon::write::{Channel, Content, ContentHash, Date, Dialog, Parties, Party, Vcon};
//!
//! let uuid = vcon::write::uuid_v8(1_655_833_980_000, 0x3e5, "vcon.example.com");
//! let created: Date = "2022-06-21T13:53:00-04:00".parse().expect("an RFC 3339 date");
//! let mut v = Vcon::new(uuid, created);
//! let alice = v.add_party(Party::new().name("Alice"));
//! let bob = v.add_party(Party::new().name("Bob"));
//! let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
//! v.add_dialog(
//!     Dialog::recording(Content::External {
//!         url: "https://example.com/call.mp3".to_owned(),
//!         hashes: vec![hash],
//!     })
//!     .parties(Parties::Channels(vec![Channel::One(alice), Channel::One(bob)])),
//! );
//! let json = v.to_json().expect("the vCon is valid");
//! assert!(
//!     json.starts_with(br#"{"uuid":""#),
//!     "the uuid is the first member"
//! );
//! ```

use std::fmt;
use std::io::Write;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

mod emit;

use emit::{Node, Obj};

/// A Date as core-04 section 2.2 defines it: an RFC 3339 date and time
/// with its offset.
///
/// It is parsed from text with [`str::parse`], which keeps the text exactly
/// as given, so a precision the caller chose (such as
/// `17:53:26.000+00:00`) is written unchanged; or made from an
/// [`OffsetDateTime`] with [`TryFrom`].
///
/// # Examples
///
/// ```
/// use vcon::write::Date;
///
/// let date: Date = "2022-06-21T17:53:26.000+00:00".parse().expect("RFC 3339");
/// assert_eq!(date.as_str(), "2022-06-21T17:53:26.000+00:00", "kept as given");
/// assert!("2022-06-21T17:53:26".parse::<Date>().is_err(), "no offset");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Date(String);

impl Date {
    /// The text written.
    ///
    /// # Examples
    ///
    /// ```
    /// let date: vcon::write::Date = "2026-10-27T00:00:00Z".parse().expect("RFC 3339");
    /// assert_eq!(date.as_str(), "2026-10-27T00:00:00Z", "the text as given");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Date {
    type Err = WriteError;

    /// Takes `text` if it is an RFC 3339 date and time with an offset.
    ///
    /// # Errors
    ///
    /// [`WriteError::BadDate`] if it is not.
    fn from_str(text: &str) -> Result<Date, WriteError> {
        OffsetDateTime::parse(text, &Rfc3339).map_err(|_| WriteError::BadDate)?;
        Ok(Date(text.to_owned()))
    }
}

impl TryFrom<OffsetDateTime> for Date {
    type Error = WriteError;

    /// Writes `time` in RFC 3339 form with its offset.
    ///
    /// # Errors
    ///
    /// [`WriteError::BadDate`] if RFC 3339 cannot write it: an offset with
    /// seconds, or a year outside 0 to 9999.
    fn try_from(time: OffsetDateTime) -> Result<Date, WriteError> {
        time.format(&Rfc3339)
            .map(Date)
            .map_err(|_| WriteError::BadDate)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The largest UnsignedInt (JMAP section 1.3): 2^53 - 1.
const MAX_UNSIGNED_INT: u64 = (1 << 53) - 1;

/// A dialog's duration in seconds: an UnsignedInt or an UnsignedFloat
/// (core-04 2.2, 4.3.3). The two are kept apart, so 2 and 2.0 are written
/// as given.
///
/// # Examples
///
/// ```
/// use vcon::write::Duration;
///
/// let whole = Duration::try_from_secs(60).expect("a whole number of seconds");
/// let fraction = Duration::try_from_secs_f64(33.12).expect("seconds with a fraction");
/// assert_ne!(whole, fraction, "different durations");
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duration(Seconds);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Seconds {
    Whole(u64),
    Fraction(f64),
}

impl Duration {
    /// A whole number of seconds, written without a decimal point.
    ///
    /// # Errors
    ///
    /// [`WriteError::BadDuration`] if `seconds` is above 2^53 - 1, the
    /// largest UnsignedInt.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::Duration;
    ///
    /// assert!(Duration::try_from_secs(60).is_ok(), "60 seconds");
    /// assert!(Duration::try_from_secs(1 << 53).is_err(), "above 2^53 - 1");
    /// ```
    pub fn try_from_secs(seconds: u64) -> Result<Duration, WriteError> {
        if seconds > MAX_UNSIGNED_INT {
            return Err(WriteError::BadDuration);
        }
        Ok(Duration(Seconds::Whole(seconds)))
    }

    /// A number of seconds written with a decimal point.
    ///
    /// # Errors
    ///
    /// [`WriteError::BadDuration`] if `seconds` is not finite, is negative
    /// (-0.0 included, which is written with a minus sign), or is written in
    /// exponent form without a decimal point, such as 1e300 or 1e-7.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::Duration;
    ///
    /// assert!(Duration::try_from_secs_f64(33.12).is_ok(), "33.12 seconds");
    /// assert!(Duration::try_from_secs_f64(-0.5).is_err(), "negative");
    /// ```
    pub fn try_from_secs_f64(seconds: f64) -> Result<Duration, WriteError> {
        let written = serde_json::Number::from_f64(seconds).map(|n| n.to_string());
        match written {
            Some(text) if !seconds.is_sign_negative() && text.contains('.') => {
                Ok(Duration(Seconds::Fraction(seconds)))
            }
            _ => Err(WriteError::BadDuration),
        }
    }
}

macro_rules! index {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// # Examples
        ///
        /// ```
        #[doc = concat!("let i = vcon::write::", stringify!($name), "::new(2);")]
        #[doc = "assert_eq!(i.get(), 2, \"the index given\");"]
        /// ```
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(usize);

        impl $name {
            /// The index `i`, as other objects write it.
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("let i = vcon::write::", stringify!($name), "::new(0);")]
            #[doc = "assert_eq!(i.get(), 0, \"the first entry\");"]
            /// ```
            #[must_use]
            pub const fn new(i: usize) -> $name {
                $name(i)
            }

            /// The number written.
            ///
            /// # Examples
            ///
            /// ```
            #[doc = concat!("assert_eq!(vcon::write::", stringify!($name), "::new(7).get(), 7, \"the number given\");")]
            /// ```
            #[must_use]
            pub const fn get(self) -> usize {
                self.0
            }
        }
    };
}

index!(
    /// An index into `parties`.
    PartyIndex
);
index!(
    /// An index into `dialog`.
    DialogIndex
);
index!(
    /// An index into `attachments`.
    AttachmentIndex
);
index!(
    /// An index into `analysis`.
    AnalysisIndex
);

/// A ContentHash (core-04 2.2): an algorithm name, a hyphen, and the
/// base64url digest. It is made from a digest with [`ContentHash::try_new`]
/// or parsed from a token with [`str::parse`].
///
/// # Examples
///
/// ```
/// use vcon::write::ContentHash;
///
/// let made = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
/// let read: ContentHash = "sha512-AQID".parse().expect("a token");
/// assert_eq!(made, read, "the same hash");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(String);

impl ContentHash {
    /// The hash of algorithm `alg` whose digest is `digest`.
    ///
    /// # Errors
    ///
    /// [`WriteError::BadHash`] if `alg` is empty or holds anything but
    /// lower-case letters and digits, or `digest` is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::ContentHash;
    ///
    /// let hash = ContentHash::try_new("sha512", &[0xfb, 0xff]).expect("a hash");
    /// assert_eq!(hash.as_str(), "sha512--_8", "the digest in base64url");
    /// assert!(ContentHash::try_new("SHA512", &[1]).is_err(), "upper case");
    /// ```
    pub fn try_new(alg: &str, digest: &[u8]) -> Result<ContentHash, WriteError> {
        if !is_hash_algorithm(alg) || digest.is_empty() {
            return Err(WriteError::BadHash);
        }
        Ok(ContentHash(format!(
            "{alg}-{}",
            URL_SAFE_NO_PAD.encode(digest)
        )))
    }

    /// The token written.
    ///
    /// # Examples
    ///
    /// ```
    /// let hash: vcon::write::ContentHash = "sha512-AQID".parse().expect("a token");
    /// assert_eq!(hash.as_str(), "sha512-AQID", "the token as given");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The algorithm name: the token up to its first hyphen, since an
    /// algorithm name has none.
    ///
    /// # Examples
    ///
    /// ```
    /// let hash: vcon::write::ContentHash = "sha512--Pj4".parse().expect("a token");
    /// assert_eq!(hash.algorithm(), "sha512", "up to the first hyphen");
    /// ```
    #[must_use]
    pub fn algorithm(&self) -> &str {
        self.0.split_once('-').map_or("", |(alg, _)| alg)
    }

    /// The digest, decoded from base64url.
    ///
    /// # Examples
    ///
    /// ```
    /// let hash: vcon::write::ContentHash = "sha512-AQID".parse().expect("a token");
    /// assert_eq!(hash.digest(), vec![1, 2, 3], "the digest bytes");
    /// ```
    #[must_use]
    pub fn digest(&self) -> Vec<u8> {
        // Every ContentHash was checked to decode when it was made.
        let digest = self.0.split_once('-').map_or("", |(_, d)| d);
        URL_SAFE_NO_PAD.decode(digest).unwrap_or_default()
    }
}

impl FromStr for ContentHash {
    type Err = WriteError;

    /// Takes a token such as `sha512-GLy6...`: the algorithm ends at the
    /// first hyphen, since an algorithm name has none.
    ///
    /// # Errors
    ///
    /// [`WriteError::BadHash`] if the token has no hyphen, its algorithm is
    /// not lower-case letters and digits, or its digest is empty or not
    /// base64url without padding.
    fn from_str(token: &str) -> Result<ContentHash, WriteError> {
        let (alg, digest) = token.split_once('-').ok_or(WriteError::BadHash)?;
        let decoded = URL_SAFE_NO_PAD
            .decode(digest)
            .map_err(|_| WriteError::BadHash)?;
        if !is_hash_algorithm(alg) || decoded.is_empty() {
            return Err(WriteError::BadHash);
        }
        Ok(ContentHash(token.to_owned()))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A hash algorithm name: lower-case letters and digits, no hyphen
/// (core-04 2.2 ContentHash).
fn is_hash_algorithm(alg: &str) -> bool {
    !alg.is_empty()
        && alg
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// An inline body and the encoding it is written with (core-04 2.3).
///
/// # Examples
///
/// ```
/// use vcon::write::{Body, Content};
///
/// let text = Content::Inline(Body::Text("Good day!".to_owned()));
/// let bytes = Content::Inline(Body::Bytes(vec![0, 1, 2]));
/// assert_ne!(text, bytes, "different bodies");
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Body {
    /// Bytes, written base64url without padding, encoding `base64url`.
    Bytes(Vec<u8>),
    /// Text, written as a JSON string, encoding `none`.
    Text(String),
    /// A JSON value, written as it is, encoding `json`.
    Json(Value),
}

/// The content of a dialog, attachment or analysis.
///
/// # Examples
///
/// ```
/// use vcon::write::{Content, ContentHash};
///
/// let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
/// let external = Content::External {
///     url: "https://example.com/call.mp3".to_owned(),
///     hashes: vec![hash],
/// };
/// assert_ne!(external, Content::Absent, "content is present");
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Content {
    /// In the vCon: `body` and `encoding` (2.3).
    Inline(Body),
    /// Elsewhere: an https `url` and at least one `content_hash` (2.4).
    External {
        /// Where the content is.
        url: String,
        /// Its hashes: one is written as a string, several as an array.
        hashes: Vec<ContentHash>,
    },
    /// The `url` redacted and its `content_hash` kept, as in core-04 A.8;
    /// only in a redacted vCon.
    HashOnly(Vec<ContentHash>),
    /// No content: only in a redacted vCon, a removed dialog, a recording
    /// placeholder, or a dialog type that has no content.
    Absent,
}

/// A reference to another vCon instance version, for `amended` or
/// `redacted` (core-04: the Redacted Object of 4.1.8 and the Amended Object
/// of 4.1.9).
///
/// # Examples
///
/// ```
/// use vcon::write::{ContentHash, Reference};
///
/// let prior = vcon::write::uuid_v8(0, 0, "vcon.example.com");
/// let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
/// let both = Reference::by_uuid(prior).with_url("https://example.com/prior.vcon", vec![hash]);
/// assert_ne!(both, Reference::by_uuid(prior), "the url is part of it");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Reference {
    uuid: Option<Uuid>,
    url: Option<String>,
    hashes: Vec<ContentHash>,
}

impl Reference {
    /// The vCon with this uuid.
    ///
    /// # Examples
    ///
    /// ```
    /// let prior = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let reference = vcon::write::Reference::by_uuid(prior);
    /// assert_eq!(reference, vcon::write::Reference::by_uuid(prior), "the same uuid");
    /// ```
    #[must_use]
    pub fn by_uuid(uuid: Uuid) -> Reference {
        Reference {
            uuid: Some(uuid),
            url: None,
            hashes: Vec::new(),
        }
    }

    /// The vCon at an https `url`, with its hashes. Both are checked when
    /// the vCon that holds the reference is written.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{ContentHash, Reference};
    ///
    /// let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
    /// let at = Reference::by_url("https://example.com/prior.vcon", vec![hash]);
    /// let prior = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// assert_ne!(at, Reference::by_uuid(prior), "a url, not a uuid");
    /// ```
    #[must_use]
    pub fn by_url(url: impl Into<String>, hashes: Vec<ContentHash>) -> Reference {
        Reference {
            uuid: None,
            url: Some(url.into()),
            hashes,
        }
    }

    /// The same reference, with the vCon's https `url` and its hashes too.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{ContentHash, Reference};
    ///
    /// let prior = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
    /// let both = Reference::by_uuid(prior).with_url("https://example.com/prior.vcon", vec![hash]);
    /// assert_ne!(both, Reference::by_uuid(prior), "the url was added");
    /// ```
    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>, hashes: Vec<ContentHash>) -> Reference {
        self.url = Some(url.into());
        self.hashes = hashes;
        self
    }
}

/// A Civicaddress Object (core-04 4.2.9): each member a string, written
/// with its lower-case name in the draft's order, whatever order it was set
/// in.
///
/// # Examples
///
/// ```
/// use vcon::write::{CivicAddress, Party};
///
/// let party = Party::new().civicaddress(CivicAddress::new().country("US").a1("CA"));
/// assert_ne!(party, Party::new(), "the party has an address");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct CivicAddress {
    members: Vec<(&'static str, String)>,
}

macro_rules! civic {
    ($($name:ident),*) => {
        impl CivicAddress {
            /// An empty Civicaddress Object.
            ///
            /// # Examples
            ///
            /// ```
            /// let address = vcon::write::CivicAddress::new();
            /// assert_eq!(address, vcon::write::CivicAddress::default(), "no members");
            /// ```
            #[must_use]
            pub fn new() -> CivicAddress {
                CivicAddress::default()
            }
            $(
                #[doc = concat!("Sets `", stringify!($name), "`.")]
                ///
                /// # Examples
                ///
                /// ```
                #[doc = concat!("let address = vcon::write::CivicAddress::new().", stringify!($name), "(\"x\");")]
                #[doc = "assert_ne!(address, vcon::write::CivicAddress::new(), \"one member set\");"]
                /// ```
                #[must_use]
                pub fn $name(mut self, value: impl Into<String>) -> CivicAddress {
                    self.set(stringify!($name), value.into());
                    self
                }
            )*
        }
        /// The members in the draft's order.
        pub(crate) const CIVIC_ORDER: &[&str] = &[$(stringify!($name)),*];
    };
}

civic!(
    country, a1, a2, a3, a4, a5, a6, prd, pod, sts, hno, hns, lmk, loc, flr, nam, pc
);

impl CivicAddress {
    fn set(&mut self, name: &'static str, value: String) {
        match self.members.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = value,
            None => self.members.push((name, value)),
        }
    }
}

/// Members an extension adds to an object, in the order they were set.
#[derive(Debug, Clone, Default, PartialEq)]
struct Members(Vec<(String, Value)>);

/// Names inspect still reads at the top of a vCon, from older syntax:
/// an extension member of that name would change what it finds.
const RESERVED: [&str; 3] = ["vcon", "must_support", "appended"];

const TOP_MEMBERS: &[&str] = &[
    "uuid",
    "extensions",
    "critical",
    "created_at",
    "updated_at",
    "subject",
    "redacted",
    "amended",
    "parties",
    "dialog",
    "analysis",
    "attachments",
    // The members that make inspect read a signed or encrypted vCon.
    "payload",
    "signatures",
    "ciphertext",
    "recipients",
];
const PARTY_MEMBERS: &[&str] = &[
    "tel",
    "sip",
    "stir",
    "mailto",
    "name",
    "did",
    "validation",
    "gmlpos",
    "civicaddress",
    "uuid",
    "type",
    "org",
    "dept",
];
const DIALOG_MEMBERS: &[&str] = &[
    "type",
    "start",
    "duration",
    "parties",
    "originator",
    "recordings",
    "recording_set",
    "mediatype",
    "filename",
    "body",
    "encoding",
    "url",
    "content_hash",
    "disposition",
    "session_id",
    "party_history",
    "transferee",
    "transferor",
    "transfer_target",
    "original",
    "consultation",
    "target_dialog",
    "application",
    "message_id",
];
const ATTACHMENT_MEMBERS: &[&str] = &[
    "purpose",
    "start",
    "party",
    "dialog",
    "mediatype",
    "filename",
    "body",
    "encoding",
    "url",
    "content_hash",
];
const ANALYSIS_MEMBERS: &[&str] = &[
    "type",
    "dialog",
    "attachment",
    "mediatype",
    "filename",
    "vendor",
    "product",
    "schema",
    "body",
    "encoding",
    "url",
    "content_hash",
];

impl Members {
    /// Sets `name`, unless `core` or [`RESERVED`] holds it; a name set
    /// again keeps its first place.
    fn set(&mut self, core: &[&str], name: &str, value: Value) -> Result<(), WriteError> {
        if core.contains(&name) || RESERVED.contains(&name) {
            return Err(WriteError::CoreMember(name.to_owned()));
        }
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value,
            None => self.0.push((name.to_owned(), value)),
        }
        Ok(())
    }

    fn put_into(&self, obj: &mut Obj) {
        for (name, value) in &self.0 {
            obj.put(name, Node::Val(value.clone()));
        }
    }
}

/// A Party Object (core-04 4.2).
///
/// # Examples
///
/// ```
/// use vcon::write::Party;
///
/// let alice = Party::new().tel("+15555550100").name("Alice").kind("person");
/// assert_ne!(alice, Party::new(), "the party has members");
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Party {
    tel: Option<String>,
    sip: Option<String>,
    stir: Option<String>,
    mailto: Option<String>,
    name: Option<String>,
    did: Option<String>,
    validation: Option<String>,
    gmlpos: Option<String>,
    civicaddress: Option<CivicAddress>,
    uuid: Option<String>,
    kind: Option<String>,
    org: Option<String>,
    dept: Option<String>,
    members: Members,
}

/// Setters of text members, by value. `$base` is the expression an
/// example starts from.
macro_rules! text_setters {
    ($ty:ident, $base:literal { $($(#[$doc:meta])* $method:ident => $field:ident),* $(,)? }) => {
        impl $ty {
            $(
                $(#[$doc])*
                ///
                /// # Examples
                ///
                /// ```
                #[doc = concat!("let set = ", $base, ".", stringify!($method), "(\"x\");")]
                #[doc = concat!("assert_ne!(set, ", $base, ", \"the member is set\");")]
                /// ```
                #[must_use]
                pub fn $method(mut self, value: impl Into<String>) -> $ty {
                    self.$field = Some(value.into());
                    self
                }
            )*
        }
    };
}

text_setters!(Party, "vcon::write::Party::new()" {
    /// Sets `tel`.
    tel => tel,
    /// Sets `sip`.
    sip => sip,
    /// Sets `stir`, a PASSporT in JWS Compact Serialization.
    stir => stir,
    /// Sets `mailto`.
    mailto => mailto,
    /// Sets `name`.
    name => name,
    /// Sets `did`.
    did => did,
    /// Sets `validation`.
    validation => validation,
    /// Sets `gmlpos`.
    gmlpos => gmlpos,
    /// Sets `uuid`, a free-form identifier of the participant (4.2.10).
    uuid => uuid,
    /// Sets `type`, such as `person`, `bot` or `organization`.
    kind => kind,
    /// Sets `org`.
    org => org,
    /// Sets `dept`.
    dept => dept,
});

impl Party {
    /// A Party Object with no members, written as `{}`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(vcon::write::Party::new(), vcon::write::Party::default(), "no members");
    /// ```
    #[must_use]
    pub fn new() -> Party {
        Party::default()
    }

    /// Sets `civicaddress`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{CivicAddress, Party};
    ///
    /// let party = Party::new().civicaddress(CivicAddress::new().country("US"));
    /// assert_ne!(party, Party::new(), "the address is set");
    /// ```
    #[must_use]
    pub fn civicaddress(mut self, address: CivicAddress) -> Party {
        self.civicaddress = Some(address);
        self
    }

    /// Sets a member an extension defines; setting a name again replaces
    /// its value and keeps its place.
    ///
    /// # Errors
    ///
    /// [`WriteError::CoreMember`] if `name` is a member core-04 defines for
    /// a Party Object, or `vcon`, `must_support` or `appended`.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use vcon::write::Party;
    ///
    /// let party = Party::new().set_member("example_role", json!("agent")).expect("free");
    /// assert_ne!(party, Party::new(), "the member is set");
    /// assert!(Party::new().set_member("name", json!("x")).is_err(), "a core member");
    /// ```
    pub fn set_member(mut self, name: &str, value: Value) -> Result<Party, WriteError> {
        self.members.set(PARTY_MEMBERS, name, value)?;
        Ok(self)
    }
}

/// The type of a Dialog Object (core-04 4.3.1).
///
/// # Examples
///
/// ```
/// use vcon::write::DialogType;
///
/// assert_eq!(DialogType::RecordingSet.as_str(), "recording-set", "as the draft spells it");
/// assert_eq!(DialogType::RecordingSet.to_string(), "recording-set", "Display too");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DialogType {
    /// `recording`
    Recording,
    /// `recording-set`
    RecordingSet,
    /// `text`
    Text,
    /// `transfer`
    Transfer,
    /// `incomplete`
    Incomplete,
}

impl DialogType {
    /// The value written.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(vcon::write::DialogType::Text.as_str(), "text", "the value written");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DialogType::Recording => "recording",
            DialogType::RecordingSet => "recording-set",
            DialogType::Text => "text",
            DialogType::Transfer => "transfer",
            DialogType::Incomplete => "incomplete",
        }
    }
}

impl fmt::Display for DialogType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a call or conversation failed (core-04 4.3.11).
///
/// # Examples
///
/// ```
/// use vcon::write::Disposition;
///
/// assert_eq!(Disposition::HungUp.as_str(), "hung-up", "as the draft spells it");
/// assert_eq!(Disposition::HungUp.to_string(), "hung-up", "Display too");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Disposition {
    /// `no-answer`
    NoAnswer,
    /// `congestion`
    Congestion,
    /// `failed`
    Failed,
    /// `busy`
    Busy,
    /// `hung-up`
    HungUp,
    /// `voicemail-no-message`
    VoicemailNoMessage,
}

impl Disposition {
    /// The value written.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(vcon::write::Disposition::Busy.as_str(), "busy", "the value written");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Disposition::NoAnswer => "no-answer",
            Disposition::Congestion => "congestion",
            Disposition::Failed => "failed",
            Disposition::Busy => "busy",
            Disposition::HungUp => "hung-up",
            Disposition::VoicemailNoMessage => "voicemail-no-message",
        }
    }
}

impl fmt::Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A Party_History event (core-04 4.3.13, the Party_History Object). A key
/// event carries the button pressed or released.
///
/// # Examples
///
/// ```
/// use vcon::write::Event;
///
/// let key = Event::Keydown("5".to_owned());
/// assert_ne!(key, Event::Keyup("5".to_owned()), "down is not up");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Event {
    /// `join`
    Join,
    /// `drop`
    Drop,
    /// `hold`
    Hold,
    /// `unhold`
    Unhold,
    /// `mute`
    Mute,
    /// `unmute`
    Unmute,
    /// `keydown`, with its `button`.
    Keydown(String),
    /// `keyup`, with its `button`.
    Keyup(String),
}

/// A Party_History Object (core-04 4.3.13).
///
/// # Examples
///
/// ```
/// use vcon::write::{Event, PartyHistory, PartyIndex};
///
/// let time = "2022-06-21T17:53:27Z".parse().expect("RFC 3339");
/// let joined = PartyHistory::new(PartyIndex::new(0), time, Event::Join);
/// assert_eq!(joined.clone(), joined, "an event");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartyHistory {
    party: PartyIndex,
    time: Date,
    event: Event,
}

impl PartyHistory {
    /// Party `party` had `event` at `time`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Event, PartyHistory, PartyIndex};
    ///
    /// let time = "2022-06-21T17:53:27Z".parse().expect("RFC 3339");
    /// let held = PartyHistory::new(PartyIndex::new(1), time, Event::Hold);
    /// assert_eq!(held.clone(), held, "an event");
    /// ```
    #[must_use]
    pub fn new(party: PartyIndex, time: Date, event: Event) -> PartyHistory {
        PartyHistory { party, time, event }
    }
}

/// One channel of a dialog's `parties` (core-04 4.3.4).
///
/// # Examples
///
/// ```
/// use vcon::write::{Channel, PartyIndex};
///
/// let mixed = Channel::Mix(vec![PartyIndex::new(0), PartyIndex::new(1)]);
/// assert_ne!(mixed, Channel::Unrecorded, "a channel with parties");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Channel {
    /// One party.
    One(PartyIndex),
    /// The parties mixed into the channel.
    Mix(Vec<PartyIndex>),
    /// A channel with no party recorded, written as `null`.
    Unrecorded,
}

/// A dialog's `parties` (core-04 4.3.4).
///
/// # Examples
///
/// ```
/// use vcon::write::{Channel, Parties, PartyIndex};
///
/// let two = Parties::Channels(vec![Channel::One(PartyIndex::new(0)), Channel::Unrecorded]);
/// assert_ne!(two, Parties::One(PartyIndex::new(0)), "an array, not one index");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Parties {
    /// One index.
    One(PartyIndex),
    /// An array: the parties mixed into a single channel, or one entry per
    /// channel.
    Channels(Vec<Channel>),
}

/// A SessionId Object (core-04 2.2), or the empty object a party with no
/// session id has (4.3.12).
///
/// # Examples
///
/// ```
/// use vcon::write::SessionId;
///
/// let ids = SessionId::new("local-id", "remote-id");
/// assert_ne!(ids, SessionId::none(), "ids, not {{}}");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(Option<(String, String)>);

impl SessionId {
    /// The session ids `local` and `remote`.
    ///
    /// # Examples
    ///
    /// ```
    /// let ids = vcon::write::SessionId::new("local-id", "remote-id");
    /// assert_eq!(ids.clone(), ids, "a session id");
    /// ```
    #[must_use]
    pub fn new(local: impl Into<String>, remote: impl Into<String>) -> SessionId {
        SessionId(Some((local.into(), remote.into())))
    }

    /// `{}`: no session id for this party.
    ///
    /// # Examples
    ///
    /// ```
    /// let none = vcon::write::SessionId::none();
    /// assert_eq!(none, vcon::write::SessionId::none(), "always {{}}");
    /// ```
    #[must_use]
    pub fn none() -> SessionId {
        SessionId(None)
    }
}

/// One entry of an array of session ids, in the shape of `parties`.
///
/// # Examples
///
/// ```
/// use vcon::write::{SessionChannel, SessionId};
///
/// let one = SessionChannel::One(SessionId::none());
/// assert_ne!(one, SessionChannel::Mix(vec![]), "one, not a mix");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionChannel {
    /// One SessionId.
    One(SessionId),
    /// An array of them.
    Mix(Vec<SessionId>),
}

/// A dialog's `session_id` (core-04 4.3.12).
///
/// # Examples
///
/// ```
/// use vcon::write::{SessionId, SessionIds};
///
/// let all = SessionIds::One(SessionId::new("local-id", "remote-id"));
/// assert_ne!(all, SessionIds::Channels(vec![]), "one for every party");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionIds {
    /// One SessionId for every party.
    One(SessionId),
    /// An array that follows the shape of `parties`.
    Channels(Vec<SessionChannel>),
}

/// One index or an array of them, as analysis `dialog` and `attachment`
/// are written.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

/// A Dialog Object (core-04 4.3). What Table 1 marks M or X for its type is
/// checked when the vCon is written.
///
/// # Examples
///
/// ```
/// use vcon::write::{Body, Content, Dialog, Parties, PartyIndex};
///
/// let text = Dialog::text(Content::Inline(Body::Text("Hi".to_owned())))
///     .mediatype("text/plain")
///     .parties(Parties::One(PartyIndex::new(0)));
/// assert_ne!(text, Dialog::transfer(), "a text dialog");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    kind: DialogType,
    start: Option<Date>,
    duration: Option<Duration>,
    parties: Option<Parties>,
    originator: Option<PartyIndex>,
    recordings: Option<Vec<DialogIndex>>,
    recording_set: Option<DialogIndex>,
    mediatype: Option<String>,
    filename: Option<String>,
    content: Content,
    disposition: Option<Disposition>,
    session_id: Option<SessionIds>,
    party_history: Option<Vec<PartyHistory>>,
    transferee: Option<PartyIndex>,
    transferor: Option<PartyIndex>,
    transfer_target: Option<PartyIndex>,
    original: Option<DialogIndex>,
    consultation: Option<DialogIndex>,
    target_dialog: Option<DialogIndex>,
    application: Option<String>,
    message_id: Option<String>,
    members: Members,
    removed: bool,
}

text_setters!(Dialog, "vcon::write::Dialog::transfer()" {
    /// Sets `mediatype`.
    mediatype => mediatype,
    /// Sets `filename`.
    filename => filename,
    /// Sets `application`.
    application => application,
    /// Sets `message_id`.
    message_id => message_id,
});

/// Setters of typed members, by value. `$base` is the expression an
/// example starts from and `$value` the value it sets.
macro_rules! setters {
    ($ty:ident { $($(#[$doc:meta])* $method:ident($arg:ty) => $field:ident, $base:literal, $value:literal);* $(;)? }) => {
        impl $ty {
            $(
                $(#[$doc])*
                ///
                /// # Examples
                ///
                /// ```
                #[doc = concat!("let set = ", $base, ".", stringify!($method), "(", $value, ");")]
                #[doc = concat!("assert_ne!(set, ", $base, ", \"the member is set\");")]
                /// ```
                #[must_use]
                pub fn $method(mut self, value: $arg) -> $ty {
                    self.$field = Some(value);
                    self
                }
            )*
        }
    };
}

setters!(Dialog {
    /// Sets `start`.
    start(Date) => start, "vcon::write::Dialog::transfer()",
        "\"2022-06-21T17:53:26Z\".parse().expect(\"RFC 3339\")";
    /// Sets `duration`.
    duration(Duration) => duration, "vcon::write::Dialog::transfer()",
        "vcon::write::Duration::try_from_secs(60).expect(\"seconds\")";
    /// Sets `parties`.
    parties(Parties) => parties, "vcon::write::Dialog::transfer()",
        "vcon::write::Parties::One(vcon::write::PartyIndex::new(0))";
    /// Sets `originator`.
    originator(PartyIndex) => originator, "vcon::write::Dialog::transfer()",
        "vcon::write::PartyIndex::new(1)";
    /// Sets `recordings`, the recording dialogs of a recording-set.
    recordings(Vec<DialogIndex>) => recordings, "vcon::write::Dialog::transfer()",
        "vec![vcon::write::DialogIndex::new(0)]";
    /// Sets `recording_set`, the recording-set this recording is part of.
    in_recording_set(DialogIndex) => recording_set, "vcon::write::Dialog::transfer()",
        "vcon::write::DialogIndex::new(2)";
    /// Sets `disposition`.
    disposition(Disposition) => disposition, "vcon::write::Dialog::transfer()",
        "vcon::write::Disposition::Busy";
    /// Sets `session_id`.
    session_id(SessionIds) => session_id, "vcon::write::Dialog::transfer()",
        "vcon::write::SessionIds::One(vcon::write::SessionId::none())";
    /// Sets `party_history`.
    party_history(Vec<PartyHistory>) => party_history, "vcon::write::Dialog::transfer()",
        "Vec::new()";
    /// Sets `transferee`.
    transferee(PartyIndex) => transferee, "vcon::write::Dialog::transfer()",
        "vcon::write::PartyIndex::new(0)";
    /// Sets `transferor`.
    transferor(PartyIndex) => transferor, "vcon::write::Dialog::transfer()",
        "vcon::write::PartyIndex::new(1)";
    /// Sets `transfer_target`.
    transfer_target(PartyIndex) => transfer_target, "vcon::write::Dialog::transfer()",
        "vcon::write::PartyIndex::new(2)";
    /// Sets `original`.
    original(DialogIndex) => original, "vcon::write::Dialog::transfer()",
        "vcon::write::DialogIndex::new(0)";
    /// Sets `consultation`.
    consultation(DialogIndex) => consultation, "vcon::write::Dialog::transfer()",
        "vcon::write::DialogIndex::new(1)";
    /// Sets `target_dialog`.
    target_dialog(DialogIndex) => target_dialog, "vcon::write::Dialog::transfer()",
        "vcon::write::DialogIndex::new(2)";
});

impl Dialog {
    /// A Dialog Object of type `kind` with no other member.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, DialogType};
    ///
    /// assert_eq!(Dialog::new(DialogType::Transfer), Dialog::transfer(), "the same dialog");
    /// ```
    #[must_use]
    pub fn new(kind: DialogType) -> Dialog {
        Dialog {
            kind,
            start: None,
            duration: None,
            parties: None,
            originator: None,
            recordings: None,
            recording_set: None,
            mediatype: None,
            filename: None,
            content: Content::Absent,
            disposition: None,
            session_id: None,
            party_history: None,
            transferee: None,
            transferor: None,
            transfer_target: None,
            original: None,
            consultation: None,
            target_dialog: None,
            application: None,
            message_id: None,
            members: Members::default(),
            removed: false,
        }
    }

    /// A recording with its content.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Content, ContentHash, Dialog};
    ///
    /// let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
    /// let call = Dialog::recording(Content::External {
    ///     url: "https://example.com/call.mp3".to_owned(),
    ///     hashes: vec![hash],
    /// });
    /// assert_ne!(call, Dialog::transfer(), "a recording");
    /// ```
    #[must_use]
    pub fn recording(content: Content) -> Dialog {
        Dialog::new(DialogType::Recording).content(content)
    }

    /// A text dialog with its content.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Body, Content, Dialog};
    ///
    /// let text = Dialog::text(Content::Inline(Body::Text("Hi".to_owned())));
    /// assert_ne!(text, Dialog::transfer(), "a text dialog");
    /// ```
    #[must_use]
    pub fn text(content: Content) -> Dialog {
        Dialog::new(DialogType::Text).content(content)
    }

    /// A recording-set of the recording dialogs `recordings`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, DialogIndex};
    ///
    /// let set = Dialog::recording_set(vec![DialogIndex::new(0), DialogIndex::new(1)]);
    /// assert_ne!(set, Dialog::transfer(), "a recording-set");
    /// ```
    #[must_use]
    pub fn recording_set(recordings: Vec<DialogIndex>) -> Dialog {
        Dialog::new(DialogType::RecordingSet).recordings(recordings)
    }

    /// A transfer; its roles and dialogs are set with the transfer setters.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, PartyIndex};
    ///
    /// let transfer = Dialog::transfer().transferee(PartyIndex::new(0));
    /// assert_ne!(transfer, Dialog::transfer(), "a transfer with its transferee");
    /// ```
    #[must_use]
    pub fn transfer() -> Dialog {
        Dialog::new(DialogType::Transfer)
    }

    /// A call or conversation that failed, and why.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, Disposition};
    ///
    /// let busy = Dialog::incomplete(Disposition::Busy);
    /// assert_ne!(busy, Dialog::incomplete(Disposition::NoAnswer), "a different reason");
    /// ```
    #[must_use]
    pub fn incomplete(disposition: Disposition) -> Dialog {
        Dialog::new(DialogType::Incomplete).disposition(disposition)
    }

    /// A placeholder (core-04 4.3): only `type`, and for an incomplete one
    /// the disposition `failed`.
    ///
    /// # Errors
    ///
    /// [`WriteError::NoPlaceholder`] for any type but recording and
    /// incomplete, the two 4.3 makes placeholders of.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, DialogType};
    ///
    /// assert!(Dialog::try_placeholder(DialogType::Recording).is_ok(), "a recording");
    /// assert!(Dialog::try_placeholder(DialogType::Text).is_err(), "not text");
    /// ```
    pub fn try_placeholder(kind: DialogType) -> Result<Dialog, WriteError> {
        match kind {
            DialogType::Recording => Ok(Dialog::new(DialogType::Recording)),
            DialogType::Incomplete => Ok(Dialog::incomplete(Disposition::Failed)),
            other => Err(WriteError::NoPlaceholder(other)),
        }
    }

    /// Sets the content: `body` and `encoding`, or `url` and
    /// `content_hash`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Body, Content, Dialog, DialogType};
    ///
    /// let text = Dialog::new(DialogType::Text).content(Content::Inline(Body::Text("Hi".to_owned())));
    /// assert_ne!(text, Dialog::new(DialogType::Text), "the content is set");
    /// ```
    #[must_use]
    pub fn content(mut self, content: Content) -> Dialog {
        self.content = content;
        self
    }

    /// Sets a member an extension defines; setting a name again replaces
    /// its value and keeps its place.
    ///
    /// # Errors
    ///
    /// [`WriteError::CoreMember`] if `name` is a member core-04 defines for
    /// a Dialog Object, or `vcon`, `must_support` or `appended`.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use vcon::write::Dialog;
    ///
    /// let set = Dialog::transfer().set_member("example_reason", json!("queue")).expect("free");
    /// assert_ne!(set, Dialog::transfer(), "the member is set");
    /// assert!(Dialog::transfer().set_member("parties", json!(0)).is_err(), "a core member");
    /// ```
    pub fn set_member(mut self, name: &str, value: Value) -> Result<Dialog, WriteError> {
        self.members.set(DIALOG_MEMBERS, name, value)?;
        Ok(self)
    }
}

/// An Attachment Object (core-04 4.4).
///
/// # Examples
///
/// ```
/// use vcon::write::{Attachment, Body, Content, DialogIndex, PartyIndex};
///
/// let start = "2022-06-21T17:55:00Z".parse().expect("RFC 3339");
/// let notes = Content::Inline(Body::Text("notes".to_owned()));
/// let a = Attachment::new(start, PartyIndex::new(0), DialogIndex::new(0), notes)
///     .mediatype("text/plain");
/// assert_eq!(a.clone(), a, "an attachment");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    purpose: Option<String>,
    start: Date,
    party: PartyIndex,
    dialog: DialogIndex,
    mediatype: Option<String>,
    filename: Option<String>,
    content: Content,
    members: Members,
}

text_setters!(Attachment, "vcon::write::Attachment::new(\"2022-06-21T17:55:00Z\".parse().expect(\"RFC 3339\"), vcon::write::PartyIndex::new(0), vcon::write::DialogIndex::new(0), vcon::write::Content::Absent)" {
    /// Sets `purpose`.
    purpose => purpose,
    /// Sets `mediatype`.
    mediatype => mediatype,
    /// Sets `filename`.
    filename => filename,
});

impl Attachment {
    /// An attachment sent at `start` by `party`, part of `dialog`; core-04
    /// requires all three.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Attachment, Content, ContentHash, DialogIndex, PartyIndex};
    ///
    /// let start = "2022-06-21T17:55:00Z".parse().expect("RFC 3339");
    /// let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
    /// let file = Content::External { url: "https://example.com/a.pdf".to_owned(), hashes: vec![hash] };
    /// let a = Attachment::new(start, PartyIndex::new(1), DialogIndex::new(0), file);
    /// assert_eq!(a.clone(), a, "an attachment");
    /// ```
    #[must_use]
    pub fn new(
        start: Date,
        party: PartyIndex,
        dialog: DialogIndex,
        content: Content,
    ) -> Attachment {
        Attachment {
            purpose: None,
            start,
            party,
            dialog,
            mediatype: None,
            filename: None,
            content,
            members: Members::default(),
        }
    }

    /// Sets a member an extension defines; setting a name again replaces
    /// its value and keeps its place.
    ///
    /// # Errors
    ///
    /// [`WriteError::CoreMember`] if `name` is a member core-04 defines for
    /// an Attachment Object, or `vcon`, `must_support` or `appended`.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use vcon::write::{Attachment, Content, DialogIndex, PartyIndex};
    ///
    /// let start = "2022-06-21T17:55:00Z".parse().expect("RFC 3339");
    /// let a = Attachment::new(start, PartyIndex::new(0), DialogIndex::new(0), Content::Absent);
    /// assert!(a.clone().set_member("example_pages", json!(3)).is_ok(), "free");
    /// assert!(a.set_member("party", json!(0)).is_err(), "a core member");
    /// ```
    pub fn set_member(mut self, name: &str, value: Value) -> Result<Attachment, WriteError> {
        self.members.set(ATTACHMENT_MEMBERS, name, value)?;
        Ok(self)
    }
}

/// An Analysis Object (core-04 4.5).
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use vcon::write::{Analysis, Body, Content, DialogIndex};
///
/// let summary = Analysis::new("summary", "Example", Content::Inline(Body::Json(json!({"text": "ok"}))))
///     .dialog(DialogIndex::new(0));
/// assert_eq!(summary.clone(), summary, "an analysis");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    kind: String,
    dialog: Option<OneOrMany<DialogIndex>>,
    attachment: Option<OneOrMany<AttachmentIndex>>,
    mediatype: Option<String>,
    filename: Option<String>,
    vendor: String,
    product: Option<String>,
    schema: Option<String>,
    content: Content,
    members: Members,
}

text_setters!(Analysis, "vcon::write::Analysis::new(\"summary\", \"Example\", vcon::write::Content::Absent)" {
    /// Sets `mediatype`.
    mediatype => mediatype,
    /// Sets `filename`.
    filename => filename,
    /// Sets `product`.
    product => product,
    /// Sets `schema`.
    schema => schema,
});

impl Analysis {
    /// An analysis of type `kind` (such as `transcript`) made by `vendor`;
    /// core-04 requires both.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use vcon::write::{Analysis, Body, Content};
    ///
    /// let a = Analysis::new("sentiment", "Example", Content::Inline(Body::Json(json!(0.5))));
    /// assert_eq!(a.clone(), a, "an analysis");
    /// ```
    #[must_use]
    pub fn new(kind: impl Into<String>, vendor: impl Into<String>, content: Content) -> Analysis {
        Analysis {
            kind: kind.into(),
            dialog: None,
            attachment: None,
            mediatype: None,
            filename: None,
            vendor: vendor.into(),
            product: None,
            schema: None,
            content,
            members: Members::default(),
        }
    }

    /// Sets `dialog` to one index.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, Content, DialogIndex};
    ///
    /// let a = Analysis::new("summary", "Example", Content::Absent);
    /// assert_ne!(a.clone().dialog(DialogIndex::new(0)), a, "the dialog is set");
    /// ```
    #[must_use]
    pub fn dialog(mut self, dialog: DialogIndex) -> Analysis {
        self.dialog = Some(OneOrMany::One(dialog));
        self
    }

    /// Sets `dialog` to an array of indices.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, Content, DialogIndex};
    ///
    /// let a = Analysis::new("summary", "Example", Content::Absent);
    /// let two = a.clone().dialogs(vec![DialogIndex::new(0), DialogIndex::new(1)]);
    /// assert_ne!(two, a, "the dialogs are set");
    /// ```
    #[must_use]
    pub fn dialogs(mut self, dialogs: Vec<DialogIndex>) -> Analysis {
        self.dialog = Some(OneOrMany::Many(dialogs));
        self
    }

    /// Sets `attachment` to one index.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, AttachmentIndex, Content};
    ///
    /// let a = Analysis::new("summary", "Example", Content::Absent);
    /// assert_ne!(a.clone().attachment(AttachmentIndex::new(0)), a, "the attachment is set");
    /// ```
    #[must_use]
    pub fn attachment(mut self, attachment: AttachmentIndex) -> Analysis {
        self.attachment = Some(OneOrMany::One(attachment));
        self
    }

    /// Sets `attachment` to an array of indices.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, AttachmentIndex, Content};
    ///
    /// let a = Analysis::new("summary", "Example", Content::Absent);
    /// assert_ne!(a.clone().attachments(vec![AttachmentIndex::new(0)]), a, "the attachments are set");
    /// ```
    #[must_use]
    pub fn attachments(mut self, attachments: Vec<AttachmentIndex>) -> Analysis {
        self.attachment = Some(OneOrMany::Many(attachments));
        self
    }

    /// Sets a member an extension defines; setting a name again replaces
    /// its value and keeps its place.
    ///
    /// # Errors
    ///
    /// [`WriteError::CoreMember`] if `name` is a member core-04 defines for
    /// an Analysis Object, or `vcon`, `must_support` or `appended`.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    /// use vcon::write::{Analysis, Content};
    ///
    /// let a = Analysis::new("summary", "Example", Content::Absent);
    /// assert!(a.clone().set_member("example_model", json!("m1")).is_ok(), "free");
    /// assert!(a.set_member("vendor", json!("x")).is_err(), "a core member");
    /// ```
    pub fn set_member(mut self, name: &str, value: Value) -> Result<Analysis, WriteError> {
        self.members.set(ANALYSIS_MEMBERS, name, value)?;
        Ok(self)
    }
}

/// The Redacted Object: the kind of redaction and, if any, the less
/// redacted version.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Redacted {
    kind: String,
    prior: Option<Reference>,
}

/// A vCon in the unsigned form (core-04 section 4).
///
/// # Examples
///
/// ```
/// use vcon::write::{Body, Content, Dialog, Party, Vcon};
///
/// let uuid = vcon::write::uuid_v8(1_655_833_980_000, 0x3e5, "vcon.example.com");
/// let mut v = Vcon::new(uuid, "2022-06-21T13:53:00-04:00".parse().expect("RFC 3339"));
/// v.add_party(Party::new().name("Alice"));
/// v.add_dialog(Dialog::text(Content::Inline(Body::Text("Hi".to_owned()))).mediatype("text/plain"));
/// let json = v.to_json().expect("a valid vCon");
/// assert!(json.ends_with(br#""analysis":[],"attachments":[]}"#), "the four arrays end it");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Vcon {
    uuid: Uuid,
    extensions: Vec<String>,
    critical: Vec<String>,
    created_at: Date,
    updated_at: Option<Date>,
    subject: Option<String>,
    redacted: Option<Redacted>,
    amended: Option<Reference>,
    parties: Vec<Option<Party>>,
    dialog: Vec<Dialog>,
    analysis: Vec<Option<Analysis>>,
    attachments: Vec<Option<Attachment>>,
    members: Members,
}

impl Vcon {
    /// A vCon with its uuid and creation time and nothing else.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// assert_eq!(v.uuid(), uuid, "its uuid");
    /// ```
    #[must_use]
    pub fn new(uuid: Uuid, created_at: Date) -> Vcon {
        Vcon {
            uuid,
            extensions: Vec::new(),
            critical: Vec::new(),
            created_at,
            updated_at: None,
            subject: None,
            redacted: None,
            amended: None,
            parties: Vec::new(),
            dialog: Vec::new(),
            analysis: Vec::new(),
            attachments: Vec::new(),
            members: Members::default(),
        }
    }

    /// Its uuid.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// assert_eq!(v.uuid(), uuid, "the uuid it was made with");
    /// ```
    #[must_use]
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Sets `subject`.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.set_subject("Billing question");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""subject":"Billing question""#), "the subject is written");
    /// ```
    pub fn set_subject(&mut self, subject: impl Into<String>) {
        self.subject = Some(subject.into());
    }

    /// Sets `updated_at`.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.set_updated_at("2026-10-27T01:00:00Z".parse().expect("RFC 3339"));
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""updated_at":"2026-10-27T01:00:00Z""#), "updated_at is written");
    /// ```
    pub fn set_updated_at(&mut self, updated_at: Date) {
        self.updated_at = Some(updated_at);
    }

    /// Lists `name` in `extensions`, once.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.use_extension("example_ext");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""extensions":["example_ext"]"#), "listed in extensions");
    /// ```
    pub fn use_extension(&mut self, name: &str) {
        if !self.extensions.iter().any(|e| e == name) {
            self.extensions.push(name.to_owned());
        }
    }

    /// Lists `name` in `extensions` and in `critical`, once each: a reader
    /// that does not support the extension must not process the vCon
    /// (core-04 4.1.4).
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.use_critical_extension("example_ext");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""critical":["example_ext"]"#), "listed in critical");
    /// ```
    pub fn use_critical_extension(&mut self, name: &str) {
        self.use_extension(name);
        if !self.critical.iter().any(|e| e == name) {
            self.critical.push(name.to_owned());
        }
    }

    /// Sets a top-level member an extension defines; setting a name again
    /// replaces its value and keeps its place.
    ///
    /// # Errors
    ///
    /// [`WriteError::CoreMember`] if `name` is a top-level member core-04
    /// defines; `vcon`, `must_support` or `appended`; or `payload`,
    /// `signatures`, `ciphertext` or `recipients`, which would make
    /// [`crate::read::inspect`] find another form.
    ///
    /// # Examples
    ///
    /// ```
    /// use serde_json::json;
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// assert!(v.set_member("example_queue", json!("billing")).is_ok(), "free");
    /// assert!(v.set_member("payload", json!("x")).is_err(), "would change the form");
    /// ```
    pub fn set_member(&mut self, name: &str, value: Value) -> Result<(), WriteError> {
        self.members.set(TOP_MEMBERS, name, value)
    }

    /// Sets `amended`: this vCon amends the one `prior` names.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Reference, Vcon};
    ///
    /// let prior = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let uuid = vcon::write::uuid_v8(1, 0, "vcon.example.com");
    /// let mut v = Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.set_amended(Reference::by_uuid(prior));
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""amended":{"uuid":"#), "the link is written");
    /// ```
    pub fn set_amended(&mut self, prior: Reference) {
        self.amended = Some(prior);
    }

    /// Sets `redacted`: this vCon is a redaction of kind `kind` of the one
    /// `prior` names.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Reference, Vcon};
    ///
    /// let prior = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let uuid = vcon::write::uuid_v8(1, 0, "vcon.example.com");
    /// let mut v = Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.set_redacted("PII Redaction", Reference::by_uuid(prior));
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""type":"PII Redaction""#), "the redaction is written");
    /// ```
    pub fn set_redacted(&mut self, kind: impl Into<String>, prior: Reference) {
        self.redacted = Some(Redacted {
            kind: kind.into(),
            prior: Some(prior),
        });
    }

    /// Sets `redacted` with only its kind: the less redacted version is not
    /// available or does not exist (core-04 4.1.8, the Redacted Object).
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// v.set_redacted_without_prior("PII Redaction");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""redacted":{"type":"PII Redaction"}"#), "only its type");
    /// ```
    pub fn set_redacted_without_prior(&mut self, kind: impl Into<String>) {
        self.redacted = Some(Redacted {
            kind: kind.into(),
            prior: None,
        });
    }

    /// Adds a party; other objects refer to it by the index returned.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let first = v.add_party(vcon::write::Party::new().name("Alice"));
    /// assert_eq!(first.get(), 0, "the first party");
    /// ```
    pub fn add_party(&mut self, party: Party) -> PartyIndex {
        self.parties.push(Some(party));
        PartyIndex(self.parties.len() - 1)
    }

    /// Adds a dialog; other objects refer to it by the index returned.
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let first = v.add_dialog(vcon::write::Dialog::transfer());
    /// assert_eq!(first.get(), 0, "the first dialog");
    /// ```
    pub fn add_dialog(&mut self, dialog: Dialog) -> DialogIndex {
        self.dialog.push(dialog);
        DialogIndex(self.dialog.len() - 1)
    }

    /// Adds an attachment; other objects refer to it by the index returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Attachment, Content, DialogIndex, PartyIndex};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let start = "2026-10-27T00:00:00Z".parse().expect("RFC 3339");
    /// let first = v.add_attachment(Attachment::new(start, PartyIndex::new(0), DialogIndex::new(0), Content::Absent));
    /// assert_eq!(first.get(), 0, "the first attachment");
    /// ```
    pub fn add_attachment(&mut self, attachment: Attachment) -> AttachmentIndex {
        self.attachments.push(Some(attachment));
        AttachmentIndex(self.attachments.len() - 1)
    }

    /// Adds an analysis; [`Vcon::remove_analysis`] and
    /// [`Vcon::replace_analysis`] refer to it by the index returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, Content};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let first = v.add_analysis(Analysis::new("summary", "Example", Content::Absent));
    /// assert_eq!(first.get(), 0, "the first analysis");
    /// ```
    pub fn add_analysis(&mut self, analysis: Analysis) -> AnalysisIndex {
        self.analysis.push(Some(analysis));
        AnalysisIndex(self.analysis.len() - 1)
    }

    /// Puts `party` in place of party `i`, as a redaction does when it
    /// copies a party partly redacted.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no party `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Party, PartyIndex};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let alice = v.add_party(Party::new().tel("+15555550100").name("Alice"));
    /// v.replace_party(alice, Party::new().name("Alice")).expect("party 0 exists");
    /// assert!(v.replace_party(PartyIndex::new(5), Party::new()).is_err(), "no party 5");
    /// ```
    pub fn replace_party(&mut self, i: PartyIndex, party: Party) -> Result<(), WriteError> {
        *slot(&mut self.parties, i.0, "parties")? = Some(party);
        Ok(())
    }

    /// Puts `dialog` in place of dialog `i`.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no dialog `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, DialogIndex, Disposition};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let call = v.add_dialog(Dialog::incomplete(Disposition::Busy));
    /// v.replace_dialog(call, Dialog::incomplete(Disposition::NoAnswer)).expect("dialog 0 exists");
    /// assert!(v.replace_dialog(DialogIndex::new(5), Dialog::transfer()).is_err(), "no dialog 5");
    /// ```
    pub fn replace_dialog(&mut self, i: DialogIndex, dialog: Dialog) -> Result<(), WriteError> {
        *slot(&mut self.dialog, i.0, "dialog")? = dialog;
        Ok(())
    }

    /// Puts `attachment` in place of attachment `i`.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no attachment `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Attachment, AttachmentIndex, Content, DialogIndex, PartyIndex};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let start: vcon::write::Date = "2026-10-27T00:00:00Z".parse().expect("RFC 3339");
    /// let make = || Attachment::new(start.clone(), PartyIndex::new(0), DialogIndex::new(0), Content::Absent);
    /// let first = v.add_attachment(make());
    /// v.replace_attachment(first, make().filename("a.pdf")).expect("attachment 0 exists");
    /// assert!(v.replace_attachment(AttachmentIndex::new(5), make()).is_err(), "no attachment 5");
    /// ```
    pub fn replace_attachment(
        &mut self,
        i: AttachmentIndex,
        attachment: Attachment,
    ) -> Result<(), WriteError> {
        *slot(&mut self.attachments, i.0, "attachments")? = Some(attachment);
        Ok(())
    }

    /// Puts `analysis` in place of analysis `i`.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no analysis `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, AnalysisIndex, Content};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let first = v.add_analysis(Analysis::new("summary", "Example", Content::Absent));
    /// v.replace_analysis(first, Analysis::new("summary", "Other", Content::Absent)).expect("analysis 0 exists");
    /// let missing = Analysis::new("summary", "Example", Content::Absent);
    /// assert!(v.replace_analysis(AnalysisIndex::new(5), missing).is_err(), "no analysis 5");
    /// ```
    pub fn replace_analysis(
        &mut self,
        i: AnalysisIndex,
        analysis: Analysis,
    ) -> Result<(), WriteError> {
        *slot(&mut self.analysis, i.0, "analysis")? = Some(analysis);
        Ok(())
    }

    /// Removes party `i`, leaving `{}` so no later index moves.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no party `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Party, PartyIndex};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let alice = v.add_party(Party::new().name("Alice"));
    /// v.remove_party(alice).expect("party 0 exists");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""parties":[{}]"#), "an empty object in its place");
    /// assert!(v.remove_party(PartyIndex::new(5)).is_err(), "no party 5");
    /// ```
    pub fn remove_party(&mut self, i: PartyIndex) -> Result<(), WriteError> {
        *slot(&mut self.parties, i.0, "parties")? = None;
        Ok(())
    }

    /// Removes dialog `i`, leaving a placeholder of its own type (and its
    /// disposition, for an incomplete one) so no later index moves.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no dialog `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Dialog, DialogIndex, Disposition};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let call = v.add_dialog(Dialog::incomplete(Disposition::Busy).application("Example"));
    /// v.remove_dialog(call).expect("dialog 0 exists");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""dialog":[{"type":"incomplete","disposition":"busy"}]"#), "a placeholder");
    /// assert!(v.remove_dialog(DialogIndex::new(5)).is_err(), "no dialog 5");
    /// ```
    pub fn remove_dialog(&mut self, i: DialogIndex) -> Result<(), WriteError> {
        let entry = slot(&mut self.dialog, i.0, "dialog")?;
        let mut placeholder = Dialog::new(entry.kind);
        if entry.kind == DialogType::Incomplete {
            placeholder.disposition = entry.disposition;
        }
        placeholder.removed = true;
        *entry = placeholder;
        Ok(())
    }

    /// Removes attachment `i`, leaving `{}`.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no attachment `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Attachment, AttachmentIndex, Content, DialogIndex, PartyIndex};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let start = "2026-10-27T00:00:00Z".parse().expect("RFC 3339");
    /// let first = v.add_attachment(Attachment::new(start, PartyIndex::new(0), DialogIndex::new(0), Content::Absent));
    /// v.remove_attachment(first).expect("attachment 0 exists");
    /// assert!(v.remove_attachment(AttachmentIndex::new(5)).is_err(), "no attachment 5");
    /// ```
    pub fn remove_attachment(&mut self, i: AttachmentIndex) -> Result<(), WriteError> {
        *slot(&mut self.attachments, i.0, "attachments")? = None;
        Ok(())
    }

    /// Removes analysis `i`, leaving `{}`.
    ///
    /// # Errors
    ///
    /// [`WriteError::UnknownIndex`] if there is no analysis `i`.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Analysis, AnalysisIndex, Content};
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let first = v.add_analysis(Analysis::new("summary", "Example", Content::Absent));
    /// v.remove_analysis(first).expect("analysis 0 exists");
    /// let json = String::from_utf8(v.to_json().expect("valid")).expect("UTF-8");
    /// assert!(json.contains(r#""analysis":[{}]"#), "an empty object in its place");
    /// assert!(v.remove_analysis(AnalysisIndex::new(5)).is_err(), "no analysis 5");
    /// ```
    pub fn remove_analysis(&mut self, i: AnalysisIndex) -> Result<(), WriteError> {
        *slot(&mut self.analysis, i.0, "analysis")? = None;
        Ok(())
    }

    /// A new vCon instance version that amends `prior` (core-04 4.1.9): a
    /// deep copy of it under `uuid`, with `amended` naming it and no
    /// `redacted` link.
    ///
    /// # Errors
    ///
    /// [`WriteError::SameUuid`] if `uuid` is the prior vCon's own.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Reference, Vcon};
    ///
    /// let first = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let prior = Vcon::new(first, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let next = vcon::write::uuid_v8(1, 0, "vcon.example.com");
    /// let amended = Vcon::amend(&prior, next, Reference::by_uuid(first)).expect("a new uuid");
    /// assert_eq!(amended.uuid(), next, "the new uuid");
    /// assert!(Vcon::amend(&prior, first, Reference::by_uuid(first)).is_err(), "the same uuid");
    /// ```
    pub fn amend(prior: &Vcon, uuid: Uuid, prior_ref: Reference) -> Result<Vcon, WriteError> {
        let mut new = prior.copy_as(uuid)?;
        new.amended = Some(prior_ref);
        Ok(new)
    }

    /// A new vCon instance version that redacts `prior` (core-04 4.1.8): a
    /// deep copy of it under `uuid`, with `redacted` of type `kind` naming
    /// it and no `amended` link. The caller then removes or replaces what
    /// is redacted.
    ///
    /// # Errors
    ///
    /// [`WriteError::SameUuid`] if `uuid` is the prior vCon's own.
    ///
    /// # Examples
    ///
    /// ```
    /// use vcon::write::{Party, PartyIndex, Reference, Vcon};
    ///
    /// let first = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let mut prior = Vcon::new(first, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// prior.add_party(Party::new().tel("+15555550100").name("Alice"));
    /// let next = vcon::write::uuid_v8(1, 0, "vcon.example.com");
    /// let mut redacted = Vcon::redact(&prior, next, "PII Redaction", Reference::by_uuid(first))
    ///     .expect("a new uuid");
    /// redacted.replace_party(PartyIndex::new(0), Party::new().name("Alice")).expect("party 0");
    /// assert_eq!(redacted.uuid(), next, "the new uuid");
    /// ```
    pub fn redact(
        prior: &Vcon,
        uuid: Uuid,
        kind: &str,
        prior_ref: Reference,
    ) -> Result<Vcon, WriteError> {
        let mut new = prior.copy_as(uuid)?;
        new.redacted = Some(Redacted {
            kind: kind.to_owned(),
            prior: Some(prior_ref),
        });
        Ok(new)
    }

    /// The unsigned form: compact JSON, core members in the draft's order,
    /// then extension members in the order they were set. The same model
    /// is always written as the same bytes.
    ///
    /// # Errors
    ///
    /// The [`WriteError`] that names the first rule the vCon breaks, with
    /// the JSON path of the object at fault: an index to nothing, a member
    /// Table 1 forbids or one it requires, content that cannot be written
    /// there, a bad reference, both links, a link to the vCon's own uuid,
    /// or output deeper than [`crate::read::MAX_JSON_DEPTH`].
    ///
    /// # Examples
    ///
    /// ```
    /// let uuid: uuid::Uuid = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46".parse().expect("a uuid");
    /// let v = vcon::write::Vcon::new(uuid, "2022-06-21T13:53:00-04:00".parse().expect("RFC 3339"));
    /// assert_eq!(
    ///     v.to_json().expect("valid"),
    ///     br#"{"uuid":"01a07da8-c2bb-83e5-b9a2-279e0d16bc46","created_at":"2022-06-21T13:53:00-04:00","parties":[],"dialog":[],"analysis":[],"attachments":[]}"#,
    ///     "the smallest vCon"
    /// );
    /// ```
    pub fn to_json(&self) -> Result<Vec<u8>, WriteError> {
        emit::render(&self.node()?)
    }

    /// The unsigned form, gzipped. The gzip header records no time and no
    /// name, so the same model is always written as the same bytes.
    ///
    /// # Errors
    ///
    /// The errors of [`Vcon::to_json`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    ///
    /// let uuid = vcon::write::uuid_v8(0, 0, "vcon.example.com");
    /// let v = vcon::write::Vcon::new(uuid, "2026-10-27T00:00:00Z".parse().expect("RFC 3339"));
    /// let gz = v.to_gzip().expect("valid");
    /// let limits = vcon::read::Limits::new(1 << 20);
    /// let found = vcon::read::inspect(&gz, None, &limits, &BTreeSet::new()).expect("a vCon");
    /// assert!(found.gzip, "read back as gzip");
    /// ```
    pub fn to_gzip(&self) -> Result<Vec<u8>, WriteError> {
        Ok(gzip(&self.to_json()?))
    }

    /// A deep copy under a new uuid, with neither link, for amend and
    /// redact.
    fn copy_as(&self, uuid: Uuid) -> Result<Vcon, WriteError> {
        if uuid == self.uuid {
            return Err(WriteError::SameUuid);
        }
        let mut new = self.clone();
        new.uuid = uuid;
        new.amended = None;
        new.redacted = None;
        Ok(new)
    }

    /// The vCon as JSON, every rule checked.
    fn node(&self) -> Result<Node, WriteError> {
        let counts = Counts {
            parties: self.parties.len(),
            dialog: self.dialog.len(),
            attachments: self.attachments.len(),
        };
        let redacted = self.redacted.is_some();
        let mut top = Obj::default();
        top.put("uuid", Node::str(&self.uuid.hyphenated().to_string()));
        if !self.extensions.is_empty() {
            top.put("extensions", strings(&self.extensions));
        }
        if !self.critical.is_empty() {
            top.put("critical", strings(&self.critical));
        }
        top.put("created_at", Node::str(self.created_at.as_str()));
        if let Some(date) = &self.updated_at {
            top.put("updated_at", Node::str(date.as_str()));
        }
        top.put_str("subject", &self.subject);
        if self.redacted.is_some() && self.amended.is_some() {
            return Err(WriteError::BothLinks);
        }
        if let Some(r) = &self.redacted {
            let mut obj = Obj::default();
            if let Some(prior) = &r.prior {
                self.check_reference(prior)?;
                if let Some(uuid) = prior.uuid {
                    obj.put("uuid", Node::str(&uuid.hyphenated().to_string()));
                }
                obj.put("type", Node::str(&r.kind));
                reference_url(prior, &mut obj);
            } else {
                obj.put("type", Node::str(&r.kind));
            }
            top.put("redacted", obj.done());
        }
        if let Some(prior) = &self.amended {
            self.check_reference(prior)?;
            let mut obj = Obj::default();
            if let Some(uuid) = prior.uuid {
                obj.put("uuid", Node::str(&uuid.hyphenated().to_string()));
            }
            reference_url(prior, &mut obj);
            top.put("amended", obj.done());
        }
        let parties = self
            .parties
            .iter()
            .map(|p| p.as_ref().map_or(Node::Obj(Vec::new()), Party::node))
            .collect();
        top.put("parties", Node::Arr(parties));
        let mut dialog = Vec::new();
        for (i, d) in self.dialog.iter().enumerate() {
            dialog.push(d.node(&format!("/dialog/{i}"), redacted, &counts)?);
        }
        top.put("dialog", Node::Arr(dialog));
        let mut analysis = Vec::new();
        for (i, a) in self.analysis.iter().enumerate() {
            analysis.push(match a {
                Some(a) => a.node(&format!("/analysis/{i}"), redacted, &counts)?,
                None => Node::Obj(Vec::new()),
            });
        }
        top.put("analysis", Node::Arr(analysis));
        let mut attachments = Vec::new();
        for (i, a) in self.attachments.iter().enumerate() {
            attachments.push(match a {
                Some(a) => a.node(&format!("/attachments/{i}"), redacted, &counts)?,
                None => Node::Obj(Vec::new()),
            });
        }
        top.put("attachments", Node::Arr(attachments));
        self.members.put_into(&mut top);
        Ok(top.done())
    }

    /// A reference's url is https and has a hash, and its uuid is not this
    /// vCon's own.
    fn check_reference(&self, r: &Reference) -> Result<(), WriteError> {
        if let Some(url) = &r.url
            && (!is_https(url) || r.hashes.is_empty())
        {
            return Err(WriteError::BadReference);
        }
        if r.uuid == Some(self.uuid) {
            return Err(WriteError::SameUuid);
        }
        Ok(())
    }
}

/// `bytes` gzipped. flate2's default header records no time and no name.
/// flate2 reports an I/O error from its writer, and a `Vec` never returns
/// one, so neither `expect` can fail.
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .expect("writing to a Vec does not fail");
    encoder.finish().expect("writing to a Vec does not fail")
}

/// The entry at `i` of `list`, or the refusal naming it.
fn slot<'a, T>(list: &'a mut [T], i: usize, name: &str) -> Result<&'a mut T, WriteError> {
    list.get_mut(i).ok_or_else(|| WriteError::UnknownIndex {
        path: format!("/{name}/{i}"),
    })
}

/// How many entries each array has, for checking indices.
struct Counts {
    parties: usize,
    dialog: usize,
    attachments: usize,
}

fn index(i: usize, count: usize, path: String) -> Result<Node, WriteError> {
    if i >= count {
        return Err(WriteError::UnknownIndex { path });
    }
    Ok(Node::Raw(i.to_string()))
}

fn strings(list: &[String]) -> Node {
    Node::Arr(list.iter().map(|s| Node::str(s)).collect())
}

/// One hash is written as a string, several as an array (core-04 2.4.2).
fn hashes(list: &[ContentHash]) -> Node {
    match list {
        [one] => Node::str(one.as_str()),
        many => Node::Arr(many.iter().map(|h| Node::str(h.as_str())).collect()),
    }
}

fn reference_url(r: &Reference, obj: &mut Obj) {
    if let Some(url) = &r.url {
        obj.put("url", Node::str(url));
        obj.put("content_hash", hashes(&r.hashes));
    }
}

/// An https URL with a host (core-04 2.4.1); the scheme in any case.
fn is_https(url: &str) -> bool {
    let Some(scheme) = url.get(..8) else {
        return false;
    };
    let host = url[8..].split(['/', '?', '#']).next().unwrap_or("");
    scheme.eq_ignore_ascii_case("https://") && !host.is_empty()
}

impl Content {
    /// The member Table 1 names for this content, for a refusal.
    fn member(&self) -> Option<&'static str> {
        match self {
            Content::Inline(_) => Some("body"),
            Content::External { .. } => Some("url"),
            Content::HashOnly(_) => Some("content_hash"),
            Content::Absent => None,
        }
    }

    /// Checks the content of the object at `path` and writes it into `obj`.
    /// `absent_ok` says whether no content is allowed there.
    fn put_into(
        &self,
        path: &str,
        redacted: bool,
        absent_ok: bool,
        obj: &mut Obj,
    ) -> Result<(), WriteError> {
        let bad = || WriteError::BadContent {
            path: path.to_owned(),
        };
        match self {
            Content::Inline(body) => {
                let (node, encoding) = match body {
                    Body::Bytes(bytes) => (Node::Str(URL_SAFE_NO_PAD.encode(bytes)), "base64url"),
                    Body::Text(text) => (Node::str(text), "none"),
                    Body::Json(value) => (Node::Val(value.clone()), "json"),
                };
                obj.put("body", node);
                obj.put("encoding", Node::str(encoding));
            }
            Content::External { url, hashes: list } => {
                if !is_https(url) || list.is_empty() {
                    return Err(bad());
                }
                obj.put("url", Node::str(url));
                obj.put("content_hash", hashes(list));
            }
            Content::HashOnly(list) => {
                if !redacted || list.is_empty() {
                    return Err(bad());
                }
                obj.put("content_hash", hashes(list));
            }
            Content::Absent => {
                if !(redacted || absent_ok) {
                    return Err(bad());
                }
            }
        }
        Ok(())
    }
}

impl Party {
    fn node(&self) -> Node {
        let mut obj = Obj::default();
        obj.put_str("tel", &self.tel);
        obj.put_str("sip", &self.sip);
        obj.put_str("stir", &self.stir);
        obj.put_str("mailto", &self.mailto);
        obj.put_str("name", &self.name);
        obj.put_str("did", &self.did);
        obj.put_str("validation", &self.validation);
        obj.put_str("gmlpos", &self.gmlpos);
        if let Some(address) = &self.civicaddress {
            let mut civic = Obj::default();
            for name in CIVIC_ORDER {
                if let Some((_, value)) = address.members.iter().find(|(n, _)| n == name) {
                    civic.put(name, Node::str(value));
                }
            }
            obj.put("civicaddress", civic.done());
        }
        obj.put_str("uuid", &self.uuid);
        obj.put_str("type", &self.kind);
        obj.put_str("org", &self.org);
        obj.put_str("dept", &self.dept);
        self.members.put_into(&mut obj);
        obj.done()
    }
}

impl Duration {
    fn node(&self) -> Node {
        match self.0 {
            Seconds::Whole(n) => Node::Raw(n.to_string()),
            Seconds::Fraction(f) => Node::Raw(
                serde_json::Number::from_f64(f).map_or_else(String::new, |n| n.to_string()),
            ),
        }
    }
}

impl SessionId {
    fn node(&self) -> Node {
        let mut obj = Obj::default();
        if let Some((local, remote)) = &self.0 {
            obj.put("local", Node::str(local));
            obj.put("remote", Node::str(remote));
        }
        obj.done()
    }
}

impl Event {
    fn put_into(&self, obj: &mut Obj) {
        let (event, button) = match self {
            Event::Join => ("join", None),
            Event::Drop => ("drop", None),
            Event::Hold => ("hold", None),
            Event::Unhold => ("unhold", None),
            Event::Mute => ("mute", None),
            Event::Unmute => ("unmute", None),
            Event::Keydown(b) => ("keydown", Some(b)),
            Event::Keyup(b) => ("keyup", Some(b)),
        };
        obj.put("event", Node::str(event));
        if let Some(b) = button {
            obj.put("button", Node::str(b));
        }
    }
}

/// Table 1 of core-04 (in 4.3.1): the types for which each member is X.
fn forbidden(kind: DialogType, name: &str) -> bool {
    use DialogType::{Incomplete, Recording, RecordingSet, Text, Transfer};
    let x: &[DialogType] = match name {
        "parties" | "originator" | "session_id" | "party_history" => &[Transfer],
        "recordings" => &[Recording, Text, Transfer, Incomplete],
        "recording_set" => &[RecordingSet, Text, Transfer, Incomplete],
        "mediatype" | "filename" | "body" | "url" | "content_hash" | "message_id" => {
            &[RecordingSet, Transfer, Incomplete]
        }
        "transferee" | "transferor" | "transfer_target" | "original" | "consultation"
        | "target_dialog" => &[Recording, RecordingSet, Text, Incomplete],
        _ => &[],
    };
    x.contains(&kind)
}

impl Dialog {
    /// Nothing but `type`: a placeholder by its shape.
    fn is_bare(&self) -> bool {
        *self == Dialog::new(self.kind)
    }

    /// The members this dialog sets, in the draft's order, for Table 1.
    fn members_set(&self) -> Vec<&'static str> {
        let mut set = Vec::new();
        let mut add = |name, present: bool| {
            if present {
                set.push(name);
            }
        };
        add("parties", self.parties.is_some());
        add("originator", self.originator.is_some());
        add("recordings", self.recordings.is_some());
        add("recording_set", self.recording_set.is_some());
        add("mediatype", self.mediatype.is_some());
        add("filename", self.filename.is_some());
        if let Some(name) = self.content.member() {
            add(name, true);
        }
        add("session_id", self.session_id.is_some());
        add("party_history", self.party_history.is_some());
        add("transferee", self.transferee.is_some());
        add("transferor", self.transferor.is_some());
        add("transfer_target", self.transfer_target.is_some());
        add("original", self.original.is_some());
        add("consultation", self.consultation.is_some());
        add("target_dialog", self.target_dialog.is_some());
        add("message_id", self.message_id.is_some());
        set
    }

    fn node(&self, path: &str, redacted: bool, counts: &Counts) -> Result<Node, WriteError> {
        let missing = |name| WriteError::MissingParameter {
            path: path.to_owned(),
            name,
        };
        for name in self.members_set() {
            if forbidden(self.kind, name) {
                return Err(WriteError::ForbiddenParameter {
                    path: path.to_owned(),
                    name,
                });
            }
        }
        // A removed dialog keeps only its type (and disposition), as
        // 4.1.8's placeholder; Table 1's M does not apply to it.
        if !self.removed {
            if self.kind == DialogType::RecordingSet && self.recordings.is_none() {
                return Err(missing("recordings"));
            }
            if self.kind == DialogType::Incomplete && self.disposition.is_none() {
                return Err(missing("disposition"));
            }
            if matches!(self.kind, DialogType::Recording | DialogType::Text)
                && matches!(self.content, Content::Inline(_))
                && self.mediatype.is_none()
            {
                return Err(missing("mediatype"));
            }
        }
        let parties = |i: PartyIndex, at: String| index(i.0, counts.parties, at);
        let dialogs = |i: DialogIndex, at: String| index(i.0, counts.dialog, at);

        let mut obj = Obj::default();
        obj.put("type", Node::str(self.kind.as_str()));
        if let Some(date) = &self.start {
            obj.put("start", Node::str(date.as_str()));
        }
        if let Some(d) = &self.duration {
            obj.put("duration", d.node());
        }
        if let Some(p) = &self.parties {
            let at = format!("{path}/parties");
            let node = match p {
                Parties::One(i) => parties(*i, at)?,
                Parties::Channels(channels) => {
                    let mut list = Vec::new();
                    for (j, c) in channels.iter().enumerate() {
                        list.push(match c {
                            Channel::One(i) => parties(*i, format!("{at}/{j}"))?,
                            Channel::Mix(mix) => {
                                let mut inner = Vec::new();
                                for (k, i) in mix.iter().enumerate() {
                                    inner.push(parties(*i, format!("{at}/{j}/{k}"))?);
                                }
                                Node::Arr(inner)
                            }
                            Channel::Unrecorded => Node::Raw("null".into()),
                        });
                    }
                    Node::Arr(list)
                }
            };
            obj.put("parties", node);
        }
        if let Some(i) = self.originator {
            obj.put("originator", parties(i, format!("{path}/originator"))?);
        }
        if let Some(list) = &self.recordings {
            let mut out = Vec::new();
            for (j, i) in list.iter().enumerate() {
                out.push(dialogs(*i, format!("{path}/recordings/{j}"))?);
            }
            obj.put("recordings", Node::Arr(out));
        }
        if let Some(i) = self.recording_set {
            obj.put(
                "recording_set",
                dialogs(i, format!("{path}/recording_set"))?,
            );
        }
        obj.put_str("mediatype", &self.mediatype);
        obj.put_str("filename", &self.filename);
        // No content: in a redacted vCon, a removed dialog, a recording
        // placeholder, and the types that have no content at all.
        let absent_ok = self.removed
            || (self.kind == DialogType::Recording && self.is_bare())
            || matches!(
                self.kind,
                DialogType::RecordingSet | DialogType::Transfer | DialogType::Incomplete
            );
        self.content.put_into(path, redacted, absent_ok, &mut obj)?;
        if let Some(d) = self.disposition {
            obj.put("disposition", Node::str(d.as_str()));
        }
        if let Some(s) = &self.session_id {
            let node = match s {
                SessionIds::One(id) => id.node(),
                SessionIds::Channels(list) => Node::Arr(
                    list.iter()
                        .map(|c| match c {
                            SessionChannel::One(id) => id.node(),
                            SessionChannel::Mix(ids) => {
                                Node::Arr(ids.iter().map(SessionId::node).collect())
                            }
                        })
                        .collect(),
                ),
            };
            obj.put("session_id", node);
        }
        if let Some(history) = &self.party_history {
            let mut out = Vec::new();
            for (j, h) in history.iter().enumerate() {
                let mut entry = Obj::default();
                entry.put(
                    "party",
                    parties(h.party, format!("{path}/party_history/{j}/party"))?,
                );
                entry.put("time", Node::str(h.time.as_str()));
                h.event.put_into(&mut entry);
                out.push(entry.done());
            }
            obj.put("party_history", Node::Arr(out));
        }
        for (name, value) in [
            ("transferee", self.transferee),
            ("transferor", self.transferor),
            ("transfer_target", self.transfer_target),
        ] {
            if let Some(i) = value {
                obj.put(name, parties(i, format!("{path}/{name}"))?);
            }
        }
        for (name, value) in [
            ("original", self.original),
            ("consultation", self.consultation),
            ("target_dialog", self.target_dialog),
        ] {
            if let Some(i) = value {
                obj.put(name, dialogs(i, format!("{path}/{name}"))?);
            }
        }
        obj.put_str("application", &self.application);
        obj.put_str("message_id", &self.message_id);
        self.members.put_into(&mut obj);
        Ok(obj.done())
    }
}

impl Attachment {
    fn node(&self, path: &str, redacted: bool, counts: &Counts) -> Result<Node, WriteError> {
        // core-04 4.4.5: inline attachment files have a mediatype.
        if matches!(self.content, Content::Inline(_)) && self.mediatype.is_none() {
            return Err(WriteError::MissingParameter {
                path: path.to_owned(),
                name: "mediatype",
            });
        }
        let mut obj = Obj::default();
        obj.put_str("purpose", &self.purpose);
        obj.put("start", Node::str(self.start.as_str()));
        obj.put(
            "party",
            index(self.party.0, counts.parties, format!("{path}/party"))?,
        );
        obj.put(
            "dialog",
            index(self.dialog.0, counts.dialog, format!("{path}/dialog"))?,
        );
        obj.put_str("mediatype", &self.mediatype);
        obj.put_str("filename", &self.filename);
        self.content.put_into(path, redacted, false, &mut obj)?;
        self.members.put_into(&mut obj);
        Ok(obj.done())
    }
}

/// One index or an array of them, each checked against `count`.
fn one_or_many<T: Copy>(
    value: &OneOrMany<T>,
    get: fn(T) -> usize,
    count: usize,
    at: &str,
) -> Result<Node, WriteError> {
    match value {
        OneOrMany::One(i) => index(get(*i), count, at.to_owned()),
        OneOrMany::Many(list) => {
            let mut out = Vec::new();
            for (j, i) in list.iter().enumerate() {
                out.push(index(get(*i), count, format!("{at}/{j}"))?);
            }
            Ok(Node::Arr(out))
        }
    }
}

impl Analysis {
    fn node(&self, path: &str, redacted: bool, counts: &Counts) -> Result<Node, WriteError> {
        let mut obj = Obj::default();
        obj.put("type", Node::str(&self.kind));
        if let Some(d) = &self.dialog {
            obj.put(
                "dialog",
                one_or_many(
                    d,
                    DialogIndex::get,
                    counts.dialog,
                    &format!("{path}/dialog"),
                )?,
            );
        }
        if let Some(a) = &self.attachment {
            obj.put(
                "attachment",
                one_or_many(
                    a,
                    AttachmentIndex::get,
                    counts.attachments,
                    &format!("{path}/attachment"),
                )?,
            );
        }
        obj.put_str("mediatype", &self.mediatype);
        obj.put_str("filename", &self.filename);
        obj.put("vendor", Node::str(&self.vendor));
        obj.put_str("product", &self.product);
        obj.put_str("schema", &self.schema);
        self.content.put_into(path, redacted, false, &mut obj)?;
        self.members.put_into(&mut obj);
        Ok(obj.done())
    }
}

/// The version 8 uuid core-04 4.1.2 recommends: the version 7 layout, 48
/// bits of Unix milliseconds then 12 bits of `rand_a`, with `rand_b` the
/// high 62 bits of the SHA-1 hash of `fqhn`. Only the low 48 bits of
/// `unix_ms` and the low 12 bits of `rand_a` are used; the caller chooses
/// the time and the random bits.
///
/// # Examples
///
/// ```
/// let u = vcon::write::uuid_v8(0x01a0_7da8_c2bb, 0x3e5, "vcon.example.com");
/// assert_eq!(u.to_string(), "01a07da8-c2bb-83e5-b7dd-b0a177be8d34", "a pinned vector");
/// assert_eq!(u.get_version_num(), 8, "version 8");
/// ```
#[must_use]
pub fn uuid_v8(unix_ms: u64, rand_a: u16, fqhn: &str) -> Uuid {
    let digest = sha1_smol::Sha1::from(fqhn).digest().bytes();
    let mut high = [0u8; 8];
    high.copy_from_slice(&digest[..8]);
    let rand_b = u64::from_be_bytes(high) >> 2;
    let value = (u128::from(unix_ms & 0xffff_ffff_ffff) << 80)
        | (0x8 << 76)
        | (u128::from(rand_a & 0xfff) << 64)
        | (0b10 << 62)
        | u128::from(rand_b);
    Uuid::from_u128(value)
}

/// Why a vCon cannot be written.
///
/// # Examples
///
/// ```
/// use vcon::write::{Dialog, DialogType, WriteError};
///
/// let refused = Dialog::try_placeholder(DialogType::Text);
/// assert_eq!(refused, Err(WriteError::NoPlaceholder(DialogType::Text)), "no text placeholder");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[must_use]
pub enum WriteError {
    /// An index refers to no entry; `path` is the JSON path of the index.
    UnknownIndex {
        /// The JSON path, such as `/dialog/0/parties/1`.
        path: String,
    },
    /// The object at `path` lacks a member its type requires.
    MissingParameter {
        /// The object's JSON path.
        path: String,
        /// The member's name.
        name: &'static str,
    },
    /// The object at `path` has a member its type does not allow.
    ForbiddenParameter {
        /// The object's JSON path.
        path: String,
        /// The member's name.
        name: &'static str,
    },
    /// The content of the object at `path` cannot be written there: a URL
    /// that is not https, no hash, or no content outside a redaction.
    BadContent {
        /// The object's JSON path.
        path: String,
    },
    /// A content hash's algorithm or digest is malformed.
    BadHash,
    /// A date is not RFC 3339 with an offset.
    BadDate,
    /// A duration is negative, not finite or too large.
    BadDuration,
    /// An `amended` or `redacted` reference is incomplete or its URL is not
    /// https.
    BadReference,
    /// Both `amended` and `redacted` are set.
    BothLinks,
    /// A new instance version has the uuid of the one it refers to.
    SameUuid,
    /// An extension member would use a name the draft reserves.
    CoreMember(String),
    /// A placeholder of this type is not made.
    NoPlaceholder(DialogType),
    /// The output would have more than [`crate::read::MAX_JSON_DEPTH`]
    /// arrays and objects open at once.
    TooDeep,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::UnknownIndex { path } => {
                write!(f, "the index at {path} refers to no entry")
            }
            WriteError::MissingParameter { path, name } => {
                write!(f, "{path} lacks {name}, which its type requires")
            }
            WriteError::ForbiddenParameter { path, name } => {
                write!(f, "{path} has {name}, which its type does not allow")
            }
            WriteError::BadContent { path } => {
                write!(f, "the content of {path} cannot be written there")
            }
            WriteError::BadHash => f.write_str("a content hash is malformed"),
            WriteError::BadDate => f.write_str("a date is not RFC 3339 with an offset"),
            WriteError::BadDuration => {
                f.write_str("a duration is not an unsigned number of seconds")
            }
            WriteError::BadReference => {
                f.write_str("a reference to a prior vCon is incomplete or its url is not https")
            }
            WriteError::BothLinks => f.write_str("both amended and redacted are set"),
            WriteError::SameUuid => f.write_str("a vCon refers to a vCon with its own uuid"),
            WriteError::CoreMember(name) => write!(f, "the member name {name} is reserved"),
            WriteError::NoPlaceholder(kind) => write!(f, "no placeholder is made of type {kind}"),
            WriteError::TooDeep => write!(
                f,
                "the output would nest more than {} arrays and objects",
                crate::read::MAX_JSON_DEPTH
            ),
        }
    }
}

impl std::error::Error for WriteError {}
