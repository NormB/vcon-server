//! The access keys in server.sqlite: made, listed and revoked here, and
//! read by `access` on every request.

use rusqlite::{OptionalExtension, Transaction, params};
use time::OffsetDateTime;

use super::{Slot, TenantError, Tenants, immediate, io};
use crate::access::{self, Scope, Scopes};
use crate::audit::{Action, Actor, Entry};
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
    /// The key with this id, read afresh on every call, so a revocation
    /// counts from the next request. Nothing is recorded here; `access`
    /// records the outcome.
    pub(crate) fn stored_key(&self, id: KeyId) -> Result<Option<StoredKey>, TenantError> {
        let row = self
            .conn()
            .query_row(
                "SELECT token_hash, tenant, scopes, revoked_at IS NOT NULL
                 FROM api_key WHERE id = ?1",
                [id.bytes()],
                |r| {
                    Ok((
                        r.get::<_, Vec<u8>>(0)?,
                        r.get::<_, Option<[u8; 16]>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(io)?;
        row.map(|(hash, tenant, scopes, revoked)| {
            Ok(StoredKey {
                hash,
                tenant: tenant.map(TenantId),
                // Scopes that do not read back are damage, not none.
                scopes: Scopes::parse(&scopes).ok_or(TenantError::Io)?,
                revoked,
            })
        })
        .transpose()
    }

    /// Makes a key. A tenant key holds any of write, read, delete and
    /// legal; a key with no tenant is an operator key, holds only admin,
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
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let refused = match (shape, tenant) {
            (Some(refused), _) => Some(refused),
            (None, Some(t)) if !tenant_exists(&tx, t)? => Some((404, TenantError::NotFound)),
            (None, Some(_)) if closing => Some((503, TenantError::Closing)),
            _ => None,
        };
        if let Some((status, error)) = refused {
            self.record(&tx, &entry(status, tenant.map(|t| t.to_string())))?;
            tx.commit().map_err(io)?;
            return Err(error);
        }
        let (id, token) = access::new_token();
        tx.execute(
            "INSERT INTO api_key (id, tenant, token_hash, scopes, label, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.bytes(),
                tenant.map(|t| t.0),
                access::token_hash(&token),
                scopes.to_string(),
                label,
                nanos(self.clock.now())
            ],
        )
        .map_err(io)?;
        self.record(&tx, &entry(201, Some(id.to_string())))?;
        tx.commit().map_err(io)?;
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
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        if let Some(t) = tenant
            && !tenant_exists(&tx, t)?
        {
            self.record(&tx, &entry(404))?;
            tx.commit().map_err(io)?;
            return Err(TenantError::NotFound);
        }
        let keys = tx
            .prepare(
                "SELECT id, tenant, scopes, label, created_at, revoked_at FROM api_key
                 WHERE ?1 IS NULL OR tenant = ?1 ORDER BY created_at, id",
            )
            .map_err(io)?
            .query_map([tenant.map(|t| t.0)], |r| {
                Ok((
                    r.get::<_, [u8; 8]>(0)?,
                    r.get::<_, Option<[u8; 16]>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(io)?
            .map(|row| {
                let (id, tenant, scopes, label, created_at, revoked_at) = row.map_err(io)?;
                let time = |at| from_nanos(at).ok_or(TenantError::Io);
                Ok(KeyInfo {
                    id: KeyId(id),
                    tenant: tenant.map(TenantId),
                    // Scopes that do not read back are damage, not none.
                    scopes: Scopes::parse(&scopes).ok_or(TenantError::Io)?,
                    label,
                    created_at: time(created_at)?,
                    revoked_at: revoked_at.map(time).transpose()?,
                })
            })
            .collect::<Result<Vec<_>, TenantError>>()?;
        self.record(&tx, &entry(200))?;
        tx.commit().map_err(io)?;
        Ok(keys)
    }

    /// Revokes a key; revoking it again changes nothing.
    pub fn revoke_key(&self, who: Actor, id: KeyId) -> Result<(), TenantError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let revoked = tx
            .execute(
                "UPDATE api_key SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id.bytes(), nanos(self.clock.now())],
            )
            .map_err(io)?;
        let known = revoked > 0
            || tx
                .query_row("SELECT 1 FROM api_key WHERE id = ?1", [id.bytes()], |_| {
                    Ok(())
                })
                .optional()
                .map_err(io)?
                .is_some();
        let (status, result) = if known {
            (204, Ok(()))
        } else {
            (404, Err(TenantError::NotFound))
        };
        let entry = Entry {
            actor: who,
            action: Action::KeyRevoke,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        self.record(&tx, &entry)?;
        tx.commit().map_err(io)?;
        result
    }
}

fn tenant_exists(tx: &Transaction, tenant: TenantId) -> Result<bool, TenantError> {
    Ok(tx
        .query_row("SELECT 1 FROM tenant WHERE id = ?1", [tenant.0], |_| Ok(()))
        .optional()
        .map_err(io)?
        .is_some())
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
        let all = Scopes::of(&[Scope::Write, Scope::Read, Scope::Delete, Scope::Legal]);
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
        assert!(first.is_some());
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
