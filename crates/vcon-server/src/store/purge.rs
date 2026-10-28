//! When a kept version is erased: the purge rule, apart from the files and
//! rows it touches, so its boundaries can be tested directly.

use time::OffsetDateTime;
use uuid::Uuid;

use super::{Store, StoreError, commit, io};
use crate::audit::{Action, Actor, Entry};
use crate::clock::{from_nanos, nanos};
use crate::ids::VersionId;
use crate::retention::Retention;

/// What the purge rule reads of one version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kept {
    /// When the version was stored; retention counts from here.
    pub stored_at: OffsetDateTime,
    /// Whether it was replaced or its uuid deleted. A current version is
    /// never purged.
    pub hidden: bool,
    /// Whether its uuid is under a legal hold.
    pub held: bool,
}

/// Whether `version` is due for purge at `now` under `retention`: it is
/// hidden, its uuid has no hold, and `retention` has passed since it was
/// stored, counted in calendar time in UTC. A period that ends past the
/// last time this build can hold never passes. A version hidden after it
/// reached that age is due at once.
pub fn due(version: &Kept, retention: Retention, now: OffsetDateTime) -> bool {
    version.hidden
        && !version.held
        && retention
            .after(version.stored_at)
            .is_some_and(|end| now >= end)
}

/// What a purge did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// Versions erased.
    pub purged: u64,
}

impl Store {
    /// Erases every version due under `retention`. Each goes in a
    /// transaction of its own that checks the rule again, so a restore or
    /// a hold that commits first keeps it. The transaction deletes the
    /// version's row, and with it the only copy of its wrapped data key,
    /// and records the purge; a deleted uuid left with no version gives up
    /// its row for a tombstone. The object file is removed after the
    /// commit. Last, the WAL is checkpointed with truncation, tried again
    /// while readers finish, so no page that held a purged key is left in
    /// it.
    pub fn purge_due(&self, retention: Retention) -> Result<PurgeReport, StoreError> {
        self.purge_hidden(Actor::Sweep, retention, None)
    }

    /// Purges, as `purge_due` does, the hidden versions that are due, with
    /// `who` as the actor of each record; with `uuid`, only that uuid's.
    pub(super) fn purge_hidden(
        &self,
        who: Actor,
        retention: Retention,
        uuid: Option<Uuid>,
    ) -> Result<PurgeReport, StoreError> {
        let hidden = self.catalog.snapshot()?.hidden_versions(uuid)?;
        // A restore, hold or read may commit here, between the listing and
        // each version's own transaction, which checks the rule again.
        self.pause();
        let mut report = PurgeReport::default();
        for version in hidden {
            if self.purge(who, version, retention)? {
                report.purged += 1;
            }
        }
        if report.purged > 0 {
            self.catalog.after_purge()?;
        }
        Ok(report)
    }

    /// Purges `version` if it is due; false if it is not, or is gone.
    fn purge(
        &self,
        who: Actor,
        version: VersionId,
        retention: Retention,
    ) -> Result<bool, StoreError> {
        let mut tx = self.catalog.immediate()?;
        let Some(row) = tx.hidden_row(version)? else {
            return Ok(false);
        };
        let now = self.clock.now();
        let kept = Kept {
            stored_at: from_nanos(row.stored_at).ok_or(StoreError::Damaged)?,
            hidden: true,
            held: row.held,
        };
        if !due(&kept, retention, now) {
            return Ok(false);
        }
        tx.delete_version(version)?;
        if row.deleted && tx.versions_of(row.uuid)? == 0 {
            tx.tombstone(row.uuid, nanos(now))?;
        }
        let entry = Entry {
            actor: who,
            action: Action::Purge,
            target: Some(row.uuid.to_string()),
            version: Some(version),
            status: 200,
        };
        self.record(&mut tx, &entry)?;
        commit(tx)?;
        self.objects.remove(version).map_err(io)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;
    use time::macros::datetime;

    use super::*;

    const SECOND: Duration = Duration::seconds(1);

    fn hidden(stored_at: OffsetDateTime) -> Kept {
        Kept {
            stored_at,
            hidden: true,
            held: false,
        }
    }

    #[test]
    fn a_hidden_version_is_due_from_the_second_its_period_ends() {
        let seven = Retention::Years(7);
        let cases = [
            // Stored on an ordinary day: due seven calendar years later.
            (
                datetime!(2026-03-15 10:00 UTC),
                datetime!(2033-03-15 10:00 UTC),
            ),
            // Stored on 29 February: due on 28 February seven years later,
            // not on 1 March.
            (
                datetime!(2028-02-29 12:00 UTC),
                datetime!(2035-02-28 12:00 UTC),
            ),
        ];
        for (stored_at, end) in cases {
            let version = hidden(stored_at);
            assert!(
                !due(&version, seven, end - SECOND),
                "{stored_at}: a second short"
            );
            assert!(due(&version, seven, end), "{stored_at}: at the end");
            assert!(
                due(&version, seven, end + SECOND),
                "{stored_at}: a second past"
            );
        }
    }

    #[test]
    fn a_hold_keeps_a_version_past_its_end() {
        let stored_at = datetime!(2028-02-29 12:00 UTC);
        let end = datetime!(2035-02-28 12:00 UTC);
        let version = Kept {
            held: true,
            ..hidden(stored_at)
        };
        for now in [end - SECOND, end, end + SECOND, end + Duration::days(3650)] {
            assert!(!due(&version, Retention::Years(7), now), "{now}");
        }
    }

    #[test]
    fn a_current_version_is_never_due() {
        let version = Kept {
            hidden: false,
            ..hidden(datetime!(2026-03-15 10:00 UTC))
        };
        let end = datetime!(2033-03-15 10:00 UTC);
        for now in [end - SECOND, end, end + SECOND] {
            assert!(!due(&version, Retention::Years(7), now), "{now}");
        }
    }

    #[test]
    fn a_zero_period_makes_a_hidden_version_due_at_once() {
        let stored_at = datetime!(2026-03-15 10:00 UTC);
        let version = hidden(stored_at);
        assert!(due(&version, Retention::Days(0), stored_at));
        assert!(!due(&version, Retention::Days(0), stored_at - SECOND));
    }

    #[test]
    fn a_period_ending_past_the_last_time_held_is_never_due() {
        let version = hidden(datetime!(9998-06-01 00:00 UTC));
        let last = datetime!(9999-12-31 23:59:59 UTC);
        assert!(!due(&version, Retention::Years(7), last));
    }
}

#[cfg(test)]
mod purge_tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration as Wait;

    use aws_lc_rs::cipher::{AES_256, DecryptingKey, DecryptionContext, UnboundCipherKey};
    use aws_lc_rs::iv::FixedLength;
    use serde_json::Value;
    use time::Duration;
    use uuid::Uuid;

    use super::super::testing::vcon;
    use super::super::tests::{Fixture, fixture};
    use super::*;
    use crate::catalog;
    use crate::clock::Clock;
    use crate::envelope::{self, Key};
    use crate::fs::Fs;
    use crate::ids::VersionId;
    use crate::store::{Created, StoreError};

    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const DAY: Retention = Retention::Days(1);

    /// Stores a vCon and returns its uuid and version.
    fn stored(f: &Fixture, subject: &str) -> (Uuid, VersionId) {
        let (found, body) = vcon(UUID, subject);
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        (found.uuid, version)
    }

    fn later(f: &Fixture, by: Duration) {
        f.clock.set(f.clock.now() + by);
    }

    fn count(f: &Fixture, sql: &str, uuid: Uuid) -> i64 {
        f.store
            .conn()
            .query_row(sql, [uuid.as_bytes()], |r| r.get(0))
            .expect(sql)
    }

    fn wrapped_key(f: &Fixture, version: VersionId) -> Vec<u8> {
        f.store
            .conn()
            .query_row(
                "SELECT wrapped_key FROM version WHERE version = ?1",
                [version.0],
                |r| r.get(0),
            )
            .expect("row")
    }

    fn object_exists(f: &Fixture, version: VersionId) -> bool {
        f.fs.read(&f.store.objects_dir.join(version.to_string()))
            .is_ok()
    }

    fn purges(f: &Fixture) -> Vec<Value> {
        f.store
            .records()
            .into_iter()
            .filter(|r| r["action"] == "purge")
            .collect()
    }

    #[test]
    fn a_purge_erases_a_deleted_vcon_and_leaves_a_tombstone() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");
        f.store
            .delete(&f.caller, uuid, Retention::Years(7))
            .expect("deletes");
        later(&f, Duration::days(2));

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 1 }));

        assert_eq!(
            count(&f, "SELECT count(*) FROM version WHERE uuid = ?1", uuid),
            0
        );
        assert_eq!(
            count(&f, "SELECT count(*) FROM vcon WHERE uuid = ?1", uuid),
            0
        );
        assert_eq!(
            count(&f, "SELECT count(*) FROM tombstone WHERE uuid = ?1", uuid),
            1
        );
        let purged_at: i64 = f
            .store
            .conn()
            .query_row(
                "SELECT purged_at FROM tombstone WHERE uuid = ?1",
                [uuid.as_bytes()],
                |r| r.get(0),
            )
            .expect("tombstone");
        assert_eq!(
            purged_at,
            nanos(f.clock.now()),
            "the tombstone holds the purge time"
        );
        assert!(!object_exists(&f, version), "the object file is removed");
        let records = purges(&f);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["actor"], "sweep");
        assert_eq!(records[0]["target"], uuid.to_string());
        assert_eq!(records[0]["version"], version.to_string());
        assert_eq!(f.store.get(&f.caller, uuid), Err(StoreError::Gone));
        let (found, body) = vcon(UUID, "again");
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Gone),
            "a purged uuid cannot be stored again"
        );
    }

    #[test]
    fn a_purge_of_a_replaced_version_keeps_the_live_vcon() {
        let f = fixture();
        let (uuid, old) = stored(&f, "first");
        let (found, body) = vcon(UUID, "second");
        let new = f
            .store
            .replace(&f.caller, old, &found, &body)
            .expect("replaces");
        later(&f, Duration::days(2));

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 1 }));

        assert_eq!(
            count(&f, "SELECT count(*) FROM version WHERE uuid = ?1", uuid),
            1
        );
        assert_eq!(
            count(&f, "SELECT count(*) FROM tombstone WHERE uuid = ?1", uuid),
            0
        );
        assert!(!object_exists(&f, old));
        assert!(object_exists(&f, new));
        assert_eq!(f.store.get(&f.caller, uuid).expect("reads").body, body);
    }

    #[test]
    fn versions_not_due_are_kept() {
        let f = fixture();
        // Current: never due.
        let (current, _) = stored(&f, "current");
        // Deleted, but held.
        let (found, body) = vcon("0199e5c6-0d7a-7c11-8000-000000000001", "held");
        f.store.create(&f.caller, &found, &body).expect("stores");
        f.store
            .delete(&f.caller, found.uuid, Retention::Years(7))
            .expect("deletes");
        f.store
            .conn()
            .execute(
                "UPDATE vcon SET hold = 1 WHERE uuid = ?1",
                [found.uuid.as_bytes()],
            )
            .expect("holds");
        later(&f, Duration::days(2));
        // Deleted, but stored too recently.
        let (young, body) = vcon("0199e5c6-0d7a-7c11-8000-000000000002", "young");
        f.store.create(&f.caller, &young, &body).expect("stores");
        f.store
            .delete(&f.caller, young.uuid, Retention::Years(7))
            .expect("deletes");

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 0 }));

        for uuid in [current, found.uuid, young.uuid] {
            assert_eq!(
                count(&f, "SELECT count(*) FROM version WHERE uuid = ?1", uuid),
                1
            );
        }
        assert!(purges(&f).is_empty());
    }

    #[test]
    fn deleting_an_old_vcon_erases_it_at_once() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");
        later(&f, Duration::days(2));

        assert_eq!(f.store.delete(&f.caller, uuid, DAY), Ok(()));

        assert_eq!(
            count(&f, "SELECT count(*) FROM version WHERE uuid = ?1", uuid),
            0
        );
        assert_eq!(
            count(&f, "SELECT count(*) FROM tombstone WHERE uuid = ?1", uuid),
            1
        );
        assert!(!object_exists(&f, version));
        let records = purges(&f);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0]["actor"],
            f.caller.key.to_string(),
            "the delete that caused the purge is its actor"
        );
    }

    #[test]
    fn deleting_a_young_vcon_keeps_it_hidden() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");
        later(&f, Duration::hours(23));

        assert_eq!(f.store.delete(&f.caller, uuid, DAY), Ok(()));

        assert_eq!(
            count(
                &f,
                "SELECT count(*) FROM version WHERE uuid = ?1 AND hidden_at IS NOT NULL",
                uuid
            ),
            1
        );
        assert!(object_exists(&f, version));
        assert!(purges(&f).is_empty());
    }

    #[test]
    fn a_zero_period_erases_inside_the_delete() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");

        assert_eq!(f.store.delete(&f.caller, uuid, Retention::Days(0)), Ok(()));

        assert_eq!(
            count(&f, "SELECT count(*) FROM tombstone WHERE uuid = ?1", uuid),
            1
        );
        assert!(!object_exists(&f, version));
    }

    #[test]
    fn a_delete_purges_only_its_own_uuid() {
        let f = fixture();
        let (uuid, _) = stored(&f, "first");
        let (other, body) = vcon("0199e5c6-0d7a-7c11-8000-000000000003", "other");
        f.store.create(&f.caller, &other, &body).expect("stores");
        f.store
            .delete(&f.caller, other.uuid, Retention::Years(7))
            .expect("deletes");
        later(&f, Duration::days(2));

        f.store.delete(&f.caller, uuid, DAY).expect("deletes");

        assert_eq!(
            count(
                &f,
                "SELECT count(*) FROM version WHERE uuid = ?1",
                other.uuid
            ),
            1,
            "the other uuid waits for the sweep"
        );
    }

    #[test]
    fn a_delete_whose_purge_fails_stands_and_answers_the_failure() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");
        later(&f, Duration::days(2));
        f.store
            .conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_purge BEFORE DELETE ON version
                 BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;",
            )
            .expect("trigger");

        assert!(f.store.delete(&f.caller, uuid, DAY).is_err());

        assert_eq!(f.store.get(&f.caller, uuid), Err(StoreError::Gone));
        assert_eq!(
            count(
                &f,
                "SELECT count(*) FROM version WHERE uuid = ?1 AND hidden_at IS NOT NULL",
                uuid
            ),
            1,
            "kept hidden for the sweep to purge"
        );
        assert!(object_exists(&f, version));
    }

    #[test]
    fn deleting_again_erases_what_has_since_come_due() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");
        f.store
            .delete(&f.caller, uuid, Retention::Years(7))
            .expect("deletes");
        later(&f, Duration::days(2));

        assert_eq!(f.store.delete(&f.caller, uuid, DAY), Ok(()));

        assert!(!object_exists(&f, version));
    }

    /// Stores a vCon, deletes it while seven years apply, and moves the
    /// clock two days on, so under one day it is due for purge.
    fn due_for_purge(f: &Fixture) -> (Uuid, VersionId, Vec<u8>) {
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        f.store
            .delete(&f.caller, found.uuid, Retention::Years(7))
            .expect("deleted");
        later(f, Duration::days(2));
        (found.uuid, version, body)
    }

    #[test]
    fn a_purge_cut_off_before_its_commit_leaves_the_version_whole() {
        let mut f = fixture();
        let (uuid, version, body) = due_for_purge(&f);
        let other = super::super::tests::block_commits(&f);
        other
            .execute_batch(
                "CREATE TRIGGER dead BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'the process has died'); END;",
            )
            .expect("trigger");

        assert!(f.store.purge_due(DAY).is_err());
        drop(other);
        super::super::tests::restart(&mut f);

        let read = f.store.legal_get(&f.caller, uuid, None).expect("whole");
        assert_eq!((read.stored.version, read.stored.body), (version, body));
        assert!(object_exists(&f, version));
        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 1 }));
        assert!(!object_exists(&f, version));
    }

    #[test]
    fn a_purge_cut_off_at_its_file_step_is_finished_by_recovery() {
        // Step 0 fails the removal itself; step 1 removes it unsynced, so a
        // crash brings the file back.
        for step in 0..=1 {
            let mut f = fixture();
            let (uuid, version, _) = due_for_purge(&f);
            f.fs.fail_from(f.fs.log().len() + step);
            let _ = f.store.purge_due(DAY);
            super::super::tests::restart(&mut f);

            assert_eq!(
                count(&f, "SELECT count(*) FROM version WHERE uuid = ?1", uuid),
                0,
                "step {step}: the commit stands"
            );
            assert_eq!(
                count(&f, "SELECT count(*) FROM tombstone WHERE uuid = ?1", uuid),
                1,
                "step {step}"
            );
            assert!(
                !object_exists(&f, version),
                "step {step}: recovery removes the file"
            );
            assert_eq!(
                f.store.legal_get(&f.caller, uuid, None),
                Err(StoreError::Gone),
                "step {step}"
            );
        }
    }

    /// Runs `legal` on the store between the purge's listing of hidden
    /// versions and its transaction for each, as a racing request would.
    fn between_listing_and_purge(f: &Arc<Fixture>, legal: impl FnOnce(&Fixture) + Send + 'static) {
        let racer = f.clone();
        *f.store.hooks.between.lock().expect("hook") = Some(Box::new(move || legal(&racer)));
    }

    #[test]
    fn a_restore_that_commits_before_the_purge_keeps_the_vcon_live() {
        let f = Arc::new(fixture());
        let (uuid, version, body) = due_for_purge(&f);
        between_listing_and_purge(&f, move |racer| {
            racer.store.restore(&racer.caller, uuid).expect("restored");
        });

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 0 }));

        let read = f.store.get(&f.caller, uuid).expect("live");
        assert_eq!((read.version, read.body), (version, body));
    }

    #[test]
    fn a_hold_that_commits_before_the_purge_keeps_the_version() {
        let f = Arc::new(fixture());
        let (uuid, version, body) = due_for_purge(&f);
        between_listing_and_purge(&f, move |racer| {
            racer
                .store
                .set_hold(&racer.caller, uuid, true)
                .expect("held");
        });

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 0 }));

        let read = f.store.legal_get(&f.caller, uuid, None).expect("kept");
        assert_eq!((read.stored.version, read.stored.body), (version, body));
    }

    #[test]
    fn a_legal_read_racing_a_purge_gets_the_whole_body_or_gone() {
        let f = Arc::new(fixture());
        let (uuid, _, body) = due_for_purge(&f);
        let (seen_tx, seen) = mpsc::channel();
        between_listing_and_purge(&f, move |racer| {
            seen_tx
                .send(racer.store.legal_get(&racer.caller, uuid, None))
                .expect("send");
        });

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 1 }));

        let before = seen
            .recv_timeout(Wait::from_secs(5))
            .expect("the racing read ran");
        assert_eq!(before.expect("before the purge").stored.body, body);
        assert_eq!(
            f.store.legal_get(&f.caller, uuid, None),
            Err(StoreError::Gone),
            "after it"
        );
    }

    /// The plain contents of every page of the catalog at `path` and of
    /// every frame in its WAL, decrypted as SQLCipher 4 writes them:
    /// AES-256-CBC under the raw catalog key, the last 80 bytes of each
    /// page reserved for its 16-byte IV and 64-byte HMAC, and the first 16
    /// bytes of page 1 holding the salt in the clear.
    fn plain_pages(path: &Path, key: &Key, page_size: usize) -> Vec<Vec<u8>> {
        const RESERVED: usize = 80;
        const IV: usize = 16;
        const SALT: usize = 16;
        let raw = key.sqlcipher_raw();
        let hex = &raw[2..66];
        let key: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex"))
            .collect();
        let decrypt = |number: u32, page: &[u8]| {
            let start = if number == 1 { SALT } else { 0 };
            let end = page_size - RESERVED;
            let iv: [u8; IV] = page[end..end + IV].try_into().expect("an IV");
            let mut text = page[start..end].to_vec();
            let cipher = DecryptingKey::cbc(UnboundCipherKey::new(&AES_256, &key).expect("key"))
                .expect("CBC");
            cipher
                .decrypt(&mut text, DecryptionContext::Iv128(FixedLength::from(iv)))
                .expect("decrypts");
            text
        };
        let mut pages = Vec::new();
        let db = std::fs::read(path).expect("catalog");
        for (i, page) in db.chunks_exact(page_size).enumerate() {
            pages.push(decrypt(u32::try_from(i + 1).expect("page number"), page));
        }
        let mut wal_path = path.as_os_str().to_owned();
        wal_path.push("-wal");
        let wal = std::fs::read(&wal_path).unwrap_or_default();
        if let Some(frames) = wal.get(32..) {
            for frame in frames.chunks_exact(24 + page_size) {
                let number = u32::from_be_bytes(frame[..4].try_into().expect("4 bytes"));
                pages.push(decrypt(number, &frame[24..]));
            }
        }
        pages
    }

    fn holds(pages: &[Vec<u8>], needle: &[u8]) -> bool {
        pages
            .iter()
            .any(|page| page.windows(needle.len()).any(|w| w == needle))
    }

    #[test]
    fn no_copy_of_a_purged_wrapped_key_is_left_in_the_catalog() {
        let f = fixture();
        let (uuid, version) = stored(&f, "first");
        let wrapped = wrapped_key(&f, version);
        f.store
            .delete(&f.caller, uuid, Retention::Years(7))
            .expect("deletes");
        let path = f.dir.path().join("catalog.sqlite");
        let key = envelope::catalog_key(&f.store.tenant_key).expect("catalog key");
        let page_size = crate::catalog::tests::cipher_setting(&f.store.conn(), "cipher_page_size");
        let page_size: usize = page_size.parse().expect("a page size");
        // The scan must see the key before the purge, or finding nothing
        // afterwards would prove nothing.
        assert!(holds(&plain_pages(&path, &key, page_size), &wrapped));
        later(&f, Duration::days(2));

        assert_eq!(f.store.purge_due(DAY), Ok(PurgeReport { purged: 1 }));

        assert!(!holds(&plain_pages(&path, &key, page_size), &wrapped));
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        assert_eq!(
            std::fs::metadata(&wal).expect("WAL").len(),
            0,
            "WAL truncated"
        );
    }

    #[test]
    fn the_checkpoint_waits_for_a_reader_without_holding_the_catalog() {
        let f = fixture();
        let (uuid, _) = stored(&f, "first");
        f.store
            .delete(&f.caller, uuid, Retention::Years(7))
            .expect("deletes");
        later(&f, Duration::days(2));
        let path = f.dir.path().join("catalog.sqlite");
        let key = envelope::catalog_key(&f.store.tenant_key).expect("catalog key");
        let mut reader = catalog::open(&path, &key, "").expect("a second connection");
        let released = Arc::new(AtomicBool::new(false));
        let (reading, started) = mpsc::channel();
        let held = {
            let released = released.clone();
            std::thread::spawn(move || {
                let tx = reader.transaction().expect("begin");
                let _: i64 = tx
                    .query_row("SELECT count(*) FROM version", [], |r| r.get(0))
                    .expect("reads");
                reading.send(()).expect("signal");
                std::thread::sleep(Wait::from_millis(500));
                released.store(true, Ordering::SeqCst);
                drop(tx);
            })
        };
        started.recv().expect("the reader started");

        std::thread::scope(|s| {
            let purging = s.spawn(|| f.store.purge_due(DAY));
            // By now the purge waits on the reader at its checkpoint.
            std::thread::sleep(Wait::from_millis(100));
            f.store.counts().expect("the catalog answers");
            assert!(
                !released.load(Ordering::SeqCst),
                "another call had to wait for the reader"
            );
            assert_eq!(
                purging.join().expect("the purge finished"),
                Ok(PurgeReport { purged: 1 })
            );
        });

        held.join().expect("the reader finished");
        let timeout: i64 = f
            .store
            .conn()
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .expect("busy timeout");
        assert_eq!(timeout, 5000, "the catalog's busy timeout is restored");
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        assert_eq!(
            std::fs::metadata(&wal).expect("WAL").len(),
            0,
            "WAL truncated"
        );
    }
}
