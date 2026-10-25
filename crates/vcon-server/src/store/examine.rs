//! What `check` reads of one tenant's Store: every kept version opened,
//! the files no row names, versions due for purge, and purges since the
//! tenant key was last rotated. Reading changes nothing.

use std::collections::BTreeSet;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use super::{Kept, Store, StoreError, due, io};
use crate::audit::Action;
use crate::clock::from_nanos;
use crate::ids::VersionId;
use crate::retention::Retention;

/// One Store as `check` found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Examination {
    /// Every kept version, live or hidden.
    pub versions: u64,
    /// Versions whose object file is missing or does not open.
    pub damaged: Vec<VersionId>,
    /// Files under `objects/` that are a version's, or its temporary, and
    /// that no row names: what start-up recovery removes.
    pub leftovers: Vec<String>,
    /// Versions due for purge now.
    pub due: u64,
    /// When the longest-waiting of them fell due.
    pub oldest_due_since: Option<OffsetDateTime>,
    /// Purges recorded since `key_since`: their keys are destroyed only by
    /// the next rotation.
    pub purges_since_key: u64,
    /// When the oldest of those purges was made.
    pub oldest_purge_since_key: Option<OffsetDateTime>,
}

impl Store {
    /// Examines the Store under `retention` at `now`, counting purges made
    /// since `key_since`.
    pub fn examine(
        &self,
        retention: Retention,
        key_since: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<Examination, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(io)?;
        // version, uuid, stored_at, hidden_at, held
        type Row = ([u8; 16], [u8; 16], i64, Option<i64>, bool);
        let rows: Vec<Row> = tx
            .prepare(
                "SELECT v.version, v.uuid, v.stored_at, v.hidden_at, c.hold != 0
                 FROM version v JOIN vcon c ON c.uuid = v.uuid ORDER BY v.version",
            )
            .map_err(io)?
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)?;
        let time = |nanos| from_nanos(nanos).ok_or(StoreError::Damaged);
        let mut found = Examination {
            versions: rows.len() as u64,
            damaged: Vec::new(),
            leftovers: Vec::new(),
            due: 0,
            oldest_due_since: None,
            purges_since_key: 0,
            oldest_purge_since_key: None,
        };
        for &(version, uuid, stored_at, hidden_at, held) in &rows {
            let version = VersionId(version);
            match self.read_version(&tx, Uuid::from_bytes(uuid), version) {
                Ok(_) => {}
                Err(StoreError::Damaged) => found.damaged.push(version),
                Err(e) => return Err(e),
            }
            let Some(hidden_at) = hidden_at else { continue };
            let stored_at = time(stored_at)?;
            let kept = Kept {
                stored_at,
                hidden: true,
                held,
            };
            if !due(&kept, retention, now) {
                continue;
            }
            // Due once its period ended, or from its hiding if that came
            // later.
            let end = retention.after(stored_at).ok_or(StoreError::Damaged)?;
            let since = end.max(time(hidden_at)?);
            found.due += 1;
            found.oldest_due_since = Some(found.oldest_due_since.map_or(since, |o| o.min(since)));
        }
        let named: BTreeSet<String> = rows
            .iter()
            .map(|(version, ..)| VersionId(*version).to_string())
            .collect();
        for name in self.fs.list(&self.objects).map_err(io)? {
            let Some(name) = name.to_str() else { continue };
            let (id, temporary) = match name.strip_suffix(".tmp") {
                Some(id) => (id, true),
                None => (name, false),
            };
            if VersionId::parse(id).is_some() && (temporary || !named.contains(id)) {
                found.leftovers.push(name.to_owned());
            }
        }
        found.leftovers.sort();
        let purged: Vec<String> = tx
            .prepare("SELECT at FROM audit WHERE action = ?1 AND status = 200")
            .map_err(io)?
            .query_map([Action::Purge.as_str()], |r| r.get(0))
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)?;
        for at in purged {
            let at = OffsetDateTime::parse(&at, &Rfc3339).map_err(|_| StoreError::Damaged)?;
            if at >= key_since {
                found.purges_since_key += 1;
                found.oldest_purge_since_key =
                    Some(found.oldest_purge_since_key.map_or(at, |o| o.min(at)));
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod examine_tests {
    use time::macros::datetime;

    use super::super::testing::vcon;
    use super::super::tests::{Fixture, fixture};
    use super::*;
    use crate::fs::Fs;
    use crate::store::Created;

    const START: OffsetDateTime = datetime!(2026-10-23 12:00 UTC);

    fn stored(f: &Fixture, n: u8) -> (uuid::Uuid, VersionId) {
        let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}"), "kept");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        (found.uuid, version)
    }

    fn examine(f: &Fixture, retention: Retention, now: OffsetDateTime) -> Examination {
        f.store.examine(retention, START, now).expect("examined")
    }

    #[test]
    fn a_sound_store_reports_its_versions_and_nothing_else() {
        let f = fixture();
        stored(&f, 0);
        stored(&f, 1);
        assert_eq!(
            examine(&f, Retention::Years(7), START),
            Examination {
                versions: 2,
                damaged: vec![],
                leftovers: vec![],
                due: 0,
                oldest_due_since: None,
                purges_since_key: 0,
                oldest_purge_since_key: None,
            }
        );
    }

    #[test]
    fn a_missing_or_unreadable_object_is_damage() {
        let f = fixture();
        let (_, missing) = stored(&f, 0);
        let (_, altered) = stored(&f, 1);
        stored(&f, 2);
        let objects = f.store.objects.clone();
        f.fs.remove(&objects.join(missing.to_string()))
            .expect("remove");
        f.fs.remove(&objects.join(altered.to_string()))
            .expect("remove");
        f.fs.write_new(&objects.join(altered.to_string()), b"not a sealed body")
            .expect("write");
        let mut expected = vec![missing, altered];
        expected.sort_by_key(|v| v.0);
        assert_eq!(examine(&f, Retention::Years(7), START).damaged, expected);
    }

    #[test]
    fn files_no_row_names_are_leftovers_and_foreign_files_are_not_ours() {
        let f = fixture();
        let (_, version) = stored(&f, 0);
        let orphan = VersionId([0x0e; 16]);
        let objects = f.store.objects.clone();
        for name in [
            orphan.to_string(),
            format!("{orphan}.tmp"),
            format!("{version}.tmp"),
            "notes.txt".to_owned(),
        ] {
            f.fs.write_new(&objects.join(name), b"x").expect("write");
        }
        let mut expected = vec![
            orphan.to_string(),
            format!("{orphan}.tmp"),
            format!("{version}.tmp"),
        ];
        expected.sort();
        let found = examine(&f, Retention::Years(7), START);
        assert_eq!(found.leftovers, expected);
        assert_eq!(found.damaged, Vec::<VersionId>::new());
    }

    #[test]
    fn versions_due_are_counted_from_when_each_fell_due_and_a_hold_keeps_them() {
        let f = fixture();
        let (deleted_early, _) = stored(&f, 0);
        let (deleted_late, _) = stored(&f, 1);
        let (held, _) = stored(&f, 2);
        stored(&f, 3);
        f.store
            .delete(&f.caller, deleted_early, Retention::Years(7))
            .expect("deleted");
        f.store
            .delete(&f.caller, held, Retention::Years(7))
            .expect("deleted");
        f.store.set_hold(&f.caller, held, true).expect("held");
        // Deleted after its period had already run out: due from the delete.
        f.clock.set(datetime!(2026-10-26 09:00 UTC));
        f.store
            .delete(&f.caller, deleted_late, Retention::Years(7))
            .expect("deleted");
        let found = examine(&f, Retention::Days(1), datetime!(2026-10-27 12:00 UTC));
        assert_eq!(found.due, 2, "the held one is not due");
        assert_eq!(
            found.oldest_due_since,
            Some(datetime!(2026-10-24 12:00 UTC)),
            "a day after it was stored"
        );
        let later = examine(&f, Retention::Years(7), datetime!(2026-10-27 12:00 UTC));
        assert_eq!((later.due, later.oldest_due_since), (0, None));
        assert_eq!(later.versions, 4);
    }

    /// A version hidden after its period had run out became due when it was
    /// hidden, not when the period ended.
    #[test]
    fn a_version_hidden_late_is_due_from_its_hiding() {
        let f = fixture();
        let (uuid, _) = stored(&f, 0);
        f.clock.set(datetime!(2026-10-26 09:00 UTC));
        f.store
            .delete(&f.caller, uuid, Retention::Years(7))
            .expect("deleted");
        let found = examine(&f, Retention::Days(1), datetime!(2026-10-27 12:00 UTC));
        assert_eq!(found.due, 1);
        assert_eq!(
            found.oldest_due_since,
            Some(datetime!(2026-10-26 09:00 UTC))
        );
    }

    #[test]
    fn purges_since_the_key_was_rotated_are_counted() {
        let f = fixture();
        let (first, _) = stored(&f, 0);
        let (second, _) = stored(&f, 1);
        f.clock.set(datetime!(2026-10-24 12:00 UTC));
        f.store
            .delete(&f.caller, first, Retention::Days(0))
            .expect("deleted and purged");
        f.clock.set(datetime!(2026-10-25 12:00 UTC));
        f.store
            .delete(&f.caller, second, Retention::Days(0))
            .expect("deleted and purged");
        // A purge that failed was recorded too; it erased nothing.
        f.store
            .conn()
            .execute(
                "INSERT INTO audit (at, actor, action, target, version, status, count)
                 VALUES ('2026-10-23T13:00:00Z', 'sweep', 'purge', NULL, NULL, 500, 1)",
                [],
            )
            .expect("a failed purge's record");
        let now = datetime!(2026-10-26 12:00 UTC);
        let all = f
            .store
            .examine(Retention::Days(0), START, now)
            .expect("examined");
        assert_eq!(all.purges_since_key, 2);
        assert_eq!(
            all.oldest_purge_since_key,
            Some(datetime!(2026-10-24 12:00 UTC))
        );
        let since = f
            .store
            .examine(Retention::Days(0), datetime!(2026-10-25 00:00 UTC), now)
            .expect("examined");
        assert_eq!(since.purges_since_key, 1);
        assert_eq!(
            since.oldest_purge_since_key,
            Some(datetime!(2026-10-25 12:00 UTC))
        );
        assert_eq!(since.versions, 0);
    }
}
