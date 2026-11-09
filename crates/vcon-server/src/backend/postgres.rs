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
//! No catalog statement runs here yet: [`preview`] connects, checks the
//! server, and refuses to go on.

mod connect;
mod pool;
mod secrets;
#[cfg(test)]
mod tests;

use crate::config::{PostgresSettings, StorageError};

use connect::Connector;
use pool::Pool;
use secrets::Secrets;

/// What a start with a PostgreSQL catalog ends with in this release: the
/// secret files read, one connection made and the server checked, then a
/// refusal that names the server reached, since the catalogs are not kept
/// there yet. Every other outcome is the refusal of the step that failed.
pub(crate) async fn preview(settings: &PostgresSettings) -> StorageError {
    let connector = match Secrets::read(settings).and_then(|s| Connector::new(settings, s)) {
        Ok(connector) => connector,
        Err(refused) => return refused,
    };
    let pool = Pool::new(connector, settings.pool, settings.connect_timeout);
    match pool.get().await {
        Ok(lease) => StorageError(format!(
            "catalog.postgres: connected to {}; this release does not keep the catalogs in \
             PostgreSQL yet (a preview), so it stops here: set catalog.backend = \"local\" \
             to serve",
            lease.server()
        )),
        Err(refused) => StorageError(format!("catalog.postgres: {refused}")),
    }
}
