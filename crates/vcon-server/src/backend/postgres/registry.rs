//! The registry in PostgreSQL: tenants, access keys, users, memberships,
//! sessions, settings, master-key use and the admin log, as the tables of
//! schema versions 1 to 6 hold them.
//!
//! A registry transaction sets the marker `registry` where a tenant's sets
//! its id, which the policy `registry_rows` asks for, and a write holds
//! the registry's advisory lock to its end, so the registry has one writer
//! at a time, as the local registry has.
//!
//! What a person typed, and what names one, is sealed under the column key
//! derived from the master key's catalog key, in the registry's context
//! ([`REGISTRY`]): a tenant's name, retention, metadata level and role
//! cap; a key's scopes and label; a user's label and identity; a
//! membership's roles; a session's scopes and authentication methods; a
//! setting's value; an admin record's target. A tenant's name and a
//! user's identity are found by their HMAC under the index key. Ids,
//! times, hashes and wrapped keys are stored as they are.
//!
//! A write that fails aborts its transaction, as PostgreSQL does: every
//! later statement is refused and the commit answers
//! [`CommitError::RolledBack`]. The local registry's transaction goes on
//! after a failed write; no caller relies on that.
#![cfg_attr(
    not(all(test, feature = "backend-tests")),
    expect(
        dead_code,
        reason = "serve opens this registry from a later change; the tests do now"
    )
)]

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use time::OffsetDateTime;
use tokio_postgres::Row;
use tokio_postgres::types::{FromSql, Type};

use super::catalog::{Begin, Database, PgTx};
use crate::audit::{Action, AuditError, Entry};
use crate::backend::{
    CommitError, KeyRow, MembershipRow, Registry, RegistryTx, SessionRow, TenantRow, UserRow,
};
use crate::envelope::{self as column, ColumnKeys, Key, REGISTRY};
use crate::ids::{KeyId, TenantId, UserId};
use crate::tenants::TenantError;

/// The registry in the catalog database.
pub(crate) struct PgRegistry {
    database: Arc<Database>,
    keys: ColumnKeys,
    #[cfg(test)]
    fault: super::catalog::Fault,
}

fn io<E>(_: E) -> TenantError {
    TenantError::Io
}

/// `bytes` as an id of `N` bytes.
fn fixed<const N: usize>(bytes: &[u8]) -> Result<[u8; N], TenantError> {
    bytes.try_into().map_err(io)
}

fn col<'r, T: FromSql<'r>>(row: &'r Row, at: usize) -> Result<T, TenantError> {
    row.try_get(at).map_err(io)
}

/// A user's identity as the bytes its HMAC is of and its sealed copy
/// holds: the issuer's length in four bytes, the issuer, then the subject,
/// so no two identities share their bytes.
pub(crate) fn identity_bytes(issuer: &str, subject: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + issuer.len() + subject.len());
    let len = u32::try_from(issuer.len()).unwrap_or(u32::MAX);
    bytes.extend_from_slice(&len.to_be_bytes());
    bytes.extend_from_slice(issuer.as_bytes());
    bytes.extend_from_slice(subject.as_bytes());
    bytes
}

/// The issuer and the subject [`identity_bytes`] wrote.
pub(crate) fn identity_of(bytes: &[u8]) -> Option<(String, String)> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    let len = usize::try_from(u32::from_be_bytes(*len)).ok()?;
    if len > rest.len() {
        return None;
    }
    let (issuer, subject) = rest.split_at(len);
    Some((
        String::from_utf8(issuer.to_vec()).ok()?,
        String::from_utf8(subject.to_vec()).ok()?,
    ))
}

/// The row key of a membership in a sealed column's context: the user,
/// then the tenant; the user alone for an operator's.
pub(crate) fn membership_key(user: UserId, tenant: Option<TenantId>) -> Vec<u8> {
    let mut key = user.0.to_vec();
    if let Some(tenant) = tenant {
        key.extend_from_slice(&tenant.0);
    }
    key
}

/// The row key of a setting: the tenant, then the setting's name.
pub(crate) fn setting_key(tenant: TenantId, name: &str) -> Vec<u8> {
    let mut key = tenant.0.to_vec();
    key.extend_from_slice(name.as_bytes());
    key
}

const TENANT_COLUMNS: &str = "id, name_sealed, retention_sealed, metadata_sealed, \
     role_cap_sealed, created_at, tenant_key_id, wrapped_tenant_key, \
     pending_wrapped_tenant_key, key_since";
const KEY_COLUMNS: &str = "id, tenant, token_hash, scopes_sealed, label_sealed, created_at, \
     revoked_at, expires_at, last_used_at";
const USER_COLUMNS: &str = "id, label_sealed, identity_sealed, created_at, deactivated_at";
const MEMBERSHIP_COLUMNS: &str = "user_id, tenant, roles_sealed, granted_at";
const SESSION_COLUMNS: &str = "id_hash, user_id, tenant, scopes_sealed, created_at, \
     last_seen_at, idle_expires_at, expires_at, idle_seconds, amr_sealed, csrf_hash";
/// The membership of a user in a tenant, or as an operator for no tenant.
const MEMBERSHIP_IS: &str = "user_id = $1 AND tenant IS NOT DISTINCT FROM $2";

impl PgRegistry {
    /// The registry in `database`, its columns sealed under the keys
    /// derived from `catalog_key`, the master key's catalog key. Nothing is
    /// read: a registry sealed under another key fails at the first sealed
    /// value read.
    ///
    /// # Errors
    ///
    /// [`TenantError::Io`] if a key's page cannot be locked.
    pub(crate) fn open(
        database: Arc<Database>,
        catalog_key: &Key,
    ) -> Result<PgRegistry, TenantError> {
        Ok(PgRegistry {
            database,
            keys: ColumnKeys::derive(catalog_key).map_err(io)?,
            #[cfg(test)]
            fault: std::sync::Mutex::new(None),
        })
    }

    fn begin(&self, how: Begin) -> Result<PgRegistryTx<'_>, TenantError> {
        let lease = self.database.lease().map_err(io)?;
        let tx = PgTx::begin(&self.database, lease, REGISTRY, Some(&self.keys), how);
        #[cfg(test)]
        let tx = tx.with_fault(&self.fault);
        Ok(PgRegistryTx(tx))
    }
}

impl Registry for PgRegistry {
    fn immediate(&self) -> Result<Box<dyn RegistryTx + '_>, TenantError> {
        Ok(Box::new(self.begin(Begin::Write)?))
    }

    fn snapshot(&self) -> Result<Box<dyn RegistryTx + '_>, TenantError> {
        Ok(Box::new(self.begin(Begin::Read)?))
    }

    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError> {
        let tx = self
            .begin(Begin::Read)
            .map_err(|_| AuditError::Database("the database could not be reached".to_owned()))?;
        tx.0.audit_lines(after, None, out)
    }

    /// Not for a registry in PostgreSQL: it is no file, and its backup is
    /// the database's own (`pg_dump` of the catalog database), which the
    /// how-to covers. Nothing is written at `to`.
    fn backup_into(&self, _to: &Path) -> Result<Vec<TenantId>, TenantError> {
        Err(TenantError::NoFileBackup)
    }

    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError) {
        *self.fault.lock().expect("fault") = Some((nth, how));
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A transaction on the registry, rolled back when dropped without a
/// commit.
struct PgRegistryTx<'a>(PgTx<'a>);

impl PgRegistryTx<'_> {
    fn seal(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        text: &[u8],
    ) -> Result<Vec<u8>, TenantError> {
        self.0.seal(table, column, row, text).map_err(io)
    }

    fn seal_some(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        text: Option<&str>,
    ) -> Result<Option<Vec<u8>>, TenantError> {
        text.map(|text| self.seal(table, column, row, text.as_bytes()))
            .transpose()
    }

    fn text(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        sealed: &[u8],
    ) -> Result<String, TenantError> {
        let bytes = self.0.unseal(table, column, row, sealed).map_err(io)?;
        String::from_utf8(bytes).map_err(io)
    }

    fn text_some(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        sealed: Option<Vec<u8>>,
    ) -> Result<Option<String>, TenantError> {
        sealed
            .map(|sealed| self.text(table, column, row, &sealed))
            .transpose()
    }

    fn name_mac(&self, name: &str) -> Result<[u8; 32], TenantError> {
        Ok(column::mac(self.0.keys().map_err(io)?, name.as_bytes()))
    }

    fn tenant_row(&self, row: &Row) -> Result<TenantRow, TenantError> {
        let id: Vec<u8> = col(row, 0)?;
        let open = |name: &str, at: usize| -> Result<Option<String>, TenantError> {
            self.text_some("tenant", name, &id, col(row, at)?)
        };
        Ok(TenantRow {
            id: TenantId(fixed(&id)?),
            name: self.text("tenant", "name_sealed", &id, &col::<Vec<u8>>(row, 1)?)?,
            retention: open("retention_sealed", 2)?,
            metadata: open("metadata_sealed", 3)?,
            role_cap: open("role_cap_sealed", 4)?,
            created_at: col(row, 5)?,
            tenant_key_id: KeyId(fixed(&col::<Vec<u8>>(row, 6)?)?),
            wrapped: col(row, 7)?,
            pending: col(row, 8)?,
            key_since: col(row, 9)?,
        })
    }

    fn key_row(&self, row: &Row) -> Result<KeyRow, TenantError> {
        let id: Vec<u8> = col(row, 0)?;
        let tenant: Option<Vec<u8>> = col(row, 1)?;
        Ok(KeyRow {
            id: KeyId(fixed(&id)?),
            tenant: tenant.map(|t| fixed(&t).map(TenantId)).transpose()?,
            token_hash: col(row, 2)?,
            scopes: self.text("api_key", "scopes_sealed", &id, &col::<Vec<u8>>(row, 3)?)?,
            label: self.text("api_key", "label_sealed", &id, &col::<Vec<u8>>(row, 4)?)?,
            created_at: col(row, 5)?,
            revoked_at: col(row, 6)?,
            expires_at: col(row, 7)?,
            last_used_at: col(row, 8)?,
        })
    }

    fn user_row(&self, row: &Row) -> Result<UserRow, TenantError> {
        let id: Vec<u8> = col(row, 0)?;
        let identity: Option<Vec<u8>> = col(row, 2)?;
        let identity = identity
            .map(|sealed| {
                let bytes = self
                    .0
                    .unseal("user_account", "identity_sealed", &id, &sealed)
                    .map_err(io)?;
                identity_of(&bytes).ok_or(TenantError::Io)
            })
            .transpose()?;
        Ok(UserRow {
            id: UserId(fixed(&id)?),
            label: self.text(
                "user_account",
                "label_sealed",
                &id,
                &col::<Vec<u8>>(row, 1)?,
            )?,
            identity,
            created_at: col(row, 3)?,
            deactivated_at: col(row, 4)?,
        })
    }

    fn membership_row(&self, row: &Row) -> Result<MembershipRow, TenantError> {
        let user = UserId(fixed(&col::<Vec<u8>>(row, 0)?)?);
        let tenant: Option<Vec<u8>> = col(row, 1)?;
        let tenant = tenant.map(|t| fixed(&t).map(TenantId)).transpose()?;
        let key = membership_key(user, tenant);
        Ok(MembershipRow {
            user,
            tenant,
            roles: self.text("membership", "roles_sealed", &key, &col::<Vec<u8>>(row, 2)?)?,
            granted_at: col(row, 3)?,
        })
    }

    fn session_row(&self, row: &Row) -> Result<SessionRow, TenantError> {
        let id_hash: Vec<u8> = col(row, 0)?;
        let tenant: Option<Vec<u8>> = col(row, 2)?;
        Ok(SessionRow {
            user: UserId(fixed(&col::<Vec<u8>>(row, 1)?)?),
            tenant: tenant.map(|t| fixed(&t).map(TenantId)).transpose()?,
            scopes: self.text(
                "session",
                "scopes_sealed",
                &id_hash,
                &col::<Vec<u8>>(row, 3)?,
            )?,
            created_at: col(row, 4)?,
            last_seen_at: col(row, 5)?,
            idle_expires_at: col(row, 6)?,
            expires_at: col(row, 7)?,
            idle_seconds: col(row, 8)?,
            amr: self.text("session", "amr_sealed", &id_hash, &col::<Vec<u8>>(row, 9)?)?,
            csrf_hash: col(row, 10)?,
            id_hash,
        })
    }

    /// Sets one sealed column of a tenant, or with None empties it; false
    /// if no tenant has the id.
    fn set_sealed(
        &self,
        id: TenantId,
        column: &str,
        value: Option<&str>,
    ) -> Result<bool, TenantError> {
        let sealed = self.seal_some("tenant", column, &id.0, value)?;
        let changed = self
            .0
            .execute(
                &format!("UPDATE vcon.tenant SET {column} = $2 WHERE id = $1"),
                &[(&id.0.as_slice(), Type::BYTEA), (&sealed, Type::BYTEA)],
            )
            .map_err(io)?;
        Ok(changed != 0)
    }

    /// Runs a write whose count of rows the caller does not need.
    fn run(
        &self,
        sql: &str,
        params: &[(&(dyn tokio_postgres::types::ToSql + Sync), Type)],
    ) -> Result<(), TenantError> {
        self.0.execute(sql, params).map(drop).map_err(io)
    }
}

impl RegistryTx for PgRegistryTx<'_> {
    fn tenant(&mut self, id: TenantId) -> Result<Option<TenantRow>, TenantError> {
        let rows = self
            .0
            .query(
                &format!("SELECT {TENANT_COLUMNS} FROM vcon.tenant WHERE id = $1"),
                &[(&id.0.as_slice(), Type::BYTEA)],
            )
            .map_err(io)?;
        rows.first().map(|row| self.tenant_row(row)).transpose()
    }

    /// Every tenant, by name. The names are sealed, so the order is made
    /// here, by their bytes, as the local registry's is.
    fn tenants(&mut self) -> Result<Vec<TenantRow>, TenantError> {
        let rows = self
            .0
            .query(&format!("SELECT {TENANT_COLUMNS} FROM vcon.tenant"), &[])
            .map_err(io)?;
        let mut tenants = rows
            .iter()
            .map(|row| self.tenant_row(row))
            .collect::<Result<Vec<_>, _>>()?;
        tenants.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        Ok(tenants)
    }

    fn name_taken(&mut self, name: &str) -> Result<bool, TenantError> {
        let mac = self.name_mac(name)?;
        self.0
            .exists(
                "SELECT 1 FROM vcon.tenant WHERE name_mac = $1",
                &[(&mac.as_slice(), Type::BYTEA)],
            )
            .map_err(io)
    }

    fn insert_tenant(&mut self, row: &TenantRow) -> Result<(), TenantError> {
        let id = row.id.0.as_slice();
        let mac = self.name_mac(&row.name)?;
        let name = self.seal("tenant", "name_sealed", id, row.name.as_bytes())?;
        let retention =
            self.seal_some("tenant", "retention_sealed", id, row.retention.as_deref())?;
        let metadata = self.seal_some("tenant", "metadata_sealed", id, row.metadata.as_deref())?;
        let role_cap = self.seal_some("tenant", "role_cap_sealed", id, row.role_cap.as_deref())?;
        self.run(
            "INSERT INTO vcon.tenant (id, name_mac, name_sealed, retention_sealed, \
             metadata_sealed, role_cap_sealed, created_at, tenant_key_id, wrapped_tenant_key, \
             pending_wrapped_tenant_key, key_since)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                (&id, Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&name, Type::BYTEA),
                (&retention, Type::BYTEA),
                (&metadata, Type::BYTEA),
                (&role_cap, Type::BYTEA),
                (&row.created_at, Type::INT8),
                (&row.tenant_key_id.0.as_slice(), Type::BYTEA),
                (&row.wrapped, Type::BYTEA),
                (&row.pending, Type::BYTEA),
                (&row.key_since, Type::INT8),
            ],
        )
    }

    fn set_retention(
        &mut self,
        id: TenantId,
        retention: Option<&str>,
    ) -> Result<bool, TenantError> {
        self.set_sealed(id, "retention_sealed", retention)
    }

    fn set_metadata(&mut self, id: TenantId, level: Option<&str>) -> Result<bool, TenantError> {
        self.set_sealed(id, "metadata_sealed", level)
    }

    fn set_role_cap(&mut self, id: TenantId, cap: Option<&str>) -> Result<bool, TenantError> {
        self.set_sealed(id, "role_cap_sealed", cap)
    }

    fn delete_tenant(&mut self, id: TenantId, at: i64) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.tenant WHERE id = $1",
            &[(&id.0.as_slice(), Type::BYTEA)],
        )?;
        self.run(
            "UPDATE vcon.api_key SET revoked_at = $2 WHERE tenant = $1 AND revoked_at IS NULL",
            &[(&id.0.as_slice(), Type::BYTEA), (&at, Type::INT8)],
        )
    }

    fn set_pending(&mut self, id: TenantId, pending: &[u8]) -> Result<(), TenantError> {
        self.run(
            "UPDATE vcon.tenant SET pending_wrapped_tenant_key = $1 WHERE id = $2",
            &[(&pending, Type::BYTEA), (&id.0.as_slice(), Type::BYTEA)],
        )
    }

    fn promote_pending(&mut self, id: TenantId, key: KeyId, at: i64) -> Result<(), TenantError> {
        self.run(
            "UPDATE vcon.tenant SET tenant_key_id = $1,
                 wrapped_tenant_key = pending_wrapped_tenant_key,
                 pending_wrapped_tenant_key = NULL, key_since = $2
             WHERE id = $3",
            &[
                (&key.0.as_slice(), Type::BYTEA),
                (&at, Type::INT8),
                (&id.0.as_slice(), Type::BYTEA),
            ],
        )
    }

    fn master_wraps(&mut self, key: KeyId) -> Result<u64, TenantError> {
        let rows = self
            .0
            .query(
                "SELECT wraps FROM vcon.master_key_use WHERE master_key_id = $1",
                &[(&key.0.as_slice(), Type::BYTEA)],
            )
            .map_err(io)?;
        let wraps: Option<i64> = rows.first().map(|row| col(row, 0)).transpose()?;
        u64::try_from(wraps.unwrap_or(0)).map_err(io)
    }

    fn add_master_wrap(&mut self, key: KeyId, at: i64) -> Result<(), TenantError> {
        self.run(
            "INSERT INTO vcon.master_key_use AS u (master_key_id, wraps, since)
             VALUES ($1, 1, $2)
             ON CONFLICT (master_key_id) DO UPDATE SET wraps = u.wraps + 1",
            &[(&key.0.as_slice(), Type::BYTEA), (&at, Type::INT8)],
        )
    }

    fn master_since(&mut self, key: KeyId) -> Result<Option<i64>, TenantError> {
        let rows = self
            .0
            .query(
                "SELECT since FROM vcon.master_key_use WHERE master_key_id = $1",
                &[(&key.0.as_slice(), Type::BYTEA)],
            )
            .map_err(io)?;
        rows.first().map(|row| col(row, 0)).transpose()
    }

    fn key(&mut self, id: KeyId) -> Result<Option<KeyRow>, TenantError> {
        let rows = self
            .0
            .query(
                &format!("SELECT {KEY_COLUMNS} FROM vcon.api_key WHERE id = $1"),
                &[(&id.0.as_slice(), Type::BYTEA)],
            )
            .map_err(io)?;
        rows.first().map(|row| self.key_row(row)).transpose()
    }

    fn keys(&mut self, tenant: Option<TenantId>) -> Result<Vec<KeyRow>, TenantError> {
        let tenant = tenant.map(|t| t.0.to_vec());
        let rows = self
            .0
            .query(
                &format!(
                    "SELECT {KEY_COLUMNS} FROM vcon.api_key
                     WHERE $1 IS NULL OR tenant = $1 ORDER BY created_at, id"
                ),
                &[(&tenant, Type::BYTEA)],
            )
            .map_err(io)?;
        rows.iter().map(|row| self.key_row(row)).collect()
    }

    fn insert_key(&mut self, row: &KeyRow) -> Result<(), TenantError> {
        let id = row.id.0.as_slice();
        let tenant = row.tenant.map(|t| t.0.to_vec());
        let scopes = self.seal("api_key", "scopes_sealed", id, row.scopes.as_bytes())?;
        let label = self.seal("api_key", "label_sealed", id, row.label.as_bytes())?;
        self.run(
            &format!(
                "INSERT INTO vcon.api_key ({KEY_COLUMNS})
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
            ),
            &[
                (&id, Type::BYTEA),
                (&tenant, Type::BYTEA),
                (&row.token_hash, Type::BYTEA),
                (&scopes, Type::BYTEA),
                (&label, Type::BYTEA),
                (&row.created_at, Type::INT8),
                (&row.revoked_at, Type::INT8),
                (&row.expires_at, Type::INT8),
                (&row.last_used_at, Type::INT8),
            ],
        )
    }

    fn revoke_key(&mut self, id: KeyId, at: i64) -> Result<(), TenantError> {
        self.run(
            "UPDATE vcon.api_key SET revoked_at = $2 WHERE id = $1 AND revoked_at IS NULL",
            &[(&id.0.as_slice(), Type::BYTEA), (&at, Type::INT8)],
        )
    }

    fn touch_key(&mut self, id: KeyId, at: i64) -> Result<bool, TenantError> {
        let changed = self
            .0
            .execute(
                "UPDATE vcon.api_key SET last_used_at = $2
                 WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2)",
                &[(&id.0.as_slice(), Type::BYTEA), (&at, Type::INT8)],
            )
            .map_err(io)?;
        Ok(changed > 0)
    }

    fn user(&mut self, id: UserId) -> Result<Option<UserRow>, TenantError> {
        let rows = self
            .0
            .query(
                &format!("SELECT {USER_COLUMNS} FROM vcon.user_account WHERE id = $1"),
                &[(&id.0.as_slice(), Type::BYTEA)],
            )
            .map_err(io)?;
        rows.first().map(|row| self.user_row(row)).transpose()
    }

    fn users(&mut self) -> Result<Vec<UserRow>, TenantError> {
        let rows = self
            .0
            .query(
                &format!("SELECT {USER_COLUMNS} FROM vcon.user_account ORDER BY created_at, id"),
                &[],
            )
            .map_err(io)?;
        rows.iter().map(|row| self.user_row(row)).collect()
    }

    fn identity_taken(&mut self, issuer: &str, subject: &str) -> Result<bool, TenantError> {
        let keys = self.0.keys().map_err(io)?;
        let mac = column::mac(keys, &identity_bytes(issuer, subject));
        self.0
            .exists(
                "SELECT 1 FROM vcon.user_account
                 WHERE identity_mac = $1 AND deactivated_at IS NULL",
                &[(&mac.as_slice(), Type::BYTEA)],
            )
            .map_err(io)
    }

    fn insert_user(&mut self, row: &UserRow) -> Result<(), TenantError> {
        let id = row.id.0.as_slice();
        let label = self.seal("user_account", "label_sealed", id, row.label.as_bytes())?;
        let (mac, identity) = match &row.identity {
            Some((issuer, subject)) => {
                let bytes = identity_bytes(issuer, subject);
                let mac = column::mac(self.0.keys().map_err(io)?, &bytes);
                let sealed = self.seal("user_account", "identity_sealed", id, &bytes)?;
                (Some(mac.to_vec()), Some(sealed))
            }
            None => (None, None),
        };
        self.run(
            "INSERT INTO vcon.user_account
                 (id, label_sealed, identity_mac, identity_sealed, created_at, deactivated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                (&id, Type::BYTEA),
                (&label, Type::BYTEA),
                (&mac, Type::BYTEA),
                (&identity, Type::BYTEA),
                (&row.created_at, Type::INT8),
                (&row.deactivated_at, Type::INT8),
            ],
        )
    }

    fn deactivate_user(&mut self, id: UserId, at: i64) -> Result<(), TenantError> {
        self.run(
            "UPDATE vcon.user_account SET deactivated_at = $2
             WHERE id = $1 AND deactivated_at IS NULL",
            &[(&id.0.as_slice(), Type::BYTEA), (&at, Type::INT8)],
        )
    }

    fn membership(
        &mut self,
        user: UserId,
        tenant: Option<TenantId>,
    ) -> Result<Option<MembershipRow>, TenantError> {
        let tenant = tenant.map(|t| t.0.to_vec());
        let rows = self
            .0
            .query(
                &format!("SELECT {MEMBERSHIP_COLUMNS} FROM vcon.membership WHERE {MEMBERSHIP_IS}"),
                &[(&user.0.as_slice(), Type::BYTEA), (&tenant, Type::BYTEA)],
            )
            .map_err(io)?;
        rows.first().map(|row| self.membership_row(row)).transpose()
    }

    /// Every membership, by user, then time, then tenant, an operator's
    /// before a tenant's at one time, as the local registry lists them.
    fn memberships(&mut self) -> Result<Vec<MembershipRow>, TenantError> {
        let rows = self
            .0
            .query(
                &format!(
                    "SELECT {MEMBERSHIP_COLUMNS} FROM vcon.membership
                     ORDER BY user_id, granted_at, tenant NULLS FIRST"
                ),
                &[],
            )
            .map_err(io)?;
        rows.iter().map(|row| self.membership_row(row)).collect()
    }

    fn insert_membership(&mut self, row: &MembershipRow) -> Result<(), TenantError> {
        let key = membership_key(row.user, row.tenant);
        let tenant = row.tenant.map(|t| t.0.to_vec());
        let roles = self.seal("membership", "roles_sealed", &key, row.roles.as_bytes())?;
        self.run(
            &format!("INSERT INTO vcon.membership ({MEMBERSHIP_COLUMNS}) VALUES ($1, $2, $3, $4)"),
            &[
                (&row.user.0.as_slice(), Type::BYTEA),
                (&tenant, Type::BYTEA),
                (&roles, Type::BYTEA),
                (&row.granted_at, Type::INT8),
            ],
        )
    }

    fn set_roles(&mut self, row: &MembershipRow) -> Result<(), TenantError> {
        let key = membership_key(row.user, row.tenant);
        let tenant = row.tenant.map(|t| t.0.to_vec());
        let roles = self.seal("membership", "roles_sealed", &key, row.roles.as_bytes())?;
        self.run(
            &format!("UPDATE vcon.membership SET roles_sealed = $3 WHERE {MEMBERSHIP_IS}"),
            &[
                (&row.user.0.as_slice(), Type::BYTEA),
                (&tenant, Type::BYTEA),
                (&roles, Type::BYTEA),
            ],
        )
    }

    fn delete_membership(
        &mut self,
        user: UserId,
        tenant: Option<TenantId>,
    ) -> Result<(), TenantError> {
        let tenant = tenant.map(|t| t.0.to_vec());
        self.run(
            &format!("DELETE FROM vcon.membership WHERE {MEMBERSHIP_IS}"),
            &[(&user.0.as_slice(), Type::BYTEA), (&tenant, Type::BYTEA)],
        )
    }

    fn delete_memberships_of_user(&mut self, user: UserId) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.membership WHERE user_id = $1",
            &[(&user.0.as_slice(), Type::BYTEA)],
        )
    }

    fn delete_memberships_in(&mut self, tenant: TenantId) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.membership WHERE tenant = $1",
            &[(&tenant.0.as_slice(), Type::BYTEA)],
        )
    }

    fn session(&mut self, id_hash: &[u8]) -> Result<Option<SessionRow>, TenantError> {
        let rows = self
            .0
            .query(
                &format!("SELECT {SESSION_COLUMNS} FROM vcon.session WHERE id_hash = $1"),
                &[(&id_hash, Type::BYTEA)],
            )
            .map_err(io)?;
        rows.first().map(|row| self.session_row(row)).transpose()
    }

    fn insert_session(&mut self, row: &SessionRow) -> Result<(), TenantError> {
        let id = row.id_hash.as_slice();
        let tenant = row.tenant.map(|t| t.0.to_vec());
        let scopes = self.seal("session", "scopes_sealed", id, row.scopes.as_bytes())?;
        let amr = self.seal("session", "amr_sealed", id, row.amr.as_bytes())?;
        self.run(
            &format!(
                "INSERT INTO vcon.session ({SESSION_COLUMNS})
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
            ),
            &[
                (&id, Type::BYTEA),
                (&row.user.0.as_slice(), Type::BYTEA),
                (&tenant, Type::BYTEA),
                (&scopes, Type::BYTEA),
                (&row.created_at, Type::INT8),
                (&row.last_seen_at, Type::INT8),
                (&row.idle_expires_at, Type::INT8),
                (&row.expires_at, Type::INT8),
                (&row.idle_seconds, Type::INT8),
                (&amr, Type::BYTEA),
                (&row.csrf_hash, Type::BYTEA),
            ],
        )
    }

    fn touch_session(
        &mut self,
        id_hash: &[u8],
        last_seen_at: i64,
        idle_expires_at: i64,
    ) -> Result<(), TenantError> {
        self.run(
            "UPDATE vcon.session SET last_seen_at = $2, idle_expires_at = $3 WHERE id_hash = $1",
            &[
                (&id_hash, Type::BYTEA),
                (&last_seen_at, Type::INT8),
                (&idle_expires_at, Type::INT8),
            ],
        )
    }

    fn delete_session(&mut self, id_hash: &[u8]) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.session WHERE id_hash = $1",
            &[(&id_hash, Type::BYTEA)],
        )
    }

    fn delete_sessions_of_user(&mut self, user: UserId) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.session WHERE user_id = $1",
            &[(&user.0.as_slice(), Type::BYTEA)],
        )
    }

    fn delete_sessions_in(&mut self, tenant: TenantId) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.session WHERE tenant = $1",
            &[(&tenant.0.as_slice(), Type::BYTEA)],
        )
    }

    fn delete_sessions_ended(&mut self, at: i64) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.session WHERE idle_expires_at <= $1 OR expires_at <= $1",
            &[(&at, Type::INT8)],
        )
    }

    /// A tenant's settings by name, in the order of the names' bytes, as
    /// the local registry lists them.
    fn settings(&mut self, tenant: TenantId) -> Result<Vec<(String, String)>, TenantError> {
        let rows = self
            .0
            .query(
                "SELECT name, value_sealed FROM vcon.tenant_setting
                 WHERE tenant = $1 ORDER BY name COLLATE \"C\"",
                &[(&tenant.0.as_slice(), Type::BYTEA)],
            )
            .map_err(io)?;
        rows.iter()
            .map(|row| {
                let name: String = col(row, 0)?;
                let value = self.text(
                    "tenant_setting",
                    "value_sealed",
                    &setting_key(tenant, &name),
                    &col::<Vec<u8>>(row, 1)?,
                )?;
                Ok((name, value))
            })
            .collect()
    }

    fn put_setting(
        &mut self,
        tenant: TenantId,
        name: &str,
        value: &str,
    ) -> Result<(), TenantError> {
        let sealed = self.seal(
            "tenant_setting",
            "value_sealed",
            &setting_key(tenant, name),
            value.as_bytes(),
        )?;
        self.run(
            "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) VALUES ($1, $2, $3)
             ON CONFLICT (tenant, name) DO UPDATE SET value_sealed = EXCLUDED.value_sealed",
            &[
                (&tenant.0.as_slice(), Type::BYTEA),
                (&name, Type::TEXT),
                (&sealed, Type::BYTEA),
            ],
        )
    }

    fn delete_setting(&mut self, tenant: TenantId, name: &str) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.tenant_setting WHERE tenant = $1 AND name = $2",
            &[(&tenant.0.as_slice(), Type::BYTEA), (&name, Type::TEXT)],
        )
    }

    fn delete_settings_in(&mut self, tenant: TenantId) -> Result<(), TenantError> {
        self.run(
            "DELETE FROM vcon.tenant_setting WHERE tenant = $1",
            &[(&tenant.0.as_slice(), Type::BYTEA)],
        )
    }

    fn record_times(&mut self, action: Action, status: u16) -> Result<Vec<String>, TenantError> {
        let status = i64::from(status);
        let rows = self
            .0
            .query(
                "SELECT at FROM vcon.admin_audit WHERE action = $1 AND status = $2",
                &[(&action.as_str(), Type::TEXT), (&status, Type::INT8)],
            )
            .map_err(io)?;
        rows.iter().map(|row| col(row, 0)).collect()
    }

    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        self.0.record(entry, now)
    }

    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        self.0.count_record(entry, now, 1)
    }

    fn count_many(
        &mut self,
        entry: &Entry,
        now: OffsetDateTime,
        times: u64,
    ) -> Result<(), AuditError> {
        self.0.count_record(entry, now, times)
    }

    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        self.0.audit_lines(after, limit.map(i64::from), out)
    }

    fn commit(self: Box<Self>) -> Result<(), CommitError> {
        self.0.finish()
    }
}
