//! Legal access: reading a deleted or replaced vCon, and listing what is
//! kept of a uuid, for a legal key. Access decides who holds a legal key;
//! these calls trust their caller and record each answer.

use rusqlite::{OptionalExtension, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Caller, Store, StoreError, Stored, commit, immediate, io, version_row};
use crate::audit::{Action, Actor, Entry};
use crate::clock::from_nanos;
use crate::ids::VersionId;

/// A version as a legal read returns it, and whether its uuid is deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Legal {
    pub stored: Stored,
    /// The uuid is deleted; otherwise it is live.
    pub deleted: bool,
}

/// One kept version of a uuid, as the legal version list shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionInfo {
    pub version: VersionId,
    pub stored_at: OffsetDateTime,
    /// When it was replaced, or its uuid deleted; None while current.
    pub hidden_at: Option<OffsetDateTime>,
    /// The uuid's current version.
    pub current: bool,
}

/// What a legal call finds of a uuid.
enum Standing {
    Purged,
    Absent,
    Live(VersionId),
    Deleted,
}

fn standing(tx: &Transaction, uuid: Uuid) -> Result<Standing, StoreError> {
    let purged = tx
        .query_row(
            "SELECT 1 FROM tombstone WHERE uuid = ?1",
            [uuid.as_bytes()],
            |_| Ok(()),
        )
        .optional()
        .map_err(io)?;
    if purged.is_some() {
        return Ok(Standing::Purged);
    }
    let row: Option<(Option<[u8; 16]>, Option<i64>)> = tx
        .query_row(
            "SELECT current, deleted_at FROM vcon WHERE uuid = ?1",
            [uuid.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(io)?;
    match row {
        None => Ok(Standing::Absent),
        Some((_, Some(_))) => Ok(Standing::Deleted),
        Some((Some(current), None)) => Ok(Standing::Live(VersionId(current))),
        Some((None, None)) => Err(StoreError::Damaged),
    }
}

/// The status a legal call records for `answer`.
fn status<T>(answer: &Result<T, StoreError>) -> u16 {
    match answer {
        Ok(_) => 200,
        Err(StoreError::Gone) => 410,
        Err(StoreError::NotFound) => 404,
        Err(_) => 500,
    }
}

impl Store {
    /// Reads `version` of `uuid`, or with None its current version, or its
    /// latest kept one when the uuid is deleted. A version purged, never
    /// stored or of another uuid is NotFound; a purged uuid is Gone. The
    /// whole read, file included, is one immediate transaction, so a purge
    /// cannot remove what is being read. Each answer is recorded.
    pub fn legal_get(
        &self,
        who: &Caller,
        uuid: Uuid,
        version: Option<VersionId>,
    ) -> Result<Legal, StoreError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let answer = self.legal_read(&tx, uuid, version);
        let entry = Entry {
            actor: Actor::Key(who.key),
            action: Action::LegalRead,
            target: Some(uuid.to_string()),
            version: answer.as_ref().ok().map(|read| read.stored.version),
            status: status(&answer),
        };
        self.record(&tx, &entry)?;
        commit(tx)?;
        answer
    }

    fn legal_read(
        &self,
        tx: &Transaction,
        uuid: Uuid,
        asked: Option<VersionId>,
    ) -> Result<Legal, StoreError> {
        let (current, deleted) = match standing(tx, uuid)? {
            Standing::Purged => return Err(StoreError::Gone),
            Standing::Absent => return Err(StoreError::NotFound),
            Standing::Live(current) => (Some(current), false),
            Standing::Deleted => (None, true),
        };
        let version = match (asked, current) {
            (Some(asked), _) => {
                let kept = tx
                    .query_row(
                        "SELECT 1 FROM version WHERE version = ?1 AND uuid = ?2",
                        (asked.0, uuid.as_bytes()),
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(io)?;
                kept.map(|()| asked).ok_or(StoreError::NotFound)?
            }
            (None, Some(current)) => current,
            (None, None) => {
                let latest: Option<[u8; 16]> = tx
                    .query_row(
                        "SELECT version FROM version WHERE uuid = ?1
                         ORDER BY stored_at DESC, rowid DESC LIMIT 1",
                        [uuid.as_bytes()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(io)?;
                // A deleted uuid left with no version is purged, with a
                // tombstone, so a deleted row always keeps one.
                VersionId(latest.ok_or(StoreError::Damaged)?)
            }
        };
        let row = version_row(tx, version)?;
        let body = self.open_version(uuid, version, &row.wrapped)?;
        Ok(Legal {
            stored: Stored {
                version,
                form: row.form,
                gzip: row.gzip,
                body,
            },
            deleted,
        })
    }

    /// Every kept version of `uuid`, oldest first, with its times. A purged
    /// uuid is Gone and one never stored NotFound. Each answer is recorded.
    pub fn legal_versions(&self, who: &Caller, uuid: Uuid) -> Result<Vec<VersionInfo>, StoreError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let answer = versions(&tx, uuid);
        let entry = Entry {
            actor: Actor::Key(who.key),
            action: Action::LegalVersions,
            target: Some(uuid.to_string()),
            version: None,
            status: status(&answer),
        };
        self.record(&tx, &entry)?;
        commit(tx)?;
        answer
    }
}

fn versions(tx: &Transaction, uuid: Uuid) -> Result<Vec<VersionInfo>, StoreError> {
    let current = match standing(tx, uuid)? {
        Standing::Purged => return Err(StoreError::Gone),
        Standing::Absent => return Err(StoreError::NotFound),
        Standing::Live(current) => Some(current),
        Standing::Deleted => None,
    };
    let time = |nanos: i64| from_nanos(nanos).ok_or(StoreError::Damaged);
    let mut rows = tx
        .prepare(
            "SELECT version, stored_at, hidden_at FROM version WHERE uuid = ?1
             ORDER BY stored_at, rowid",
        )
        .map_err(io)?;
    let found = rows
        .query_map([uuid.as_bytes()], |r| {
            Ok((
                VersionId(r.get(0)?),
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(io)?;
    found
        .map(|row| {
            let (version, stored_at, hidden_at) = row.map_err(io)?;
            Ok(VersionInfo {
                version,
                stored_at: time(stored_at)?,
                hidden_at: hidden_at.map(time).transpose()?,
                current: Some(version) == current,
            })
        })
        .collect()
}

#[cfg(test)]
mod legal_tests {
    use serde_json::Value;
    use time::Duration;
    use time::macros::datetime;
    use uuid::Uuid;

    use super::super::testing::vcon;
    use super::super::tests::{Fixture, fixture};
    use super::*;
    use crate::clock::Clock;
    use crate::fs::Fs;
    use crate::ids::VersionId;
    use crate::retention::Retention;
    use crate::store::{Created, StoreError, Stored};

    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const OTHER: &str = "0199e5c6-0d7a-7c11-8000-000000000001";

    /// Stores a vCon of `uuid` and returns its version and body.
    fn stored(f: &Fixture, uuid: &str, subject: &str) -> (VersionId, Vec<u8>) {
        let (found, body) = vcon(uuid, subject);
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        (version, body)
    }

    /// Replaces the current version of `UUID` and returns the new one.
    fn replaced(f: &Fixture, current: VersionId, subject: &str) -> (VersionId, Vec<u8>) {
        let (found, body) = vcon(UUID, subject);
        let version = f
            .store
            .replace(&f.caller, current, &found, &body)
            .expect("replaced");
        (version, body)
    }

    fn uuid() -> Uuid {
        UUID.parse().expect("uuid")
    }

    fn later(f: &Fixture, by: Duration) {
        f.clock.set(f.clock.now() + by);
    }

    /// The last record of `action`.
    fn last(f: &Fixture, action: &str) -> Value {
        f.store
            .records()
            .into_iter()
            .rfind(|r| r["action"] == action)
            .expect("a record")
    }

    #[test]
    fn a_deleted_vcon_reads_back_byte_for_byte_and_says_it_is_deleted() {
        let f = fixture();
        let (version, body) = stored(&f, UUID, "first");
        f.store
            .delete(&f.caller, uuid(), Retention::Years(7))
            .expect("deleted");

        let read = f.store.legal_get(&f.caller, uuid(), None).expect("read");

        assert_eq!(
            read,
            Legal {
                stored: Stored {
                    version,
                    form: vcon_form::Form::Unsigned,
                    gzip: false,
                    body,
                },
                deleted: true,
            }
        );
        let record = last(&f, "legal_read");
        assert_eq!(record["status"], 200);
        assert_eq!(record["target"], UUID);
        assert_eq!(record["version"], version.to_string());
    }

    #[test]
    fn a_deleted_vcon_reads_its_latest_kept_version() {
        let f = fixture();
        let (first, _) = stored(&f, UUID, "first");
        later(&f, Duration::hours(1));
        let (second, body) = replaced(&f, first, "second");
        f.store
            .delete(&f.caller, uuid(), Retention::Years(7))
            .expect("deleted");

        let read = f.store.legal_get(&f.caller, uuid(), None).expect("read");

        assert_eq!((read.stored.version, read.stored.body), (second, body));
    }

    #[test]
    fn a_live_vcon_reads_its_current_version() {
        let f = fixture();
        let (first, _) = stored(&f, UUID, "first");
        let (second, body) = replaced(&f, first, "second");

        let read = f.store.legal_get(&f.caller, uuid(), None).expect("read");

        assert_eq!((read.stored.version, read.stored.body), (second, body));
        assert!(!read.deleted);
    }

    #[test]
    fn a_named_kept_version_reads_back_until_it_is_purged() {
        let f = fixture();
        let (first, body) = stored(&f, UUID, "first");
        replaced(&f, first, "second");

        let read = f
            .store
            .legal_get(&f.caller, uuid(), Some(first))
            .expect("read");
        assert_eq!((read.stored.version, read.stored.body), (first, body));
        assert!(!read.deleted, "the uuid is live");

        f.store.purge_due(Retention::Days(0)).expect("purged");
        assert_eq!(
            f.store.legal_get(&f.caller, uuid(), Some(first)),
            Err(StoreError::NotFound)
        );
        assert_eq!(last(&f, "legal_read")["status"], 404);
    }

    #[test]
    fn a_version_of_another_uuid_is_not_found() {
        let f = fixture();
        stored(&f, UUID, "first");
        let (theirs, _) = stored(&f, OTHER, "other");

        assert_eq!(
            f.store.legal_get(&f.caller, uuid(), Some(theirs)),
            Err(StoreError::NotFound)
        );
    }

    #[test]
    fn a_purged_uuid_is_gone_and_one_never_stored_is_not_found() {
        let f = fixture();
        stored(&f, UUID, "first");
        f.store
            .delete(&f.caller, uuid(), Retention::Days(0))
            .expect("deleted and purged");

        assert_eq!(
            f.store.legal_get(&f.caller, uuid(), None),
            Err(StoreError::Gone)
        );
        assert_eq!(last(&f, "legal_read")["status"], 410);
        assert_eq!(
            f.store.legal_versions(&f.caller, uuid()),
            Err(StoreError::Gone)
        );
        assert_eq!(last(&f, "legal_versions")["status"], 410);

        let never: Uuid = OTHER.parse().expect("uuid");
        assert_eq!(
            f.store.legal_get(&f.caller, never, None),
            Err(StoreError::NotFound)
        );
        assert_eq!(
            f.store.legal_versions(&f.caller, never),
            Err(StoreError::NotFound)
        );
        assert_eq!(last(&f, "legal_versions")["status"], 404);
    }

    #[test]
    fn the_version_list_gives_every_kept_version_with_its_times() {
        let f = fixture();
        let (first, _) = stored(&f, UUID, "first");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let (second, _) = replaced(&f, first, "second");

        let live = f.store.legal_versions(&f.caller, uuid()).expect("list");
        assert_eq!(
            live,
            [
                VersionInfo {
                    version: first,
                    stored_at: datetime!(2026-10-23 12:00 UTC),
                    hidden_at: Some(datetime!(2026-10-23 13:00 UTC)),
                    current: false,
                },
                VersionInfo {
                    version: second,
                    stored_at: datetime!(2026-10-23 13:00 UTC),
                    hidden_at: None,
                    current: true,
                },
            ]
        );
        let record = last(&f, "legal_versions");
        assert_eq!(
            (record["status"].clone(), record["target"].clone()),
            (200.into(), UUID.into())
        );

        f.clock.set(datetime!(2026-10-23 14:00 UTC));
        f.store
            .delete(&f.caller, uuid(), Retention::Years(7))
            .expect("deleted");
        let deleted = f.store.legal_versions(&f.caller, uuid()).expect("list");
        assert_eq!(deleted.len(), 2);
        assert_eq!(deleted[1].hidden_at, Some(datetime!(2026-10-23 14:00 UTC)));
        assert!(deleted.iter().all(|v| !v.current), "nothing is current");
    }

    #[test]
    fn a_damaged_kept_version_is_a_500_never_a_body() {
        let f = fixture();
        let (version, _) = stored(&f, UUID, "first");
        f.store
            .delete(&f.caller, uuid(), Retention::Years(7))
            .expect("deleted");
        let file = f.store.objects.join(version.to_string());
        let mut blob = f.fs.read(&file).expect("file");
        let last_byte = blob.len() - 1;
        blob[last_byte] ^= 1;
        f.fs.remove(&file).expect("remove");
        f.fs.write_new(&file, &blob).expect("write");

        assert_eq!(
            f.store.legal_get(&f.caller, uuid(), None),
            Err(StoreError::Damaged)
        );
        assert_eq!(last(&f, "legal_read")["status"], 500);
    }
}
