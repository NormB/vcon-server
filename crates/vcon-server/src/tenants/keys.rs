//! The access keys in server.sqlite: made, listed and revoked here, and
//! read by `access` on every request.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{Slot, TenantError, Tenants, commit};
use crate::access::{self, Scope, Scopes};
use crate::audit::{Action, Actor, Entry};
use crate::backend::KeyRow;
use crate::clock::{from_nanos, nanos};
use crate::ids::{KeyId, TenantId};

/// A key as `list_keys` shows it: never its token or hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    pub id: KeyId,
    /// None for an operator key.
    pub tenant: Option<TenantId>,
    pub scopes: Scopes,
    pub label: String,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
}

impl KeyInfo {
    /// The key as the admin API and the command line show it, never its
    /// token: id, tenant (null for an operator key), scopes, label and
    /// times in RFC 3339. None when a time cannot be written so.
    pub fn to_json(&self) -> Option<serde_json::Value> {
        let time = |at: OffsetDateTime| at.format(&Rfc3339).ok();
        let revoked_at = match self.revoked_at {
            Some(at) => Some(time(at)?),
            None => None,
        };
        Some(serde_json::json!({
            "id": self.id.to_string(),
            "tenant": self.tenant.map(|t| t.to_string()),
            "scopes": self.scopes.names(),
            "label": self.label,
            "created_at": time(self.created_at)?,
            "revoked_at": revoked_at,
        }))
    }
}

/// A key just made. Its token is shown this once and never stored.
#[derive(Debug)]
pub struct NewKey {
    pub id: KeyId,
    pub token: String,
}

/// A key as `access` checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredKey {
    pub(crate) hash: Vec<u8>,
    pub(crate) tenant: Option<TenantId>,
    pub(crate) scopes: Scopes,
    pub(crate) revoked: bool,
}

impl Tenants {
    /// Counts a failed check of a token. A known key's failures are
    /// counted under its id in its tenant's log, or the admin log for an
    /// operator key or a tenant whose log cannot be opened. Any other
    /// failure is counted with no actor in the admin log, so tokens made up
    /// one after another share a record. A failure to count leaves the
    /// request refused all the same.
    pub(crate) fn count_auth_failure(&self, known: Option<(KeyId, Option<TenantId>)>) {
        let entry = |actor| Entry {
            actor,
            action: Action::AuthFailed,
            target: None,
            version: None,
            status: 401,
        };
        if let Some((id, Some(tenant))) = known
            && self
                .store(tenant)
                .is_ok_and(|store| store.count(&entry(Actor::Key(id))).is_ok())
        {
            return;
        }
        let actor = known.map_or(Actor::Unparsed, |(id, _)| Actor::Key(id));
        let _ = self.count_admin(&entry(actor));
    }

    /// The key with this id, read afresh on every call, so a revocation
    /// counts from the next request. Nothing is recorded here; `access`
    /// records the outcome.
    pub(crate) fn stored_key(&self, id: KeyId) -> Result<Option<StoredKey>, TenantError> {
        let row = self.registry.snapshot()?.key(id)?;
        row.map(|row| {
            Ok(StoredKey {
                hash: row.token_hash,
                tenant: row.tenant,
                // Scopes that do not read back are damage, not none.
                scopes: Scopes::parse(&row.scopes).ok_or(TenantError::Io)?,
                revoked: row.revoked_at.is_some(),
            })
        })
        .transpose()
    }

    /// Makes a key. A tenant key holds any of write, read, delete, legal
    /// and audit; a key with no tenant is an operator key, holds only admin,
    /// and only the command line may make one.
    pub fn create_key(
        &self,
        who: Actor,
        tenant: Option<TenantId>,
        scopes: Scopes,
        label: &str,
    ) -> Result<NewKey, TenantError> {
        let entry = |status, target: Option<String>| Entry {
            actor: who,
            action: Action::KeyCreate,
            target,
            version: None,
            status,
        };
        let shape = match tenant {
            None if who != Actor::Cli => Some((403, TenantError::CliOnly)),
            None if scopes != Scopes::of(&[Scope::Admin]) => Some((422, TenantError::BadScopes)),
            Some(_) if scopes.is_empty() || scopes.contains(Scope::Admin) => {
                Some((422, TenantError::BadScopes))
            }
            _ => None,
        };
        let closing = tenant.is_some_and(|t| matches!(self.stores().get(&t), Some(Slot::Closing)));
        let mut tx = self.registry.immediate()?;
        let refused = match (shape, tenant) {
            (Some(refused), _) => Some(refused),
            (None, Some(t)) if tx.tenant(t)?.is_none() => Some((404, TenantError::NotFound)),
            (None, Some(_)) if closing => Some((503, TenantError::Closing)),
            _ => None,
        };
        if let Some((status, error)) = refused {
            self.record(&mut tx, &entry(status, tenant.map(|t| t.to_string())))?;
            commit(tx)?;
            return Err(error);
        }
        let (id, token) = access::new_token();
        tx.insert_key(&KeyRow {
            id,
            tenant,
            token_hash: access::token_hash(&token).to_vec(),
            scopes: scopes.to_string(),
            label: label.to_owned(),
            created_at: nanos(self.clock.now()),
            revoked_at: None,
        })?;
        let made = entry(201, Some(id.to_string()));
        self.record(&mut tx, &made)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        if let Some(t) = tenant {
            self.copy_to_tenant(t, &made)?;
        }
        Ok(NewKey { id, token })
    }

    /// The keys of one tenant, or with None every key, oldest first.
    pub fn list_keys(
        &self,
        who: Actor,
        tenant: Option<TenantId>,
    ) -> Result<Vec<KeyInfo>, TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::KeyList,
            target: tenant.map(|t| t.to_string()),
            version: None,
            status,
        };
        let mut tx = self.registry.immediate()?;
        if let Some(t) = tenant
            && tx.tenant(t)?.is_none()
        {
            self.record(&mut tx, &entry(404))?;
            commit(tx)?;
            return Err(TenantError::NotFound);
        }
        let keys = tx
            .keys(tenant)?
            .into_iter()
            .map(|row| {
                let time = |at| from_nanos(at).ok_or(TenantError::Io);
                Ok(KeyInfo {
                    id: row.id,
                    tenant: row.tenant,
                    // Scopes that do not read back are damage, not none.
                    scopes: Scopes::parse(&row.scopes).ok_or(TenantError::Io)?,
                    label: row.label,
                    created_at: time(row.created_at)?,
                    revoked_at: row.revoked_at.map(time).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, TenantError>>()?;
        self.record(&mut tx, &entry(200))?;
        commit(tx)?;
        Ok(keys)
    }

    /// Revokes a key; revoking it again changes nothing.
    pub fn revoke_key(&self, who: Actor, id: KeyId) -> Result<(), TenantError> {
        let mut tx = self.registry.immediate()?;
        // The key's tenant, if the key is known: None for an operator key.
        let found = tx.key(id)?.map(|row| row.tenant);
        if found.is_some() {
            tx.revoke_key(id, nanos(self.clock.now()))?;
        }
        let entry = Entry {
            actor: who,
            action: Action::KeyRevoke,
            target: Some(id.to_string()),
            version: None,
            status: if found.is_some() { 204 } else { 404 },
        };
        self.record(&mut tx, &entry)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        match found {
            None => Err(TenantError::NotFound),
            Some(Some(t)) => self.copy_to_tenant(t, &entry),
            Some(None) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value as Sql;
    use time::macros::datetime;

    use super::*;
    use crate::access::{self, Scope};
    use crate::tenants::Removal;
    use crate::tenants::tests::{Fixture, fixture};

    fn vcon_scopes() -> Scopes {
        Scopes::of(&[Scope::Write, Scope::Read])
    }

    fn admin() -> Scopes {
        Scopes::of(&[Scope::Admin])
    }

    fn tenant(f: &Fixture, name: &str) -> TenantId {
        f.tenants.create(Actor::Cli, name, None).expect("made")
    }

    /// Every value in the key's row, as SQLite holds it.
    fn row(f: &Fixture, id: KeyId) -> Vec<Sql> {
        let conn = f.conn();
        let mut stmt = conn
            .prepare("SELECT * FROM api_key WHERE id = ?1")
            .expect("prepare");
        let columns = stmt.column_count();
        stmt.query_row([id.bytes()], |r| {
            (0..columns).map(|i| r.get::<_, Sql>(i)).collect()
        })
        .expect("a row")
    }

    fn holds(value: &Sql, needle: &[u8]) -> bool {
        let bytes: &[u8] = match value {
            Sql::Text(t) => t.as_bytes(),
            Sql::Blob(b) => b,
            _ => return false,
        };
        bytes.windows(needle.len()).any(|w| w == needle)
    }

    fn revoked_at(f: &Fixture, id: KeyId) -> Option<i64> {
        f.conn()
            .query_row(
                "SELECT revoked_at FROM api_key WHERE id = ?1",
                [id.bytes()],
                |r| r.get(0),
            )
            .expect("a row")
    }

    #[test]
    fn a_key_is_stored_as_its_hash_and_its_token_is_shown_once() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let key = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "ingest")
            .expect("made");
        assert_eq!(access::parse(&key.token), Ok(key.id));
        let secret = &key.token[21..];
        let values = row(&f, key.id);
        for value in &values {
            assert!(!holds(value, key.token.as_bytes()), "the token is stored");
            assert!(!holds(value, secret.as_bytes()), "the secret is stored");
        }
        let hash: Vec<u8> = f
            .conn()
            .query_row(
                "SELECT token_hash FROM api_key WHERE id = ?1",
                [key.id.bytes()],
                |r| r.get(0),
            )
            .expect("a row");
        assert_eq!(hash, access::token_hash(&key.token));
        let made = f.admin_records("key_create");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0]["actor"], "cli");
        assert_eq!(made[0]["target"], key.id.to_string());
        assert_eq!(made[0]["status"], 201);
    }

    #[test]
    fn an_operator_key_holds_only_admin_and_only_the_command_line_makes_one() {
        let f = fixture();
        let key = f
            .tenants
            .create_key(Actor::Cli, None, admin(), "operator")
            .expect("made");
        let listed = f.tenants.list_keys(Actor::Cli, None).expect("listed");
        assert_eq!(listed[0].id, key.id);
        assert_eq!(listed[0].tenant, None);
        assert_eq!(listed[0].scopes, admin());

        let api = Actor::Key(key.id);
        assert_eq!(
            f.tenants.create_key(api, None, admin(), "another").err(),
            Some(TenantError::CliOnly)
        );
        for scopes in [
            Scopes::of(&[]),
            Scopes::of(&[Scope::Admin, Scope::Read]),
            vcon_scopes(),
        ] {
            assert_eq!(
                f.tenants.create_key(Actor::Cli, None, scopes, "x").err(),
                Some(TenantError::BadScopes),
                "{scopes}"
            );
        }
        let statuses: Vec<_> = f
            .admin_records("key_create")
            .into_iter()
            .map(|r| r["status"].clone())
            .collect();
        assert_eq!(statuses, [201, 403, 422, 422, 422]);
        assert_eq!(
            f.tenants.list_keys(Actor::Cli, None).map(|k| k.len()),
            Ok(1)
        );
    }

    #[test]
    fn a_tenant_key_holds_only_vcon_scopes() {
        let f = fixture();
        let t = tenant(&f, "acme");
        for scopes in [
            Scopes::of(&[]),
            admin(),
            Scopes::of(&[Scope::Write, Scope::Admin]),
        ] {
            assert_eq!(
                f.tenants.create_key(Actor::Cli, Some(t), scopes, "x").err(),
                Some(TenantError::BadScopes),
                "{scopes}"
            );
        }
        let all = Scopes::of(&[
            Scope::Write,
            Scope::Read,
            Scope::Delete,
            Scope::Legal,
            Scope::Audit,
        ]);
        assert!(
            f.tenants
                .create_key(Actor::Cli, Some(t), all, "all")
                .is_ok()
        );
    }

    #[test]
    fn a_key_for_an_unknown_or_closing_tenant_is_refused() {
        let f = fixture();
        let unknown = TenantId::generate();
        assert_eq!(
            f.tenants
                .create_key(Actor::Cli, Some(unknown), vcon_scopes(), "x")
                .err(),
            Some(TenantError::NotFound)
        );
        let t = tenant(&f, "acme");
        f.tenants.drain(t).expect("drains");
        assert_eq!(
            f.tenants
                .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
                .err(),
            Some(TenantError::Closing)
        );
        let refused = f.admin_records("key_create");
        assert_eq!(refused[0]["status"], 404);
        assert_eq!(refused[0]["target"], unknown.to_string());
        assert_eq!(refused[1]["status"], 503);
        assert_eq!(f.count_keys(), 0);
    }

    #[test]
    fn a_key_whose_record_fails_is_not_made() {
        let f = fixture();
        let t = tenant(&f, "acme");
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants
                .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
                .err(),
            Some(TenantError::AuditFailed)
        );
        assert_eq!(f.count_keys(), 0);
    }

    #[test]
    fn keys_are_listed_per_tenant_or_all_oldest_first() {
        let f = fixture();
        let (a, b) = (tenant(&f, "a"), tenant(&f, "b"));
        let first = f
            .tenants
            .create_key(Actor::Cli, Some(a), vcon_scopes(), "first")
            .expect("made");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let other = f
            .tenants
            .create_key(Actor::Cli, Some(b), vcon_scopes(), "other")
            .expect("made");
        f.clock.set(datetime!(2026-10-23 14:00 UTC));
        let second = f
            .tenants
            .create_key(Actor::Cli, Some(a), Scopes::of(&[Scope::Legal]), "second")
            .expect("made");
        f.tenants
            .revoke_key(Actor::Cli, second.id)
            .expect("revoked");

        let a_keys = f.tenants.list_keys(Actor::Cli, Some(a)).expect("listed");
        assert_eq!(
            a_keys,
            [
                KeyInfo {
                    id: first.id,
                    tenant: Some(a),
                    scopes: vcon_scopes(),
                    label: "first".to_owned(),
                    created_at: datetime!(2026-10-23 12:00 UTC),
                    revoked_at: None,
                },
                KeyInfo {
                    id: second.id,
                    tenant: Some(a),
                    scopes: Scopes::of(&[Scope::Legal]),
                    label: "second".to_owned(),
                    created_at: datetime!(2026-10-23 14:00 UTC),
                    revoked_at: Some(datetime!(2026-10-23 14:00 UTC)),
                },
            ]
        );
        let all: Vec<KeyId> = f
            .tenants
            .list_keys(Actor::Cli, None)
            .expect("listed")
            .into_iter()
            .map(|k| k.id)
            .collect();
        assert_eq!(all, [first.id, other.id, second.id]);
        assert_eq!(
            f.tenants
                .list_keys(Actor::Cli, Some(TenantId::generate()))
                .err(),
            Some(TenantError::NotFound)
        );
        let lists = f.admin_records("key_list");
        let statuses: Vec<_> = lists.iter().map(|r| r["status"].clone()).collect();
        assert_eq!(statuses, [200, 200, 404]);
        assert_eq!(lists[0]["target"], a.to_string());
        assert_eq!(lists[1]["target"], serde_json::Value::Null);
    }

    #[test]
    fn a_stored_scope_that_does_not_read_back_is_damage() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let key = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
            .expect("made");
        f.conn()
            .execute(
                "UPDATE api_key SET scopes = 'owner' WHERE id = ?1",
                [key.id.bytes()],
            )
            .expect("damage");
        assert_eq!(
            f.tenants.list_keys(Actor::Cli, None).err(),
            Some(TenantError::Io)
        );
    }

    #[test]
    fn a_key_is_revoked_once_and_an_unknown_one_is_not_found() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let key = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
            .expect("made");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        f.tenants.revoke_key(Actor::Cli, key.id).expect("revoked");
        let first = revoked_at(&f, key.id);
        assert_eq!(
            first,
            Some(crate::clock::nanos(datetime!(2026-10-23 13:00 UTC))),
            "revoked at the time of the revocation"
        );
        f.clock.set(datetime!(2026-10-23 14:00 UTC));
        f.tenants
            .revoke_key(Actor::Cli, key.id)
            .expect("again, no change");
        assert_eq!(revoked_at(&f, key.id), first, "the first time stands");
        let unknown = KeyId::generate();
        assert_eq!(
            f.tenants.revoke_key(Actor::Cli, unknown),
            Err(TenantError::NotFound)
        );
        let records = f.admin_records("key_revoke");
        let statuses: Vec<_> = records.iter().map(|r| r["status"].clone()).collect();
        assert_eq!(statuses, [204, 204, 404]);
        assert_eq!(records[0]["target"], key.id.to_string());
        assert_eq!(records[2]["target"], unknown.to_string());
    }

    fn tenant_records(f: &Fixture, t: TenantId, action: &str) -> Vec<serde_json::Value> {
        crate::audit::testing::exported(&f.tenant_catalog(t), None)
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }

    #[test]
    fn a_tenant_key_made_or_revoked_is_recorded_in_its_tenants_log_too() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let key = f
            .tenants
            .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
            .expect("made");
        f.tenants.revoke_key(Actor::Cli, key.id).expect("revoked");
        let made = tenant_records(&f, t, "key_create");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0]["actor"], "cli");
        assert_eq!(made[0]["target"], key.id.to_string());
        assert_eq!(made[0]["status"], 201);
        let revoked = tenant_records(&f, t, "key_revoke");
        assert_eq!(revoked.len(), 1);
        assert_eq!(revoked[0]["target"], key.id.to_string());
        assert_eq!(revoked[0]["status"], 204);

        let operator = f
            .tenants
            .create_key(Actor::Cli, None, admin(), "op")
            .expect("made");
        f.tenants
            .revoke_key(Actor::Cli, operator.id)
            .expect("revoked");
        assert_eq!(tenant_records(&f, t, "key_create").len(), 1, "only its own");
        let refused = f
            .tenants
            .create_key(Actor::Cli, Some(t), admin(), "x")
            .err();
        assert_eq!(refused, Some(TenantError::BadScopes));
        assert_eq!(
            tenant_records(&f, t, "key_create").len(),
            1,
            "a refusal stays in the admin log"
        );
    }

    /// The change commits with its admin record; a failed copy in the
    /// tenant's log leaves it standing and answers AuditFailed.
    #[test]
    fn a_key_change_whose_tenant_copy_fails_stands() {
        let f = fixture();
        let t = tenant(&f, "acme");
        f.tenant_catalog(t)
            .execute_batch(
                "CREATE TRIGGER dead BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants
                .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
                .err(),
            Some(TenantError::AuditFailed)
        );
        let keys = f.tenants.list_keys(Actor::Cli, Some(t)).expect("listed");
        assert_eq!(keys.len(), 1, "the key stands");
        assert_eq!(
            f.tenants.revoke_key(Actor::Cli, keys[0].id),
            Err(TenantError::AuditFailed)
        );
        assert!(
            revoked_at(&f, keys[0].id).is_some(),
            "the revocation stands"
        );
    }

    #[test]
    fn removing_a_tenant_revokes_its_keys_in_the_same_transaction() {
        let f = fixture();
        let (a, b) = (tenant(&f, "a"), tenant(&f, "b"));
        let mine: Vec<KeyId> = (0..2)
            .map(|_| {
                f.tenants
                    .create_key(Actor::Cli, Some(a), vcon_scopes(), "a")
                    .expect("made")
                    .id
            })
            .collect();
        let theirs = f
            .tenants
            .create_key(Actor::Cli, Some(b), vcon_scopes(), "b")
            .expect("made")
            .id;
        let operator = f
            .tenants
            .create_key(Actor::Cli, None, admin(), "op")
            .expect("made")
            .id;

        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 WHEN NEW.action = 'tenant_remove'
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants.remove(Actor::Cli, a, Removal::Empty),
            Err(TenantError::AuditFailed)
        );
        for id in &mine {
            assert_eq!(
                revoked_at(&f, *id),
                None,
                "a failed removal revokes nothing"
            );
        }
        f.conn()
            .execute_batch("DROP TRIGGER no_audit")
            .expect("drop trigger");

        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        f.tenants
            .remove(Actor::Cli, a, Removal::Empty)
            .expect("removed");
        let at = crate::clock::nanos(datetime!(2026-10-23 13:00 UTC));
        for id in &mine {
            assert_eq!(revoked_at(&f, *id), Some(at));
        }
        assert_eq!(revoked_at(&f, theirs), None);
        assert_eq!(revoked_at(&f, operator), None);
    }

    impl Fixture {
        fn count_keys(&self) -> i64 {
            self.conn()
                .query_row("SELECT count(*) FROM api_key", [], |r| r.get(0))
                .expect("count")
        }
    }
}
