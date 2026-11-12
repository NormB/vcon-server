//! The PostgreSQL catalog (cargo feature `postgres`), a preview: the
//! connector and the pool the catalog's statements will run on.
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
//! No catalog statement runs here yet: [`preview`] connects, checks the
//! server, and refuses to go on.

mod catalog;
mod connect;
mod pool;
mod schema;
mod secrets;
#[cfg(test)]
mod tests;

use crate::config::{PostgresSettings, StorageError};

use connect::{Connector, Purpose};
use pool::Pool;
use secrets::Secrets;

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

/// What a start with a PostgreSQL catalog ends with in this release: the
/// secret files read, one connection made and the server checked, then a
/// refusal that names the server reached, since the catalogs are not kept
/// there yet. Every other outcome is the refusal of the step that failed.
pub(crate) async fn preview(settings: &PostgresSettings) -> StorageError {
    let connector =
        match Secrets::read(settings).and_then(|s| Connector::new(settings, s, Purpose::Serve)) {
            Ok(connector) => connector,
            Err(refused) => return refused,
        };
    let pool = Pool::new(connector, settings.pool, settings.connect_timeout);
    match pool.get().await {
        Ok(lease) => StorageError(stopped_after(&lease.server().to_string())),
        Err(refused) => StorageError(format!("catalog.postgres: {refused}")),
    }
}

/// What the preview says once it has reached and checked `server`: where it
/// connected, why it stops, and how to serve again. The way back is to
/// remove both tables: `backend = "local"` beside a `[catalog.postgres]`
/// table is refused by the configuration.
fn stopped_after(server: &str) -> String {
    format!(
        "catalog.postgres: connected to {server}; this release does not keep the catalogs in \
         PostgreSQL yet (a preview), so it stops here: remove the [catalog] and \
         [catalog.postgres] tables to serve"
    )
}
