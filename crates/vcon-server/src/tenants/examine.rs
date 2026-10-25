//! What `check` reads of the registry: each tenant's Store examined under
//! its own retention period, rotations left part way, and tenant removals
//! since the master key was last rotated. Reading changes nothing.

use rusqlite::OptionalExtension;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{TenantError, Tenants, io, stored_retention};
use crate::audit::Action;
use crate::clock::from_nanos;
use crate::ids::TenantId;
use crate::retention::Retention;
use crate::store::Examination;

/// One tenant as `check` found it.
#[derive(Debug)]
pub struct TenantExamination {
    pub id: TenantId,
    pub name: String,
    /// A key rotation of this tenant was stopped part way.
    pub rotation_stopped: bool,
    /// Its Store examined, or why its catalog did not open.
    pub store: Result<Examination, TenantError>,
}

/// The registry as `check` found it.
#[derive(Debug)]
pub struct RegistryExamination {
    pub tenants: Vec<TenantExamination>,
    /// Tenants removed since the master key was last rotated: their keys
    /// are destroyed only by the next rotation.
    pub removed_since_master: u64,
    /// When the oldest of those removals was made.
    pub oldest_removed_since_master: Option<OffsetDateTime>,
}

impl Tenants {
    /// Examines every tenant at `now`, under its own retention period or
    /// `default`.
    pub fn examine(
        &self,
        default: Retention,
        now: OffsetDateTime,
    ) -> Result<RegistryExamination, TenantError> {
        type Row = ([u8; 16], String, Option<String>, i64, bool);
        let (rows, master_since, removals) =
            {
                let conn = self.conn();
                let rows: Vec<Row> = conn
                .prepare(
                    "SELECT id, name, retention, key_since, pending_wrapped_tenant_key IS NOT NULL
                     FROM tenant ORDER BY name",
                )
                .map_err(io)?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
                .map_err(io)?
                .collect::<Result<_, _>>()
                .map_err(io)?;
                let master_since: Option<i64> = conn
                    .query_row(
                        "SELECT since FROM master_key_use WHERE master_key_id = ?1",
                        [self.master.id().bytes()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(io)?;
                let removals: Vec<String> = conn
                    .prepare("SELECT at FROM audit WHERE action = ?1 AND status = 204")
                    .map_err(io)?
                    .query_map([Action::TenantRemove.as_str()], |r| r.get(0))
                    .map_err(io)?
                    .collect::<Result<_, _>>()
                    .map_err(io)?;
                (rows, master_since, removals)
            };
        let time = |nanos| from_nanos(nanos).ok_or(TenantError::Io);
        let mut tenants = Vec::new();
        for (id, name, retention, key_since, rotation_stopped) in rows {
            let id = TenantId(id);
            let retention = stored_retention(retention)?.unwrap_or(default);
            let key_since = time(key_since)?;
            let store = self
                .store(id)
                .and_then(|store| store.examine(retention, key_since, now).map_err(io));
            tenants.push(TenantExamination {
                id,
                name,
                rotation_stopped,
                store,
            });
        }
        let since = master_since.map(time).transpose()?;
        let mut found = RegistryExamination {
            tenants,
            removed_since_master: 0,
            oldest_removed_since_master: None,
        };
        for at in removals {
            let at = OffsetDateTime::parse(&at, &Rfc3339).map_err(io)?;
            if since.is_none_or(|since| at >= since) {
                found.removed_since_master += 1;
                found.oldest_removed_since_master =
                    Some(found.oldest_removed_since_master.map_or(at, |o| o.min(at)));
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod examine_tests {
    use time::macros::datetime;

    use super::super::tests::fixture;
    use super::*;
    use crate::audit::Actor;
    use crate::store::Caller;
    use crate::store::testing::vcon;

    const CALLER: Caller = Caller {
        key: crate::ids::KeyId([7; 8]),
    };

    #[test]
    fn each_tenant_is_examined_under_its_own_period_or_the_default() {
        let f = fixture();
        let own = f
            .tenants
            .create(Actor::Cli, "own", Some(Retention::Days(1)))
            .expect("made");
        let default = f.tenants.create(Actor::Cli, "default", None).expect("made");
        for id in [own, default] {
            let store = f.tenants.store(id).expect("store");
            let (found, body) = vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc46", "kept");
            store.create(&CALLER, &found, &body).expect("stored");
            store
                .delete(&CALLER, found.uuid, Retention::Years(7))
                .expect("deleted");
        }
        let found = f
            .tenants
            .examine(Retention::Years(7), datetime!(2026-10-30 12:00 UTC))
            .expect("examined");
        let by_name = |name: &str| found.tenants.iter().find(|t| t.name == name).expect(name);
        let own = by_name("own");
        assert_eq!(
            own.store.as_ref().expect("opens").due,
            1,
            "a day has passed"
        );
        assert!(!own.rotation_stopped);
        assert_eq!(by_name("default").store.as_ref().expect("opens").due, 0);
        assert_eq!(found.tenants.len(), 2);
    }

    #[test]
    fn a_rotation_stopped_part_way_is_reported_and_a_catalog_that_does_not_open_too() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        *f.tenants.hooks.rekey_stop.lock().expect("hook") =
            Some(super::super::rekey::RekeyStep::Reencrypted);
        assert_eq!(f.tenants.rekey_tenant(Actor::Cli, id), Err(TenantError::Io));
        let found = f
            .tenants
            .examine(Retention::Years(7), datetime!(2026-10-23 12:00 UTC))
            .expect("examined");
        let acme = &found.tenants[0];
        assert!(acme.rotation_stopped);
        assert!(acme.store.is_err(), "its catalog is under the pending key");
    }

    #[test]
    fn removals_since_the_master_key_was_rotated_are_counted() {
        let f = fixture();
        let gone = f.tenants.create(Actor::Cli, "gone", None).expect("made");
        f.tenants.create(Actor::Cli, "kept", None).expect("made");
        f.clock.set(datetime!(2026-10-24 08:00 UTC));
        f.tenants
            .remove(Actor::Cli, gone, crate::tenants::Removal::Empty)
            .expect("removed");
        // A removal from before the master key's rotation, and a refused one.
        f.conn()
            .execute_batch(
                "INSERT INTO audit (at, actor, action, target, version, status, count)
                 VALUES ('2026-10-01T00:00:00Z', 'cli', 'tenant_remove', NULL, NULL, 204, 1),
                        ('2026-10-24T09:00:00Z', 'cli', 'tenant_remove', NULL, NULL, 409, 1);",
            )
            .expect("older records");
        let found = f
            .tenants
            .examine(Retention::Years(7), datetime!(2026-10-25 12:00 UTC))
            .expect("examined");
        assert_eq!(found.removed_since_master, 1);
        assert_eq!(
            found.oldest_removed_since_master,
            Some(datetime!(2026-10-24 08:00 UTC))
        );
        assert_eq!(found.tenants.len(), 1);
    }
}
