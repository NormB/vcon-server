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

use super::catalog::{
    AuditKey, Begin, Database, PgTx, all_changed, audit_key_id, new_audit_key, open_audit_keys,
    reseal_audit_keys,
};
use crate::audit::{Action, Actor, AuditError, Entry};
use crate::backend::{
    CommitError, KeyRow, MembershipRow, Registry, RegistryTx, SessionRow, TenantRow, UserRow,
};
use crate::clock::nanos;
use crate::envelope::{self as column, ColumnKeys, Key, REGISTRY};
use crate::ids::{KeyId, TenantId, UserId};
use crate::tenants::TenantError;

/// The registry in the catalog database.
pub(crate) struct PgRegistry {
    database: Arc<Database>,
    keys: ColumnKeys,
    /// The keys of the registry's audit log that opened, oldest first:
    /// the last seals the new records (schema version 12).
    audit: Vec<AuditKey>,
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

/// A column of a registry table's primary key, as a rotation of the
/// master key reads it and finds the row by it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyColumn {
    /// Bytes, never NULL.
    Bytes(&'static str),
    /// Bytes, or NULL.
    BytesOrNone(&'static str),
    /// Text, never NULL.
    Text(&'static str),
}

/// A column a rotation of the master key makes again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Column {
    /// A sealed value, or NULL.
    Sealed(&'static str),
    /// A sealed value, or NULL, and the column of its HMAC.
    SealedWithMac {
        /// The sealed copy.
        sealed: &'static str,
        /// The HMAC the value is found by.
        mac: &'static str,
    },
    /// A tenant key wrapped under the master key, or NULL.
    TenantKey(&'static str),
}

/// A table of the registry whose columns a rotation makes again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Remade {
    /// The table, in the schema `vcon`.
    pub(super) table: &'static str,
    /// The columns of its primary key, in the order their bytes make the
    /// row's key in a sealed column's context.
    pub(super) key: &'static [KeyColumn],
    /// The columns made again.
    pub(super) columns: &'static [Column],
}

/// One value of a row's primary key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum KeyValue {
    /// Bytes, or NULL.
    Bytes(Option<Vec<u8>>),
    /// Text.
    Text(String),
}

/// Every table a rotation of the master key makes again, with every
/// column of it that is sealed, is an HMAC or holds a wrapped key. The
/// registry's log is not here: no record of it is written again, and its
/// keys are sealed again by the function a tenant's log has.
pub(super) const REMADE: [Remade; 6] = [
    Remade {
        table: "tenant",
        key: &[KeyColumn::Bytes("id")],
        columns: &[
            Column::SealedWithMac {
                sealed: "name_sealed",
                mac: "name_mac",
            },
            Column::Sealed("retention_sealed"),
            Column::Sealed("metadata_sealed"),
            Column::Sealed("role_cap_sealed"),
            Column::TenantKey("wrapped_tenant_key"),
            Column::TenantKey("pending_wrapped_tenant_key"),
        ],
    },
    Remade {
        table: "api_key",
        key: &[KeyColumn::Bytes("id")],
        columns: &[
            Column::Sealed("scopes_sealed"),
            Column::Sealed("label_sealed"),
        ],
    },
    Remade {
        table: "user_account",
        key: &[KeyColumn::Bytes("id")],
        columns: &[
            Column::Sealed("label_sealed"),
            Column::SealedWithMac {
                sealed: "identity_sealed",
                mac: "identity_mac",
            },
        ],
    },
    Remade {
        table: "membership",
        key: &[
            KeyColumn::Bytes("user_id"),
            KeyColumn::BytesOrNone("tenant"),
        ],
        columns: &[Column::Sealed("roles_sealed")],
    },
    Remade {
        table: "tenant_setting",
        key: &[KeyColumn::Bytes("tenant"), KeyColumn::Text("name")],
        columns: &[Column::Sealed("value_sealed")],
    },
    Remade {
        table: "session",
        key: &[KeyColumn::Bytes("id_hash")],
        columns: &[
            Column::Sealed("scopes_sealed"),
            Column::Sealed("amr_sealed"),
        ],
    },
];

/// The row's key in a sealed column's context: the bytes of each value
/// of its primary key, one after the other. A NULL adds none, and a text
/// its UTF-8. [`membership_key`] and [`setting_key`] give the same bytes
/// for the rows they name.
pub(super) fn row_key(values: &[KeyValue]) -> Vec<u8> {
    let mut key = Vec::new();
    for value in values {
        match value {
            KeyValue::Bytes(Some(bytes)) => key.extend_from_slice(bytes),
            KeyValue::Bytes(None) => {}
            KeyValue::Text(text) => key.extend_from_slice(text.as_bytes()),
        }
    }
    key
}

impl KeyColumn {
    fn name(self) -> &'static str {
        match self {
            KeyColumn::Bytes(name) | KeyColumn::BytesOrNone(name) | KeyColumn::Text(name) => name,
        }
    }
}

impl Remade {
    /// The columns a rotation writes, in the order it reads them: an
    /// HMAC before the sealed copy it is made from.
    fn written(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        for column in self.columns {
            match *column {
                Column::Sealed(name) | Column::TenantKey(name) => names.push(name),
                Column::SealedWithMac { sealed, mac } => names.extend([mac, sealed]),
            }
        }
        names
    }

    /// The names of the key's columns, then of the columns written.
    fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<&str> = self.key.iter().map(|column| column.name()).collect();
        names.extend(self.written());
        names
    }

    /// The statement of the cursor that reads the table's rows.
    pub(super) fn fetch_text(&self) -> String {
        format!(
            "SELECT {} FROM vcon.{}",
            self.names().join(", "),
            self.table
        )
    }

    /// The statement that writes the rows of one batch: one array for
    /// each column [`Remade::fetch_text`] reads, and each row found by
    /// its key.
    pub(super) fn update_text(&self) -> String {
        let written = self.written();
        let set: Vec<String> = written.iter().map(|c| format!("{c} = n.{c}")).collect();
        let kinds = self
            .key
            .iter()
            .map(|column| match column {
                KeyColumn::Text(_) => "text",
                KeyColumn::Bytes(_) | KeyColumn::BytesOrNone(_) => "bytea",
            })
            .chain(written.iter().map(|_| "bytea"));
        let arrays: Vec<String> = kinds
            .enumerate()
            .map(|(at, kind)| format!("${}::{kind}[]", at + 1))
            .collect();
        let found: Vec<String> = self
            .key
            .iter()
            .map(|column| match *column {
                KeyColumn::BytesOrNone(c) => format!("t.{c} IS NOT DISTINCT FROM n.{c}"),
                KeyColumn::Bytes(c) | KeyColumn::Text(c) => format!("t.{c} = n.{c}"),
            })
            .collect();
        format!(
            "UPDATE vcon.{} t SET {} FROM unnest({}) AS n({}) WHERE {}",
            self.table,
            set.join(", "),
            arrays.join(", "),
            self.names().join(", "),
            found.join(" AND ")
        )
    }
}

/// The values of one column of a primary key, for the rows of a batch.
enum KeyArray {
    Bytes(Vec<Option<Vec<u8>>>),
    Text(Vec<String>),
}

impl KeyArray {
    fn of(column: KeyColumn) -> KeyArray {
        match column {
            KeyColumn::Text(_) => KeyArray::Text(Vec::new()),
            KeyColumn::Bytes(_) | KeyColumn::BytesOrNone(_) => KeyArray::Bytes(Vec::new()),
        }
    }

    /// Adds a row's value; false when it is not of the column's kind.
    fn push(&mut self, value: KeyValue) -> bool {
        match (self, value) {
            (KeyArray::Bytes(all), KeyValue::Bytes(bytes)) => all.push(bytes),
            (KeyArray::Text(all), KeyValue::Text(text)) => all.push(text),
            _ => return false,
        }
        true
    }

    fn param(&self) -> (&(dyn tokio_postgres::types::ToSql + Sync), Type) {
        match self {
            KeyArray::Bytes(all) => (all, Type::BYTEA_ARRAY),
            KeyArray::Text(all) => (all, Type::TEXT_ARRAY),
        }
    }
}

impl PgRegistry {
    /// The registry in `database`, its columns sealed under the keys
    /// derived from `catalog_key`, the master key's catalog key.
    ///
    /// The database names the key the registry is under, in the one row
    /// of `vcon.registry_key`. The first open of a database writes that
    /// row and makes the key of the registry's audit log, with the
    /// registry's lock held, so of several first opens at once one
    /// writes; every later open compares the id and opens the log's keys.
    ///
    /// # Errors
    ///
    /// [`TenantError::WrongKey`] when the database names another key, or
    /// names none and holds a row that does not open under this one.
    /// [`TenantError::Io`] when the database cannot be read or written,
    /// when a key's page cannot be locked, and when the newest key of the
    /// audit log is missing or does not open.
    pub(crate) fn open(
        database: Arc<Database>,
        catalog_key: &Key,
    ) -> Result<PgRegistry, TenantError> {
        let mut registry = PgRegistry {
            database,
            keys: ColumnKeys::derive(catalog_key).map_err(io)?,
            audit: Vec::new(),
            #[cfg(test)]
            fault: std::sync::Mutex::new(None),
        };
        let ours = catalog_key.id().0;
        let audit = {
            let read = registry.begin(Begin::Read)?;
            match read.named()? {
                Some(held) if held == ours => read.log_keys()?,
                Some(_) => return Err(TenantError::WrongKey),
                None => {
                    // The read ends before the write begins: one
                    // connection at a time.
                    drop(read);
                    registry.claim(&ours)?
                }
            }
        };
        if audit.is_empty() {
            // A log with no key takes no record.
            return Err(TenantError::Io);
        }
        registry.audit = audit;
        Ok(registry)
    }

    /// The first open: names the key `ours` in the database and gives the
    /// audit log its first key, in one write, which holds the registry's
    /// lock. A row another open wrote while this one waited is compared,
    /// not written again. Gives the keys of the log.
    fn claim(&self, ours: &[u8]) -> Result<Vec<AuditKey>, TenantError> {
        let tx = self.begin(Begin::Write)?;
        match tx.named()? {
            Some(held) if held == ours => return tx.log_keys(),
            Some(_) => return Err(TenantError::WrongKey),
            None => {}
        }
        // No id is named. Rows that are there were sealed by a registry
        // of before schema version 12, under its key: this key must open
        // them, or it is not that key.
        if !tx.opens_what_is_there()? {
            return Err(TenantError::WrongKey);
        }
        let mut audit = tx.log_keys().map_err(|_| TenantError::WrongKey)?;
        if audit.is_empty() {
            let (key, wrapped) = new_audit_key(REGISTRY, &self.keys).map_err(io)?;
            tx.0.execute(
                "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) VALUES ($1, 1, $2)",
                &[(&audit_key_id(&key), Type::BYTEA), (&wrapped, Type::BYTEA)],
            )
            .map_err(io)?;
            audit.push(key);
        }
        tx.0.execute(
            "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ($1)",
            &[(&ours, Type::BYTEA)],
        )
        .map_err(io)?;
        Box::new(tx).commit().map_err(io)?;
        Ok(audit)
    }

    /// Moves the registry in `database` from the master key `old` to
    /// `new`, in one transaction that holds the registry's lock: every
    /// tenant key, a pending one too, is wrapped again under `new`; every
    /// sealed column and every HMAC of the tables in [`REMADE`] is made
    /// again with the keys of `new`'s catalog key; each key of the
    /// registry's audit log is sealed again; the rows of the master
    /// keys' use give way to one for `new`, which counts the tenant keys
    /// wrapped and is in use since `now`; the database then names `new`'s
    /// catalog key, and the rotation is recorded in the registry's log.
    /// It reads and writes at most `batch` rows of a table in one
    /// statement.
    ///
    /// No audit record is changed: a record's target is sealed under the
    /// log's audit key, which stays the same. A registry that is under
    /// `new` already is left as it is, so a rotation that was made can be
    /// asked for again. A database that names no key is first opened
    /// under `old`, as [`PgRegistry::open`] opens it. Every server is
    /// stopped for a rotation: an open registry holds the keys it was
    /// opened with.
    ///
    /// # Errors
    ///
    /// [`TenantError::WrongKey`] when the registry is under neither key.
    /// [`TenantError::Io`] when a value does not open under `old`, the
    /// database cannot be read or written, or `batch` is not positive,
    /// and [`TenantError::AuditFailed`] when the record cannot be
    /// written. After an error nothing is changed, unless the commit got
    /// no answer: a second run then finds which key the registry is
    /// under.
    pub(crate) fn rekey_master(
        database: &Arc<Database>,
        old: &Key,
        new: &Key,
        now: OffsetDateTime,
        batch: i64,
    ) -> Result<(), TenantError> {
        if batch < 1 {
            return Err(TenantError::Io);
        }
        let old_catalog = column::catalog_key(old).map_err(io)?;
        match PgRegistry::open(database.clone(), &old_catalog) {
            Ok(registry) => registry.rotate(old, new, now, batch),
            Err(TenantError::WrongKey) => {
                // Under the new key, a rotation was made already.
                let new_catalog = column::catalog_key(new).map_err(io)?;
                PgRegistry::open(database.clone(), &new_catalog).map(drop)
            }
            Err(e) => Err(e),
        }
    }

    /// The rotation [`PgRegistry::rekey_master`] describes, of this
    /// registry, which was opened under `old`'s catalog key. The id the
    /// database names is compared again with the registry's lock held:
    /// another run may have made the rotation since the open.
    pub(super) fn rotate(
        &self,
        old: &Key,
        new: &Key,
        now: OffsetDateTime,
        batch: i64,
    ) -> Result<(), TenantError> {
        if batch < 1 {
            return Err(TenantError::Io);
        }
        let old_id = column::catalog_key(old).map_err(io)?.id().0;
        let new_catalog = column::catalog_key(new).map_err(io)?;
        let new_id = new_catalog.id().0;
        let to = ColumnKeys::derive(&new_catalog).map_err(io)?;
        let tx = self.begin(Begin::Write)?;
        match tx.named()? {
            Some(held) if held == new_id => return Ok(()),
            Some(held) if held == old_id => {}
            _ => return Err(TenantError::WrongKey),
        }
        reseal_audit_keys(&tx.0, &to).map_err(io)?;
        let mut wraps = 0_i64;
        for table in &REMADE {
            wraps += tx.remake(table, &to, old, new, batch)?;
        }
        tx.run("DELETE FROM vcon.master_key_use", &[])?;
        tx.run(
            "INSERT INTO vcon.master_key_use (master_key_id, wraps, since) VALUES ($1, $2, $3)",
            &[
                (&new.id().0.as_slice(), Type::BYTEA),
                (&wraps, Type::INT8),
                (&nanos(now), Type::INT8),
            ],
        )?;
        let named =
            tx.0.execute(
                "UPDATE vcon.registry_key SET catalog_key_id = $1 WHERE catalog_key_id = $2",
                &[
                    (&new_id.as_slice(), Type::BYTEA),
                    (&old_id.as_slice(), Type::BYTEA),
                ],
            )
            .map_err(io)?;
        all_changed(named, 1).map_err(io)?;
        let entry = Entry {
            actor: Actor::Cli,
            action: Action::RekeyMaster,
            target: None,
            version: None,
            status: 200,
        };
        tx.0.record(&entry, now)
            .map_err(|_| TenantError::AuditFailed)?;
        tx.0.finish().map_err(io)
    }

    fn begin(&self, how: Begin) -> Result<PgRegistryTx<'_>, TenantError> {
        let lease = self.database.lease().map_err(io)?;
        let tx = PgTx::begin(&self.database, lease, REGISTRY, Some(&self.keys), how)
            .with_audit(&self.audit);
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
    /// The id of the catalog key the database names for the registry, or
    /// None when it names none yet.
    fn named(&self) -> Result<Option<Vec<u8>>, TenantError> {
        let rows = self
            .0
            .query("SELECT catalog_key_id FROM vcon.registry_key", &[])
            .map_err(io)?;
        rows.first().map(|row| col(row, 0)).transpose()
    }

    /// The keys of the registry's audit log that open under the
    /// registry's keys, oldest first; none when the log has no key.
    ///
    /// # Errors
    ///
    /// [`TenantError::Io`] when the newest does not open.
    fn log_keys(&self) -> Result<Vec<AuditKey>, TenantError> {
        let rows = self
            .0
            .query(
                "SELECT key_id, wrapped FROM vcon.admin_audit_key ORDER BY ordinal",
                &[],
            )
            .map_err(io)?;
        open_audit_keys(REGISTRY, self.0.keys().map_err(io)?, &rows).map_err(io)
    }

    /// Whether the registry's keys open the rows that are there: one
    /// tenant, one API key and one user, each the first by its id. Every
    /// other sealed row of the registry belongs to one of those three
    /// tables' rows (a setting to a tenant, a membership and a session to
    /// a user), and all were sealed under one key.
    fn opens_what_is_there(&self) -> Result<bool, TenantError> {
        let first = |columns: &str, table: &str| {
            self.0
                .query(
                    &format!("SELECT {columns} FROM vcon.{table} ORDER BY id LIMIT 1"),
                    &[],
                )
                .map_err(io)
        };
        let tenant = first(TENANT_COLUMNS, "tenant")?;
        let key = first(KEY_COLUMNS, "api_key")?;
        let user = first(USER_COLUMNS, "user_account")?;
        Ok(tenant
            .first()
            .is_none_or(|row| self.tenant_row(row).is_ok())
            && key.first().is_none_or(|row| self.key_row(row).is_ok())
            && user.first().is_none_or(|row| self.user_row(row).is_ok()))
    }

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

    /// Opens `sealed`, a value of `table.column` in the row `row`, and
    /// seals it again with the keys `to`: the plain value and its new
    /// sealed copy.
    fn sealed_again(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        sealed: &[u8],
        to: &ColumnKeys,
    ) -> Result<(Vec<u8>, Vec<u8>), TenantError> {
        let plain = self.0.unseal(table, column, row, sealed).map_err(io)?;
        let ctx = column::ColumnContext {
            tenant: REGISTRY,
            table,
            column,
            row,
        };
        let again = column::seal_column(to, &ctx, &plain).map_err(io)?;
        Ok((plain, again))
    }

    /// Makes every column of `spec` again in every row of its table:
    /// a sealed value with the keys `to`, an HMAC from the value it is
    /// of, and a tenant key from the master key `old` to `new`. The rows
    /// are read through a cursor, `batch` at a time, and each batch is
    /// written in one statement. Gives the number of tenant keys wrapped.
    fn remake(
        &self,
        spec: &Remade,
        to: &ColumnKeys,
        old: &Key,
        new: &Key,
        batch: i64,
    ) -> Result<i64, TenantError> {
        let tx = &self.0;
        tx.batch(&format!(
            "DECLARE rekey_rows NO SCROLL CURSOR FOR {}",
            spec.fetch_text()
        ))
        .map_err(io)?;
        let fetch = format!("FETCH {batch} FROM rekey_rows");
        let update = spec.update_text();
        let columns = spec.written().len();
        let mut wraps = 0_i64;
        loop {
            let rows = tx.query(&fetch, &[]).map_err(io)?;
            if rows.is_empty() {
                break;
            }
            let mut keys: Vec<KeyArray> = spec.key.iter().map(|c| KeyArray::of(*c)).collect();
            let mut made: Vec<Vec<Option<Vec<u8>>>> = vec![Vec::new(); columns];
            for row in &rows {
                let mut values = Vec::with_capacity(spec.key.len());
                for (at, column) in spec.key.iter().enumerate() {
                    values.push(match column {
                        KeyColumn::Text(_) => KeyValue::Text(col(row, at)?),
                        KeyColumn::Bytes(_) | KeyColumn::BytesOrNone(_) => {
                            KeyValue::Bytes(col(row, at)?)
                        }
                    });
                }
                let key = row_key(&values);
                // Where the column is in the row read, and in `made`.
                let mut at = spec.key.len();
                let mut out = 0;
                for column in spec.columns {
                    match *column {
                        Column::Sealed(name) => {
                            let sealed: Option<Vec<u8>> = col(row, at)?;
                            let again = sealed
                                .map(|s| self.sealed_again(spec.table, name, &key, &s, to))
                                .transpose()?;
                            made[out].push(again.map(|(_, sealed)| sealed));
                            at += 1;
                            out += 1;
                        }
                        Column::SealedWithMac { sealed: name, .. } => {
                            let sealed: Option<Vec<u8>> = col(row, at + 1)?;
                            let again = sealed
                                .map(|s| self.sealed_again(spec.table, name, &key, &s, to))
                                .transpose()?;
                            let (mac, sealed) = match again {
                                Some((plain, sealed)) => {
                                    (Some(column::mac(to, &plain).to_vec()), Some(sealed))
                                }
                                None => (None, None),
                            };
                            made[out].push(mac);
                            made[out + 1].push(sealed);
                            at += 2;
                            out += 2;
                        }
                        Column::TenantKey(_) => {
                            let wrapped: Option<Vec<u8>> = col(row, at)?;
                            let again = wrapped
                                .map(|wrapped| {
                                    let Some(KeyValue::Bytes(Some(id))) = values.first() else {
                                        return Err(TenantError::Io);
                                    };
                                    let ctx = column::Context::Tenant {
                                        tenant: TenantId(fixed(id)?),
                                    };
                                    column::rewrap(old, new, &ctx, &wrapped).map_err(io)
                                })
                                .transpose()?;
                            wraps += i64::from(again.is_some());
                            made[out].push(again);
                            at += 1;
                            out += 1;
                        }
                    }
                }
                for (array, value) in keys.iter_mut().zip(values) {
                    if !array.push(value) {
                        return Err(TenantError::Io);
                    }
                }
            }
            let mut params: Vec<_> = keys.iter().map(KeyArray::param).collect();
            for values in &made {
                params.push((values, Type::BYTEA_ARRAY));
            }
            let changed = tx.execute(&update, &params).map_err(io)?;
            all_changed(changed, rows.len()).map_err(io)?;
        }
        tx.batch("CLOSE rekey_rows").map_err(io)?;
        Ok(wraps)
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
