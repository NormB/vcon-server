//! A tenant key's rotation: a new tenant key replaces the old one, every
//! data key is rewrapped under it, and the tenant's catalog is re-encrypted
//! under its catalog key, so the old key opens nothing that is kept. A run
//! stopped at any step is finished by running it again.

use rusqlite::{OptionalExtension, params};

use super::{TenantError, Tenants, add_wrap, immediate, io, wraps};
use crate::audit::{Action, Actor, Entry};
use crate::catalog::{self, CatalogError};
use crate::clock::nanos;
use crate::envelope::{self, Context, Key, WrapLimit};
use crate::ids::TenantId;
use crate::store::{Store, StoreError};

/// How many data keys one catalog transaction rewraps.
pub(super) const REKEY_BATCH: usize = 1000;

/// The steps a rotation may be stopped after, for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RekeyStep {
    /// The new key is committed as pending.
    Pending,
    /// One batch of data keys is rewrapped.
    Batch,
    /// Every data key is rewrapped.
    Rewrapped,
    /// The catalog is re-encrypted.
    Reencrypted,
}

impl Tenants {
    /// Rotates the tenant's key. Only the command line does this, with the
    /// server stopped.
    pub fn rekey_tenant(&self, who: Actor, id: TenantId) -> Result<(), TenantError> {
        self.rekey_in_batches(who, id, REKEY_BATCH)
    }

    pub(super) fn rekey_in_batches(
        &self,
        who: Actor,
        id: TenantId,
        batch: usize,
    ) -> Result<(), TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::RekeyTenant,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        let ctx = Context::Tenant { tenant: id };

        // 1. The new key, committed as pending before anything uses it, so a
        // stopped run's data keys are never wrapped under a key that is lost.
        let (old, new) = {
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            let row: Option<(Vec<u8>, Option<Vec<u8>>)> = tx
                .query_row(
                    "SELECT wrapped_tenant_key, pending_wrapped_tenant_key FROM tenant WHERE id = ?1",
                    [id.0],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(io)?;
            let Some((wrapped, pending)) = row else {
                self.record(&tx, &entry(404))?;
                tx.commit().map_err(io)?;
                return Err(TenantError::NotFound);
            };
            let old = envelope::unwrap(&self.master, &ctx, &wrapped).map_err(io)?;
            let new = match pending {
                Some(pending) => envelope::unwrap(&self.master, &ctx, &pending).map_err(io)?,
                None => {
                    let master_id = self.master.id();
                    if envelope::wrap_limit(wraps(&tx, master_id)?) == WrapLimit::Refuse {
                        self.record(&tx, &entry(503))?;
                        tx.commit().map_err(io)?;
                        return Err(TenantError::LimitReached);
                    }
                    add_wrap(&tx, master_id, nanos(self.clock.now()))?;
                    let new = Key::generate().map_err(io)?;
                    let pending = envelope::wrap(&self.master, &ctx, &new);
                    tx.execute(
                        "UPDATE tenant SET pending_wrapped_tenant_key = ?1 WHERE id = ?2",
                        params![pending, id.0],
                    )
                    .map_err(io)?;
                    new
                }
            };
            tx.commit().map_err(io)?;
            (old, new)
        };
        self.stop_after(RekeyStep::Pending)?;

        // No Store handle may outlive the old key; the next use of the
        // tenant opens it afresh under whichever key is current.
        drop(self.drain(id)?);
        let moved = self.move_catalog(id, old, &new, batch);
        self.stores().remove(&id);
        moved?;

        // 4. The new key becomes current and the old one goes, in one
        // transaction with its record.
        {
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            tx.execute(
                "UPDATE tenant SET tenant_key_id = ?1,
                    wrapped_tenant_key = pending_wrapped_tenant_key,
                    pending_wrapped_tenant_key = NULL, key_since = ?2
                 WHERE id = ?3",
                params![new.id().bytes(), nanos(self.clock.now()), id.0],
            )
            .map_err(io)?;
            self.record(&tx, &entry(200))?;
            tx.commit().map_err(io)?;
        }
        self.copy_to_tenant(id, &entry(200))
    }

    /// 2 and 3: every data key rewrapped from `old` to `new`, then the
    /// catalog re-encrypted under `new`'s catalog key. A catalog already under it
    /// was finished by a run that stopped after; it is left as it is.
    fn move_catalog(
        &self,
        id: TenantId,
        old: Key,
        new: &Key,
        batch: usize,
    ) -> Result<(), TenantError> {
        let dir = self.tenants.join(id.to_string());
        let path = dir.join("catalog.sqlite");
        let old_catalog = envelope::catalog_key(&old).map_err(io)?;
        let new_catalog = envelope::catalog_key(new).map_err(io)?;
        match catalog::open(&path, &old_catalog, "") {
            Ok(conn) => drop(conn),
            Err(CatalogError::WrongKey) => {
                drop(catalog::open(&path, &new_catalog, "").map_err(io)?);
                return Ok(());
            }
            Err(e) => return Err(io(e)),
        }
        let store = Store::open(&dir, id, old, self.fs.clone(), self.clock.clone()).map_err(io)?;
        loop {
            let done = store.rewrap(new, batch).map_err(|e| match e {
                StoreError::LimitReached => TenantError::LimitReached,
                other => io(other),
            })?;
            if done == 0 {
                break;
            }
            self.stop_after(RekeyStep::Batch)?;
        }
        drop(store);
        self.stop_after(RekeyStep::Rewrapped)?;
        catalog::reencrypt(&path, &old_catalog, &new_catalog).map_err(io)?;
        self.stop_after(RekeyStep::Reencrypted)
    }

    /// In tests, fails the rotation after `step` when a test asked for it.
    fn stop_after(&self, step: RekeyStep) -> Result<(), TenantError> {
        #[cfg(test)]
        {
            let mut stop = self.hooks.rekey_stop.lock().expect("hook");
            if *stop == Some(step) {
                *stop = None;
                return Err(TenantError::Io);
            }
        }
        let _ = step;
        Ok(())
    }
}

#[cfg(test)]
mod rekey_tests {
    use uuid::Uuid;

    use super::super::tests::{Fixture, fixture};
    use super::*;
    use crate::catalog::{self, CatalogError};
    use crate::envelope::{self, Context, Key};
    use crate::ids::VersionId;
    use crate::store::testing::vcon;
    use crate::store::{Caller, Created};

    /// A version row as stored: version, uuid, wrapping key id, wrapped key.
    type RawVersion = (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>);

    const CALLER: Caller = Caller {
        key: crate::ids::KeyId([7; 8]),
    };

    /// A tenant with five vCons, one of them deleted, so a hidden version
    /// is kept too.
    fn tenant_with_vcons(f: &Fixture) -> (TenantId, Vec<Uuid>) {
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let store = f.tenants.store(id).expect("store");
        let mut uuids = Vec::new();
        for n in 0..5 {
            let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}"), "kept");
            assert!(matches!(
                store.create(&CALLER, &found, &body),
                Ok(Created::New(_))
            ));
            uuids.push(found.uuid);
        }
        store
            .delete(&CALLER, uuids[4], crate::retention::Retention::Years(7))
            .expect("deleted");
        (id, uuids)
    }

    /// Every row of the tenant's catalog opens under the current key, and
    /// nothing kept opens under `old`.
    fn all_under_the_new_key_only(f: &Fixture, id: TenantId, uuids: &[Uuid], old: &Key) {
        let store = f.tenants.store(id).expect("store");
        for uuid in &uuids[..4] {
            assert!(store.get(&CALLER, *uuid).is_ok(), "{uuid} reads");
        }
        assert!(
            store.legal_get(&CALLER, uuids[4], None).is_ok(),
            "the deleted vCon's kept version reads"
        );
        let path = f.tenants_dir().join(id.to_string()).join("catalog.sqlite");
        let old_catalog = envelope::catalog_key(old).expect("catalog key");
        assert!(matches!(
            catalog::open(&path, &old_catalog, ""),
            Err(CatalogError::WrongKey)
        ));
        let new = f.tenant_key(id);
        assert_ne!(new.id(), old.id());
        let conn = catalog::open(&path, &envelope::catalog_key(&new).expect("key"), "")
            .expect("opens under the new key");
        let rows: Vec<RawVersion> = conn
            .prepare("SELECT version, uuid, tenant_key_id, wrapped_key FROM version")
            .expect("prepare")
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(rows.len(), 5);
        for (version, uuid, key_id, wrapped) in rows {
            let ctx = Context::Version {
                tenant: id,
                uuid: Uuid::from_slice(&uuid).expect("uuid"),
                version: VersionId(version.try_into().expect("16 bytes")),
            };
            assert_eq!(key_id, new.id().bytes().to_vec());
            assert!(envelope::unwrap(&new, &ctx, &wrapped).is_ok());
            assert!(envelope::unwrap(old, &ctx, &wrapped).is_err());
        }
        let pending: Option<Vec<u8>> = f
            .conn()
            .query_row(
                "SELECT pending_wrapped_tenant_key FROM tenant WHERE id = ?1",
                [id.0],
                |r| r.get(0),
            )
            .expect("row");
        assert_eq!(pending, None, "no key is left pending");
    }

    #[test]
    fn a_rekey_moves_everything_kept_to_a_new_key_and_records_it() {
        let f = fixture();
        let (id, uuids) = tenant_with_vcons(&f);
        let old = f.tenant_key(id);
        let since_before: i64 = f
            .conn()
            .query_row("SELECT key_since FROM tenant WHERE id = ?1", [id.0], |r| {
                r.get(0)
            })
            .expect("row");
        f.clock.set(time::macros::datetime!(2026-10-23 13:00 UTC));
        f.tenants.rekey_tenant(Actor::Cli, id).expect("rekeyed");
        all_under_the_new_key_only(&f, id, &uuids, &old);
        let since_after: i64 = f
            .conn()
            .query_row("SELECT key_since FROM tenant WHERE id = ?1", [id.0], |r| {
                r.get(0)
            })
            .expect("row");
        assert!(
            since_after > since_before,
            "key_since moves to the rotation"
        );
        let records = f.admin_records("rekey_tenant");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["status"], 200);
        assert_eq!(records[0]["target"], id.to_string().as_str());
        let own = crate::audit::testing::exported(&f.store(id).conn_for_test(), None);
        assert!(
            own.iter().any(|r| r["action"] == "rekey_tenant"),
            "the tenant's log keeps a copy"
        );
        assert_eq!(f.master_wraps(), 2, "the new tenant key's wrap is counted");
    }

    #[test]
    fn a_rekey_stopped_at_any_step_is_finished_by_running_it_again() {
        for step in [
            RekeyStep::Pending,
            RekeyStep::Batch,
            RekeyStep::Rewrapped,
            RekeyStep::Reencrypted,
        ] {
            let f = fixture();
            let (id, uuids) = tenant_with_vcons(&f);
            let old = f.tenant_key(id);
            *f.tenants.hooks.rekey_stop.lock().expect("hook") = Some(step);
            assert_eq!(
                f.tenants.rekey_in_batches(Actor::Cli, id, 2),
                Err(TenantError::Io),
                "{step:?}"
            );
            f.tenants
                .rekey_in_batches(Actor::Cli, id, 2)
                .unwrap_or_else(|e| panic!("{step:?}: {e:?}"));
            all_under_the_new_key_only(&f, id, &uuids, &old);
            assert_eq!(f.master_wraps(), 2, "{step:?}: one new key, made once");
        }
    }

    #[test]
    fn an_unknown_tenant_is_refused_and_recorded() {
        let f = fixture();
        let unknown = TenantId::generate();
        assert_eq!(
            f.tenants.rekey_tenant(Actor::Cli, unknown),
            Err(TenantError::NotFound)
        );
        assert_eq!(f.admin_records("rekey_tenant")[0]["status"], 404);
    }

    #[test]
    fn a_master_key_at_its_wrap_limit_makes_no_new_tenant_key() {
        let f = fixture();
        let (id, _) = tenant_with_vcons(&f);
        f.conn()
            .execute("UPDATE master_key_use SET wraps = ?1", [1_i64 << 32])
            .expect("at the limit");
        assert_eq!(
            f.tenants.rekey_tenant(Actor::Cli, id),
            Err(TenantError::LimitReached)
        );
        assert_eq!(f.admin_records("rekey_tenant")[0]["status"], 503);
        let pending: Option<Vec<u8>> = f
            .conn()
            .query_row(
                "SELECT pending_wrapped_tenant_key FROM tenant WHERE id = ?1",
                [id.0],
                |r| r.get(0),
            )
            .expect("row");
        assert_eq!(pending, None);
        assert!(
            f.tenants.store(id).is_ok(),
            "the tenant is served as before"
        );
    }
}
