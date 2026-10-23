//! The registry of tenants: each one's id, name, retention period and
//! wrapped tenant key, kept in `server.sqlite` under a catalog key derived
//! from the master key.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::catalog::{self, CatalogError};
use crate::envelope::{self, Key};

/// The tables of server.sqlite. A key column says NOT NULL because SQLite
/// lets a PRIMARY KEY other than an INTEGER one hold NULL. Times are nanoseconds since the epoch and
/// a retention period is in seconds; a NULL retention means the server
/// default.
const TABLES: &str = "
CREATE TABLE tenant (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    retention INTEGER,
    created_at INTEGER NOT NULL,
    tenant_key_id BLOB NOT NULL,
    wrapped_tenant_key BLOB NOT NULL,
    pending_wrapped_tenant_key BLOB,
    key_since INTEGER NOT NULL
);
CREATE TABLE master_key_use (
    master_key_id BLOB PRIMARY KEY NOT NULL,
    wraps INTEGER NOT NULL,
    since INTEGER NOT NULL
);
";

/// Why a Tenants call was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantError {
    /// server.sqlite was made under another master key.
    WrongKey,
    Io,
}

/// The registry of tenants.
pub struct Tenants {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "nothing outside the tests reads a tenant yet")
    )]
    conn: Mutex<Connection>,
}

impl Tenants {
    /// Opens `<dir>/server.sqlite` under the master key's catalog key,
    /// making it if new.
    pub fn open(dir: &Path, master: Key) -> Result<Tenants, TenantError> {
        let catalog_key = envelope::catalog_key(&master).map_err(|_| TenantError::Io)?;
        let conn = catalog::open(&dir.join("server.sqlite"), &catalog_key, TABLES).map_err(
            |e| match e {
                CatalogError::WrongKey => TenantError::WrongKey,
                _ => TenantError::Io,
            },
        )?;
        Ok(Tenants {
            conn: Mutex::new(conn),
        })
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use super::*;
    use crate::catalog::testing::columns_taking_null;
    use crate::envelope::Context;
    use crate::ids::TenantId;

    const MARKER: &str = "plain-record-marker-8d21e4";

    /// A master key kept wrapped, so a test can have it more than once.
    struct Master {
        holder: Key,
        wrapped: Vec<u8>,
    }

    const CTX: Context = Context::Tenant {
        tenant: TenantId([0; 16]),
    };

    impl Master {
        fn new() -> Master {
            let holder = Key::generate().expect("key");
            let wrapped = envelope::wrap(&holder, &CTX, &Key::generate().expect("key"));
            Master { holder, wrapped }
        }

        fn key(&self) -> Key {
            envelope::unwrap(&self.holder, &CTX, &self.wrapped).expect("unwraps")
        }
    }

    #[test]
    fn server_sqlite_is_a_catalog_under_the_master_keys_catalog_key() {
        let dir = tempfile::tempdir().expect("temp dir");
        let master = Master::new();
        drop(Tenants::open(dir.path(), master.key()).expect("opens"));
        let catalog_key = envelope::catalog_key(&master.key()).expect("catalog key");
        let conn = catalog::open(&dir.path().join("server.sqlite"), &catalog_key, "")
            .expect("opens under the catalog key");
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("names");
        assert_eq!(tables, ["master_key_use", "tenant"]);
    }

    #[test]
    fn a_wrong_master_key_does_not_open_server_sqlite() {
        let dir = tempfile::tempdir().expect("temp dir");
        let master = Master::new();
        drop(Tenants::open(dir.path(), master.key()).expect("opens"));
        assert_eq!(
            Tenants::open(dir.path(), Master::new().key()).err(),
            Some(TenantError::WrongKey)
        );
        assert!(
            Tenants::open(dir.path(), master.key()).is_ok(),
            "the right key still opens it"
        );
    }

    #[test]
    fn only_retention_and_a_pending_key_may_be_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = Tenants::open(dir.path(), Master::new().key()).expect("opens");
        let conn = tenants.conn.lock().expect("lock");
        let tenant = [
            ("id", "x'11'"),
            ("name", "'a'"),
            ("retention", "86400"),
            ("created_at", "1"),
            ("tenant_key_id", "x'22'"),
            ("wrapped_tenant_key", "x'33'"),
            ("pending_wrapped_tenant_key", "x'44'"),
            ("key_since", "1"),
        ];
        let key_use = [("master_key_id", "x'55'"), ("wraps", "1"), ("since", "1")];
        let mut may_be_empty = columns_taking_null(&conn, "tenant", &tenant);
        may_be_empty.extend(columns_taking_null(&conn, "master_key_use", &key_use));
        assert_eq!(
            may_be_empty,
            ["tenant.retention", "tenant.pending_wrapped_tenant_key"]
        );
    }

    #[test]
    fn a_name_belongs_to_one_tenant() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = Tenants::open(dir.path(), Master::new().key()).expect("opens");
        let conn = tenants.conn.lock().expect("lock");
        let add = |id: u8, name: &str| {
            conn.execute(
                "INSERT INTO tenant (id, name, created_at, tenant_key_id,
                     wrapped_tenant_key, key_since)
                 VALUES (?1, ?2, 1, x'22', x'33', 1)",
                params![[id; 16], name],
            )
        };
        add(1, "a").expect("the first tenant");
        add(2, "b").expect("another name");
        assert!(add(3, "a").is_err(), "a second tenant named a");
    }

    /// Run while the connection is open, so the commit is still in the WAL.
    #[test]
    fn no_plain_record_reaches_server_sqlite_or_its_wal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = Tenants::open(dir.path(), Master::new().key()).expect("opens");
        tenants
            .conn
            .lock()
            .expect("lock")
            .execute(
                "INSERT INTO tenant (id, name, retention, created_at, tenant_key_id,
                     wrapped_tenant_key, pending_wrapped_tenant_key, key_since)
                 VALUES (?1, ?2, 86400, 1, ?3, ?4, ?4, 1)",
                params![[0x11u8; 16], MARKER, [0x22u8; 8], MARKER.as_bytes()],
            )
            .expect("a tenant row");
        tenants
            .conn
            .lock()
            .expect("lock")
            .execute(
                "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, 1, 1)",
                [MARKER.as_bytes()],
            )
            .expect("a key-use row");
        let wal = std::fs::metadata(dir.path().join("server.sqlite-wal")).expect("a WAL");
        assert!(wal.len() > 0, "the commits are in the WAL");
        for entry in std::fs::read_dir(dir.path()).expect("list") {
            let file = entry.expect("entry").path();
            let bytes = std::fs::read(&file).expect("read");
            assert!(
                !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
                "plain record in {}",
                file.display()
            );
        }
        drop(tenants);
    }
}
