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
mod on_a_server {
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use super::super::super::catalog::{CatalogError, Database, PgCatalog};
    use super::super::super::registry::PgRegistry;
    use super::super::catalog_tests::on_a_server::database;
    use super::super::schema_tests::live::{Db, in_scope};
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
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// Runs `sql` as the schema's owner in the registry's scope, around
    /// the application role, and gives every row's columns as the server
    /// writes them, joined by a space.
    fn as_owner(db: &Db, sql: &str) -> Vec<String> {
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
    fn a_registry_under_another_key_reads_no_sealed_value() {
        let (_db, database, registry, _key) = registry();
        write(&registry, |tx| {
            tx.insert_tenant(&tenant(1, "alpha")).expect("inserts");
            tx.insert_key(&key(1, "label")).expect("inserts");
        });
        let other = Key::generate().expect("a key");
        let wrong = PgRegistry::open(database, &other).expect("opens: nothing is read");
        let mut tx = wrong.snapshot().expect("a read begins");
        assert_eq!(
            tx.tenant(TenantId([1; 16])),
            Err(TenantError::Io),
            "a tenant's row"
        );
        assert_eq!(tx.tenants(), Err(TenantError::Io), "the list of tenants");
        assert_eq!(tx.key(KeyId([1; 8])), Err(TenantError::Io), "a key's row");
        assert_eq!(
            tx.name_taken("alpha"),
            Ok(false),
            "the name's HMAC under another key finds no row"
        );
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
}
