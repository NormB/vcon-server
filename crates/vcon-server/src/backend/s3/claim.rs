//! Which data directory a bucket prefix belongs to. Start-up recovery
//! removes every object and every tenant under the prefix that this data
//! directory's catalogs do not name, so a second data directory on the
//! same prefix (a copied configuration, or a new install pointed at an old
//! bucket) would remove the first one's vCons. A claim, a random id kept
//! both in the data directory and in the bucket under the prefix, makes
//! the second one refuse to start instead.
//!
//! Catalogs in PostgreSQL keep their side of the claim in a row of the
//! database (`backend/postgres/claim.rs`) where a data directory has a
//! file: [`Home`] is either, and [`settle`] is the one sequence for both.

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

/// Where the catalogs keep their side of a claim: a data directory's
/// file, or a database's row.
pub(crate) trait Home {
    /// What the catalogs are kept in, as a refusal names it.
    fn noun(&self) -> &'static str;

    /// The claim kept, if there is one.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming what could not be read.
    fn read(&self) -> Result<Option<String>, StorageError>;

    /// Keeps `id` as the claim where there is none, durably, and gives
    /// the claim that is kept: `id`, or the id of a server on the same
    /// catalogs that made its claim first.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming what could not be written.
    fn keep(&self, id: &str) -> Result<String, StorageError>;
}

/// A data directory as the home of a claim: the file [`FILE`] in it. One
/// server has a data directory, by its lock, so the id given is the id
/// kept.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DataDir<'a>(pub(crate) &'a Path);

impl Home for DataDir<'_> {
    fn noun(&self) -> &'static str {
        "data directory"
    }

    fn read(&self) -> Result<Option<String>, StorageError> {
        read_file(self.0)
    }

    fn keep(&self, id: &str) -> Result<String, StorageError> {
        write_file(self.0, id)?;
        Ok(id.to_owned())
    }
}

/// The bucket's side of a claim, under one prefix.
pub(crate) trait Prefix {
    /// The claim under the prefix, if there is one, and whether any
    /// tenant has objects there.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the bucket and prefix.
    fn found(&self) -> Result<(Option<String>, bool), StorageError>;

    /// Writes `id` as the claim under the prefix, where there is none.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the bucket and prefix.
    fn write(&self, id: &str) -> Result<(), StorageError>;
}

/// Binds `prefix` to the catalogs of `home`: uses it, claims it, or
/// refuses, with `place` naming the bucket and prefix in a refusal. The
/// step taken is returned. A new claim is kept by the home first and
/// written to the bucket after, so a claim cut off between the two is
/// finished at the next start; the bucket gets the id the home kept.
///
/// # Errors
///
/// A `StorageError` from [`decide`], or naming the side that could not be
/// read or written.
pub(crate) fn settle(
    home: &dyn Home,
    prefix: &dyn Prefix,
    place: &str,
) -> Result<Step, StorageError> {
    let local = home.read()?;
    let (bucket, holds_tenants) = prefix.found()?;
    let step = decide(
        local.as_deref(),
        bucket.as_deref(),
        holds_tenants,
        place,
        home.noun(),
    )?;
    match step {
        Step::Use => {}
        Step::Claim => prefix.write(&home.keep(&new_id())?)?,
        Step::Finish => prefix.write(local.as_deref().unwrap_or_default())?,
    }
    Ok(step)
}

/// The step for `local` (the claim the catalogs keep), `bucket` (the
/// bucket's) and `holds_tenants` (whether any tenant has objects under the
/// prefix), with `place` naming the bucket and prefix in a refusal and
/// `noun` what the catalogs are kept in.
///
/// # Errors
///
/// A `StorageError` when the prefix belongs to other catalogs, or holds
/// objects no claim covers.
pub(crate) fn decide(
    local: Option<&str>,
    bucket: Option<&str>,
    holds_tenants: bool,
    place: &str,
    noun: &str,
) -> Result<Step, StorageError> {
    match (local, bucket) {
        (Some(local), Some(bucket)) if local == bucket => Ok(Step::Use),
        (_, Some(bucket)) => Err(StorageError(format!(
            "{place} belongs to another {noun} (its claim is {bucket}); start-up \
             recovery here would remove that server's vCons. Give this server a prefix of \
             its own, or point it at the {noun} that claimed this one"
        ))),
        (local, None) if holds_tenants => Err(StorageError(format!(
            "{place} holds tenant objects but no claim{}; start-up recovery would remove \
             every object no catalog here names. Give this server an empty prefix",
            if local.is_some() {
                format!(", although this {noun} claimed it")
            } else {
                String::new()
            }
        ))),
        (Some(_), None) => Ok(Step::Finish),
        (None, None) => Ok(Step::Claim),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    const PLACE: &str = "bucket \"vcon\", prefix \"server-a/\"";
    /// What a refusal calls each kind of home.
    const DIR: &str = "data directory";
    const DB: &str = "database";
    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn a_prefix_is_used_only_by_the_data_directory_that_claimed_it() {
        for holds in [false, true] {
            assert_eq!(
                decide(Some(A), Some(A), holds, PLACE, DIR),
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
            let refused = decide(local, bucket, holds, PLACE, DIR).expect_err("another's prefix");
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
            decide(None, None, false, PLACE, DIR),
            Ok(Step::Claim),
            "an empty prefix with no claim is claimed"
        );
        assert_eq!(
            decide(Some(A), None, false, PLACE, DIR),
            Ok(Step::Finish),
            "a claim made only in the data directory is finished in the bucket"
        );
    }

    #[test]
    fn objects_no_claim_covers_are_never_taken_over() {
        for local in [None, Some(A)] {
            let refused = decide(local, None, true, PLACE, DIR).expect_err("unclaimed objects");
            assert!(
                refused.0.contains(PLACE) && refused.0.contains("no claim"),
                "{local:?}: {refused}"
            );
        }
    }

    /// A home in memory: the claim it keeps, what it does, and what was
    /// asked of it, in order.
    struct FakeHome<'a> {
        noun: &'static str,
        kept: RefCell<Option<String>>,
        /// The id another server on the same catalogs keeps between this
        /// server's read and its keep.
        raced: Option<&'static str>,
        read_fails: bool,
        keep_fails: bool,
        calls: &'a RefCell<Vec<String>>,
    }

    impl<'a> FakeHome<'a> {
        fn holding(
            noun: &'static str,
            kept: Option<&str>,
            calls: &'a RefCell<Vec<String>>,
        ) -> FakeHome<'a> {
            FakeHome {
                noun,
                kept: RefCell::new(kept.map(str::to_owned)),
                raced: None,
                read_fails: false,
                keep_fails: false,
                calls,
            }
        }
    }

    impl Home for FakeHome<'_> {
        fn noun(&self) -> &'static str {
            self.noun
        }

        fn read(&self) -> Result<Option<String>, StorageError> {
            self.calls.borrow_mut().push("home read".to_owned());
            if self.read_fails {
                return Err(StorageError("the home cannot be read".to_owned()));
            }
            Ok(self.kept.borrow().clone())
        }

        fn keep(&self, id: &str) -> Result<String, StorageError> {
            self.calls.borrow_mut().push("home keep".to_owned());
            if self.keep_fails {
                return Err(StorageError("the home cannot be written".to_owned()));
            }
            assert_eq!(parse_id(id), Some(id), "the id given to keep is a claim id");
            let mut kept = self.kept.borrow_mut();
            if kept.is_none() {
                *kept = Some(self.raced.unwrap_or(id).to_owned());
            }
            Ok(kept.clone().expect("a claim is kept"))
        }
    }

    /// A prefix in memory.
    struct FakePrefix<'a> {
        claim: RefCell<Option<String>>,
        holds_tenants: bool,
        found_fails: bool,
        write_fails: bool,
        calls: &'a RefCell<Vec<String>>,
    }

    impl<'a> FakePrefix<'a> {
        fn holding(
            claim: Option<&str>,
            holds_tenants: bool,
            calls: &'a RefCell<Vec<String>>,
        ) -> FakePrefix<'a> {
            FakePrefix {
                claim: RefCell::new(claim.map(str::to_owned)),
                holds_tenants,
                found_fails: false,
                write_fails: false,
                calls,
            }
        }
    }

    impl Prefix for FakePrefix<'_> {
        fn found(&self) -> Result<(Option<String>, bool), StorageError> {
            self.calls.borrow_mut().push("prefix found".to_owned());
            if self.found_fails {
                return Err(StorageError("the prefix cannot be read".to_owned()));
            }
            Ok((self.claim.borrow().clone(), self.holds_tenants))
        }

        fn write(&self, id: &str) -> Result<(), StorageError> {
            self.calls.borrow_mut().push("prefix write".to_owned());
            if self.write_fails {
                return Err(StorageError("the prefix cannot be claimed".to_owned()));
            }
            *self.claim.borrow_mut() = Some(id.to_owned());
            Ok(())
        }
    }

    // What must work.

    #[test]
    fn a_data_directory_is_a_home_that_keeps_the_claim_in_its_file() {
        let dir = tempfile::tempdir().expect("dir");
        let home = DataDir(dir.path());
        assert_eq!(home.noun(), "data directory", "as a refusal names it");
        assert_eq!(home.read(), Ok(None), "no claim yet");
        assert_eq!(
            home.keep(A),
            Ok(A.to_owned()),
            "the id given is the id kept"
        );
        assert_eq!(home.read(), Ok(Some(A.to_owned())), "the claim kept");
        assert_eq!(
            read_file(dir.path()),
            Ok(Some(A.to_owned())),
            "in the file storage.claim"
        );
    }

    #[test]
    fn an_empty_prefix_is_claimed_by_the_home_first_and_the_bucket_after() {
        let calls = RefCell::new(Vec::new());
        let home = FakeHome::holding(DB, None, &calls);
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Claim), "claimed");
        let id = home.kept.borrow().clone().expect("the home keeps a claim");
        assert_eq!(parse_id(&id), Some(id.as_str()), "a claim id");
        assert_eq!(
            *prefix.claim.borrow(),
            Some(id),
            "the bucket holds the same"
        );
        assert_eq!(
            *calls.borrow(),
            ["home read", "prefix found", "home keep", "prefix write"],
            "a claim cut off after the home's step is finished at the next start"
        );
    }

    #[test]
    fn a_prefix_whose_claim_is_the_home_s_is_used_and_nothing_is_written() {
        for holds in [false, true] {
            let calls = RefCell::new(Vec::new());
            let home = FakeHome::holding(DB, Some(A), &calls);
            let prefix = FakePrefix::holding(Some(A), holds, &calls);
            assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Use), "used");
            assert_eq!(
                *calls.borrow(),
                ["home read", "prefix found"],
                "holding tenants: {holds}"
            );
        }
    }

    #[test]
    fn a_claim_the_home_alone_keeps_is_written_to_the_bucket() {
        let calls = RefCell::new(Vec::new());
        let home = FakeHome::holding(DB, Some(A), &calls);
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Finish), "finished");
        assert_eq!(
            *prefix.claim.borrow(),
            Some(A.to_owned()),
            "the home's claim"
        );
        assert_eq!(
            *calls.borrow(),
            ["home read", "prefix found", "prefix write"],
            "the home keeps what it has"
        );
    }

    #[test]
    fn a_prefix_claimed_once_is_used_at_the_next_start() {
        let calls = RefCell::new(Vec::new());
        let home = FakeHome::holding(DIR, None, &calls);
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Claim), "the first");
        let first = home.kept.borrow().clone();
        assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Use), "the second");
        assert_eq!(*home.kept.borrow(), first, "the claim is as made");
        assert_eq!(*prefix.claim.borrow(), first, "in the bucket too");
    }

    #[test]
    fn the_refusals_of_a_data_directory_are_word_for_word_as_before() {
        assert_eq!(
            decide(Some(A), Some(B), false, PLACE, DIR)
                .expect_err("another's")
                .0,
            format!(
                "{PLACE} belongs to another data directory (its claim is {B}); start-up \
                 recovery here would remove that server's vCons. Give this server a prefix of \
                 its own, or point it at the data directory that claimed this one"
            ),
            "another's prefix"
        );
        assert_eq!(
            decide(Some(A), None, true, PLACE, DIR)
                .expect_err("unclaimed")
                .0,
            format!(
                "{PLACE} holds tenant objects but no claim, although this data directory \
                 claimed it; start-up recovery would remove every object no catalog here \
                 names. Give this server an empty prefix"
            ),
            "objects and the bucket's claim gone"
        );
        assert_eq!(
            decide(None, None, true, PLACE, DIR)
                .expect_err("unclaimed")
                .0,
            format!(
                "{PLACE} holds tenant objects but no claim; start-up recovery would remove \
                 every object no catalog here names. Give this server an empty prefix"
            ),
            "objects no claim covers"
        );
    }

    // What must be refused.

    #[test]
    fn a_database_is_told_the_prefix_belongs_to_another_database() {
        for local in [None, Some(A)] {
            let refused = decide(local, Some(B), false, PLACE, DB).expect_err("another's");
            assert_eq!(
                refused.0,
                format!(
                    "{PLACE} belongs to another database (its claim is {B}); start-up \
                     recovery here would remove that server's vCons. Give this server a \
                     prefix of its own, or point it at the database that claimed this one"
                ),
                "{local:?}"
            );
        }
    }

    #[test]
    fn a_database_is_told_when_its_prefix_holds_objects_and_no_claim() {
        assert_eq!(
            decide(Some(A), None, true, PLACE, DB)
                .expect_err("unclaimed")
                .0,
            format!(
                "{PLACE} holds tenant objects but no claim, although this database claimed \
                 it; start-up recovery would remove every object no catalog here names. Give \
                 this server an empty prefix"
            ),
            "the bucket's claim is gone"
        );
        let refused = decide(None, None, true, PLACE, DB).expect_err("unclaimed");
        assert!(
            !refused.0.contains("database") && !refused.0.contains("data directory"),
            "no home has claimed it, so none is named: {refused}"
        );
    }

    #[test]
    fn a_refused_prefix_is_not_written_and_the_home_keeps_nothing() {
        for (local, bucket, holds) in [
            (None, Some(B), false),
            (Some(A), Some(B), true),
            (None, None, true),
            (Some(A), None, true),
        ] {
            let calls = RefCell::new(Vec::new());
            let home = FakeHome::holding(DB, local, &calls);
            let prefix = FakePrefix::holding(bucket, holds, &calls);
            let refused = settle(&home, &prefix, PLACE).expect_err("refused");
            assert!(refused.0.contains(PLACE), "the place is named: {refused}");
            assert_eq!(
                *calls.borrow(),
                ["home read", "prefix found"],
                "{local:?} {bucket:?} {holds}: nothing is written"
            );
            assert_eq!(
                home.kept.borrow().as_deref(),
                local,
                "the home is as it was"
            );
            assert_eq!(
                prefix.claim.borrow().as_deref(),
                bucket,
                "the bucket is as it was"
            );
        }
    }

    #[test]
    fn the_refusal_names_the_home_s_kind_of_catalogs() {
        for (noun, other) in [(DIR, DB), (DB, DIR)] {
            let calls = RefCell::new(Vec::new());
            let home = FakeHome::holding(noun, Some(A), &calls);
            let prefix = FakePrefix::holding(Some(B), true, &calls);
            let refused = settle(&home, &prefix, PLACE).expect_err("another's");
            assert!(
                refused.0.contains(&format!("another {noun} "))
                    && !refused.0.contains(&format!("another {other} ")),
                "{noun}: {refused}"
            );
        }
    }

    #[test]
    fn a_home_that_cannot_be_read_stops_the_claim_before_the_bucket_is_asked() {
        let calls = RefCell::new(Vec::new());
        let mut home = FakeHome::holding(DB, None, &calls);
        home.read_fails = true;
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(
            settle(&home, &prefix, PLACE),
            Err(StorageError("the home cannot be read".to_owned())),
            "the home's error"
        );
        assert_eq!(*calls.borrow(), ["home read"], "nothing else is asked");
    }

    #[test]
    fn a_prefix_that_cannot_be_read_leaves_the_home_without_a_claim() {
        let calls = RefCell::new(Vec::new());
        let home = FakeHome::holding(DB, None, &calls);
        let mut prefix = FakePrefix::holding(None, false, &calls);
        prefix.found_fails = true;
        assert_eq!(
            settle(&home, &prefix, PLACE),
            Err(StorageError("the prefix cannot be read".to_owned())),
            "the bucket's error"
        );
        assert_eq!(*home.kept.borrow(), None, "no claim is kept");
        assert_eq!(*calls.borrow(), ["home read", "prefix found"], "no write");
    }

    #[test]
    fn a_home_that_cannot_keep_the_claim_leaves_the_bucket_unwritten() {
        let calls = RefCell::new(Vec::new());
        let mut home = FakeHome::holding(DB, None, &calls);
        home.keep_fails = true;
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(
            settle(&home, &prefix, PLACE),
            Err(StorageError("the home cannot be written".to_owned())),
            "the home's error"
        );
        assert_eq!(*prefix.claim.borrow(), None, "the bucket has no claim");
        assert_eq!(
            *calls.borrow(),
            ["home read", "prefix found", "home keep"],
            "the bucket is not written"
        );
    }

    // The edges.

    #[test]
    fn a_bucket_that_refuses_the_claim_leaves_one_the_next_start_finishes() {
        let calls = RefCell::new(Vec::new());
        let home = FakeHome::holding(DB, None, &calls);
        let mut prefix = FakePrefix::holding(None, false, &calls);
        prefix.write_fails = true;
        assert_eq!(
            settle(&home, &prefix, PLACE),
            Err(StorageError("the prefix cannot be claimed".to_owned())),
            "the bucket's error"
        );
        let kept = home.kept.borrow().clone();
        assert!(kept.is_some(), "the home kept its claim first");
        prefix.write_fails = false;
        assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Finish), "finished");
        assert_eq!(*prefix.claim.borrow(), kept, "with the claim the home kept");
    }

    #[test]
    fn the_bucket_gets_the_claim_another_server_on_the_catalogs_kept_first() {
        let calls = RefCell::new(Vec::new());
        let mut home = FakeHome::holding(DB, None, &calls);
        home.raced = Some(B);
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(settle(&home, &prefix, PLACE), Ok(Step::Claim), "claimed");
        assert_eq!(
            *prefix.claim.borrow(),
            Some(B.to_owned()),
            "the id the home kept, not the one this server made"
        );
    }

    #[test]
    fn each_claim_is_a_new_id() {
        let ids: Vec<String> = (0..2)
            .map(|_| {
                let calls = RefCell::new(Vec::new());
                let home = FakeHome::holding(DB, None, &calls);
                let prefix = FakePrefix::holding(None, false, &calls);
                settle(&home, &prefix, PLACE).expect("claims");
                prefix.claim.borrow().clone().expect("a claim")
            })
            .collect();
        assert_ne!(ids[0], ids[1], "two prefixes, two ids");
    }

    #[test]
    fn a_data_directory_s_damaged_claim_stops_the_claim_and_names_the_file() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join(FILE), "not a claim\n").expect("damages it");
        let calls = RefCell::new(Vec::new());
        let prefix = FakePrefix::holding(None, false, &calls);
        let refused = settle(&DataDir(dir.path()), &prefix, PLACE).expect_err("damaged");
        assert!(refused.0.contains(FILE), "the file is named: {refused}");
        assert!(calls.borrow().is_empty(), "the bucket is not asked");
    }

    #[test]
    fn a_data_directory_claims_a_prefix_and_keeps_the_id_in_its_file() {
        let dir = tempfile::tempdir().expect("dir");
        let calls = RefCell::new(Vec::new());
        let prefix = FakePrefix::holding(None, false, &calls);
        assert_eq!(
            settle(&DataDir(dir.path()), &prefix, PLACE),
            Ok(Step::Claim),
            "claimed"
        );
        assert_eq!(
            read_file(dir.path()).expect("reads"),
            *prefix.claim.borrow(),
            "the file and the bucket hold one id"
        );
        assert!(prefix.claim.borrow().is_some(), "and there is one");
    }
}
