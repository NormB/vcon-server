//! What the fuzz targets share: the limits and supported extensions they
//! inspect with, the ways they wrap an input, and the result each wrapping
//! must give. The targets are in `fuzz_targets/`.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::write::GzEncoder;
use vcon::{Form, Inspection, Limits, Rejection};

/// The inflation limit the vCon targets inspect with: small, so a body
/// that passes it is quick to find and quick to run.
pub const LIMITS: Limits = Limits { inflated: 64 << 10 };

/// The one extension the targets' server supports.
pub const SUPPORTED: &str = "example-extension";

/// The supported extensions, as `inspect` takes them.
pub fn supported() -> BTreeSet<String> {
    BTreeSet::from([SUPPORTED.to_owned()])
}

/// A uuid no input is expected to carry, for a header that must differ.
pub const OTHER_UUID: &str = "00000000-0000-8000-8000-000000000000";

/// `data` gzipped.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).expect("writing to a Vec");
    encoder.finish().expect("writing to a Vec")
}

/// A JWS in general JSON serialization whose payload is `payload`, with one
/// signature whose header carries `header_uuid` if given.
pub fn jws(payload: &[u8], header_uuid: Option<&str>) -> Vec<u8> {
    let header = match header_uuid {
        Some(uuid) => serde_json::json!({ "alg": "none", "uuid": uuid }),
        None => serde_json::json!({ "alg": "none" }),
    };
    serde_json::json!({
        "payload": URL_SAFE_NO_PAD.encode(payload),
        "signatures": [{ "header": header, "protected": "", "signature": "" }],
    })
    .to_string()
    .into_bytes()
}

/// An encrypted vCon whose `unprotected` member is `unprotected`, spliced
/// in as JSON text, so the input decides what that member is.
pub fn jwe(unprotected: &[u8]) -> Vec<u8> {
    let mut body = br#"{"ciphertext":"","recipients":[],"unprotected":"#.to_vec();
    body.extend_from_slice(unprotected);
    body.push(b'}');
    body
}

/// Whether a body and the same body gzipped were read alike. Gzip is
/// transparent, except where the body is itself gzip (only one layer is
/// inflated before the JSON is read) or either run reached the inflation
/// limit (the gzipped body spends part of its budget on the outer layer).
pub fn gzip_agrees(
    body: &[u8],
    plain: &Result<Inspection, Rejection>,
    gzipped: &Result<Inspection, Rejection>,
) -> bool {
    let too_large = |r: &Result<Inspection, Rejection>| r == &Err(Rejection::TooLarge);
    if body.starts_with(&[0x1f, 0x8b]) || too_large(plain) || too_large(gzipped) {
        return true;
    }
    let expected = match plain {
        Ok(found) => Ok(Inspection {
            gzip: true,
            ..*found
        }),
        Err(other) => Err(other.clone()),
    };
    gzipped == &expected
}

/// How many signed layers `body` has: JWS objects one inside the other,
/// from the body (inflated if gzip) to what the innermost payload holds.
pub fn signed_layers(body: &[u8]) -> usize {
    let mut text = inflated(body);
    let mut layers = 0;
    while let Ok(serde_json::Value::Object(object)) = serde_json::from_slice(&text) {
        let (Some(serde_json::Value::String(payload)), true) =
            (object.get("payload"), object.contains_key("signatures"))
        else {
            break;
        };
        let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload) else {
            break;
        };
        layers += 1;
        text = inflated(&bytes);
    }
    layers
}

/// `data` inflated if it starts as gzip does, else as it is.
fn inflated(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if data.starts_with(&[0x1f, 0x8b])
        && flate2::read::MultiGzDecoder::new(data)
            .read_to_end(&mut out)
            .is_ok()
    {
        return out;
    }
    data.to_vec()
}

/// Whether a JWS whose payload is `payload` was read as `payload` was read
/// as a body of its own. The payload is read as a body is, with the same
/// budget, except that it must hold a signed or unsigned vCon, that every
/// header uuid must be its uuid, and that a payload already at the signed
/// layer limit is one layer past it once signed.
pub fn signed_agrees(
    payload: &[u8],
    plain: &Result<Inspection, Rejection>,
    header_uuid: Option<&str>,
    signed: &Result<Inspection, Rejection>,
) -> bool {
    let expected = match plain {
        Ok(found) if found.form == Form::Encrypted => Err(Rejection::BadPayload),
        // Signing adds a layer, so a payload at the limit is one past it.
        Ok(found)
            if found.form == Form::Signed && signed_layers(payload) == vcon::MAX_SIGNED_LAYERS =>
        {
            Err(Rejection::TooDeep)
        }
        Ok(found) => match header_uuid {
            Some(text) if vcon::parse_uuid(text) != Some(found.uuid) => {
                Err(Rejection::UuidMismatch)
            }
            _ => Ok(Inspection {
                form: Form::Signed,
                gzip: false,
                uuid: found.uuid,
            }),
        },
        Err(Rejection::NotJson | Rejection::NoForm | Rejection::AmbiguousForm) => {
            Err(Rejection::BadPayload)
        }
        // An encrypted payload is refused before its uuid is read, so a
        // body refused for its uuid may be refused otherwise when signed.
        Err(Rejection::MissingUuid | Rejection::BadUuid) => return true,
        Err(other) => Err(other.clone()),
    };
    signed == &expected
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    fn inspect(body: &[u8]) -> Result<Inspection, Rejection> {
        vcon::inspect(body, None, &LIMITS, &supported())
    }

    fn unsigned() -> Vec<u8> {
        format!(r#"{{"uuid":"{ID}","created_at":"2026-10-25T00:00:00Z","parties":[]}}"#)
            .into_bytes()
    }

    #[test]
    fn a_gzipped_body_is_read_as_the_body() {
        let bodies = [
            unsigned(),
            b"not json".to_vec(),
            br#"{"uuid":"x"}"#.to_vec(),
            jws(&unsigned(), Some(ID)),
            jwe(format!(r#"{{"uuid":"{ID}"}}"#).as_bytes()),
        ];
        for body in bodies {
            let (plain, gzipped) = (inspect(&body), inspect(&gzip(&body)));
            assert!(gzip_agrees(&body, &plain, &gzipped));
        }
    }

    #[test]
    fn a_gzipped_body_read_otherwise_disagrees() {
        let body = unsigned();
        let plain = inspect(&body);
        let mut other = plain.clone().expect("unsigned");
        other.gzip = false;
        assert!(!gzip_agrees(&body, &plain, &Ok(other)));
        assert!(!gzip_agrees(&body, &plain, &Err(Rejection::NotJson)));
        assert!(!gzip_agrees(
            b"x",
            &Err(Rejection::NotJson),
            &Err(Rejection::NoForm)
        ));
    }

    #[test]
    fn gzip_is_not_compared_where_it_may_differ() {
        let gz = gzip(&unsigned());
        assert!(gzip_agrees(&gz, &inspect(&gz), &Err(Rejection::NotJson)));
        let ok = inspect(&unsigned());
        assert!(gzip_agrees(&unsigned(), &ok, &Err(Rejection::TooLarge)));
        assert!(gzip_agrees(b"{}", &Err(Rejection::TooLarge), &ok));
    }

    #[test]
    fn a_signed_payload_is_read_as_the_payload() {
        let encrypted = jwe(format!(r#"{{"uuid":"{ID}"}}"#).as_bytes());
        let cases: [(Vec<u8>, Option<&str>); 10] = [
            (unsigned(), None),
            (unsigned(), Some(ID)),
            (unsigned(), Some(OTHER_UUID)),
            (unsigned(), Some("not a uuid")),
            (jws(&unsigned(), None), Some(ID)),
            (gzip(&unsigned()), None),
            (b"[]".to_vec(), None),
            (b"{}".to_vec(), None),
            (br#"{"uuid":"a","uuid":"b"}"#.to_vec(), None),
            (encrypted, None),
        ];
        for (payload, header) in cases {
            let (plain, signed) = (inspect(&payload), inspect(&jws(&payload, header)));
            assert!(signed_agrees(&payload, &plain, header, &signed));
        }
    }

    #[test]
    fn a_signed_payload_read_otherwise_disagrees() {
        let plain = inspect(&unsigned());
        let signed = inspect(&jws(&unsigned(), None));
        assert!(!signed_agrees(
            &unsigned(),
            &plain,
            Some(OTHER_UUID),
            &signed
        ));
        assert!(!signed_agrees(&unsigned(), &plain, None, &plain));
        assert!(!signed_agrees(
            b"x",
            &Err(Rejection::NotJson),
            None,
            &Err(Rejection::NotJson)
        ));
        assert!(!signed_agrees(
            b"x",
            &Err(Rejection::DuplicateKey),
            None,
            &Err(Rejection::BadPayload)
        ));
    }

    #[test]
    fn a_signed_payload_refused_for_its_uuid_is_not_compared() {
        for refused in [Rejection::MissingUuid, Rejection::BadUuid] {
            assert!(signed_agrees(
                b"x",
                &Err(refused),
                None,
                &Err(Rejection::BadPayload)
            ));
        }
    }

    /// `layers` signed layers around an unsigned vCon, each payload gzipped.
    fn nested(layers: usize) -> Vec<u8> {
        (0..layers).fold(unsigned(), |body, _| jws(&gzip(&body), None))
    }

    #[test]
    fn a_payload_at_the_layer_limit_is_too_deep_once_signed() {
        // Past the targets' small inflation limit, so read with a large one.
        let big = Limits { inflated: 1 << 30 };
        let read = |body: &[u8]| vcon::inspect(body, None, &big, &supported());
        let payload = nested(vcon::MAX_SIGNED_LAYERS);
        let (plain, signed) = (read(&payload), read(&jws(&payload, None)));
        assert_eq!(plain.as_ref().map(|found| found.form), Ok(Form::Signed));
        assert_eq!(signed, Err(Rejection::TooDeep));
        assert!(signed_agrees(&payload, &plain, None, &signed));
        assert!(signed_agrees(&payload, &plain, Some(OTHER_UUID), &signed));
        let shallower = nested(vcon::MAX_SIGNED_LAYERS - 1);
        let plain = read(&shallower);
        assert!(!signed_agrees(
            &shallower,
            &plain,
            None,
            &Err(Rejection::TooDeep)
        ));
        assert!(signed_agrees(
            &shallower,
            &plain,
            None,
            &read(&jws(&shallower, None))
        ));
    }

    #[test]
    fn signed_layers_are_counted_through_gzip() {
        assert_eq!(signed_layers(&unsigned()), 0);
        assert_eq!(signed_layers(&nested(3)), 3);
        assert_eq!(signed_layers(&gzip(&nested(3))), 3);
        assert_eq!(signed_layers(&jws(&jws(&unsigned(), None), None)), 2);
    }

    #[test]
    fn a_jwe_carries_the_unprotected_member() {
        let found = inspect(&jwe(format!(r#"{{"uuid":"{ID}"}}"#).as_bytes())).expect("encrypted");
        assert_eq!(found.form, Form::Encrypted);
    }
}
