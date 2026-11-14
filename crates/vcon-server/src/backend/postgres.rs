//! The PostgreSQL catalog (cargo feature `postgres`), a preview: the
//! connector, the pool, and the registry and tenant catalogs kept in a
//! database, which one server serves.
//!
//! - Every connection is TLS, with the server's certificate chain and host
//!   name verified by rustls against `catalog.postgres.ca_file` or the
//!   system's certificate authorities; a server that does not offer TLS is
//!   refused.
//! - With a password, the only login accepted is SCRAM-SHA-256 with channel
//!   binding (SCRAM-SHA-256-PLUS): tokio-postgres's `ChannelBinding::Require`
//!   refuses an `md5`, `password` or `trust` answer before it sends the
//!   password. With a client certificate and no password, the server's
//!   `cert` and `trust` methods both answer AuthenticationOk, which a
//!   client cannot tell apart.
//! - A standby, a server with `transaction_read_only` on, a superuser and a
//!   role with BYPASSRLS are refused on each new connection.
//! - The secret files are read once, before any connection
//!   ([`secrets::Secrets::read`]); the pool reconnects from memory.
//!
//! - [`schema`] makes the catalog's schema, for `vcon-server migrate`, and
//!   a serving connection refuses any version but this binary's, and a role
//!   that could change the schema.
//!
//! - [`Database`] is a process's connections to the catalog database, with
//!   the locks of its work held on one of them ([`Holds`]): a server's,
//!   those of a command beside a server, or those of a command alone.
//! - [`PgClaim`] keeps the claim of the bucket's prefix in the database.

mod catalog;
mod claim;
mod connect;
mod pool;
mod registry;
mod schema;
mod secrets;
mod space;
#[cfg(test)]
mod tests;

use crate::config::{PostgresSettings, StorageError};
use crate::tenants::{RegistryError, TenantError};

pub(crate) use catalog::Database;
pub(crate) use claim::PgClaim;
use connect::{Connector, Purpose};
pub(crate) use registry::PgRegistry;
pub(crate) use schema::Holds;
use secrets::Secrets;

/// The registry and the tenants' space in `database`, as
/// `Tenants::with_backends` takes them: the registry under `catalog_key`,
/// the master key's catalog key, and each tenant's catalog rows in the
/// database with its objects in the bucket of `objects`.
///
/// # Errors
///
/// [`RegistryError::WrongDatabaseKey`] when the database names another
/// key. [`RegistryError::Database`] when the objects are not in a bucket,
/// which the configuration refuses first, and when the registry cannot be
/// read or written. Nothing is written for either refusal.
pub(crate) fn open(
    database: &std::sync::Arc<Database>,
    catalog_key: &crate::envelope::Key,
    objects: super::ObjectPlace,
    fs: std::sync::Arc<dyn crate::fs::Fs>,
) -> Result<super::Backends, RegistryError> {
    let super::ObjectPlace::S3(s3) = objects else {
        return Err(RegistryError::Database(
            "catalog.backend = \"postgres\" needs storage.objects = \"s3\"".to_owned(),
        ));
    };
    let registry = PgRegistry::open(database.clone(), catalog_key).map_err(|e| match e {
        TenantError::WrongKey => RegistryError::WrongDatabaseKey,
        _ => RegistryError::Database(
            "the tables of the schema vcon could not be read or written, or a key of the \
             registry's audit log does not open"
                .to_owned(),
        ),
    })?;
    let space = space::PgTenantSpace::new(database.clone(), s3, fs);
    Ok((Box::new(registry), Box::new(space)))
}

/// `vcon-server migrate`: applies the migrations this database has not
/// yet, and says what it did.
///
/// # Errors
///
/// A `StorageError` naming the step that refused.
pub(crate) async fn migrate(settings: &PostgresSettings) -> Result<schema::Migrated, StorageError> {
    let secrets = Secrets::read(settings)?;
    let connector = Connector::new(settings, secrets, Purpose::Migrate)?;
    let mut connected = connector
        .connect()
        .await
        .map_err(|e| StorageError(format!("catalog.postgres: {e}")))?;
    schema::run(&mut connected.client, schema::MIGRATIONS)
        .await
        .map_err(|why| StorageError(format!("catalog.postgres: {why}")))
}
