//! The PostgreSQL registry's tests: the text its transactions begin with
//! and the bytes its row keys are made of, which need no server, and on a
//! server the registry conformance suite and what only this backend has
//! (sealed columns, the writer's lock, a transaction a failed write ends).

use super::super::catalog::{Begin, REGISTRY_LOCK, begin_text, registry_begin_text};
use super::super::registry::{identity_bytes, identity_of, membership_key, setting_key};
use super::super::schema::{COMMANDS_LOCK, SERVERS_LOCK};
use crate::envelope::REGISTRY;
use crate::ids::{TenantId, UserId};

// What must work.

#[test]
fn a_registry_write_begins_read_committed_with_the_marker_and_the_lock() {
    let text = registry_begin_text(Begin::Write);
    assert!(
        text.starts_with("BEGIN ISOLATION LEVEL READ COMMITTED; "),
        "{text}"
    );
    assert!(
        text.contains("SET LOCAL vcon.tenant = 'registry'"),
        "{text}"
    );
    assert!(
        text.ends_with(&format!("SELECT pg_advisory_xact_lock({REGISTRY_LOCK})")),
        "the lock is taken last, in the transaction, and its end gives it up: {text}"
    );
}

#[test]
fn a_registry_read_begins_repeatable_read_read_only_with_the_marker() {
    let text = registry_begin_text(Begin::Read);
    assert_eq!(
        text, "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL vcon.tenant = 'registry'",
        "one consistent state, no write, and no lock"
    );
}

#[test]
fn an_identity_reads_back_from_its_bytes() {
    let bytes = identity_bytes("https://idp.example.com", "subject-1");
    assert_eq!(
        identity_of(&bytes),
        Some(("https://idp.example.com".to_owned(), "subject-1".to_owned())),
        "the issuer and the subject as they were written"
    );
}

#[test]
fn a_setting_key_is_the_tenant_then_the_name() {
    let key = setting_key(TenantId([7; 16]), "purge.grace");
    assert_eq!(&key[..16], [7; 16], "the tenant's 16 bytes first");
    assert_eq!(&key[16..], b"purge.grace", "then the name");
}

// What must be refused.

#[test]
fn identity_bytes_cut_short_do_not_read() {
    let bytes = identity_bytes("issuer", "subject");
    assert_eq!(identity_of(&bytes[..3]), None, "less than the length");
    assert_eq!(identity_of(&bytes[..6]), None, "less than the issuer");
    assert_eq!(identity_of(&[]), None, "no bytes");
}

#[test]
fn identity_bytes_that_are_not_text_do_not_read() {
    assert_eq!(identity_of(&[0, 0, 0, 1, 0xff, b's']), None, "the issuer");
    assert_eq!(identity_of(&[0, 0, 0, 1, b'i', 0xff]), None, "the subject");
}

#[test]
fn identity_bytes_with_a_length_past_their_end_do_not_read() {
    assert_eq!(
        identity_of(&[0xff, 0xff, 0xff, 0xff, b'a']),
        None,
        "a length of 4 GiB over one byte"
    );
}

#[test]
fn no_registry_begin_names_a_tenant_or_its_lock_row() {
    for how in [Begin::Write, Begin::Read, Begin::Create] {
        let text = registry_begin_text(how);
        assert!(!text.contains("tenant_lock"), "{how:?}: {text}");
        assert!(!text.contains(&REGISTRY.to_string()), "{how:?}: {text}");
    }
    let tenant = begin_text(Begin::Write, TenantId([0xab; 16]));
    assert!(
        !tenant.contains("registry") && !tenant.contains(&REGISTRY_LOCK.to_string()),
        "a tenant's write takes neither the marker nor the registry's lock: {tenant}"
    );
}

// Limits and odd inputs.

#[test]
fn the_registry_lock_is_not_a_servers_or_a_commands_lock() {
    assert_ne!(
        REGISTRY_LOCK, SERVERS_LOCK,
        "a server holds its lock for its life"
    );
    assert_ne!(REGISTRY_LOCK, COMMANDS_LOCK, "so does a command");
}

#[test]
fn two_identities_never_share_their_bytes() {
    assert_ne!(
        identity_bytes("ab", "c"),
        identity_bytes("a", "bc"),
        "the issuer's length keeps the two parts apart"
    );
    assert_ne!(
        identity_bytes("", "a"),
        identity_bytes("a", ""),
        "an empty part"
    );
}

#[test]
fn an_empty_identity_reads_back_as_two_empty_texts() {
    assert_eq!(
        identity_of(&identity_bytes("", "")),
        Some((String::new(), String::new())),
        "four bytes of length and nothing after"
    );
}

#[test]
fn an_operators_membership_key_is_the_user_alone() {
    let user = UserId([3; 16]);
    assert_eq!(membership_key(user, None), [3; 16], "16 bytes");
    let in_tenant = membership_key(user, Some(TenantId([0; 16])));
    assert_eq!(
        in_tenant.len(),
        32,
        "the tenant's 16 bytes follow the user's"
    );
    assert_ne!(
        membership_key(user, None),
        in_tenant,
        "a membership in the tenant of all zeros is not the operator's"
    );
}

#[test]
fn a_write_that_makes_begins_as_a_write() {
    assert_eq!(
        registry_begin_text(Begin::Create),
        registry_begin_text(Begin::Write),
        "the registry has no row to make before its first write"
    );
}

#[cfg(feature = "backend-tests")]
pub(super) mod on_a_server {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::super::super::catalog::{CatalogError, Database, PgCatalog, REGISTRY_LOCK};
    use super::super::super::registry::PgRegistry;
    use super::super::catalog_tests::on_a_server::database;
    use super::super::schema_tests::live::{Db, in_scope, waiters};
    use crate::audit::{Action, Actor, Entry};
    use crate::backend::registry_conformance::{Fresh, registry_conformance};
    use crate::backend::{CommitError, KeyRow, Registry, TenantRow, UserRow};
    use crate::envelope::{Key, REGISTRY};
    use crate::ids::{KeyId, TenantId, UserId};
    use crate::tenants::TenantError;

    const T0: i64 = 1_792_756_800;

    /// A registry in a migrated database of the test's own, under a new
    /// key.
    fn registry() -> (Db, Arc<Database>, PgRegistry, Key) {
        let (db, database) = database();
        let key = Key::generate().expect("a key");
        let registry = PgRegistry::open(database.clone(), &key).expect("the registry opens");
        (db, database, registry, key)
    }

    fn fresh() -> Fresh {
        let (db, _database, registry, _key) = registry();
        Fresh {
            registry: Box::new(registry),
            _keep: Box::new(db),
        }
    }

    registry_conformance!(fresh);

    fn tenant(n: u8, name: &str) -> TenantRow {
        TenantRow {
            id: TenantId([n; 16]),
            name: name.to_owned(),
            retention: Some("P30D-retention".to_owned()),
            metadata: Some("metadata-level".to_owned()),
            role_cap: Some("role-cap-text".to_owned()),
            created_at: T0,
            tenant_key_id: KeyId([n; 8]),
            wrapped: vec![n; 48],
            pending: None,
            key_since: T0,
        }
    }

    fn key(n: u8, label: &str) -> KeyRow {
        KeyRow {
            id: KeyId([n; 8]),
            tenant: None,
            token_hash: vec![n; 32],
            scopes: "scopes-of-the-key".to_owned(),
            label: label.to_owned(),
            created_at: T0,
            revoked_at: None,
            expires_at: None,
            last_used_at: None,
        }
    }

    fn write<T>(r: &PgRegistry, f: impl FnOnce(&mut dyn crate::backend::RegistryTx) -> T) -> T {
        let mut tx = r.immediate().expect("a write begins");
        let out = f(tx.as_mut());
        tx.commit().expect("the write commits");
        out
    }

    /// Runs `call` on a runtime of its own, for a test's own connection.
    pub(crate) fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// Runs `sql` as the schema's owner in the registry's scope, around
    /// the application role, and gives every row's columns as the server
    /// writes them, joined by a space.
    pub(crate) fn as_owner(db: &Db, sql: &str) -> Vec<String> {
        wait(async {
            let admin = db.admin().await;
            admin
                .batch_execute("SET ROLE vcon_owner")
                .await
                .expect("the owner role");
            in_scope(&admin, "registry", sql)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
                .iter()
                .filter_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(row) => Some(
                        (0..row.len())
                            .map(|i| row.get(i).unwrap_or("").to_owned())
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    _ => None,
                })
                .collect()
        })
    }

    // What must work.

    #[test]
    fn the_database_holds_no_registry_text_in_the_clear() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "tenant-name-text"))
                .expect("inserts");
            tx.insert_key(&key(1, "label-of-the-key")).expect("inserts");
            tx.insert_user(&UserRow {
                id: UserId([1; 16]),
                label: "label-of-the-user".to_owned(),
                identity: Some(("issuer-text".to_owned(), "subject-text".to_owned())),
                created_at: T0,
                deactivated_at: None,
            })
            .expect("inserts");
            tx.insert_membership(&crate::backend::MembershipRow {
                user: UserId([1; 16]),
                tenant: None,
                roles: "roles-text".to_owned(),
                granted_at: T0,
            })
            .expect("inserts");
            tx.insert_session(&crate::backend::SessionRow {
                id_hash: vec![1; 32],
                user: UserId([1; 16]),
                tenant: None,
                scopes: "scopes-of-the-session".to_owned(),
                created_at: T0,
                last_seen_at: T0,
                idle_expires_at: T0 + 9,
                expires_at: T0 + 9,
                idle_seconds: 9,
                amr: "amr-text".to_owned(),
                csrf_hash: vec![2; 32],
            })
            .expect("inserts");
            tx.put_setting(TenantId([1; 16]), "name", "setting-value-text")
                .expect("puts");
            tx.record(
                &Entry {
                    actor: Actor::Cli,
                    action: Action::Create,
                    target: Some("target-text".to_owned()),
                    version: None,
                    status: 201,
                },
                time::OffsetDateTime::from_unix_timestamp(T0).expect("a time"),
            )
            .expect("records");
        });
        // Every column of every registry table, as text: bytea columns in
        // PostgreSQL's hex, so each word is looked for in both forms.
        let mut held = String::new();
        for table in [
            "tenant",
            "api_key",
            "user_account",
            "membership",
            "session",
            "tenant_setting",
            "admin_audit",
        ] {
            let rows = as_owner(&db, &format!("SELECT t::text FROM vcon.{table} t"));
            assert_eq!(rows.len(), 1, "{table} holds the row written");
            held.push_str(&rows.join("\n"));
        }
        for word in [
            "tenant-name-text",
            "P30D-retention",
            "metadata-level",
            "role-cap-text",
            "scopes-of-the-key",
            "label-of-the-key",
            "label-of-the-user",
            "issuer-text",
            "subject-text",
            "roles-text",
            "scopes-of-the-session",
            "amr-text",
            "setting-value-text",
            "target-text",
        ] {
            let hex: String = word.bytes().map(|b| format!("{b:02x}")).collect();
            assert!(!held.contains(word), "`{word}` is in a row as text");
            assert!(!held.contains(&hex), "`{word}` is in a row as bytes");
        }
    }

    #[test]
    fn a_second_registry_on_the_database_reads_what_the_first_wrote() {
        let (_db, database, registry, key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
        let second = PgRegistry::open(database, &key).expect("opens under the same key");
        let found = second
            .snapshot()
            .expect("a read begins")
            .tenant(TenantId([1; 16]))
            .expect("reads");
        assert_eq!(
            found,
            Some(tenant(1, "alpha")),
            "as another instance would read it"
        );
    }

    #[test]
    fn a_commit_after_only_reads_succeeds_and_frees_the_connection() {
        let (_db, database, registry, _key) = registry();
        // A first transaction, so the pool holds the connection it made.
        write(&registry, |tx| drop(tx.tenants().expect("lists")));
        let idle = database.idle();
        assert_eq!(idle, 1, "the first transaction's connection waits unused");
        let mut tx = registry.immediate().expect("a write begins");
        assert!(tx.tenants().expect("lists").is_empty(), "an empty registry");
        assert_eq!(database.idle(), 0, "the transaction holds the connection");
        assert_eq!(tx.commit(), Ok(()), "the commit");
        assert_eq!(database.idle(), idle, "the connection is back in the pool");
        let again = registry.immediate().expect("the lock was given up");
        assert_eq!(again.commit(), Ok(()), "a second write begins and ends");
    }

    // What must be refused.

    #[test]
    fn a_copy_into_a_file_is_refused_and_writes_nothing() {
        let (_db, _database, registry, _key) = registry();
        let dir = tempfile::tempdir().expect("a directory");
        let to = dir.path().join("server.sqlite");
        assert_eq!(
            registry.backup_into(&to),
            Err(TenantError::NoFileBackup),
            "this backend's backup is the database's own"
        );
        let left = std::fs::read_dir(dir.path()).expect("reads").count();
        assert_eq!(left, 0, "no file was made");
    }

    #[test]
    fn a_sealed_value_moved_to_another_row_does_not_open() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_key(&key(1, "first")).expect("inserts");
            tx.insert_key(&key(2, "second")).expect("inserts");
        });
        as_owner(
            &db,
            "UPDATE vcon.api_key SET label_sealed = \
             (SELECT label_sealed FROM vcon.api_key WHERE id = '\\x0202020202020202') \
             WHERE id = '\\x0101010101010101'",
        );
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.key(KeyId([1; 8])),
            Err(TenantError::Io),
            "a label sealed for another key's row"
        );
        assert_eq!(
            tx.key(KeyId([2; 8])).expect("reads").map(|k| k.label),
            Some("second".to_owned()),
            "the row it was sealed for still opens"
        );
    }

    #[test]
    fn a_failed_write_ends_its_transaction() {
        let (_db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
        let mut tx = registry.immediate().expect("a write begins");
        tx.insert_tenant(&tenant(2, "beta")).expect("inserts");
        assert_eq!(
            tx.insert_tenant(&tenant(3, "alpha")),
            Err(TenantError::Io),
            "a taken name"
        );
        assert_eq!(
            tx.insert_tenant(&tenant(4, "gamma")),
            Err(TenantError::Io),
            "the server refuses what follows"
        );
        assert_eq!(
            tx.commit(),
            Err(CommitError::RolledBack),
            "and keeps nothing"
        );
        let names: Vec<String> = registry
            .snapshot()
            .expect("a read begins")
            .tenants()
            .expect("lists")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(
            names,
            ["alpha"],
            "the write before the failure went with it"
        );
    }

    #[test]
    fn a_snapshot_refuses_a_write() {
        let (_db, _database, registry, _key) = registry();
        let mut tx = registry.snapshot().expect("a read begins");
        assert_eq!(
            tx.insert_tenant(&tenant(1, "alpha")),
            Err(TenantError::Io),
            "the transaction is read only"
        );
        drop(tx);
        let found = registry
            .snapshot()
            .expect("a read begins")
            .tenants()
            .expect("lists");
        assert!(found.is_empty(), "nothing was written");
    }

    #[test]
    fn no_tenant_catalog_is_made_for_the_registrys_id() {
        let (_db, database, _registry, _key) = registry();
        let key = Key::generate().expect("a key");
        let made = PgCatalog::create(database.clone(), REGISTRY, &key).map(drop);
        assert_eq!(
            made,
            Err(CatalogError::NoTenant),
            "the id of all zeros is no tenant's"
        );
        let opened = PgCatalog::open(database, REGISTRY, &key).map(drop);
        assert_eq!(opened, Err(CatalogError::NoTenant), "nor does one open");
    }

    // Limits and odd inputs.

    #[test]
    fn a_second_write_waits_for_the_first_to_end() {
        let (_db, _database, registry, _key) = registry();
        let registry = Arc::new(registry);
        let mut first = registry.immediate().expect("a write begins");
        first.insert_tenant(&tenant(1, "alpha")).expect("inserts");
        let (done, second_done) = mpsc::channel();
        let other = registry.clone();
        let second = std::thread::spawn(move || {
            let mut tx = other.immediate().expect("a write begins");
            let seen = tx.tenants().expect("lists").len();
            tx.insert_tenant(&tenant(2, "beta")).expect("inserts");
            tx.commit().expect("commits");
            done.send(seen).expect("the test waits");
        });
        assert_eq!(
            second_done.recv_timeout(Duration::from_millis(400)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "the second write has not read while the first holds the lock"
        );
        first.commit().expect("the first commits");
        let seen = second_done
            .recv_timeout(Duration::from_secs(30))
            .expect("the second write ends once the first has");
        second.join().expect("the second thread");
        assert_eq!(seen, 1, "and it read the first write's row");
    }

    #[test]
    fn a_read_does_not_wait_for_a_write() {
        let (_db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
        let mut writing = registry.immediate().expect("a write begins");
        writing.insert_tenant(&tenant(2, "beta")).expect("inserts");
        let seen = registry
            .snapshot()
            .expect("a read begins")
            .tenants()
            .expect("lists")
            .len();
        assert_eq!(
            seen, 1,
            "the read sees the committed row and not the open write's"
        );
        writing.commit().expect("commits");
    }

    #[test]
    fn a_dropped_write_gives_up_the_lock() {
        let (_db, database, registry, _key) = registry();
        // A first transaction, so the pool holds the connection it made.
        write(&registry, |tx| drop(tx.tenants().expect("lists")));
        let idle = database.idle();
        assert_eq!(idle, 1, "the first transaction's connection waits unused");
        {
            let mut tx = registry.immediate().expect("a write begins");
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
        }
        assert_eq!(database.idle(), idle, "the connection is back in the pool");
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
    }

    #[test]
    fn the_same_text_is_sealed_differently_in_each_row() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_key(&key(1, "same")).expect("inserts");
            tx.insert_key(&key(2, "same")).expect("inserts");
        });
        let sealed = as_owner(&db, "SELECT label_sealed FROM vcon.api_key ORDER BY id");
        assert_eq!(sealed.len(), 2, "two rows");
        assert_ne!(sealed[0], sealed[1], "equal labels do not show as equal");
    }

    #[test]
    fn settings_are_in_byte_order_whatever_the_databases_collation() {
        let (db, _database, registry, _key) = registry();
        // The test servers' databases compare text by its bytes already;
        // a database made with a language's collation does not. The
        // column is given ICU's root collation here, which puts `audit`
        // before `Zeta`, as such a database would.
        wait(async {
            db.admin()
                .await
                .batch_execute(
                    "ALTER TABLE vcon.tenant_setting \
                     ALTER COLUMN name TYPE text COLLATE \"und-x-icu\"",
                )
                .await
                .expect("the column takes the collation");
        });
        let id = TenantId([1; 16]);
        write(&registry, |tx| {
            for name in ["audit", "Zeta", "beta"] {
                tx.put_setting(id, name, "1").expect("puts");
            }
        });
        let names: Vec<String> = registry
            .snapshot()
            .expect("a read begins")
            .settings(id)
            .expect("lists")
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            ["Zeta", "audit", "beta"],
            "as the local registry lists them"
        );
    }

    #[test]
    fn a_tenants_transaction_reads_no_registry_row() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
        let refused = wait(async {
            let admin = db.admin().await;
            admin
                .batch_execute("SET ROLE vcon_owner")
                .await
                .expect("the owner role, which the policies hold as they hold a server's");
            in_scope(
                &admin,
                &TenantId([1; 16]).to_string(),
                "SELECT id FROM vcon.tenant",
            )
            .await
        });
        let why = refused.expect_err("the policy refuses a tenant's scope");
        assert!(why.contains("not the registry's"), "{why}");
    }

    // The key the registry is under, and the key of its log (schema
    // version 12).

    pub(crate) fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The id the database names for the registry's key, in hex: one
    /// text for each row of the table.
    pub(crate) fn named(db: &Db) -> Vec<String> {
        as_owner(
            db,
            "SELECT encode(catalog_key_id, 'hex') FROM vcon.registry_key",
        )
    }

    /// Each key of the registry's log, oldest first: its ordinal, its id
    /// and its sealed copy, in hex.
    pub(crate) fn log_keys(db: &Db) -> Vec<String> {
        as_owner(
            db,
            "SELECT ordinal, encode(key_id, 'hex'), encode(wrapped, 'hex') \
             FROM vcon.admin_audit_key ORDER BY ordinal",
        )
    }

    /// An entry of the registry's log with a target.
    fn about(target: &str) -> Entry {
        Entry {
            actor: Actor::Cli,
            action: Action::Read,
            target: Some(target.to_owned()),
            version: None,
            status: 200,
        }
    }

    /// The registry's log as its export writes it.
    pub(crate) fn exported(registry: &PgRegistry) -> Result<String, String> {
        let mut out = Vec::new();
        registry
            .export(0, &mut out)
            .map(|()| String::from_utf8(out).expect("text"))
            .map_err(|e| format!("{e:?}"))
    }

    /// A record written around the registry, as the owner: with a target
    /// or none, naming the key `id` (hex) or none.
    fn raw_record(db: &Db, target: bool, id: Option<&str>) {
        let (mac, sealed) = if target {
            ("'\\x01'", "'\\x02'")
        } else {
            ("NULL", "NULL")
        };
        let id = id.map_or("NULL".to_owned(), |id| format!("'\\x{id}'"));
        as_owner(
            db,
            &format!(
                "INSERT INTO vcon.admin_audit \
                     (at, actor, action, target_mac, target_sealed, status, count, counted, key_id) \
                 VALUES ('2026-10-24T00:00:00Z', 'cli', 'read', {mac}, {sealed}, 200, 1, false, {id})"
            ),
        );
    }

    /// Leaves the registry's rows as a registry of before schema version
    /// 12 would have left them: the database names no key, and the log
    /// has none.
    fn before_version_12(db: &Db) {
        as_owner(db, "DELETE FROM vcon.registry_key");
        as_owner(db, "DELETE FROM vcon.admin_audit_key");
    }

    #[test]
    fn a_first_open_over_rows_of_before_version_12_names_the_key_and_makes_the_log_s() {
        let (db, database, registry, key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
            tx.insert_key(&self::key(1, "label")).expect("inserts");
        });
        drop(registry);
        before_version_12(&db);
        let again = PgRegistry::open(database, &key).expect("the rows open under this key");
        assert_eq!(named(&db), [hex(&key.id().0)], "the key is named");
        assert_eq!(log_keys(&db).len(), 1, "and the log has its first key");
        write(&again, |tx| {
            tx.record(&about("the-target"), minute()).expect("records")
        });
        assert!(
            exported(&again)
                .expect("exports")
                .contains("\"target\":\"the-target\""),
            "which seals and opens a record"
        );
    }

    #[test]
    fn the_first_open_names_the_key_the_registry_is_under() {
        let (db, _database, _registry, key) = registry();
        assert_eq!(
            named(&db),
            [hex(&key.id().0)],
            "one row: the id of the key it was opened under"
        );
    }

    #[test]
    fn the_first_open_makes_the_key_of_the_log() {
        let (db, _database, _registry, key) = registry();
        let keys = log_keys(&db);
        assert_eq!(keys.len(), 1, "one key: {keys:?}");
        let fields: Vec<&str> = keys[0].split(' ').collect();
        assert_eq!(fields[0], "1", "the first ordinal");
        assert_eq!(fields[1].len(), 16, "an id of 8 bytes: {}", fields[1]);
        assert_ne!(
            fields[1],
            hex(&key.id().0),
            "a key of its own, not the registry's"
        );
        assert!(
            fields[2].len() > 64,
            "the key sealed: more than its 32 bytes: {}",
            fields[2]
        );
    }

    #[test]
    fn a_second_open_under_the_same_key_writes_nothing() {
        let (db, database, _registry, key) = registry();
        let (id, keys) = (named(&db), log_keys(&db));
        let rows = as_owner(&db, "SELECT xmin FROM vcon.registry_key");
        drop(PgRegistry::open(database, &key).expect("opens under the same key"));
        assert_eq!(named(&db), id, "the id is as it was");
        assert_eq!(
            log_keys(&db),
            keys,
            "no second key, and the first as sealed"
        );
        assert_eq!(
            as_owner(&db, "SELECT xmin FROM vcon.registry_key"),
            rows,
            "the row was not written again"
        );
    }

    #[test]
    fn a_record_names_the_key_of_the_log() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.record(&about("the-target"), minute()).expect("records")
        });
        let key_id = log_keys(&db)[0]
            .split(' ')
            .nth(1)
            .expect("an id")
            .to_owned();
        assert_eq!(
            as_owner(&db, "SELECT encode(key_id, 'hex') FROM vcon.admin_audit"),
            [key_id],
            "the record names the one key"
        );
    }

    #[test]
    fn a_record_s_target_reads_back_in_another_registry() {
        let (_db, database, registry, key) = registry();
        write(&registry, |tx| {
            tx.record(&about("the-target"), minute()).expect("records")
        });
        let second = PgRegistry::open(database, &key).expect("opens");
        let lines = exported(&second).expect("exports");
        assert!(
            lines.contains("\"target\":\"the-target\""),
            "another instance opens the log's key and reads the target: {lines}"
        );
    }

    #[test]
    fn a_counted_record_names_the_key_and_is_added_to() {
        let (db, _database, registry, _key) = registry();
        for _ in 0..2 {
            write(&registry, |tx| {
                tx.count(&about("counted-target"), minute())
                    .expect("counts")
            });
        }
        assert_eq!(
            as_owner(
                &db,
                "SELECT count, key_id IS NOT NULL FROM vcon.admin_audit"
            ),
            ["2 t"],
            "one record, counted twice, that names its key"
        );
    }

    #[test]
    fn a_record_with_no_target_names_the_key_too() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.record(&refused_read(), minute()).expect("records")
        });
        assert_eq!(
            as_owner(
                &db,
                "SELECT target_mac IS NULL, key_id IS NOT NULL FROM vcon.admin_audit"
            ),
            ["t t"],
            "the key that would seal a target"
        );
    }

    #[test]
    fn a_target_is_sealed_under_the_log_s_key_not_the_registry_s() {
        let (db, _database, registry, key) = registry();
        write(&registry, |tx| {
            tx.record(&about("the-target"), minute()).expect("records")
        });
        let keys = crate::envelope::ColumnKeys::derive(&key).expect("derives");
        let under_the_registry = hex(&crate::envelope::mac(&keys, b"the-target"));
        let stored = as_owner(
            &db,
            "SELECT encode(target_mac, 'hex') FROM vcon.admin_audit",
        );
        assert_eq!(stored.len(), 1, "one record");
        assert_eq!(stored[0].len(), 64, "a MAC of 32 bytes");
        assert_ne!(
            stored[0], under_the_registry,
            "a rotation of the master key changes the registry's keys and not this MAC"
        );
    }

    #[test]
    fn two_first_opens_at_once_name_one_key_and_make_one_log_key() {
        let (db, database) = database();
        let key = Key::generate().expect("a key");
        let opened: Vec<bool> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| PgRegistry::open(database.clone(), &key).is_ok()))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("joins"))
                .collect()
        });
        assert_eq!(opened, [true; 4], "each opens");
        assert_eq!(named(&db), [hex(&key.id().0)], "one id");
        assert_eq!(log_keys(&db).len(), 1, "one key of the log");
    }

    #[test]
    fn a_first_open_that_waited_for_another_s_write_compares_what_it_wrote() {
        use super::super::super::catalog::{audit_key_id, new_audit_key};
        let (db, database) = database();
        let key = Key::generate().expect("a key");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        // This connection is the other first open: it holds the
        // registry's lock, as that open's write does.
        let holder = runtime.block_on(db.app());
        runtime
            .block_on(holder.batch_execute(&format!(
                "BEGIN; SET LOCAL vcon.tenant = 'registry'; \
                 SELECT pg_advisory_xact_lock({REGISTRY_LOCK})"
            )))
            .expect("the lock");
        let (opened, waiting) = {
            let (database, key) = (database.clone(), &key);
            std::thread::scope(|scope| {
                let open = scope.spawn(move || PgRegistry::open(database, key).map(drop));
                // The open has read no id and now waits for the lock.
                let mut waiting = 0_i64;
                for _ in 0..1500 {
                    waiting = runtime
                        .block_on(holder.query_one(&waiters(Some("advisory")), &[]))
                        .expect("reads the locks")
                        .get(0);
                    if waiting == 1 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                // What the other first open writes, then its commit.
                let keys = crate::envelope::ColumnKeys::derive(key).expect("derives");
                let (log_key, wrapped) = new_audit_key(REGISTRY, &keys).expect("a key");
                runtime
                    .block_on(async {
                        holder
                            .execute(
                                "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
                                 VALUES ($1, 1, $2)",
                                &[&audit_key_id(&log_key), &wrapped],
                            )
                            .await?;
                        holder
                            .execute(
                                "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ($1)",
                                &[&key.id().0.as_slice()],
                            )
                            .await?;
                        holder.batch_execute("COMMIT").await
                    })
                    .expect("the other open's write");
                (open.join().expect("joins"), waiting)
            })
        };
        assert_eq!(waiting, 1, "the open waited for the lock");
        assert_eq!(
            opened,
            Ok(()),
            "the id the other wrote is this key's: the open goes on"
        );
        assert_eq!(named(&db), [hex(&key.id().0)], "one id");
        assert_eq!(
            log_keys(&db).len(),
            1,
            "and the one key of the log, the other's"
        );
    }

    #[test]
    fn a_first_open_that_waited_is_refused_when_the_other_wrote_another_key() {
        let (db, database) = database();
        let key = Key::generate().expect("a key");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let holder = runtime.block_on(db.app());
        runtime
            .block_on(holder.batch_execute(&format!(
                "BEGIN; SET LOCAL vcon.tenant = 'registry'; \
                 SELECT pg_advisory_xact_lock({REGISTRY_LOCK})"
            )))
            .expect("the lock");
        let opened = {
            let (database, key) = (database.clone(), &key);
            std::thread::scope(|scope| {
                let open = scope.spawn(move || PgRegistry::open(database, key).map(drop));
                for _ in 0..1500 {
                    let waiting: i64 = runtime
                        .block_on(holder.query_one(&waiters(Some("advisory")), &[]))
                        .expect("reads the locks")
                        .get(0);
                    if waiting == 1 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                runtime
                    .block_on(holder.batch_execute(
                        "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ('\\x0102030405060708'); \
                         COMMIT",
                    ))
                    .expect("the other open's write");
                open.join().expect("joins")
            })
        };
        assert_eq!(
            opened,
            Err(TenantError::WrongKey),
            "the other open named another key first"
        );
        assert_eq!(named(&db), ["0102030405060708"], "the id the other wrote");
        assert_eq!(
            log_keys(&db),
            Vec::<String>::new(),
            "the refused open made no key"
        );
    }

    #[test]
    fn a_first_open_over_rows_under_the_same_key_names_it_again() {
        let (db, database, registry, key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
        drop(registry);
        as_owner(&db, "DELETE FROM vcon.registry_key");
        let again = PgRegistry::open(database, &key).expect("the rows open under this key");
        assert_eq!(named(&db), [hex(&key.id().0)], "named again");
        assert_eq!(log_keys(&db).len(), 1, "the log keeps its one key");
        assert_eq!(
            again
                .snapshot()
                .expect("a read begins")
                .tenant(TenantId([1; 16]))
                .expect("reads"),
            Some(tenant(1, "alpha")),
            "and the rows read"
        );
    }

    // What must be refused.

    #[test]
    fn a_registry_under_another_key_is_refused_when_it_is_opened() {
        let (db, database, registry, key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
            tx.insert_key(&self::key(1, "label")).expect("inserts");
        });
        let (id, keys) = (named(&db), log_keys(&db));
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::open(database, &other).err(),
            Some(TenantError::WrongKey),
            "the database names another key"
        );
        assert_eq!(named(&db), id, "the id is as it was");
        assert_eq!(log_keys(&db), keys, "and so is the log's key");
        assert_eq!(id, [hex(&key.id().0)], "the first key's");
    }

    #[test]
    fn a_first_open_over_a_tenant_sealed_under_another_key_is_refused() {
        let (db, database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts")
        });
        before_version_12(&db);
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::open(database, &other).err(),
            Some(TenantError::WrongKey),
            "the tenant's name does not open under this key"
        );
        assert_eq!(named(&db), Vec::<String>::new(), "no id is written");
    }

    #[test]
    fn a_first_open_over_an_api_key_sealed_under_another_key_is_refused() {
        let (db, database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_key(&self::key(1, "label")).expect("inserts")
        });
        before_version_12(&db);
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::open(database, &other).err(),
            Some(TenantError::WrongKey),
            "the key's label does not open under this key"
        );
        assert_eq!(named(&db), Vec::<String>::new(), "no id is written");
    }

    #[test]
    fn a_first_open_over_a_user_sealed_under_another_key_is_refused() {
        let (db, database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_user(&UserRow {
                id: UserId([3; 16]),
                label: "a user".to_owned(),
                identity: None,
                created_at: T0,
                deactivated_at: None,
            })
            .expect("inserts")
        });
        before_version_12(&db);
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::open(database, &other).err(),
            Some(TenantError::WrongKey),
            "the user's label does not open under this key"
        );
        assert_eq!(
            log_keys(&db),
            Vec::<String>::new(),
            "and no key of the log is made"
        );
        assert_eq!(named(&db), Vec::<String>::new(), "no id is written");
    }

    #[test]
    fn a_first_open_over_a_log_key_sealed_under_another_key_is_refused() {
        let (db, database, _registry, _key) = registry();
        as_owner(&db, "DELETE FROM vcon.registry_key");
        let keys = log_keys(&db);
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgRegistry::open(database, &other).err(),
            Some(TenantError::WrongKey),
            "the log's key does not open under this key"
        );
        assert_eq!(named(&db), Vec::<String>::new(), "no id is written");
        assert_eq!(log_keys(&db), keys, "and no second key of the log");
    }

    #[test]
    fn a_log_key_that_does_not_open_refuses_the_registry() {
        let (db, database, _registry, key) = registry();
        as_owner(
            &db,
            "UPDATE vcon.admin_audit_key SET wrapped = decode(repeat('00', 60), 'hex')",
        );
        assert_eq!(
            PgRegistry::open(database, &key).err(),
            Some(TenantError::Io),
            "the newest key seals the new records: without it no record is written"
        );
    }

    #[test]
    fn a_newest_log_key_that_does_not_open_refuses_the_registry_though_an_older_opens() {
        let (db, database, _registry, key) = registry();
        as_owner(
            &db,
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
             VALUES ('\\xaa', 2, decode(repeat('00', 60), 'hex'))",
        );
        assert_eq!(
            PgRegistry::open(database, &key).err(),
            Some(TenantError::Io),
            "new records would be sealed under the older key and name it"
        );
    }

    #[test]
    fn a_registry_that_names_its_key_and_has_no_log_key_is_refused() {
        let (db, database, _registry, key) = registry();
        as_owner(&db, "DELETE FROM vcon.admin_audit_key");
        assert_eq!(
            PgRegistry::open(database, &key).err(),
            Some(TenantError::Io),
            "a log with no key takes no record"
        );
        assert_eq!(
            log_keys(&db),
            Vec::<String>::new(),
            "a later open makes none: the records before it would name no key"
        );
    }

    #[test]
    fn a_record_that_names_no_key_and_has_a_target_cannot_be_read() {
        let (db, _database, registry, _key) = registry();
        raw_record(&db, true, None);
        let why = exported(&registry).expect_err("refused");
        assert!(why.contains("an audit record cannot be read"), "{why}");
    }

    #[test]
    fn a_copy_of_a_record_that_names_no_key_cannot_be_read() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.record(&about("the-target"), minute()).expect("records")
        });
        assert!(exported(&registry).is_ok(), "the record itself reads");
        // The same sealed target, which the log's key opens, in a record
        // that names no key.
        as_owner(
            &db,
            "INSERT INTO vcon.admin_audit \
                 (at, actor, action, target_mac, target_sealed, status, count, counted) \
             SELECT at, actor, action, target_mac, target_sealed, status, count, counted \
             FROM vcon.admin_audit",
        );
        let why = exported(&registry).expect_err("refused");
        assert!(
            why.contains("an audit record cannot be read"),
            "no key is tried for a record that names none: {why}"
        );
    }

    #[test]
    fn a_record_under_a_key_that_did_not_open_cannot_be_read() {
        let (db, database, registry, key) = registry();
        drop(registry);
        // An older key, which nothing opens, before the registry's own.
        as_owner(&db, "UPDATE vcon.admin_audit_key SET ordinal = 2");
        as_owner(
            &db,
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
             VALUES ('\\xaa', 1, decode(repeat('00', 60), 'hex'))",
        );
        raw_record(&db, true, Some("aa"));
        let registry = PgRegistry::open(database, &key).expect("the newest key opens");
        let why = exported(&registry).expect_err("refused");
        assert!(why.contains("an audit record cannot be read"), "{why}");
    }

    #[test]
    fn a_target_sealed_by_another_hand_under_the_log_s_key_id_does_not_open() {
        let (db, _database, registry, _key) = registry();
        let id = log_keys(&db)[0]
            .split(' ')
            .nth(1)
            .expect("an id")
            .to_owned();
        raw_record(&db, true, Some(&id));
        let why = exported(&registry).expect_err("refused");
        assert!(
            why.contains("an audit record cannot be read"),
            "bytes the key did not seal: {why}"
        );
    }

    // The edges.

    #[test]
    fn an_id_of_another_length_is_another_key() {
        let (db, database, _registry, key) = registry();
        as_owner(&db, "UPDATE vcon.registry_key SET catalog_key_id = '\\x01'");
        assert_eq!(
            PgRegistry::open(database.clone(), &key).err(),
            Some(TenantError::WrongKey),
            "one byte"
        );
        as_owner(&db, "UPDATE vcon.registry_key SET catalog_key_id = ''");
        assert_eq!(
            PgRegistry::open(database, &key).err(),
            Some(TenantError::WrongKey),
            "no byte"
        );
    }

    #[test]
    fn a_record_with_no_target_and_no_key_reads() {
        let (db, _database, registry, _key) = registry();
        raw_record(&db, false, None);
        let lines = exported(&registry).expect("exports");
        assert!(
            lines.contains("\"target\":null"),
            "nothing was sealed, so no key is needed: {lines}"
        );
    }

    #[test]
    fn an_older_log_key_that_does_not_open_is_left_out() {
        let (db, database, registry, key) = registry();
        drop(registry);
        as_owner(&db, "UPDATE vcon.admin_audit_key SET ordinal = 2");
        as_owner(
            &db,
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
             VALUES ('\\xaa', 1, decode(repeat('00', 60), 'hex'))",
        );
        let registry = PgRegistry::open(database, &key).expect("the newest key opens");
        write(&registry, |tx| {
            tx.record(&about("the-target"), minute()).expect("records")
        });
        let lines = exported(&registry).expect("exports");
        assert!(
            lines.contains("\"target\":\"the-target\""),
            "new records are sealed under the newest key and read back: {lines}"
        );
        assert_eq!(
            as_owner(
                &db,
                "SELECT k.ordinal FROM vcon.admin_audit a JOIN vcon.admin_audit_key k USING (key_id)"
            ),
            ["2"],
            "the record names the newest key"
        );
    }

    #[test]
    fn an_ordinal_is_an_order_not_a_count() {
        let (db, database, registry, key) = registry();
        write(&registry, |tx| {
            tx.record(&about("first"), minute()).expect("records")
        });
        drop(registry);
        // The one key, at a later ordinal: the ordinals need not start
        // at 1 or follow each other.
        as_owner(&db, "UPDATE vcon.admin_audit_key SET ordinal = 7");
        let registry = PgRegistry::open(database, &key).expect("opens");
        write(&registry, |tx| {
            tx.record(&about("second"), minute()).expect("records")
        });
        let lines = exported(&registry).expect("exports");
        assert!(
            lines.contains("\"target\":\"first\"") && lines.contains("\"target\":\"second\""),
            "the record before the change and the one after it both read: {lines}"
        );
    }

    #[test]
    fn the_log_s_key_is_bound_to_its_id() {
        let (db, database, _registry, key) = registry();
        as_owner(&db, "UPDATE vcon.admin_audit_key SET key_id = '\\xaabb'");
        assert_eq!(
            PgRegistry::open(database, &key).err(),
            Some(TenantError::Io),
            "the sealed copy opens only under the id it was sealed for"
        );
    }

    #[test]
    fn a_dropped_registry_s_database_still_names_its_key() {
        let (db, database, registry, key) = registry();
        drop(registry);
        drop(database);
        assert_eq!(named(&db), [hex(&key.id().0)], "the row is the database's");
    }

    #[test]
    fn a_new_database_names_no_key_until_a_registry_opens() {
        let (db, _database) = database();
        assert_eq!(named(&db), Vec::<String>::new(), "no id");
        assert_eq!(log_keys(&db), Vec::<String>::new(), "no key of the log");
    }

    // Counting many: one statement for a record that is there (schema
    // version 9).

    /// A counted entry of the admin log.
    fn refused_read() -> Entry {
        Entry {
            actor: Actor::Cli,
            action: Action::Read,
            target: None,
            version: None,
            status: 429,
        }
    }

    /// The minute every count of these tests falls in.
    fn minute() -> time::OffsetDateTime {
        time::OffsetDateTime::from_unix_timestamp(T0).expect("a time")
    }

    /// Where each record of the admin log is in its table, and its count,
    /// in the order stored. A row's place is `(page,item)`: an INSERT into
    /// an empty table puts it at `(0,1)`, and each UPDATE of a transaction
    /// writes the row's next version at the next item, so the item says
    /// how many statements changed the row.
    fn places(db: &Db) -> Vec<String> {
        as_owner(db, "SELECT ctid, count FROM vcon.admin_audit ORDER BY seq")
    }

    #[test]
    fn many_are_added_to_a_record_that_is_there_in_one_statement() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 3).expect("counts")
        });
        assert_eq!(
            places(&db),
            ["(0,1) 3"],
            "a new record takes its whole count"
        );
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 5).expect("counts")
        });
        assert_eq!(
            places(&db),
            ["(0,2) 8"],
            "five more, and the record was written once more"
        );
    }

    #[test]
    fn a_thousand_are_added_in_one_statement_as_five_are() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 1).expect("counts")
        });
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 1000)
                .expect("counts")
        });
        assert_eq!(
            places(&db),
            ["(0,2) 1001"],
            "the statements do not grow with the sum"
        );
    }

    #[test]
    fn one_more_is_one_statement() {
        let (db, _database, registry, _key) = registry();
        for _ in 0..2 {
            write(&registry, |tx| {
                tx.count(&refused_read(), minute()).expect("counts")
            });
        }
        assert_eq!(places(&db), ["(0,2) 2"], "an INSERT, then one UPDATE");
    }

    #[test]
    fn two_sums_in_one_write_are_two_statements() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 2).expect("counts")
        });
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 30)
                .expect("counts");
            tx.count_many(&refused_read(), minute(), 400)
                .expect("counts");
        });
        assert_eq!(
            places(&db),
            ["(0,3) 432"],
            "each call writes the record once"
        );
    }

    #[test]
    fn a_sum_of_nothing_writes_nothing_to_a_record_that_is_there() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 7).expect("counts")
        });
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 0).expect("counts")
        });
        assert_eq!(places(&db), ["(0,1) 7"], "no statement at all");
    }

    #[test]
    fn a_sum_past_the_largest_count_is_an_error_and_the_count_stays() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 7).expect("counts")
        });
        let mut tx = registry.immediate().expect("a write begins");
        let e = tx
            .count_many(&refused_read(), minute(), u64::MAX)
            .expect_err("seven and the largest number");
        assert!(
            format!("{e:?}").contains("bigint out of range"),
            "the server's reason: {e:?}"
        );
        assert_eq!(
            tx.commit(),
            Err(CommitError::RolledBack),
            "a failed write ends its transaction"
        );
        assert_eq!(places(&db), ["(0,1) 7"], "as it was");
    }

    #[test]
    fn a_record_at_the_largest_count_takes_no_more() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), u64::MAX)
                .expect("a new record takes the largest count the column holds")
        });
        let mut tx = registry.immediate().expect("a write begins");
        let e = tx
            .count_many(&refused_read(), minute(), 1)
            .expect_err("no number is larger");
        assert!(
            format!("{e:?}").contains("bigint out of range"),
            "the server's reason: {e:?}"
        );
        drop(tx);
        assert_eq!(places(&db), ["(0,1) 9223372036854775807"], "as it was");
    }

    #[test]
    fn a_sum_in_another_minute_makes_a_record_of_its_own() {
        let (db, _database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute(), 4).expect("counts")
        });
        write(&registry, |tx| {
            tx.count_many(&refused_read(), minute() + Duration::from_secs(60), 9)
                .expect("counts")
        });
        assert_eq!(
            places(&db),
            ["(0,1) 4", "(0,2) 9"],
            "the first record is not written again"
        );
    }
}
