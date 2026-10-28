//! Gzip detection and bounded inflation, for inspection only. The stored
//! body always stays exactly as received.

use std::borrow::Cow;
use std::io::Read;

use flate2::read::MultiGzDecoder;

use crate::read::Rejection;

/// The first two bytes of every gzip member (RFC 1952, section 2.3.1).
const MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Returns the body to inspect and whether it was gzip. A body is gzip when
/// its media type says so or it starts with the gzip magic number.
pub(crate) fn inflate<'a>(
    body: &'a [u8],
    content_type: Option<&str>,
    limit: u64,
) -> Result<(Cow<'a, [u8]>, bool), Rejection> {
    let typed_gzip = content_type.is_some_and(is_gzip_media_type);
    if !body.starts_with(&MAGIC) {
        return if typed_gzip {
            Err(Rejection::BadGzip)
        } else {
            Ok((Cow::Borrowed(body), false))
        };
    }
    read_bounded(MultiGzDecoder::new(body), limit).map(|inflated| (Cow::Owned(inflated), true))
}

/// Reads at most `limit` bytes, plus one to tell that the limit was passed,
/// so a small body can never make the reader do unbounded work.
fn read_bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>, Rejection> {
    let mut out = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|_| Rejection::BadGzip)?;
    if out.len() as u64 > limit {
        return Err(Rejection::TooLarge);
    }
    Ok(out)
}

/// Media type parameters are ignored, and type names are case-insensitive.
fn is_gzip_media_type(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim();
    essence.eq_ignore_ascii_case("application/vcon+gzip")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    const LIMIT: u64 = 1024;

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).expect("compress");
        e.finish().expect("finish")
    }

    #[test]
    fn plain_bodies_pass_through_unchanged() {
        for ct in [None, Some("application/vcon"), Some("application/json")] {
            let (out, gzip) = inflate(b"{}", ct, LIMIT).expect("plain");
            assert_eq!((&*out, gzip), (&b"{}"[..], false), "{ct:?}");
            assert!(matches!(out, Cow::Borrowed(_)), "no copy for {ct:?}");
        }
    }

    #[test]
    fn a_gzip_body_is_inflated_whatever_its_media_type() {
        let body = gz(b"{\"a\":1}");
        let cases = [
            None,
            Some("application/vcon"),
            Some("application/vcon+gzip"),
            Some("Application/VCON+GZIP"),
            Some("application/vcon+gzip; charset=utf-8"),
            Some(" application/vcon+gzip ;q=1"),
        ];
        for ct in cases {
            let (out, gzip) = inflate(&body, ct, LIMIT).expect("gzip");
            assert_eq!((&*out, gzip), (&b"{\"a\":1}"[..], true), "{ct:?}");
        }
    }

    #[test]
    fn several_gzip_members_are_read_as_one_stream() {
        let mut body = gz(b"{\"a\":");
        body.extend(gz(b"1}"));
        let (out, _) = inflate(&body, None, LIMIT).expect("two members");
        assert_eq!(&*out, b"{\"a\":1}");
    }

    #[test]
    fn broken_gzip_is_refused() {
        let good = gz(b"{\"a\":1}");
        let mut bad_crc = good.clone();
        let n = bad_crc.len();
        bad_crc[n - 8] ^= 0xff;
        let mut trailing = good.clone();
        trailing.extend(b"junk");
        let cases: &[(&str, Option<&str>, &[u8])] = &[
            (
                "gzip media type, plain body",
                Some("application/vcon+gzip"),
                b"{}",
            ),
            (
                "gzip media type, empty body",
                Some("application/vcon+gzip"),
                b"",
            ),
            ("magic number only", None, &[0x1f, 0x8b]),
            ("truncated", None, &good[..good.len() / 2]),
            ("bad checksum", None, &bad_crc),
            ("trailing bytes", None, &trailing),
        ];
        for (what, ct, body) in cases {
            assert_eq!(inflate(body, *ct, LIMIT), Err(Rejection::BadGzip), "{what}");
        }
    }

    #[test]
    fn inflation_stops_at_the_limit() {
        let exactly = gz(&vec![b' '; LIMIT as usize]);
        assert_eq!(
            inflate(&exactly, None, LIMIT).expect("at limit").0.len(),
            LIMIT as usize
        );
        let over = gz(&vec![b' '; LIMIT as usize + 1]);
        assert_eq!(inflate(&over, None, LIMIT), Err(Rejection::TooLarge));
    }

    #[test]
    fn the_typed_gzip_check_ignores_parameters_and_case() {
        for ct in [
            "Application/VCON+GZIP",
            "application/vcon+gzip; charset=utf-8",
            " application/vcon+gzip ;q=1",
        ] {
            assert_eq!(
                inflate(b"{}", Some(ct), LIMIT),
                Err(Rejection::BadGzip),
                "{ct}"
            );
        }
    }

    /// Endless zeros that count what was read, and fail well past the limit
    /// instead of running forever if the reader is unbounded.
    struct Endless<'a> {
        read: &'a mut u64,
        stop: u64,
    }

    impl Read for Endless<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if *self.read >= self.stop {
                return Err(std::io::Error::other("read far past the limit"));
            }
            buf.fill(0);
            *self.read += buf.len() as u64;
            Ok(buf.len())
        }
    }

    #[test]
    fn reading_stops_just_past_the_limit() {
        let mut read = 0;
        let endless = Endless {
            read: &mut read,
            stop: LIMIT * 64,
        };
        assert_eq!(read_bounded(endless, LIMIT), Err(Rejection::TooLarge));
        assert!(
            read <= LIMIT + 1,
            "read {read} bytes for a limit of {LIMIT}"
        );
    }

    #[test]
    fn a_gzip_bomb_is_refused() {
        // 64 MiB of zeros compresses to about 64 KiB.
        let bomb = gz(&vec![0u8; 64 << 20]);
        assert!(bomb.len() < 128 << 10);
        assert_eq!(inflate(&bomb, None, 1 << 20), Err(Rejection::TooLarge));
    }
}
