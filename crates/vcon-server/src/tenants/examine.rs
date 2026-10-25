//! What `check` reads of the registry: each tenant's Store examined under
//! its own retention period, rotations left part way, and tenant removals
//! since the master key was last rotated. Reading changes nothing.

use rusqlite::OptionalExtension;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{TenantError, Tenants, io, stored_retention};
use crate::audit::Action;
use crate::clock::from_nanos;
use crate::envelope::{self, Context};
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

/// Whether every tenant's data is under its current key, as `serve`
/// requires before it starts: counts of tenants only, never which.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyOrder {
    /// Tenants whose key does not unwrap under the master key.
    pub wrapped_elsewhere: u64,
    /// Tenants with a key rotation stopped part way.
    pub rotations_stopped: u64,
    /// Tenants whose catalog does not open under their key, or with a
    /// version row naming another key.
    pub data_under_other_keys: u64,
}

impl KeyOrder {
    pub fn is_sound(&self) -> bool {
        *self == KeyOrder::default()
    }
}

/// Work that only a key rotation finishes: purges since each tenant key's
/// rotation, and tenant removals since the master key's. Read from the
/// audit logs only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RotationBacklog {
    pub tenants_with_purges: u64,
    pub purges: u64,
    pub oldest_purge: Option<OffsetDateTime>,
    pub removals: u64,
    pub oldest_removal: Option<OffsetDateTime>,
}

impl Tenants {
    /// Counts the tenants whose data is not all under their current key.
    pub fn key_order(&self) -> Result<KeyOrder, TenantError> {
        let rows: Vec<([u8; 16], Vec<u8>, bool)> = self
            .conn()
            .prepare(
                "SELECT id, wrapped_tenant_key, pending_wrapped_tenant_key IS NOT NULL FROM tenant",
            )
            .map_err(io)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)?;
        let mut order = KeyOrder::default();
        for (id, wrapped, pending) in rows {
            let id = TenantId(id);
            if envelope::unwrap(&self.master, &Context::Tenant { tenant: id }, &wrapped).is_err() {
                order.wrapped_elsewhere += 1;
                continue;
            }
            if pending {
                order.rotations_stopped += 1;
            }
            let elsewhere = match self.store(id) {
                Ok(store) => store.rows_under_other_keys().map_err(io)? > 0,
                Err(_) => true,
            };
            if elsewhere {
                order.data_under_other_keys += 1;
            }
        }
        Ok(order)
    }

    /// The rotation backlog: purges per tenant since its key's rotation,
    /// and removals since the master key's.
    pub fn rotation_backlog(&self) -> Result<RotationBacklog, TenantError> {
        let rows: Vec<([u8; 16], i64)> = self
            .conn()
            .prepare("SELECT id, key_since FROM tenant")
            .map_err(io)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)?;
        let mut backlog = RotationBacklog::default();
        for (id, key_since) in rows {
            let key_since = from_nanos(key_since).ok_or(TenantError::Io)?;
            let (purges, oldest) = self
                .store(TenantId(id))?
                .purges_since(key_since)
                .map_err(io)?;
            if purges > 0 {
                backlog.tenants_with_purges += 1;
                backlog.purges += purges;
                backlog.oldest_purge = earliest(backlog.oldest_purge, oldest);
            }
        }
        (backlog.removals, backlog.oldest_removal) = self.removals_since_master()?;
        Ok(backlog)
    }

    /// Tenant removals since the master key's rotation, and the oldest.
    fn removals_since_master(&self) -> Result<(u64, Option<OffsetDateTime>), TenantError> {
        let (since, removals) = {
            let conn = self.conn();
            let since: Option<i64> = conn
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
            (since, removals)
        };
        let since = since
            .map(|s| from_nanos(s).ok_or(TenantError::Io))
            .transpose()?;
        let (mut count, mut oldest) = (0, None);
        for at in removals {
            let at = OffsetDateTime::parse(&at, &Rfc3339).map_err(io)?;
            if since.is_none_or(|since| at >= since) {
                count += 1;
                oldest = earliest(oldest, Some(at));
            }
        }
        Ok((count, oldest))
    }

    /// Examines every tenant at `now`, under its own retention period or
    /// `default`.
    pub fn examine(
        &self,
        default: Retention,
        now: OffsetDateTime,
    ) -> Result<RegistryExamination, TenantError> {
        type Row = ([u8; 16], String, Option<String>, i64, bool);
        let rows: Vec<Row> = self
            .conn()
            .prepare(
                "SELECT id, name, retention, key_since, pending_wrapped_tenant_key IS NOT NULL
                 FROM tenant ORDER BY name",
            )
            .map_err(io)?
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)?;
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
        let (removed_since_master, oldest_removed_since_master) = self.removals_since_master()?;
        let found = RegistryExamination {
            tenants,
            removed_since_master,
            oldest_removed_since_master,
        };
        Ok(found)
    }
}

/// The earlier of two times, either of which may be absent.
fn earliest(a: Option<OffsetDateTime>, b: Option<OffsetDateTime>) -> Option<OffsetDateTime> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
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

    mod key_order {
        use super::*;
        use crate::envelope::{self, Context, Key};
        use crate::tenants::rekey::RekeyStep;

        fn with_vcons(f: &super::super::super::tests::Fixture) -> TenantId {
            let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
            let store = f.tenants.store(id).expect("store");
            for n in 0..3 {
                let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}"), "k");
                store.create(&CALLER, &found, &body).expect("stored");
            }
            id
        }

        #[test]
        fn a_sound_registry_is_in_order() {
            let f = fixture();
            with_vcons(&f);
            assert!(f.tenants.key_order().expect("read").is_sound());
        }

        #[test]
        fn a_rotation_stopped_at_any_step_is_out_of_order() {
            for (step, data) in [
                (RekeyStep::Pending, 0),
                (RekeyStep::Batch, 1),
                (RekeyStep::Rewrapped, 1),
                (RekeyStep::Reencrypted, 1),
            ] {
                let f = fixture();
                let id = with_vcons(&f);
                *f.tenants.hooks.rekey_stop.lock().expect("hook") = Some(step);
                assert!(f.tenants.rekey_in_batches(Actor::Cli, id, 2).is_err());
                assert_eq!(
                    f.tenants.key_order().expect("read"),
                    KeyOrder {
                        wrapped_elsewhere: 0,
                        rotations_stopped: 1,
                        data_under_other_keys: data,
                    },
                    "{step:?}"
                );
                f.tenants.rekey_tenant(Actor::Cli, id).expect("finished");
                assert!(f.tenants.key_order().expect("read").is_sound(), "{step:?}");
            }
        }

        #[test]
        fn a_tenant_key_wrapped_under_another_key_is_out_of_order() {
            let f = fixture();
            let id = with_vcons(&f);
            let other = Key::generate().expect("key");
            let foreign =
                envelope::wrap(&other, &Context::Tenant { tenant: id }, &f.tenant_key(id));
            f.conn()
                .execute(
                    "UPDATE tenant SET wrapped_tenant_key = ?1 WHERE id = ?2",
                    rusqlite::params![foreign, id.0],
                )
                .expect("rewritten");
            f.tenants.close(id);
            assert_eq!(f.tenants.key_order().expect("read").wrapped_elsewhere, 1);
        }
    }

    #[test]
    fn the_rotation_backlog_counts_purges_and_removals_from_the_logs() {
        let f = fixture();
        let purging = f
            .tenants
            .create(Actor::Cli, "purging", Some(Retention::Days(0)))
            .expect("made");
        f.tenants.create(Actor::Cli, "quiet", None).expect("made");
        let gone = f.tenants.create(Actor::Cli, "gone", None).expect("made");
        let store = f.tenants.store(purging).expect("store");
        for (n, at) in [
            (0, datetime!(2026-10-24 10:00 UTC)),
            (1, datetime!(2026-10-24 11:00 UTC)),
        ] {
            f.clock.set(at);
            let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}"), "k");
            store.create(&CALLER, &found, &body).expect("stored");
            store
                .delete(&CALLER, found.uuid, Retention::Days(0))
                .expect("deleted and purged");
        }
        drop(store);
        f.clock.set(datetime!(2026-10-24 12:00 UTC));
        f.tenants
            .remove(Actor::Cli, gone, crate::tenants::Removal::Empty)
            .expect("removed");
        assert_eq!(
            f.tenants.rotation_backlog().expect("read"),
            RotationBacklog {
                tenants_with_purges: 1,
                purges: 2,
                oldest_purge: Some(datetime!(2026-10-24 10:00 UTC)),
                removals: 1,
                oldest_removal: Some(datetime!(2026-10-24 12:00 UTC)),
            }
        );
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
