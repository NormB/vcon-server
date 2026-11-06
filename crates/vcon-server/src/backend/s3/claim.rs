//! Which data directory a bucket prefix belongs to. Start-up recovery
//! removes every object and every tenant under the prefix that this data
//! directory's catalogs do not name, so a second data directory on the
//! same prefix (a copied configuration, or a new install pointed at an old
//! bucket) would remove the first one's vCons. A claim, a random id kept
//! both in the data directory and in the bucket under the prefix, makes
//! the second one refuse to start instead.

use std::path::Path;

use crate::config::StorageError;

/// The data directory's claim file, and the bucket's claim object under
/// the prefix.
pub(crate) const FILE: &str = "storage.claim";
pub(crate) const OBJECT: &str = "vcon-server.claim";

/// A new claim id: 128 random bits as 32 lowercase hex digits.
pub(crate) fn new_id() -> String {
    let mut bytes = [0; 16];
    aws_lc_rs::rand::fill(&mut bytes).expect("AWS-LC aborts instead of failing");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `text` as a claim id, its line ending left out; None if it is not one.
pub(crate) fn parse_id(text: &str) -> Option<&str> {
    let id = text.trim_end_matches(['\r', '\n']);
    (id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(id)
}

/// The claim in `data_dir`, if there is one.
///
/// # Errors
///
/// A `StorageError` naming the file when it cannot be read or does not
/// hold a claim id.
pub(crate) fn read_file(data_dir: &Path) -> Result<Option<String>, StorageError> {
    let path = data_dir.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StorageError(format!("{}: {e}", path.display()))),
    };
    parse_id(&text)
        .map(|id| Some(id.to_owned()))
        .ok_or_else(|| {
            StorageError(format!(
                "{} does not hold a claim id (32 hexadecimal digits)",
                path.display()
            ))
        })
}

/// Writes `id` as the claim of `data_dir`, durably: a new file, synced,
/// renamed into place, and the directory synced.
///
/// # Errors
///
/// A `StorageError` naming the file.
pub(crate) fn write_file(data_dir: &Path, id: &str) -> Result<(), StorageError> {
    use crate::fs::{Fs, OsFs};

    let path = data_dir.join(FILE);
    let tmp = data_dir.join(format!("{FILE}.tmp"));
    let failed = |e: std::io::Error| StorageError(format!("{}: {e}", path.display()));
    match OsFs.remove(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(failed(e)),
        _ => {}
    }
    OsFs.write_new(&tmp, format!("{id}\n").as_bytes())
        .map_err(failed)?;
    OsFs.sync_file(&tmp).map_err(failed)?;
    OsFs.rename(&tmp, &path).map_err(failed)?;
    OsFs.sync_dir(data_dir).map_err(failed)
}

/// What start-up does with the prefix, given the claims found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Both claims are there and agree: the prefix is this data
    /// directory's.
    Use,
    /// Neither claim is there and the prefix holds no tenant: claim it,
    /// the data directory first, then the bucket.
    Claim,
    /// The data directory's claim is there, the bucket's is not, and the
    /// prefix holds no tenant: a first claim was cut off after its first
    /// step, or the bucket was emptied. Write the bucket's.
    Finish,
}

/// The step for `local` (the data directory's claim), `bucket` (the
/// bucket's) and `holds_tenants` (whether any tenant has objects under the
/// prefix), with `place` naming the bucket and prefix in a refusal.
///
/// # Errors
///
/// A `StorageError` when the prefix belongs to another data directory, or
/// holds objects no claim covers.
pub(crate) fn decide(
    local: Option<&str>,
    bucket: Option<&str>,
    holds_tenants: bool,
    place: &str,
) -> Result<Step, StorageError> {
    match (local, bucket) {
        (Some(local), Some(bucket)) if local == bucket => Ok(Step::Use),
        (_, Some(bucket)) => Err(StorageError(format!(
            "{place} belongs to another data directory (its claim is {bucket}); start-up \
             recovery here would remove that server's vCons. Give this server a prefix of \
             its own, or point it at the data directory that claimed this one"
        ))),
        (local, None) if holds_tenants => Err(StorageError(format!(
            "{place} holds tenant objects but no claim{}; start-up recovery would remove \
             every object no catalog here names. Give this server an empty prefix",
            if local.is_some() {
                ", although this data directory claimed it"
            } else {
                ""
            }
        ))),
        (Some(_), None) => Ok(Step::Finish),
        (None, None) => Ok(Step::Claim),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACE: &str = "bucket \"vcon\", prefix \"server-a/\"";
    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn a_prefix_is_used_only_by_the_data_directory_that_claimed_it() {
        for holds in [false, true] {
            assert_eq!(
                decide(Some(A), Some(A), holds, PLACE),
                Ok(Step::Use),
                "the claims agree, holding tenants: {holds}"
            );
        }
        for (local, bucket, holds) in [
            (Some(A), Some(B), false),
            (Some(A), Some(B), true),
            (None, Some(B), false),
            (None, Some(B), true),
        ] {
            let refused = decide(local, bucket, holds, PLACE).expect_err("another's prefix");
            assert!(
                refused.0.contains(PLACE) && refused.0.contains("another data directory"),
                "{local:?} {bucket:?} {holds}: {refused}"
            );
        }
    }

    #[test]
    fn a_claim_id_is_32_lowercase_hex_digits() {
        let id = new_id();
        assert_eq!(parse_id(&id), Some(id.as_str()), "a new id parses");
        assert_ne!(new_id(), id, "each id is new");
        assert_eq!(
            parse_id(&format!("{A}\n")),
            Some(A),
            "its line ending is left out"
        );
        for text in [
            "",
            "0123",
            &A.to_uppercase(),
            &format!("{A}0"),
            &format!("{A}\nmore"),
        ] {
            assert_eq!(parse_id(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_claim_file_reads_back_and_a_damaged_one_is_named() {
        let dir = tempfile::tempdir().expect("dir");
        assert_eq!(read_file(dir.path()), Ok(None), "no claim yet");
        write_file(dir.path(), A).expect("writes");
        assert_eq!(
            read_file(dir.path()),
            Ok(Some(A.to_owned())),
            "the claim written"
        );
        write_file(dir.path(), B).expect("writes over a cut-off write's temporary file");
        assert_eq!(
            read_file(dir.path()),
            Ok(Some(B.to_owned())),
            "the claim written last"
        );
        std::fs::write(dir.path().join(FILE), "not a claim\n").expect("damages it");
        let refused = read_file(dir.path()).expect_err("damaged");
        assert!(refused.0.contains(FILE), "the file is named: {refused}");
    }

    #[test]
    fn an_empty_prefix_is_claimed_and_a_cut_off_claim_finished() {
        assert_eq!(
            decide(None, None, false, PLACE),
            Ok(Step::Claim),
            "an empty prefix with no claim is claimed"
        );
        assert_eq!(
            decide(Some(A), None, false, PLACE),
            Ok(Step::Finish),
            "a claim made only in the data directory is finished in the bucket"
        );
    }

    #[test]
    fn objects_no_claim_covers_are_never_taken_over() {
        for local in [None, Some(A)] {
            let refused = decide(local, None, true, PLACE).expect_err("unclaimed objects");
            assert!(
                refused.0.contains(PLACE) && refused.0.contains("no claim"),
                "{local:?}: {refused}"
            );
        }
    }
}
