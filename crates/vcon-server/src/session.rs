//! What a request by session carries: the cookie, the CSRF token and the
//! origin it must come from.
//!
//! A session is a row in the registry (`crate::tenants`), found by the
//! SHA-256 of a random 256-bit id. The id travels in the cookie
//! [`COOKIE`] and nowhere else. The CSRF token is made from the id
//! (`csrf_token`), so `GET /session` can hand it to the page again while
//! the registry keeps only its hash; page script cannot read the cookie,
//! and the token does not show the id.

use std::fmt;

use aws_lc_rs::constant_time;
use aws_lc_rs::digest::{SHA256, digest};
use axum::http::header::{COOKIE as COOKIE_HEADER, ORIGIN};
use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// The session cookie's name. The `__Host-` prefix has a browser refuse
/// the cookie unless it is `Secure`, has `Path=/` and names no domain, so
/// no other host and no plain-HTTP page can set or read it.
pub const COOKIE: &str = "__Host-vcs_session";

/// The request header that carries the session's CSRF token.
pub const CSRF_HEADER: &str = "x-csrf-token";

/// The request header a browser sets to say where a fetch came from.
const FETCH_SITE: &str = "sec-fetch-site";

/// The bytes of a session id, and of the CSRF token made from it.
const SECRET_LEN: usize = 32;

/// The characters [`SECRET_LEN`] bytes take in base64url with no padding.
const SECRET_CHARS: usize = 43;

/// What the SHA-256 that makes a CSRF token starts with, so the token is
/// not the hash the registry finds the session by.
const CSRF_DOMAIN: &[u8] = b"vcon-server session csrf token\0";

/// The attributes of the cookie, set and cleared alike.
const ATTRIBUTES: &str = "Path=/; Secure; HttpOnly; SameSite=Strict";

/// The most authentication methods a session keeps, and the most
/// characters of one.
const AMR_MOST: usize = 8;
const AMR_LONGEST: usize = 32;

/// The longest host name of an origin, as DNS limits a name.
const HOST_LONGEST: usize = 253;

/// A listener's public origin (`public_origin`, `operator.public_origin`):
/// `https://` and a host, with a port when it is not 443, as a browser
/// writes it in the `Origin` header of a request from the listener's
/// pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin(String);

impl Origin {
    /// Reads an origin: `https://host` or `https://host:port`, the host in
    /// lower case, an IPv6 address in brackets, and nothing after it.
    #[must_use]
    pub fn parse(text: &str) -> Option<Origin> {
        let rest = text.strip_prefix("https://")?;
        let (host, port) = match rest.strip_prefix('[') {
            Some(inner) => {
                let (address, after) = inner.split_once(']')?;
                address.parse::<std::net::Ipv6Addr>().ok()?;
                let host = &rest[..address.len() + 2];
                match after {
                    "" => (host, None),
                    after => (host, Some(after.strip_prefix(':')?)),
                }
            }
            None => match rest.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (rest, None),
            },
        };
        let named = host.starts_with('[')
            || (!host.is_empty()
                && host.len() <= HOST_LONGEST
                && host.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
                }));
        let numbered = port.is_none_or(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        });
        (named && numbered).then(|| Origin(text.to_owned()))
    }

    /// The origin as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The origin's host, without the scheme and the port. A browser keeps
    /// a cookie by host alone, so two listeners on one host share one.
    #[must_use]
    pub fn host(&self) -> &str {
        let rest = &self.0["https://".len()..];
        if rest.starts_with('[') {
            let end = rest.find(']').map_or(rest.len(), |at| at + 1);
            &rest[..end]
        } else {
            rest.split_once(':').map_or(rest, |(host, _)| host)
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A new session id: 256 random bits in base64url.
pub(crate) fn new_secret() -> String {
    let mut secret = [0; SECRET_LEN];
    // AWS-LC's RAND_bytes aborts the process when it cannot get enough
    // randomness, so `fill` never returns its error.
    aws_lc_rs::rand::fill(&mut secret).expect("AWS-LC aborts instead of failing");
    URL_SAFE_NO_PAD.encode(secret)
}

/// Whether `secret` is written as a session id or a CSRF token is: 43
/// characters of the base64url alphabet.
pub(crate) fn well_formed(secret: &str) -> bool {
    secret.len() == SECRET_CHARS
        && secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The SHA-256 of a session id or a CSRF token, as the registry keeps it.
pub(crate) fn hash(text: &str) -> [u8; 32] {
    digest(&SHA256, text.as_bytes())
        .as_ref()
        .try_into()
        .expect("SHA-256 is 32 bytes")
}

/// The CSRF token of the session with this id: a SHA-256 over a fixed
/// start and the id, in base64url. Whoever holds the id can make it; the
/// token gives no way back to the id.
pub(crate) fn csrf_token(id: &str) -> String {
    let made = digest(&SHA256, &[CSRF_DOMAIN, id.as_bytes()].concat());
    URL_SAFE_NO_PAD.encode(made.as_ref())
}

/// Whether two byte strings are equal, in time that does not depend on
/// where they differ.
pub(crate) fn same(one: &[u8], other: &[u8]) -> bool {
    constant_time::verify_slices_are_equal(one, other).is_ok()
}

/// The session cookie of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cookie<'a> {
    /// The request has none.
    Absent,
    /// The request has more than one, one whose value no session id has,
    /// or a `Cookie` header that is not text.
    Bad,
    /// The one cookie's value, written as a session id is.
    Id(&'a str),
}

/// Reads the session cookie from a request's `Cookie` headers. A name is
/// matched whole and in its case.
pub(crate) fn cookie(headers: &HeaderMap) -> Cookie<'_> {
    let mut found = Cookie::Absent;
    for header in headers.get_all(COOKIE_HEADER) {
        let Ok(text) = header.to_str() else {
            return Cookie::Bad;
        };
        for pair in text.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name != COOKIE {
                continue;
            }
            if found != Cookie::Absent || !well_formed(value) {
                return Cookie::Bad;
            }
            found = Cookie::Id(value);
        }
    }
    found
}

/// The `Set-Cookie` value that gives a browser the session with this id.
/// It names no lifetime: the browser drops it when it closes, and the
/// registry holds the limits. A sign-in method answers with it once
/// `Tenants::start_session` has given it the id; none is built yet.
///
/// # Panics
///
/// If `id` holds a character a header value cannot; a session id never
/// does.
#[must_use]
pub fn set_cookie(id: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{COOKIE}={id}; {ATTRIBUTES}"))
        .expect("a session id is visible ASCII")
}

/// The `Set-Cookie` value that removes the session cookie from a browser.
pub(crate) fn clear_cookie() -> HeaderValue {
    HeaderValue::from_str(&format!("{COOKIE}=; {ATTRIBUTES}; Max-Age=0"))
        .expect("the cookie's name and attributes are visible ASCII")
}

/// The one value of a header, or None for no header, two, or one that is
/// not text.
fn one<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut all = headers.get_all(name).iter();
    match (all.next(), all.next()) {
        (Some(value), None) => value.to_str().ok(),
        _ => None,
    }
}

/// Whether a request by session comes from the listener's own pages.
///
/// Every request but the hand-out needs one `Origin` header equal to the
/// listener's public origin. The hand-out (`GET /session`) is a GET a page
/// makes to its own origin, which a browser sends with no `Origin`: it
/// needs one `Sec-Fetch-Site: same-origin`, a header page script cannot
/// set, and an `Origin`, when there is one, equal as on any other request.
/// A listener with no public origin passes nothing.
pub(crate) fn from_origin(headers: &HeaderMap, origin: Option<&Origin>, hand_out: bool) -> bool {
    let Some(origin) = origin else {
        return false;
    };
    let equal = || one(headers, ORIGIN.as_str()) == Some(origin.as_str());
    if hand_out {
        one(headers, FETCH_SITE) == Some("same-origin")
            && (!headers.contains_key(ORIGIN) || equal())
    } else {
        equal()
    }
}

/// The authentication methods a sign-in showed (RFC 8176 names such as
/// `hwk` or `mfa`), as a session keeps them: joined with spaces, in the
/// order given. None unless there are 1 to 8, each 1 to 32 characters of
/// lower-case letters, digits, `-` and `_`, and no two alike.
pub(crate) fn amr_text(methods: &[&str]) -> Option<String> {
    let good = |method: &&str| {
        (1..=AMR_LONGEST).contains(&method.len())
            && method
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    };
    let distinct = methods
        .iter()
        .enumerate()
        .all(|(at, method)| !methods[..at].contains(method));
    ((1..=AMR_MOST).contains(&methods.len()) && methods.iter().all(good) && distinct)
        .then(|| methods.join(" "))
}

#[cfg(test)]
mod tests;
